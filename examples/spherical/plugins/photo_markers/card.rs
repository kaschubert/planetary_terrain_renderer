//! The card a marker's photo is drawn on: a node in the viewport rather than a quad in the world.
//!
//! The plate this replaces spent four fifths of its pixels on a border, and the photo on the
//! remaining fifth was letterboxed onto the quad's fixed shape on top of that. A card is the
//! photo and nothing else: a node the size of the photo's own proportions, corners rounded by a
//! slider, placed each frame from where its marker falls on screen.
//!
//! Nothing about a card is in the world, so nothing about it is scaled by distance, turned to
//! face the camera or lit. What is left in the scene is a dot at the marker's place, see
//! draw_anchors in the parent module: that is what a marker is when its card is not drawn, and
//! what the tether will run to once there is one.
//!
//! A card carries a cream background of its own under the image. A frameless card with no photo
//! would otherwise be nothing at all, and a marker just placed would be invisible until a file
//! landed on it; the cream is the role the dark screen quad used to play, in code rather than as
//! a mesh. Bevy's default `ImageNode` is a transparent 1×1 that the renderer skips outright, so
//! the empty case needs no branch: the image handle is simply swapped in when a photo arrives.
//!
//! The cards come up with the panel and go down with it. Two columns of photographs is a great
//! deal of viewport to spend on something that is not being edited, and every other overlay in
//! this example is already on a key: F4 the lines, F9 the stations, F5 the editor.

use bevy::image::TRANSPARENT_IMAGE_HANDLE;
use bevy::math::DVec3;
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::ui::{ComputedNode, UiGlobalTransform};
use bevy::window::PrimaryWindow;
use bevy_terrain::prelude::OrbitalCameraController;
use big_space::prelude::Grids;

use super::layout::{
    self, CARD_GAP, Candidate, Column, EDGE_MARGIN, KEEP_OUT, KEEP_OUT_LEAVE, Side,
};
use super::panel::MarkerPanel;
use super::{MarkerCamera, MarkerId, PhotoMarkers, SELECTION_COLOUR, photo};

/// How big a card is drawn, measured on its long edge, in pixels, and where the panel's slider
/// starts. The card is in screen space, so this is the size itself rather than a size in metres
/// that has to be turned into one: there is no distance crossover any more and no pixel floor.
pub(super) const CARD_PIXELS: f32 = 160.0;

/// The range the card slider covers: small enough to be a thumbnail in a column of them, large
/// enough to see what the photograph is of.
pub(super) const MIN_CARD: f32 = 60.0;
pub(super) const MAX_CARD: f32 = 480.0;

/// How far a card's corners are rounded, in pixels, and the range the panel's slider covers.
/// Applied through `corner_radius`, which holds it under half the card's short edge.
pub(super) const CORNER_RADIUS: f32 = 12.0;
pub(super) const MIN_RADIUS: f32 = 0.0;
pub(super) const MAX_RADIUS: f32 = 40.0;

/// The cream a card shows before a photo lands on it, and when the file it names will not read.
/// Light rather than dark, so an empty card reads as a blank print waiting for one rather than
/// as a screen that is off.
pub(super) const EMPTY_CARD: Color = Color::srgb(0.96, 0.94, 0.89);

/// The shape of a card with no photo to take a shape from. Four by three, which is what most of
/// the photographs that land on these are, so a card changes size rather than proportion when
/// one arrives.
pub(super) const EMPTY_ASPECT: f32 = 4.0 / 3.0;

/// How thick the selected card's ring is, and how far it stands off the card. Its colour is the
/// parent module's SELECTION_COLOUR, which the dot's ring uses too.
const OUTLINE_WIDTH: f32 = 2.0;
const OUTLINE_OFFSET: f32 = 2.0;

/// Where the cards sit in the UI stack: under everything else, so the F5 and F10 panels are
/// never covered by a photograph. A global index rather than a local one, because the cards are
/// root nodes of their own and have no parent to be ordered within.
const CARD_LAYER: i32 = -1;

/// One marker's card. Which marker is held by id rather than by index, since an index moves when
/// a marker before it is removed and the card would then follow the wrong one.
#[derive(Component)]
pub(super) struct MarkerCard(pub(super) MarkerId);

/// Where a card actually is at this moment, and which side it is docked to.
///
/// Both are what the next frame needs back. The layout answers with where a card ought to be, and
/// the card eases towards that from where it is rather than jumping; the side is remembered so
/// that a marker hovering near the middle of the viewport keeps the column it is already in.
#[derive(Component, Default)]
pub(super) struct CardPlace {
    at: Option<Vec2>,
    size: Vec2,
    side: Option<Side>,
}

impl CardPlace {
    /// Where the card is in the viewport this frame, which the tether leaves from. None while it
    /// is put away and has no place to speak of.
    pub(super) fn rect(&self) -> Option<Rect> {
        self.at.map(|at| Rect::from_corners(at, at + self.size))
    }
}

/// The width and height of a card holding a photo of this shape, in pixels: the long edge is the
/// slider's, and the short one follows the photo. A portrait photograph gets a portrait card,
/// which is the whole point of dropping the frame — there is no fixed shape left to pad onto.
pub(super) fn card_size(pixels: f32, aspect: f32) -> Vec2 {
    match aspect >= 1.0 {
        true => Vec2::new(pixels, pixels / aspect),
        false => Vec2::new(pixels * aspect, pixels),
    }
}

/// The radius a card is actually drawn with: the slider's, held under half the short edge. Above
/// that a rounded rectangle has nothing straight left between its corners, and the renderer
/// clamps it anyway; doing it here means the number the panel reads out is the one on screen.
pub(super) fn corner_radius(radius: f32, size: Vec2) -> f32 {
    radius.clamp(0.0, size.min_element() / 2.0)
}

/// Where a marker falls in the viewport, if it is in front of the camera, this side of the
/// horizon, and within the view.
///
/// The horizon test is the station labels', and it is the one thing a projection cannot answer
/// by itself: a marker on the far side of the planet projects perfectly well onto the screen,
/// and without this its card would hang there with the globe between.
pub(super) fn viewport_position(
    at: DVec3,
    camera_position: DVec3,
    camera: &Camera,
    global: &GlobalTransform,
    cell_origin: DVec3,
    viewport: Vec2,
) -> Option<Vec2> {
    if (camera_position - at).dot(at) < 0.0 {
        return None;
    }

    camera
        .world_to_viewport(global, (at - cell_origin).as_vec3())
        .ok()
        .filter(|at| at.cmpge(Vec2::ZERO).all() && at.cmple(viewport).all())
}

pub(super) fn cards_unspawned(markers: Res<PhotoMarkers>) -> bool {
    markers.markers.iter().any(|marker| marker.card.is_none())
}

/// Spawns a card for every marker that has none: the ones a click just placed, and the ones the
/// file brought back. Hidden until `place_cards` has worked out whether it belongs on screen,
/// which is the frame after this.
pub(super) fn spawn_cards(mut commands: Commands, mut markers: ResMut<PhotoMarkers>) {
    let (mut spawned, mut loading) = (0, 0);

    for marker in &mut markers.markers {
        if marker.card.is_some() {
            continue;
        }

        let card = commands
            .spawn((
                MarkerCard(marker.id),
                CardPlace::default(),
                Node {
                    position_type: PositionType::Absolute,
                    border_radius: BorderRadius::all(Val::Px(CORNER_RADIUS)),
                    ..default()
                },
                // Transparent until a photo arrives, which the renderer skips entirely, so the
                // cream below shows through without anything having to ask whether it should.
                ImageNode::default(),
                BackgroundColor(EMPTY_CARD),
                Outline {
                    width: Val::Px(OUTLINE_WIDTH),
                    offset: Val::Px(OUTLINE_OFFSET),
                    // Set rather than inserted and removed, so selecting a marker does not cost
                    // an archetype move every time.
                    color: Color::NONE,
                },
                // Clicking a card selects its marker, which Interaction reports; dragging from
                // one still pans, which IGNORE allows. The two do not fight: ui_focus_system
                // works Interaction out from the cursor and the UI stack and never looks at
                // Pickable, while the camera's PointerCapture reads the picking hover map.
                Interaction::default(),
                Pickable::IGNORE,
                GlobalZIndex(CARD_LAYER),
                Visibility::Hidden,
            ))
            .id();

        // A marker read back from the file knows which photo was on it but has no texture yet,
        // so the decode starts here, once, as the card it goes on comes into being.
        if let Some(path) = marker.photo.clone() {
            photo::start_decode(&mut commands, marker.id, path);
            loading += 1;
        }

        marker.card = Some(card);
        spawned += 1;
    }

    if spawned > 0 {
        info!(
            "photo markers: {spawned} cards at {:.0} px{}",
            markers.card_pixels,
            match loading {
                0 => String::new(),
                loading => format!(", {loading} reading their photos"),
            }
        );
    }
}

/// Lays the cards out down the sides of the viewport and moves each one towards its place.
///
/// The arranging itself is layout.rs, which is a function of its arguments and knows nothing of
/// the world. What happens here is the gathering and the applying: which markers are even
/// candidates, how much room the columns have once the F10 panel has taken its corner, and the
/// easing that keeps a card from jumping when its place changes.
#[allow(clippy::too_many_arguments)]
pub(super) fn place_cards(
    markers: Res<PhotoMarkers>,
    mut view: ResMut<super::MarkerView>,
    time: Res<Time>,
    grids: Grids,
    camera: Query<MarkerCamera, With<OrbitalCameraController>>,
    window: Query<&Window, With<PrimaryWindow>>,
    panel: Query<(&ComputedNode, &UiGlobalTransform), With<MarkerPanel>>,
    mut engaged: Local<[bool; 2]>,
    mut cards: Query<(&mut Node, &mut Visibility, &mut Outline, &mut CardPlace)>,
) {
    let seen = camera.single().ok().and_then(|camera| {
        let grid = grids.parent_grid(camera.entity)?;
        let viewport = camera.camera.logical_viewport_size()?;

        Some((
            camera.camera,
            camera.global,
            grid.cell_to_float(camera.cell),
            grid.grid_position_double(camera.cell, camera.transform),
            viewport,
        ))
    });

    // Nothing is drawn while the panel is down, which is what ties the cards to F10, and nothing
    // before the window has told the camera its size. A card put away forgets where it was, so
    // that it does not fly across the viewport from a stale place when it comes back.
    let Some((camera, global, cell_origin, camera_position, viewport)) =
        seen.filter(|_| markers.editing)
    else {
        for (_, mut visibility, _, mut place) in &mut cards {
            visibility.set_if_neq(Visibility::Hidden);
            place.at = None;
        }
        view.shown = 0;
        return;
    };

    // Who is even a candidate, and what the layout needs to know about each.
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut of_marker: Vec<usize> = Vec::new();
    for (index, marker) in markers.markers.iter().enumerate() {
        let Some(entity) = marker.card else {
            continue;
        };
        let Ok((_, _, _, place)) = cards.get(entity) else {
            continue;
        };
        let Some(anchor) = viewport_position(
            marker.at,
            camera_position,
            camera,
            global,
            cell_origin,
            viewport,
        ) else {
            continue;
        };

        candidates.push(Candidate {
            anchor,
            size: card_size(markers.card_pixels, marker.aspect),
            distance: camera_position.distance(marker.at),
            selected: Some(index) == markers.selected,
            was: place.side,
        });
        of_marker.push(index);
    }

    let columns = columns(viewport, markers.card_pixels, panel.iter().next());
    let cursor = window
        .single()
        .ok()
        .and_then(|window| window.cursor_position());

    // The circle is wider for a column it has already pushed something out of, so the pointer has
    // to retreat further to let the cards back than it took to move them. Without that a card on
    // the boundary flickers as the pointer jitters by a pixel.
    let mut keep_out = [KEEP_OUT; 2];
    for (slot, radius) in keep_out.iter_mut().enumerate() {
        if engaged[slot] {
            *radius += KEEP_OUT_LEAVE;
        }
        engaged[slot] = cursor.is_some_and(|cursor| {
            let span = column_span(&columns[slot], markers.card_pixels);
            layout::forbidden_band(cursor, *radius, span).is_some()
        });
    }

    let placements = layout::lay_out(&candidates, &columns, viewport.x / 2.0, cursor, &keep_out);

    view.shown = placements.len();

    // Everything with a placement is moved towards it; everything else is put away.
    let mut placed = vec![None; candidates.len()];
    for placement in &placements {
        placed[placement.index] = Some(placement);
    }

    let delta = time.delta_secs();
    for (candidate, marker) in of_marker.iter().copied().enumerate() {
        let Some(entity) = markers.markers[marker].card else {
            continue;
        };
        let Ok((mut node, mut visibility, mut outline, mut place)) = cards.get_mut(entity) else {
            continue;
        };
        let Some(placement) = placed[candidate] else {
            visibility.set_if_neq(Visibility::Hidden);
            place.at = None;
            continue;
        };

        let size = placement.rect.size();
        let target = placement.rect.min;
        let at = match place.at {
            Some(at) => layout::ease(at, target, delta),
            None => target,
        };
        place.at = Some(at);
        place.size = size;
        place.side = Some(placement.side);

        node.width = Val::Px(size.x);
        node.height = Val::Px(size.y);
        node.left = Val::Px(at.x);
        node.top = Val::Px(at.y);
        node.border_radius = BorderRadius::all(Val::Px(corner_radius(markers.corner_radius, size)));

        outline.color = match markers.selected == Some(marker) {
            true => SELECTION_COLOUR,
            false => Color::NONE,
        };
        visibility.set_if_neq(Visibility::Visible);
    }
}

/// The two columns the cards stack in: inset from the edges of the viewport, and on the right
/// stopping above the F10 panel rather than running under it.
fn columns(
    viewport: Vec2,
    widest: f32,
    panel: Option<(&ComputedNode, &UiGlobalTransform)>,
) -> [Column; 2] {
    let (top, bottom) = (EDGE_MARGIN, viewport.y - EDGE_MARGIN);
    let left = Column {
        side: Side::Left,
        top,
        bottom,
        outer: EDGE_MARGIN,
    };
    let mut right = Column {
        side: Side::Right,
        top,
        bottom,
        outer: viewport.x - EDGE_MARGIN,
    };

    // The panel is the one piece of furniture the viewport already has. Only a column it actually
    // overlaps is shortened, so a narrow window that puts the panel clear of the cards loses
    // nothing.
    if let Some((node, transform)) = panel {
        let size = node.size();
        let centre = transform.translation;
        let panel = Rect::from_center_size(centre, size);
        let span = column_span(&right, widest);

        if panel.max.x >= span.0 && panel.min.x <= span.1 {
            right.bottom = right.bottom.min(panel.min.y - CARD_GAP);
        }
    }

    [left, right]
}

/// The horizontal stretch a column's cards can cover, taking the widest card they could be. The
/// cursor's circle is measured against this, and so is the panel's corner.
fn column_span(column: &Column, widest: f32) -> (f32, f32) {
    match column.side {
        Side::Left => (column.outer, column.outer + widest),
        Side::Right => (column.outer - widest, column.outer),
    }
}

/// Selects a marker when its card is pressed.
///
/// On the press rather than the release, because a card does not block the pointer and a drag
/// begun on one is the camera's pan: waiting for a release that may land anywhere would mean
/// deciding afterwards whether the gesture had been a click, which the scene's own clicks need
/// and a button does not.
pub(super) fn select_on_click(
    mut markers: ResMut<PhotoMarkers>,
    cards: Query<(&MarkerCard, &Interaction), Changed<Interaction>>,
) {
    for (card, interaction) in &cards {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(index) = markers.index_of(card.0) else {
            continue;
        };

        markers.select(index);
    }
}

/// Puts a decoded photo on a card, and takes the card's shape from it.
pub(super) fn show_photo(
    card: Option<Entity>,
    image: Handle<Image>,
    images: &mut Query<&mut ImageNode>,
) {
    let Some(mut node) = card.and_then(|card| images.get_mut(card).ok()) else {
        return;
    };

    node.image = image;
}

/// Takes the photo off a card again, leaving the cream.
pub(super) fn clear_card(card: Option<Entity>, images: &mut Query<&mut ImageNode>) {
    show_photo(card, TRANSPARENT_IMAGE_HANDLE, images);
}
