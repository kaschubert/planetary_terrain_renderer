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
use bevy::prelude::*;
use bevy::ui::{ComputedNode, UiGlobalTransform};
use bevy_terrain::prelude::{OrbitalCameraController, TerrainShape};
use big_space::prelude::Grids;
use std::collections::HashMap;

use super::layout::{self, CARD_GAP, Candidate, Column, EDGE_MARGIN, Side};
use super::panel::MarkerPanel;
use super::tether::{SELECTED_WIDTH, TETHER_WIDTH};
use super::{MarkerCamera, MarkerId, PhotoMarkers, photo};
use crate::plugins::rail_editor::frame::unit_under;

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

/// The cream this plugin draws its own marks in: the blank a card shows before a photograph
/// lands on it, and the frame and ring that say which marker is selected.
///
/// One colour for both, and a constant rather than the selected marker's own. Painting the
/// selection in the marker's colour put a second thing in that colour on a marker that already
/// had one, and left a pale marker's frame invisible against a pale photograph. A constant says
/// "this one" and nothing else; cream rather than white because it sits against a photograph
/// without glaring, and because it is already the colour of a card with nothing on it.
pub(super) const CREAM: Color = Color::srgb(0.96, 0.94, 0.89);

/// The shape of a card with no photo to take a shape from. Four by three, which is what most of
/// the photographs that land on these are, so a card changes size rather than proportion when
/// one arrives.
pub(super) const EMPTY_ASPECT: f32 = 4.0 / 3.0;

/// Where the cards sit in the UI stack: under everything else, so the F5 and F10 panels are
/// never covered by a photograph. A global index rather than a local one, because the cards are
/// root nodes of their own and have no parent to be ordered within.
const CARD_LAYER: i32 = -1;

/// One marker's card. Which marker is held by id rather than by index, since an index moves when
/// a marker before it is removed and the card would then follow the wrong one.
#[derive(Component)]
pub(super) struct MarkerCard(pub(super) MarkerId);

/// Where a card is at this moment, which is what the next frame eases it away from.
#[derive(Component, Default)]
pub(super) struct CardPlace {
    at: Option<Rect>,
}

impl CardPlace {
    /// Where the card is in the viewport this frame, which the tether leaves from. None while it
    /// is put away and has no place to speak of.
    pub(super) fn rect(&self) -> Option<Rect> {
        self.at
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
                BackgroundColor(CREAM),
                // Every card has a frame, in the colour of its own tether and at the same
                // width, so a photograph down the side of the viewport can be matched to the
                // mark on the ground it belongs to without following the line. Both are set
                // each frame in place_cards; set rather than inserted and removed, so selecting
                // a marker does not cost an archetype move every time.
                //
                // No offset: Bevy gives an outline's corners a radius of the node's plus the
                // width plus the offset, so at zero the frame's inner edge is exactly
                // concentric with the card's own rounded corner, and there is nowhere along it,
                // corners included, for the terrain to show between the two.
                Outline {
                    width: Val::Px(SELECTED_WIDTH),
                    offset: Val::ZERO,
                    color: Color::NONE,
                },
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
    seasons: Res<super::season::Seasons>,
    mut view: ResMut<super::MarkerView>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut focus: Local<Option<DVec3>>,
    mut order: Local<[Vec<super::MarkerId>; 2]>,
    grids: Grids,
    camera: Query<MarkerCamera, With<OrbitalCameraController>>,
    panel: Query<(&ComputedNode, &UiGlobalTransform), With<MarkerPanel>>,
    mut cards: Query<(
        Entity,
        &mut Node,
        &mut Visibility,
        &mut Outline,
        &mut CardPlace,
    )>,
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
            // The terrain under the pointer, as the picking pass read it back, which is the
            // same hit a Ctrl+click places a marker on. None over sky, or before the readback
            // has caught up.
            camera.picking.translation.map(|translation| {
                grid.grid_position_double(
                    &camera.picking.cell,
                    &Transform::from_translation(translation),
                )
            }),
        ))
    });

    // Nothing is drawn while F10 has the cards away, and nothing before the window has told the
    // camera its size. A card put away forgets where it was, so
    // that it does not fly across the viewport from a stale place when it comes back.
    let Some((camera, global, cell_origin, camera_position, viewport, picked)) =
        seen.filter(|_| markers.showing.cards())
    else {
        for (_, _, mut visibility, _, mut place) in &mut cards {
            visibility.set_if_neq(Visibility::Hidden);
            place.at = None;
        }
        view.shown = 0;
        return;
    };

    // Where the interest is: a point on the ground, which only Shift and the pointer move.
    //
    // A point on the ground and not a point on the screen, which is what this was. A screen
    // point stays where it is while the terrain slides under it, so turning the camera swept the
    // choice across the landscape and the photographs reshuffled themselves for a gesture that
    // was never about them. Anchored to the ground, flying about changes nothing: the cards you
    // asked for stay the cards you have until you ask for others.
    //
    // Shift reads the same terrain pick a Ctrl+click places a marker on, so what it chooses is
    // the ground under the pointer. Over sky, or with the pick not yet in, the last choice
    // stands. The first one is the ground under the camera, because something has to be.
    let asking = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if let (true, Some(picked)) = (asking, picked) {
        *focus = Some(picked);
    }
    let focus = *focus.get_or_insert_with(|| {
        let unit = unit_under(camera_position);

        TerrainShape::WGS84.position_unit_to_local(unit, 0.0)
    });

    // Who is even a candidate, and what the layout needs to know about each.
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut of_marker: Vec<usize> = Vec::new();
    for (index, marker) in markers.markers.iter().enumerate() {
        let Some(entity) = marker.card else {
            continue;
        };
        let Ok((_, _, _, _, place)) = cards.get(entity) else {
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
            id: marker.id,
            anchor,
            size: card_size(markers.card_pixels, marker.aspect),
            nearness: marker.at.distance(focus),
            selected: Some(index) == markers.selected,
            at: place.at.map(|at| at.center().y),
        });
        of_marker.push(index);
    }

    // The panel's footprint, in the logical pixels everything else here is in, and only while
    // it is actually on screen.
    //
    // Two traps in one line. ComputedNode is measured in physical pixels — inverse_scale_factor
    // is what Bevy hands over to convert back — while the viewport, the margins and the cursor
    // are all logical, so on a scaled window the untouched rect is both too big and too far
    // down and right to overlap anything. And Bevy lays a node out whether or not it is drawn,
    // so a panel hidden by Visibility still has a full-sized node: without the gate, the state
    // whose whole point is to hand the corner back would go on reserving it.
    let panel = markers
        .showing
        .editing()
        .then(|| panel.iter().next())
        .flatten()
        .map(|(node, transform)| {
            let scale = node.inverse_scale_factor();

            Rect::from_center_size(transform.translation * scale, node.size() * scale)
        });

    let columns = columns(viewport, markers.card_pixels, panel);
    // The order each column was in last frame, so that it only changes when the cards do.
    let (placements, settled) = layout::lay_out(&candidates, &columns, &order, time.delta_secs());
    *order = settled;

    view.shown = placements.len();

    // Keyed by the card's own entity, because what comes next has to be a loop over every card
    // there is rather than over the ones that got a place.
    //
    // A marker drops out of the candidates for three ordinary reasons: it went off the screen,
    // it went over the horizon, or a nearer marker took the last room in its column. Walking the
    // placements would visit none of those, and their cards would stay exactly where they last
    // were, at the size they last were, while the layout went on arranging the others around
    // them. That is what put a stray card across the middle of the viewport.
    let mut placed: HashMap<Entity, (&layout::Placement, usize)> = HashMap::new();
    for placement in &placements {
        let marker = of_marker[placement.index];
        if let Some(entity) = markers.markers[marker].card {
            placed.insert(entity, (placement, marker));
        }
    }

    for (entity, mut node, mut visibility, mut outline, mut place) in &mut cards {
        let Some(&(placement, marker)) = placed.get(&entity) else {
            visibility.set_if_neq(Visibility::Hidden);
            place.at = None;
            continue;
        };

        // Already eased, and already swept clear of its neighbours at that eased height: what
        // the layout hands back is where to draw this frame, not where it is headed.
        let size = placement.rect.size();
        place.at = Some(placement.rect);

        node.width = Val::Px(size.x);
        node.height = Val::Px(size.y);
        node.left = Val::Px(placement.rect.min.x);
        node.top = Val::Px(placement.rect.min.y);
        node.border_radius = BorderRadius::all(Val::Px(corner_radius(markers.corner_radius, size)));

        // The frame matches this marker's tether in both colour and weight, and gives both up
        // for cream and the slider's width while it is the selected one.
        let (width, colour) = match markers.selected == Some(marker) {
            true => (markers.selected_width, CREAM),
            false => (TETHER_WIDTH, seasons.colour_of(&markers.markers[marker])),
        };
        outline.width = Val::Px(width);
        outline.color = colour;
        visibility.set_if_neq(Visibility::Visible);
    }
}

/// The two columns the cards stack in: inset from the edges of the viewport, and on the right
/// stopping above the F10 panel rather than running under it.
pub(super) fn columns(viewport: Vec2, widest: f32, panel: Option<Rect>) -> [Column; 2] {
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
    if let Some(panel) = panel {
        let span = column_span(&right, widest);

        if panel.max.x >= span.0 && panel.min.x <= span.1 {
            right.bottom = right.bottom.min(panel.min.y - CARD_GAP);
        }
    }

    [left, right]
}

/// The horizontal stretch a column's cards can cover, taking the widest card they could be,
/// which is what the panel's corner is measured against.
fn column_span(column: &Column, widest: f32) -> (f32, f32) {
    match column.side {
        Side::Left => (column.outer, column.outer + widest),
        Side::Right => (column.outer - widest, column.outer),
    }
}

/// The marker whose card a click at this point lands on, if any.
///
/// Cards never overlap, which the layout engine guarantees, so at most one can contain a point
/// and the first found is the answer. Fifty rectangles is nothing to walk.
///
/// This rather than Bevy's `Interaction`, which would seem the obvious way round. Two reasons.
/// The cards sit at the bottom of the UI stack so that a photograph never covers a panel, and
/// `ui_focus_system` gives the press to the topmost node under the pointer and stops there, so
/// anything at all drawn over a card swallows the click meant for it. And `Interaction` presses
/// on the way down, where everything else this plugin picks is decided on the way up, through
/// the editor's click rules, so that a press and a drag are told apart.
///
/// A card does block the pointer, which is what keeps a drag across one from swinging the
/// planet under it. That blocking would also throw the selecting press away, so edit_with_mouse
/// asks this first and does not count a press over one of our own cards as the UI's.
pub(super) fn card_under(cards: &[(usize, Rect)], cursor: Vec2) -> Option<usize> {
    cards
        .iter()
        .find(|(_, rect)| rect.contains(cursor))
        .map(|&(index, _)| index)
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
