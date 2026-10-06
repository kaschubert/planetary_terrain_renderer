//! Flying the camera to a point: the pose that looks at it from the south and above, the
//! stepping through the points the ground moved under, and the write into the view entity.

use super::RailEditor;
use crate::plugins::auckland_rail::MovedGround;
use crate::plugins::rail_editor::frame::unit_under;
use crate::plugins::shared::rail_network::RailNetwork;
use crate::plugins::track_frames::move_to;
use bevy::{math::DVec3, prelude::*};
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};

/// How far above a point and how far south of it a fly-to puts the camera, in metres. From
/// 400 m at 45 degrees a few hundred metres of line fill the view, the discs are well apart
/// and the ground under them can be read against the imagery.
const FLY_DISTANCE: f64 = 400.0;

/// The pose that looks at a position from the south and above, as the camera's position and
/// rotation: FLY_DISTANCE up from the position and as far south of it, looking back down at
/// 45 degrees with up as up, so the view faces north. Built as the start pose in spherical.rs
/// is: up is the position's direction on the unit sphere and north the meridian's direction
/// there, the part of the planet's axis that is level at up. The rotation is in f32, as a
/// Transform holds it, which is a hair's width at any distance; the position stays in f64 for
/// the grid to split.
pub(super) fn fly_to_pose(target: DVec3) -> (DVec3, Quat) {
    let up = unit_under(target);
    let north = (DVec3::Y - up * up.y).normalize();

    let position = target + FLY_DISTANCE * (up - north);
    let rotation = Transform::IDENTITY
        .looking_to((north - up).normalize().as_vec3(), up.as_vec3())
        .rotation;

    (position, rotation)
}

/// Steps the cursor through the moved points, forward or back and round at either end, and
/// selects the point it lands on, which makes it the anchor. Returns where that point is, for
/// the camera, from the point itself rather than the resolved positions so that a dirty frame
/// gives the same answer. None when there are no moved points, or when the entry names a
/// point the editor has since moved or removed: the entry is found by place, so an insert or
/// a delete elsewhere on the line does not send the camera to the point's neighbour.
pub(super) fn step_moved(
    forward: bool,
    cursor: &mut Option<usize>,
    moved: &[MovedGround],
    network: &RailNetwork,
    editor: &mut RailEditor,
) -> Option<DVec3> {
    let count = moved.len();
    if count == 0 {
        return None;
    }
    let next = match (*cursor, forward) {
        (None, true) => 0,
        (None, false) => count - 1,
        (Some(at), true) => (at + 1) % count,
        (Some(at), false) => (at + count - 1) % count,
    };
    *cursor = Some(next);

    let entry = &moved[next];
    let line = network.lines.get(entry.line)?;
    let index = entry.index_in(network)?;
    editor.select((entry.line, index), false);

    let height = line.resolved_heights()[index];
    Some(TerrainShape::WGS84.position_unit_to_local(line.points[index].unit(), height))
}

/// Puts the camera over a position, FLY_DISTANCE above it and as far south, looking at it,
/// written into the view entity's Transform and CellCoord through move_to, see there.
pub(super) fn fly_to(
    target: DVec3,
    grids: &Grids,
    camera: &mut Query<(Entity, &mut Transform, &mut CellCoord), With<OrbitalCameraController>>,
) {
    let Ok((entity, mut transform, mut cell)) = camera.single_mut() else {
        return;
    };
    let Some(grid) = grids.parent_grid(entity) else {
        return;
    };

    let (position, rotation) = fly_to_pose(target);
    move_to(grid, &mut cell, &mut transform, position, rotation);
}
