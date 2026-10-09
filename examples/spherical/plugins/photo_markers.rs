//! Markers pinned to the terrain, each holding a photo on its screen.
//!
//! A marker is a place on the ground, a colour and a photo, and nothing more. Ctrl and the left
//! button stand one where the cursor is: the picking pass already reads the terrain depth under
//! the cursor back as a position, so the hit costs nothing here, and Control says the press is
//! this plugin's rather than the camera's, which pans on the same button. The modifier is what
//! makes it safe, and it is the same one the rail editor's Ctrl chords use; the library stands
//! aside for it, in toggle_debug for the keys and in the orbital camera for the left button.
//!
//! A marker remembers where it is as a direction on the unit sphere and the terrain height under
//! it, not as a position, so its position is recomputed every frame and a saved marker comes back
//! where it was. It floats HOVER_HEIGHT above that ground, which is where its dot sits.
//!
//! A marker is drawn as a card in the viewport rather than as anything in the world: a node
//! holding the photo at its own proportions, with rounded corners, which is all the pixels a
//! photograph needs. What is left in the scene is a dot at the marker's place. The plate that
//! used to stand there spent four fifths of its pixels on a border, which is what this is for.
//!
//! F10 cycles what is shown: nothing, the photographs on their own, the photographs and the
//! panel. The dots are drawn in all three, so a marker is still a thing on the terrain with
//! everything else away, and still clickable.
//!
//! What is where: the card in card.rs, the dropped photos in photo.rs, the F10 panel in
//! panel.rs, the saved file in file.rs. Everything that places or reads a marker runs in
//! PostUpdate after the transforms propagate, as the editor's mouse does, so a click is tested
//! against where things are this frame.

use bevy::ecs::query::QueryData;
use bevy::gizmos::config::GizmoLineJoint;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::ui::UiSystems;
use bevy::window::PrimaryWindow;
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};
use std::path::PathBuf;

use super::auckland_rail::AucklandRail;
use super::rail_editor::clicks::ClickDetector;
use super::rail_editor::frame::{Frame, unit_under};
use super::sheet_grid::IN_FRONT_OF_TERRAIN;
use super::track_frames::TrackSplines;
use super::trains::{CARRIAGE_HEIGHT, Trains, carriage_frame, carriage_under};
use card::{CardPlace, EMPTY_ASPECT, MarkerCard, card_under};

mod card;
mod file;
mod layout;
mod panel;
mod photo;
mod scatter;
mod season;
mod tether;

/// How far the base of a marker floats above the terrain, in metres. Kept now that the plate has
/// gone because it is what a riding marker clears the roof of its carriage by, and what a marker
/// on the ground is anchored at: the dot sits where the plate's base used to.
pub(super) const HOVER_HEIGHT: f64 = 10.0;

/// How near a marker's dot the cursor has to be for a plain click to select it, in pixels. The
/// editor's discs use 10 for points a few pixels across, and a dot is one of those; a click on
/// the card itself is tested against the card's own rectangle instead, see card::card_under.
const PICK_PIXELS: f32 = 12.0;

/// The colour a marker takes when nothing has said otherwise: a very light grey. Held as hue,
/// saturation and value rather than as a colour, so that the panel's hue slider still means
/// something at zero saturation, where a grey has no hue to read back.
const DEFAULT_COLOUR: Hsva = Hsva::hsv(0.0, 0.0, 0.9);

/// How big a marker's dot is drawn, as a radius in pixels, and the ring round the selected one.
/// Held at a size on screen rather than in metres: a dot is a mark on the map, not a thing in
/// the world, so its radius is worked back from the distance the way the editor's discs are.
const DOT_PIXELS: f32 = 4.0;
const SELECTED_PIXELS: f32 = 9.0;

/// The selection ring's own gizmo group, so that it can be drawn in front of the terrain the way
/// the rail overlays are without the default group's settings following it.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(super) struct PhotoMarkerGizmos;

/// The selected marker's dot and tether, drawn thicker.
///
/// A group of its own because that is the only way to have two widths: a gizmo's line width is
/// one number for a whole config group, so a bold line and a thin one cannot come from the same
/// one. Everything else about it matches PhotoMarkerGizmos, the depth bias included.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(super) struct SelectedMarkerGizmos;

/// A marker's identity, which outlives its index in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerId(u32);

/// How much of the collection is actually in front of you, which is what the panel reads out and
/// what says whether the card slider has gone too big.
///
/// A resource rather than a field on the markers, for the reason the size sample was one before
/// it: this changes every frame the camera moves, and writing it onto the markers would mark
/// them edited every frame and wake everything that watches them for a real change.
#[derive(Resource, Default)]
pub struct MarkerView {
    /// Markers with a card actually drawn, which the layout decides, see card::place_cards.
    pub shown: usize,
    /// Markers in the collection, drawn or not.
    pub total: usize,
}

/// One marker: where it is, what colour it is, and what is on its screen.
#[derive(Debug, Clone)]
pub struct PhotoMarker {
    pub id: MarkerId,
    /// The direction on the unit sphere the marker stands over, which with the height below
    /// gives its position. A direction and a height rather than a position, so that a marker
    /// saved in one run comes back in the next where the terrain puts it, not where it was.
    pub unit: DVec3,
    /// The terrain height under it above the ellipsoid, in metres, from the pick that placed it.
    pub ground: f64,
    pub colour: Hsva,
    /// The file the photo on its screen came from, if any. Saved with the marker and read back
    /// at startup, which is what puts a picture on a screen again without it being dropped twice.
    pub photo: Option<PathBuf>,
    /// Set when that file would not read, so the panel can say so. Not saved: whether a file is
    /// there is a fact about the disk now, not about the marker, and a drive plugged back in
    /// should not have to argue with the file about it.
    pub missing: bool,
    /// The carriage this marker rides, when it was placed on a train rather than on the ground.
    /// A train is named by its carriage entity, the way the chase camera follows one, because a
    /// stand-in has no name of its own and the feed's trains come and go. Not saved: an entity
    /// means nothing in the next run, so a riding marker is written down where it last rode.
    pub riding: Option<Entity>,
    /// Where the marker actually is this frame, which place_markers works out and writes. The
    /// anchor alone cannot say, since a riding marker's place is wherever its train has got to;
    /// everything that has to know where a marker is on screen reads this, as the rail lines'
    /// resolved positions are read rather than their points.
    pub at: DVec3,
    /// The photo's width over its height, which is the card's shape. A card with no photo takes
    /// EMPTY_ASPECT, so one changes size rather than proportion when a picture arrives.
    pub aspect: f32,
    /// The card's node, once spawned. A marker has no entity in the world any more: the dot is
    /// drawn by a gizmo, which owns nothing.
    pub card: Option<Entity>,
}

impl PhotoMarker {
    /// Where the marker floats when it is anchored to the ground: the terrain under it plus the
    /// hover height. A marker riding a train is placed from the train instead, see place_markers,
    /// and this is where it falls back to if the train goes.
    pub fn anchor_position(&self) -> DVec3 {
        TerrainShape::WGS84.position_unit_to_local(self.unit, self.ground + HOVER_HEIGHT)
    }

    /// Longitude and latitude in degrees, for the readout and the file: the inverse of
    /// unit_position, whose x runs the other way, so the longitude is measured against -x.
    pub fn longitude(&self) -> f64 {
        self.unit.z.atan2(-self.unit.x).to_degrees()
    }

    pub fn latitude(&self) -> f64 {
        self.unit.y.asin().to_degrees()
    }
}

/// What F10 is showing, which it cycles through on every press.
///
/// Three rather than two because looking at the photographs and editing them are different
/// things wanting different amounts of the screen: a panel in the corner is in the way of the
/// very thing it was opened to make. The middle state is not merely the panel hidden, it is not
/// editing &mdash; so Delete and Escape mean nothing here again, the corner is given back, and
/// the rail editor can be worked in while the photographs are up.
///
/// Which state you are in needs no caption: nothing on screen is Nothing, cards are Photos, and
/// cards with a panel under them are Editing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Showing {
    #[default]
    Nothing,
    /// The cards and their tethers, with nothing to edit them by.
    Photos,
    /// Those and the panel, which is the only state that claims the keys and the corner.
    Editing,
}

impl Showing {
    /// Whether the cards are drawn at all. The dots are drawn in every state, so a marker is
    /// always a thing on the terrain.
    pub fn cards(self) -> bool {
        self != Self::Nothing
    }

    /// Whether the panel is up, which is the same question as whether Delete and Escape are
    /// this plugin's and whether the rail editor has been stood down.
    pub fn editing(self) -> bool {
        self == Self::Editing
    }

    /// What a press of F10 moves to: open for work, stand back and look, put away.
    pub fn next(self) -> Self {
        match self {
            Self::Nothing => Self::Editing,
            Self::Editing => Self::Photos,
            Self::Photos => Self::Nothing,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Nothing => "off",
            Self::Photos => "photos, no panel",
            Self::Editing => "photos and the panel",
        }
    }
}

/// The markers, the selection, and the switch that shows the panel.
#[derive(Resource, Default)]
pub struct PhotoMarkers {
    /// Cycled by F10, in panel.rs. It gates the cards, the panel and the keys that destroy
    /// something, Delete and Escape, and nothing else: placing and selecting both work in every
    /// state, since neither can be done by accident, and the dots are always drawn.
    pub showing: Showing,
    pub markers: Vec<PhotoMarker>,
    /// The marker last placed or clicked, as an index into markers. A dropped photo goes to it.
    pub selected: Option<usize>,
    /// The colour the next marker takes, which the panel's sliders set while nothing is
    /// selected. Selecting a marker brings its colour here, so the sliders always read as the
    /// colour they would apply.
    pub colour: Hsva,
    /// How big every card is drawn, on its long edge, in pixels. One number for all of them
    /// rather than one each: what is being settled is how much of the viewport a photograph is
    /// worth, and the panel's slider moves this while the example runs so it can be judged by eye.
    pub card_pixels: f32,
    /// How far every card's corners are rounded, in pixels, before the clamp to half the short
    /// edge. One number for the same reason.
    pub corner_radius: f32,
    /// How heavy the selected marker is drawn, in pixels: its tether, its dot, the ring round
    /// it and the frame round its card. Everything else keeps TETHER_WIDTH.
    pub selected_width: f32,
    /// How many markers the panel's scatter button makes. A testing affordance, see scatter.rs,
    /// and the one field here that is not about what a marker is.
    pub scatter_count: f32,
    /// Edits not yet written to the file, which dims the panel's save button when there are none.
    pub dirty: bool,
    /// Set when there is a file that would not read, which blocks saving over it; see file.rs.
    pub file_unreadable: bool,
    clicks: ClickDetector,
    /// Whether Control was down when the press began, so that the release knows which it was,
    /// and whether Shift was with it, which is the scatter's chord rather than the placing's.
    chord: bool,
    shifted: bool,
    next_id: u32,
}

impl PhotoMarkers {
    /// A marker on the ground at a direction on the unit sphere, in the current colour,
    /// selected. Its card follows on the next frame, see card::spawn_cards.
    #[cfg(test)]
    pub fn place(&mut self, unit: DVec3, ground: f64) -> MarkerId {
        self.place_riding(unit, ground, None)
    }

    /// The same, riding a carriage: the marker stands over that train and goes where it goes,
    /// and the ground given is where it falls back to should the train be gone.
    pub fn place_riding(&mut self, unit: DVec3, ground: f64, riding: Option<Entity>) -> MarkerId {
        let id = MarkerId(self.next_id);
        self.next_id += 1;

        let marker = PhotoMarker {
            id,
            unit,
            ground,
            colour: self.colour,
            photo: None,
            missing: false,
            riding,
            // Overwritten by place_markers on the next frame; set here so that a marker never
            // reads as being at the centre of the earth in between.
            at: DVec3::ZERO,
            aspect: EMPTY_ASPECT,
            card: None,
        };
        self.markers.push(PhotoMarker {
            at: marker.anchor_position(),
            ..marker
        });
        self.selected = Some(self.markers.len() - 1);
        self.dirty = true;

        id
    }

    /// Selects a marker, and brings its colour to the panel with it, so that the sliders read as
    /// the colour they would apply rather than as whatever was set before. Without this a drag
    /// after a selection would start from the colour of the marker selected before it.
    pub fn select(&mut self, index: usize) {
        let Some(marker) = self.markers.get(index) else {
            return;
        };

        self.colour = marker.colour;
        self.selected = Some(index);
    }

    /// Which marker an id belongs to. A card holds its marker's id rather than its index, since
    /// an index moves when a marker before it is removed, so this is how one finds the other.
    pub fn index_of(&self, id: MarkerId) -> Option<usize> {
        self.markers.iter().position(|marker| marker.id == id)
    }

    /// The marker the panel reads out and a photo lands on.
    pub fn selected(&self) -> Option<&PhotoMarker> {
        self.selected.and_then(|index| self.markers.get(index))
    }

    pub fn selected_mut(&mut self) -> Option<&mut PhotoMarker> {
        match self.selected {
            Some(index) => self.markers.get_mut(index),
            None => None,
        }
    }

    /// Removes the selection, returning its card for despawning. The selection moves to nothing
    /// rather than to a neighbour, so nothing is edited by surprise afterwards.
    pub fn remove_selected(&mut self) -> Option<Entity> {
        let index = self.selected.take()?;
        if index >= self.markers.len() {
            return None;
        }

        let marker = self.markers.remove(index);
        self.dirty = true;

        marker.card
    }
}

/// The mouse systems, which the panel's readout orders itself after so that it reads the marker
/// clicked this frame.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhotoMarkerSystems;

pub struct PhotoMarkersPlugin;

impl Plugin for PhotoMarkersPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PhotoMarkers::load())
            .init_resource::<MarkerView>()
            .init_resource::<scatter::Scatter>()
            .insert_resource(season::Seasons::load())
            .init_gizmo_group::<PhotoMarkerGizmos>()
            .init_gizmo_group::<SelectedMarkerGizmos>()
            .add_systems(Startup, configure_marker_gizmos)
            // Every frame rather than on a change of the markers: the widths depend on the
            // window's scale factor too, and that changes when it is dragged to another screen.
            .add_systems(Update, resize_marker_gizmos)
            .add_plugins(panel::PhotoPanelPlugin)
            .add_systems(
                Update,
                (
                    card::spawn_cards.run_if(card::cards_unspawned),
                    // After the spawn, so a photo dropped on the frame a marker was placed finds
                    // the card it goes on rather than waiting for the next one.
                    (
                        photo::tint_on_hover,
                        photo::receive_drops,
                        photo::poll_photos,
                        scatter::poll_folder,
                    ),
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                // After the transforms have propagated, as the editor's mouse is: the markers
                // are projected through the camera rather than placed in the world, so
                // everything here wants the camera settled where it is this frame. And before
                // the nodes are laid out, as the stations' and the trains' labels are, or a
                // card would be drawn where it was last frame while its tether left from where
                // it is this one.
                (
                    place_markers,
                    card::place_cards,
                    (edit_with_mouse, draw_anchors, tether::draw_tethers)
                        .chain()
                        .in_set(PhotoMarkerSystems),
                )
                    .chain()
                    .after(TransformSystems::Propagate)
                    .before(UiSystems::Prepare),
            );
    }
}

/// The camera as the mouse and the placing need it: to project a marker onto the screen, to
/// shift positions into its cell, to size a dot in pixels, and to read the terrain hit under
/// the cursor.
#[derive(QueryData)]
pub(super) struct MarkerCamera {
    entity: Entity,
    camera: &'static Camera,
    global: &'static GlobalTransform,
    transform: &'static Transform,
    cell: &'static CellCoord,
    projection: &'static Projection,
    picking: &'static PickingData,
}

/// The depth bias and the joins, once. The widths are resize_marker_gizmos's, every frame,
/// because they depend on the window as well as on the panel.
///
/// The joins are what make a curve a curve. A gizmo line strip is drawn as one quad per segment,
/// each squared off across its own direction, and with `GizmoLineJoint::None` — the default, and
/// what this was leaving it at — nothing is drawn in the wedge the two quads leave open on the
/// outside of every bend. At three pixels that wedge is under a pixel and nobody sees it; at the
/// sixteen the selected marker can be set to, a sampled curve reads as a row of separate slabs.
/// Miter rather than round: the curve is a cubic cut into twenty, so each turn is a degree or
/// two, which is exactly where extending the two edges until they meet is both right and
/// cheapest. Miter spikes only at angles far sharper than anything drawn here.
fn configure_marker_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<PhotoMarkerGizmos>();
    config.depth_bias = IN_FRONT_OF_TERRAIN;
    config.line.joints = GizmoLineJoint::Miter;

    let (config, _) = store.config_mut::<SelectedMarkerGizmos>();
    config.depth_bias = IN_FRONT_OF_TERRAIN;
    config.line.joints = GizmoLineJoint::Miter;
}

/// Keeps both gizmo groups at their widths, in the pixels the renderer actually means.
///
/// A gizmo's width is in **physical** pixels. The shader adds it straight to a position measured
/// in `view.viewport.zw`, which is the physical target, and nothing multiplies it by anything on
/// the way. Everything else here is in **logical** pixels: the viewport the layout works in, the
/// card's size, the corner radius, and the `Val::Px` of the card's frame, which bevy_ui resolves
/// by multiplying by the scale factor. So the panel's numbers mean logical pixels, as the rest of
/// it does, and the scale factor is put back here.
///
/// Without that, on a screen at scale 2 a frame and a tether set to the same 6 came out 12
/// physical pixels and 6 — which is exactly what they looked like.
///
/// One width for a whole group, which is why the selected marker needs a group of its own; a
/// tether that widens where it meets the card wants a mesh of its own, see tether.rs.
fn resize_marker_gizmos(
    markers: Res<PhotoMarkers>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut store: ResMut<GizmoConfigStore>,
) {
    let Ok(window) = window.single() else {
        return;
    };
    let scale = window.scale_factor();
    let (thin, bold) = (tether::TETHER_WIDTH * scale, markers.selected_width * scale);

    // Written only when one of them has actually moved. The store is shared with every other
    // overlay's gizmos, so touching it marks it changed for all of them.
    if store.config::<PhotoMarkerGizmos>().0.line.width != thin
        || store.config::<SelectedMarkerGizmos>().0.line.width != bold
    {
        store.config_mut::<PhotoMarkerGizmos>().0.line.width = thin;
        store.config_mut::<SelectedMarkerGizmos>().0.line.width = bold;
    }
}

/// The focal length in pixels: a metre at a metre's distance covers this many pixels, as the
/// editor's discs compute it. None for a frame or two at startup, before the window has told the
/// camera its size, and then nothing that wants a size on screen is drawn.
fn focal_pixels(camera: &Camera, projection: &Projection) -> Option<f32> {
    match (camera.logical_viewport_size(), projection) {
        (Some(viewport), Projection::Perspective(perspective)) => {
            Some(viewport.y / (2.0 * (perspective.fov / 2.0).tan()))
        }
        _ => None,
    }
}

/// Works out where every marker actually is this frame and writes it onto the marker, which is
/// what the card and the dot are then drawn from.
///
/// A marker riding a train stands over the roof of its carriage wherever it has got to; one that
/// has lost its train, because the feed dropped it or F8 swapped the roster, falls back to the
/// ground it was placed over. Nothing is placed in the world any more: there is no entity to
/// carry a transform, and no rotation to turn anything towards the camera.
fn place_markers(
    mut markers: ResMut<PhotoMarkers>,
    mut view: ResMut<MarkerView>,
    trains: Res<Trains>,
    splines: Res<TrackSplines>,
) {
    // Worked out before anything is written, so that the borrow of the markers ends before they
    // are written back.
    let positions: Vec<DVec3> = markers
        .markers
        .iter()
        .map(|marker| {
            marker
                .riding
                .and_then(|carriage| riding_position(carriage, &trains, &splines))
                .unwrap_or_else(|| marker.anchor_position())
        })
        .collect();

    view.total = positions.len();

    // Through bypass_change_detection, because a riding marker moves every frame and the panel
    // would otherwise rebuild itself every frame with it.
    let markers = markers.bypass_change_detection();
    for (marker, position) in markers.markers.iter_mut().zip(positions) {
        marker.at = position;
    }
}

/// Where a marker riding a carriage stands: over the roof of the train whose carriage that is,
/// by the same hover a marker on the ground keeps over the terrain. None when the train has
/// gone, or when its line has no spline this frame, as on the frame after an edit.
fn riding_position(carriage: Entity, trains: &Trains, splines: &TrackSplines) -> Option<DVec3> {
    let train = trains
        .trains
        .iter()
        .find(|train| train.entity == Some(carriage))?;
    let spline = splines.splines.get(train.line).and_then(Option::as_ref)?;
    let frame = carriage_frame(spline, train);

    Some(frame.position + frame.up() * (CARRIAGE_HEIGHT as f64 + HOVER_HEIGHT))
}

/// Ctrl+click on the terrain places a marker, a plain click selects one, Delete removes the
/// selection and Escape clears it. The press and the release are told apart by the editor's
/// click rules, so a drag does neither: a Ctrl-drag places nothing, and a plain drag is the
/// camera's pan as it always was.
///
/// Placing and selecting are not gated on the panel being open. A photo is dropped onto whichever
/// marker is selected, so needing the panel up to choose one would be a hidden step between
/// having a marker and putting a picture on it. Delete and Escape are gated, since those lose
/// work and the rail editor wants the same two keys.
#[allow(clippy::too_many_arguments)]
fn edit_with_mouse(
    mut commands: Commands,
    mut markers: ResMut<PhotoMarkers>,
    trains: Res<Trains>,
    splines: Res<TrackSplines>,
    rail: Res<AucklandRail>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    capture: Res<PointerCapture>,
    scatter: Res<scatter::Scatter>,
    time: Res<Time<Real>>,
    window: Query<&Window, With<PrimaryWindow>>,
    grids: Grids,
    camera: Query<MarkerCamera, With<OrbitalCameraController>>,
    cards: Query<(&MarkerCard, &CardPlace, &Visibility)>,
) {
    if markers.showing.editing() {
        if keys.just_pressed(KeyCode::Escape) {
            markers.selected = None;
        }
        if keys.just_pressed(KeyCode::Delete)
            && let Some(entity) = markers.remove_selected()
        {
            commands.entity(entity).despawn();
        }
    }

    let Ok(window) = window.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let now = time.elapsed_secs_f64();

    // Where the cards are, which both the press and the release want.
    let drawn: Vec<(usize, Rect)> = cards
        .iter()
        .filter(|(_, _, visibility)| **visibility == Visibility::Visible)
        .filter_map(|(card, place, _)| Some((markers.index_of(card.0)?, place.rect()?)))
        .collect();

    if buttons.just_pressed(MouseButton::Left) {
        markers.chord = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
        markers.shifted = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

        // A press over the UI is the UI's and is no click of ours — except over one of our own
        // cards, which is UI this plugin picks for itself. The cards block the pointer so that a
        // drag across one does not swing the planet underneath it, and that same blocking would
        // otherwise throw away the press that selects one.
        let ours = card_under(&drawn, cursor).is_some();
        let blocked = capture.blocks_pointer() && !ours;
        if blocked && markers.chord {
            warn!("photo markers: a chord pressed over the interface is the interface's");
        }
        markers.clicks.press(cursor, now, blocked);
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let (chord, shifted) = (markers.chord, markers.shifted);
    let Some(click) = markers.clicks.release(cursor, now) else {
        return;
    };

    let Ok(camera) = camera.single() else {
        return;
    };
    let Some(grid) = grids.parent_grid(camera.entity) else {
        return;
    };

    // A plain click selects, whether or not the panel is open: a photo is dropped on the
    // selected marker, and wanting to drop one is no reason to have the panel up. A click that
    // lands on no marker leaves the selection alone, so nothing is lost by missing.
    if !chord {
        let cell_origin = grid.cell_to_float(camera.cell);
        let camera_position = grid.grid_position_double(camera.cell, camera.transform);

        // The cards first, since a click on a photograph can only have meant that photograph,
        // and then the dots, which is the route for a marker whose card is not drawn.
        if let Some(index) = card_under(&drawn, click.position()).or_else(|| {
            nearest_marker(
                &markers,
                camera_position,
                camera.camera,
                camera.global,
                cell_origin,
                click.position(),
            )
        }) {
            markers.select(index);
            let marker = &markers.markers[index];
            info!(
                "photo markers: marker {} of {} selected, {}",
                index + 1,
                markers.markers.len(),
                match &marker.photo {
                    Some(path) => format!(
                        "showing {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    None => "no photo yet: drop one on the window".to_string(),
                }
            );
        }
        return;
    }

    // Control and Shift together scatter the panel's count of test markers about the ground
    // clicked, rather than standing one there. The same chord the button uses, with a place to
    // put them: a button has none and takes the ground under the camera instead.
    if shifted {
        // Every way this can fail says so. A chord that quietly does nothing is indis-
        // tinguishable from one that is not bound at all, and the two want different fixes.
        let Some(translation) = camera.picking.translation else {
            warn!("photo markers: nothing under the pointer to scatter about");
            return;
        };
        if !scatter.ready() {
            warn!(
                "photo markers: no folder of photographs to scatter from; press F10 and then \
                 folder... The folder is chosen afresh each run."
            );
            return;
        }

        let around = grid.grid_position_double(
            &camera.picking.cell,
            &Transform::from_translation(translation),
        );
        let camera_position = grid.grid_position_double(camera.cell, camera.transform);
        let (_, altitude) = scatter::over_the_camera(camera_position);

        scatter::scatter_markers(&mut markers, &scatter, around, altitude, &mut commands);
        return;
    }

    // A train under the cursor takes the marker instead of the ground does, and it rides from
    // then on. Tested on screen rather than by the pick, which reads the terrain's own depth
    // and so sees straight through a carriage to the ground behind it.
    let cell_origin = grid.cell_to_float(camera.cell);
    let camera_position = grid.grid_position_double(camera.cell, camera.transform);
    let riding = trains.drawn(&rail).then(|| {
        carriage_under(
            &trains,
            &splines,
            camera.camera,
            camera.global,
            cell_origin,
            camera_position,
            click.position(),
        )
    });

    // Where the marker goes, and what it falls back to if it is riding and loses its train: the
    // carriage's own place for a train, and the terrain under the cursor otherwise.
    let anchor = match riding.flatten() {
        Some((_, position)) => position,
        None => {
            // The hit is in render space relative to the cell the readback was taken in, which
            // is not always the camera's current one, so it is that cell it is resolved against.
            let Some(translation) = camera.picking.translation else {
                return;
            };
            grid.grid_position_double(
                &camera.picking.cell,
                &Transform::from_translation(translation),
            )
        }
    };

    // The height the anchor stands at above the ellipsoid, along the direction heights run in,
    // so that anchor_position puts the marker back exactly there plus the hover.
    let unit = unit_under(anchor);
    let ground = (anchor - TerrainShape::WGS84.scale() * unit).dot(Frame::at_unit(unit).up);

    markers.place_riding(unit, ground, riding.flatten().map(|(carriage, _)| carriage));
    let marker = markers.markers.last().expect("just placed");
    match riding.flatten() {
        Some((carriage, _)) => {
            let train = trains
                .trains
                .iter()
                .find(|train| train.entity == Some(carriage));
            info!(
                "photo markers: riding the {} {}, and going where it goes",
                train.map_or("", |train| train.line_name(&rail)),
                train.map_or("train", |train| match train.id.is_empty() {
                    true => "stand-in",
                    false => train.id.as_str(),
                }),
            );
        }
        None => info!(
            "photo markers: placed at {:.5}, {:.5}, ground {:.1} m",
            marker.latitude(),
            marker.longitude(),
            marker.ground,
        ),
    }
}

/// The marker whose dot is nearest the cursor, if the cursor is within PICK_PIXELS of it.
///
/// Only the dot: a click on the card itself goes through the card's own Interaction, see
/// card::select_on_click, so this is the route for a marker whose card is not drawn, which with
/// the cards on F10 is most of the time.
fn nearest_marker(
    markers: &PhotoMarkers,
    camera_position: DVec3,
    camera: &Camera,
    camera_global: &GlobalTransform,
    cell_origin: DVec3,
    cursor: Vec2,
) -> Option<usize> {
    let mut nearest: Option<(usize, f32)> = None;

    for (index, marker) in markers.markers.iter().enumerate() {
        // Over the horizon, which a projection cannot tell on its own: a marker on the far side
        // of the planet lands on the screen perfectly well, with the globe in between.
        if (camera_position - marker.at).dot(marker.at) < 0.0 {
            continue;
        }
        let Ok(on_screen) =
            camera.world_to_viewport(camera_global, (marker.at - cell_origin).as_vec3())
        else {
            continue;
        };

        let distance = on_screen.distance(cursor);
        if distance <= PICK_PIXELS && nearest.is_none_or(|(_, best)| distance < best) {
            nearest = Some((index, distance));
        }
    }

    nearest.map(|(index, _)| index)
}

/// A dot at every marker's place, in its season's colour, and a wider ring round the selected one.
///
/// This is what a marker is when its card is not drawn, and what its tether runs to. The dot
/// takes the same colour as the tether and the card's frame, see season.rs, so that a line can be
/// followed to the right mark; the ring takes CREAM, the one colour selection is ever said in, so
/// that which marker is selected is never a question about shades.
///
/// Drawn at a size on screen rather than in metres, by working the radius back from the distance
/// and the focal length, so a dot is a dot from a kilometre up and from orbit.
fn draw_anchors(
    mut gizmos: Gizmos<PhotoMarkerGizmos>,
    mut bold: Gizmos<SelectedMarkerGizmos>,
    markers: Res<PhotoMarkers>,
    seasons: Res<season::Seasons>,
    grids: Grids,
    camera: Query<MarkerCamera, With<OrbitalCameraController>>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let Some(grid) = grids.parent_grid(camera.entity) else {
        return;
    };
    let Some(focal_pixels) = focal_pixels(camera.camera, camera.projection) else {
        return;
    };

    let cell_origin = grid.cell_to_float(camera.cell);
    let camera_position = grid.grid_position_double(camera.cell, camera.transform);

    for (index, marker) in markers.markers.iter().enumerate() {
        let at = marker.at;
        let towards = camera_position - at;
        if towards.dot(at) < 0.0 {
            continue;
        }

        // What one pixel is worth in metres out there, which is what holds the dot at a size.
        let metres = towards.length() as f32 / focal_pixels;
        let isometry = Isometry3d::new(
            (at - cell_origin).as_vec3(),
            Quat::from_rotation_arc(Vec3::Z, towards.normalize().as_vec3()),
        );

        let colour = seasons.colour_of(marker);
        if Some(index) == markers.selected {
            bold.circle(isometry, DOT_PIXELS * metres, colour);
            bold.circle(isometry, SELECTED_PIXELS * metres, card::CREAM);
        } else {
            gizmos.circle(isometry, DOT_PIXELS * metres, colour);
        }
    }
}

#[cfg(test)]
mod tests;
