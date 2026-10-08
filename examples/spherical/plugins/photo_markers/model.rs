//! The marker model's measured numbers, and the pure geometry that stands one on the ground.
//!
//! Everything here is derived from assets/models/marker.glb rather than typed in, so a model
//! with another box needs only the constants below changed. The file holds two meshes and no
//! materials: `marker`, the plate with an open recess in its front, and `screen`, a flat quad
//! that fits the recess. Both nodes carry the same translation and scale, Blender's placement
//! of the pair, so one parent transform keeps them interlocked.
//!
//! The model is not centred on its origin and does not stand on it, so model_transform undoes
//! the node's placement, puts the plate's base at the entity's origin and scales the whole to
//! MARKER_HEIGHT metres. The entity is then placed HOVER_HEIGHT above the ground, and the base
//! is what floats at that height.

use bevy::math::{DMat3, DQuat, DVec3};
use bevy::prelude::*;

/// The model, under the asset root. Two meshes, no materials, 4 KB, tracked in git.
pub(super) const MODEL_PATH: &str = "models/marker.glb";

/// The mesh names inside the file, which is how they are looked up rather than by index, so a
/// re-export that reorders them changes nothing.
pub(super) const MARKER_MESH: &str = "marker";
pub(super) const SCREEN_MESH: &str = "screen";

/// The translation and scale both of the file's nodes carry, measured from it. Dropping them
/// would leave the screen out of its recess; applying them to one mesh and not the other
/// would too.
const NODE_TRANSLATION: Vec3 = Vec3::new(-0.7620853, -0.0049493, -0.2331615);
const NODE_SCALE: Vec3 = Vec3::new(0.3534846, 1.0, 1.0);

/// The marker mesh's own bounding box in the file, before its node's transform.
const MARKER_MIN: Vec3 = Vec3::new(-1.0, -0.494_065, -0.0560187);
const MARKER_MAX: Vec3 = Vec3::new(1.0, 0.5049493, 0.1057776);

/// The screen quad's, the same way. It is flat: both z are 0.0903050.
const SCREEN_MIN: Vec3 = Vec3::new(-0.8882679, 0.0125148, 0.0903050);
const SCREEN_MAX: Vec3 = Vec3::new(0.8882679, 0.4758111, 0.0903050);

/// How tall a marker is drawn, in metres, base to top, and where the panel's size slider starts.
///
/// The model is a metre tall in its own units, which from the example's opening view a kilometre
/// up would be about a pixel, so it has to be given a size. A hundred and thirty metres is what
/// looked right by eye from that view. Far larger than the 24 m carriages, which is the point: a
/// marker is a label on the landscape rather than a thing standing in it, and at anything near a
/// building's height it reads as a speck over a city the size of Auckland. The slider moves it
/// while the example runs, and `note size` writes the judgements to marker_sizes.csv.
pub(super) const MARKER_HEIGHT: f32 = 130.0;

/// How far the base floats above the terrain, in metres.
pub(super) const HOVER_HEIGHT: f64 = 10.0;

/// The smallest a marker is ever drawn on screen, in pixels. Further away than this it is scaled
/// up to hold that height, the way a map pin stays a pin however far out the map goes, and zero
/// would leave a true world-sized object that shrinks into a dot.
///
/// A hundred pixels, which is what the sizing samples came back saying: in every one of them the
/// marker stood at its floor rather than at its height, so what was being judged by eye was this
/// and not the metres. Beyond about 1.7 km it is the floor that decides how big a marker is, and
/// MARKER_HEIGHT only matters nearer than that. See marker_sizes.csv.
pub(super) const MIN_PIXELS: f32 = 100.0;

/// The size range the panel's slider covers, in metres: small enough to be a pin on a hillside,
/// large enough to be read from well out. The top is three times the default rather than just
/// over it, so that there is as much room to try a larger marker as a smaller one; the default
/// then sits about a third along the track.
pub(super) const MIN_HEIGHT: f32 = 2.0;
pub(super) const MAX_HEIGHT: f32 = 400.0;

/// The one scale on all three axes that makes the placed model MARKER_HEIGHT tall.
const MODEL_SCALE: f32 = MARKER_HEIGHT / (MARKER_MAX.y - MARKER_MIN.y);

/// The screen's width over its height once the node's x scale is in, 1.3555. A photo is
/// letterboxed onto a canvas of this shape, so that it fills the quad without stretching.
pub(super) const SCREEN_ASPECT: f32 =
    (SCREEN_MAX.x - SCREEN_MIN.x) * NODE_SCALE.x / (SCREEN_MAX.y - SCREEN_MIN.y);

/// The one transform each mesh child carries, taking a vertex as the file holds it to where the
/// holder entity wants it: the file's node placement, the plate's base brought to the origin,
/// and the scale to metres. Applied to either mesh it keeps the pair as Blender had them.
///
/// The three collapse into the scale and translation a Transform holds, since a Transform is
/// `translation + rotation * (scale * vertex)`. Through the node a vertex is
/// `vertex * NODE_SCALE + NODE_TRANSLATION`, and then recentring and scaling gives
/// `(vertex * NODE_SCALE + NODE_TRANSLATION - base) * MODEL_SCALE`, which is a scale of
/// `NODE_SCALE * MODEL_SCALE` and a translation of `(NODE_TRANSLATION - base) * MODEL_SCALE`.
pub(super) fn model_transform() -> Transform {
    Transform {
        translation: (NODE_TRANSLATION - placed_base()) * MODEL_SCALE,
        rotation: Quat::IDENTITY,
        scale: NODE_SCALE * MODEL_SCALE,
    }
}

/// The plate's base after the file's node transform: the middle of its x and z extents at the
/// bottom of its y. This is the point the marker stands on, and the one brought to the origin.
fn placed_base() -> Vec3 {
    let min = MARKER_MIN * NODE_SCALE + NODE_TRANSLATION;
    let max = MARKER_MAX * NODE_SCALE + NODE_TRANSLATION;

    Vec3::new((min.x + max.x) / 2.0, min.y, (min.z + max.z) / 2.0)
}

/// The rotation that stands a marker up at a point and turns its screen towards the camera.
///
/// Local Y is the ground's up, so the marker stands rather than leans. Local Z is the screen's
/// normal, which is turned to face the camera in the plane level with the ground: the marker
/// yaws about its own up and never tips, so a photo is never seen from below. Looking straight
/// down, where the camera's direction has nothing level in it, it faces north.
pub(super) fn billboard_rotation(up: DVec3, to_camera: DVec3) -> DQuat {
    let north = (DVec3::Y - up * up.y).try_normalize().unwrap_or(DVec3::X);

    // The direction is normalised before the up is taken out of it, so that what is left is the
    // sine of the angle off vertical and can be judged against a fixed threshold. Straight
    // overhead that remainder is rounding error, which normalising would turn into an arbitrary
    // direction, so anything under about a ten-thousandth of a degree off vertical faces north.
    let direction = to_camera.try_normalize().unwrap_or(north);
    let level = direction - up * direction.dot(up);
    let facing = if level.length_squared() > 1e-12 {
        level.normalize()
    } else {
        north
    };

    // Right-handed, with local Z the facing and local Y up: x is y across z.
    DQuat::from_mat3(&DMat3::from_cols(up.cross(facing), up, facing))
}

/// How tall a marker of this height stands on screen, in pixels, at this distance. The focal
/// length in pixels is the viewport's height over twice the tangent of half the vertical field of
/// view, as the editor's discs compute it: a metre at a metre's distance covers that many pixels.
pub(super) fn apparent_pixels(height: f32, distance: f64, focal_pixels: f32) -> f32 {
    if distance <= 0.0 {
        return 0.0;
    }

    height * focal_pixels / distance as f32
}

/// How much to scale a marker up so that it holds its floor on screen, and 1.0 whenever it is
/// near enough to be taller than that already. A floor of zero never scales anything.
pub(super) fn pixel_floor_scale(
    height: f32,
    distance: f64,
    focal_pixels: f32,
    min_pixels: f32,
) -> f32 {
    let pixels = apparent_pixels(height, distance, focal_pixels);
    if min_pixels <= 0.0 || pixels <= 0.0 {
        return 1.0;
    }

    (min_pixels / pixels).max(1.0)
}
