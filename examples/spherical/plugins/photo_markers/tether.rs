//! The line from a card to the place on the ground it is about.
//!
//! Two pieces of geometry, both pure. Where it leaves the card: a ray from the card's centre
//! towards the anchor, intersected with the card's rounded rectangle. And the curve between: a
//! cubic that leaves along the border's outward normal and comes into the dot from above, which
//! is what makes it read as a tether rather than as a stick.
//!
//! The attach point is not on the border. It is on the card **eroded by the tether's half
//! width**, and that is the whole trick of the flare. A rounded rectangle is convex, so the flat
//! cap at the start of the ribbon is covered as soon as its two ends are, and both ends are
//! covered whenever the disc of that radius around the start fits inside the card — which is
//! direction-free, and holds for any attach angle. "A disc of radius W fits" is exactly "the
//! start lies in the card eroded by W", and eroding a rounded rectangle takes W off the half size
//! and off the radius, clamped at zero. So the same routine is called twice against different
//! numbers and there is no second piece of geometry to get wrong.
//!
//! Drawn with the gizmos the dots already use: the curve is sampled in screen space and each
//! point unprojected to a fixed distance in front of the camera, so what is drawn is a ray
//! through that pixel whatever distance is picked. Gizmo lines are one width per config group, so
//! there is no taper yet; that wants a mesh of its own, and it is a look rather than a mechanism.

use bevy::prelude::*;
use bevy_terrain::prelude::OrbitalCameraController;
use big_space::prelude::Grids;

use super::card::{CardPlace, viewport_position};
use super::{MarkerCamera, PhotoMarkerGizmos, PhotoMarkers};

/// How wide the tether is drawn, in pixels. One number for the whole gizmo group, so the dots are
/// drawn at this too; a ribbon that widens at the card and comes to a point at the dot needs a
/// mesh of its own, which is not what this is yet.
pub(super) const TETHER_WIDTH: f32 = 3.0;

/// A pixel of slack on the erosion, because the card's edge is antialiased and a cap exactly
/// tangent to the border would show as a hairline seam along it.
const BLEED: f32 = 1.0;

/// How much of the card's shorter half the tether may ever be, so that a long thin card still has
/// somewhere for the cap to hide. At the 60 px floor of the card slider this never binds.
const WIDEST: f32 = 0.4;

/// How far the cubic's handles reach, as fractions of the span between the two ends. The first
/// leaves along the border's outward normal, the second comes into the dot from above, so the
/// tether rises out of the mark on the ground and bends towards the card.
const LEAVE: f32 = 0.45;
const ARRIVE: f32 = 0.32;

/// How many straight pieces the curve is drawn in. Twenty is smooth at any size a card is drawn
/// at, and fifty markers at twenty pieces is a thousand segments, which is nothing.
const SEGMENTS: usize = 20;

/// How far in front of the camera the curve is drawn, in metres. Any distance puts a point on the
/// same pixel, since what is unprojected is a ray through it; this one is simply well inside the
/// frustum at every altitude the example flies at.
const DEPTH: f32 = 100.0;

/// The half width the tether actually gets on a card this size: its own, held under a share of
/// the card's shorter half so that the cap always has room to hide.
fn half_width(half: Vec2) -> f32 {
    (TETHER_WIDTH / 2.0).min(WIDEST * half.min_element())
}

/// Where a ray from the centre of a rounded rectangle leaves it.
///
/// The plain rectangle first; if that hit lands in a corner's box, the corner's circle instead.
/// One test covers every direction, so the attach point slides round the border as the card or
/// the anchor moves and never jumps as it crosses a corner.
pub(super) fn attach(centre: Vec2, half: Vec2, radius: f32, toward: Vec2) -> Vec2 {
    let direction = toward.try_normalize().unwrap_or(Vec2::X);
    let half = half.max(Vec2::ZERO);
    let radius = radius.clamp(0.0, half.min_element());

    let reach = |extent: f32, along: f32| match along.abs() > 1e-6 {
        true => extent / along.abs(),
        false => f32::INFINITY,
    };
    let step = reach(half.x, direction.x).min(reach(half.y, direction.y));
    let hit = centre + direction * step;

    // Inside a corner's box in both axes, so it is the corner's arc that was crossed and not an
    // edge. The arc's centre is the corner inset by the radius either way.
    let inner = half - Vec2::splat(radius);
    if (hit - centre).abs().cmple(inner).any() {
        return hit;
    }

    let corner = centre + Vec2::new(inner.x.copysign(direction.x), inner.y.copysign(direction.y));
    let offset = centre - corner;
    let b = 2.0 * offset.dot(direction);
    let c = offset.length_squared() - radius * radius;
    let step = (-b + (b * b - 4.0 * c).max(0.0).sqrt()) / 2.0;

    centre + direction * step
}

/// Where the tether starts and which way it leaves, for a card of this rectangle and radius
/// pointing at this anchor.
///
/// The start is against the card eroded by the half width, so the cap it begins with is covered;
/// the direction is the outward normal where the ribbon actually becomes visible, which is the
/// same ray against the card itself.
pub(super) fn start(card: Rect, radius: f32, anchor: Vec2) -> (Vec2, Vec2) {
    let centre = card.center();
    let half = card.size() / 2.0;
    let toward = anchor - centre;

    let seen = attach(centre, half, radius, toward);
    let inset = Vec2::splat(half_width(half) + BLEED);
    let from = attach(centre, half - inset, radius - inset.x, toward);

    // The outward normal where it shows: straight out of an edge, or out of the corner's centre
    // on an arc. Taken from the visible point rather than from the eroded one, so the tether
    // leaves the border the way the border faces there.
    let inner = half - Vec2::splat(radius.clamp(0.0, half.min_element()));
    let local = seen - centre;
    let normal = if local.abs().cmple(inner).any() {
        // An edge: whichever one it crossed.
        match (local.x.abs() - inner.x) > (local.y.abs() - inner.y) {
            true => Vec2::new(local.x.signum(), 0.0),
            false => Vec2::new(0.0, local.y.signum()),
        }
    } else {
        let corner = Vec2::new(inner.x.copysign(local.x), inner.y.copysign(local.y));
        (local - corner).try_normalize().unwrap_or(Vec2::X)
    };

    (from, normal)
}

/// The curve from the card to the anchor, sampled into points.
pub(super) fn curve(from: Vec2, normal: Vec2, anchor: Vec2) -> Vec<Vec2> {
    let span = from.distance(anchor);
    let first = from + normal * span * LEAVE;
    // Screen `y` grows downward, so the handle above the dot is the negative one.
    let second = anchor - Vec2::Y * span * ARRIVE;

    (0..=SEGMENTS)
        .map(|step| {
            let t = step as f32 / SEGMENTS as f32;
            let u = 1.0 - t;

            from * (u * u * u)
                + first * (3.0 * u * u * t)
                + second * (3.0 * u * t * t)
                + anchor * (t * t * t)
        })
        .collect()
}

/// The two ends of the flat cap the ribbon starts with, which are what must stay under the card.
/// The cap is square to the curve's first direction, which is the border's outward normal.
///
/// Nothing in the drawing needs this: the point of putting the start on the eroded card is that
/// the cap can then be taken for granted. It is here so a test can refuse to take it for granted.
#[cfg(test)]
pub(super) fn cap(from: Vec2, normal: Vec2, half: Vec2) -> [Vec2; 2] {
    let across = normal.perp() * half_width(half);

    [from + across, from - across]
}

/// The signed distance to a rounded rectangle: negative inside, and exact in the interior as well
/// as outside, which is what makes it a real containment test rather than an approximate one.
#[cfg(test)]
pub(super) fn distance_to(point: Vec2, centre: Vec2, half: Vec2, radius: f32) -> f32 {
    let radius = radius.clamp(0.0, half.min_element());
    let q = (point - centre).abs() - half + Vec2::splat(radius);

    q.max(Vec2::ZERO).length() + q.max_element().min(0.0) - radius
}

/// Draws a tether from every card on screen to the mark on the ground it is about.
///
/// Only cards: a marker with no card has nothing for a tether to come from, and its dot says
/// where it is on its own. The colour is the marker's, which is what the eight presets paint now
/// that there is no plate to paint.
pub(super) fn draw_tethers(
    mut gizmos: Gizmos<PhotoMarkerGizmos>,
    markers: Res<PhotoMarkers>,
    grids: Grids,
    camera: Query<MarkerCamera, With<OrbitalCameraController>>,
    cards: Query<(&CardPlace, &Visibility)>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let Some(grid) = grids.parent_grid(camera.entity) else {
        return;
    };
    let Some(viewport) = camera.camera.logical_viewport_size() else {
        return;
    };

    let cell_origin = grid.cell_to_float(camera.cell);
    let camera_position = grid.grid_position_double(camera.cell, camera.transform);

    for marker in &markers.markers {
        let Some(entity) = marker.card else {
            continue;
        };
        let Ok((place, visibility)) = cards.get(entity) else {
            continue;
        };
        let (Some(card), Visibility::Visible) = (place.rect(), *visibility) else {
            continue;
        };
        let Some(anchor) = viewport_position(
            marker.at,
            camera_position,
            camera.camera,
            camera.global,
            cell_origin,
            viewport,
        ) else {
            continue;
        };

        let radius = super::card::corner_radius(markers.corner_radius, card.size());
        let (from, normal) = start(card, radius, anchor);
        let points = curve(from, normal, anchor)
            .into_iter()
            .filter_map(|point| unproject(point, camera.camera, camera.global));

        gizmos.linestrip(points, Color::from(marker.colour));
    }
}

/// A point in the viewport as a point in front of the camera, in the frame the gizmos are drawn
/// in: the same one `world_to_viewport` projects from, so the two are inverses of each other.
fn unproject(point: Vec2, camera: &Camera, global: &GlobalTransform) -> Option<Vec3> {
    let ray = camera.viewport_to_world(global, point).ok()?;

    Some(ray.origin + *ray.direction * DEPTH)
}
