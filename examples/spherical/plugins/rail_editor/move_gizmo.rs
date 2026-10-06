//! The move gizmo: the selected points dragged in east, north and up, with the heights
//! following the terrain.
//!
//! The arrows come from transform-gizmo-bevy, which knows nothing of planets: it draws a
//! gizmo on an entity's Transform and writes a dragged Transform back. The trick that makes
//! it fit a spheroid is the frame. The handle entity's rotation is set to the east, north
//! and up directions at the anchor, and the gizmo runs in local orientation, so its three
//! arrows are east, up and south wherever the anchor is on the planet, and its plane handles
//! are "slide along the ground" and "move in a vertical wall". The frame, and why its up is
//! the direction heights move along rather than the geodetic normal, is in frame.rs. The
//! crate colours an arrow by its axis, red for X, green for Y and blue for Z, and a square by
//! the axis it stands square to; so the red arrow is east, the green arrow is up, the blue
//! arrow is south, the green square lies flat and slides along the ground, and the red and
//! blue squares stand upright and move in a vertical wall.
//!
//! The crate reads the target's Transform as world space, never its GlobalTransform, so the
//! handle cannot live in the big_space grid where a Transform is relative to a cell. It is a
//! plain top-level entity whose Transform is rebuilt every frame in render space, which is
//! absolute space shifted so the camera's cell is at the origin: the frame the camera's
//! GlobalTransform is in, and the frame the gizmo casts its pointer rays in. Nothing renders
//! the handle itself. It carries ClaimsPointer, so while an arrow is hovered or held the
//! orbital camera does not pan and the editor registers no click.
//!
//! A drag is applied from a snapshot. The gizmo reports the total translation since the drag
//! began, in the handle's local axes, and every frame each selected point is placed at its
//! snapshot plus that total, so the points are a function of where the mouse is now and not
//! a chain of per-frame deltas that would drift as the handle is re-placed under them. The
//! total's up part is the lift and the rest slides along the ground, each point in its own
//! east, north and up frame rather than the anchor's: by the curve of the planet, level at
//! the anchor is a slope three kilometres along, steep enough that a drag of two hundred
//! metres would lift a point there nine centimetres, while in its own frame a drag along the
//! ground changes no height at all. The lift changes a ground point's offset or a fixed
//! point's height, or turns a between point fixed. Every moved point is resampled as it goes,
//! so a ground point slides over the terrain, and the terrain under a fixed one is known for
//! the save.

use super::{DISC_REACH, EditorCamera, RailEditor, selected_by_line};
use crate::plugins::auckland_rail::{AucklandRail, RailSampler};
use crate::plugins::rail_editor::frame::{Frame, unit_under};
use crate::plugins::shared::rail_network::{PointMode, RailNetwork, RailPoint, lon_lat_from_unit};
use bevy::{math::DVec3, prelude::*};
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};
use transform_gizmo_bevy::{GizmoMode, GizmoOptions, GizmoOrientation, GizmoResult, GizmoTarget};

/// The entity the gizmo draws on and reads the drag from. One at most, at the anchor. The
/// parent never names it, but the queries of the systems it registers do, and a system whose
/// signature names a type the parent cannot see is one the parent cannot register.
#[derive(Component)]
pub(super) struct RailHandle;

/// A between point lifted less than this stays between, in metres. A drag along the ground
/// reports no lift at all but for rounding, and a drag meant to lift a point is centimetres
/// at the least, so a millimetre sits a long way from both.
const BETWEEN_TOLERANCE: f64 = 0.001;

/// A selected point as it was when the drag began.
#[derive(Debug, Clone)]
struct PointSnapshot {
    line: usize,
    index: usize,
    point: RailPoint,
    /// Its height above the ellipsoid as resolved, which is what a between point is given
    /// when a lift turns it fixed.
    resolved: f64,
    /// Where it was, in absolute metres.
    position: DVec3,
    /// Its own east, north and up, which the gizmo's total is read in.
    frame: Frame,
}

impl PointSnapshot {
    fn new(line: usize, index: usize, point: &RailPoint, resolved: f64) -> Self {
        let unit = point.unit();

        Self {
            line,
            index,
            point: point.clone(),
            resolved,
            position: TerrainShape::WGS84.position_unit_to_local(unit, resolved),
            frame: Frame::at_unit(unit),
        }
    }
}

/// The drag in progress, if there is one. Present from the frame the gizmo reports itself
/// active to the frame it stops.
#[derive(Resource, Default)]
pub(super) struct MoveDrag(Option<Drag>);

impl MoveDrag {
    /// True while the handle is held, whether or not the points still follow it.
    pub(super) fn in_progress(&self) -> bool {
        self.0.is_some()
    }
}

struct Drag {
    /// The camera's cell when the drag began, which the gizmo's totals are only good in;
    /// see apply_drag.
    cell: CellCoord,
    /// Set once the camera has left that cell. The points stay where the last total put
    /// them, and nothing more is read from the gizmo until the mouse is released.
    ended: bool,
    points: Vec<PointSnapshot>,
    /// The total last applied, so a frame in which the mouse has not moved does nothing.
    /// None until the mouse has moved at all, see is_new.
    applied: Option<DVec3>,
}

impl Drag {
    /// Snapshots the selection, with the camera's cell. None when the anchor is not among
    /// the points snapshotted, which it always is while a handle exists to be dragged.
    fn begin(editor: &RailEditor, network: &RailNetwork, cell: CellCoord) -> Option<Self> {
        let anchor = editor.anchor?;
        let mut points = Vec::with_capacity(editor.selection.len());

        for (line_index, (line, indices)) in network
            .lines
            .iter()
            .zip(selected_by_line(&editor.selection, network))
            .enumerate()
        {
            if indices.is_empty() {
                continue;
            }
            // Once per line that has a selected point: the heights of a whole line, for the
            // between points among them.
            let heights = line.resolved_heights();
            for index in indices {
                points.push(PointSnapshot::new(
                    line_index,
                    index,
                    &line.points[index],
                    heights[index],
                ));
            }
        }

        points
            .iter()
            .any(|snapshot| (snapshot.line, snapshot.index) == anchor)
            .then_some(Self {
                cell,
                ended: false,
                points,
                applied: None,
            })
    }

    /// Whether a total the gizmo reports is one to apply: not the one applied last, which a
    /// frame the mouse has not moved in repeats, and not the nothing it reports on the frame
    /// of the press, before the mouse has moved at all. A press let go without a move is
    /// then no edit: nothing is applied and nothing recorded.
    fn is_new(&self, total: DVec3) -> bool {
        self.applied != Some(total) && (self.applied.is_some() || total != DVec3::ZERO)
    }
}

/// The point as the gizmo's total leaves it. The total is in the handle's axes, east, up and
/// south, and is read in the point's own frame, so its up part is the lift wherever the point
/// is and the rest moves it along its own ground. Its longitude and latitude are those of
/// the moved position, and by its mode: a ground point's offset and a fixed point's height
/// change by the lift; a between point lifted or lowered by more than BETWEEN_TOLERANCE
/// becomes fixed at its resolved height plus the lift, else stays between and only moves
/// along the ground. The sample is the snapshot's; the caller resamples, having a sampler to
/// do it with.
fn displace(snapshot: &PointSnapshot, total: DVec3) -> RailPoint {
    let mut point = snapshot.point.clone();
    let moved = snapshot.position + snapshot.frame.displacement(total);
    let lift = total.y;

    (point.longitude, point.latitude) = lon_lat_from_unit(unit_under(moved));

    match point.mode {
        PointMode::Ground | PointMode::Fixed => {
            point.height = Some(snapshot.point.height.unwrap_or(0.0) + lift);
        }
        PointMode::Between if lift.abs() > BETWEEN_TOLERANCE => {
            point.mode = PointMode::Fixed;
            point.height = Some(snapshot.resolved + lift);
        }
        PointMode::Between => {}
    }

    point
}

/// Translation only, along the three axes and in the three planes, in the handle's own
/// axes, one gizmo for the whole selection, and no hotkeys: the crate's defaults would take
/// R, G, S, X, Y, Z and Escape, which the debug plugin and the editor have. The crate's
/// translation set also has a circle at the centre that drags in the camera's own plane,
/// which is left out: from any camera that is not level such a drag is partly a lift, and
/// would turn a between point fixed without the green arrow having been touched. Snapping
/// is off; a point goes where the mouse puts it.
pub(super) fn configure_move_gizmo(mut options: ResMut<GizmoOptions>) {
    options.gizmo_modes = GizmoMode::TranslateX
        | GizmoMode::TranslateY
        | GizmoMode::TranslateZ
        | GizmoMode::TranslateXY
        | GizmoMode::TranslateXZ
        | GizmoMode::TranslateYZ;
    options.gizmo_orientation = GizmoOrientation::Local;
    options.group_targets = true;
    options.snapping = false;
    options.hotkeys = None;
}

/// Keeps one handle at the anchor while there is one to drag, and none otherwise. Its
/// Transform is rebuilt every frame from the anchor's resolved position, the same position
/// the lines and the discs draw from, shifted into render space.
#[allow(clippy::type_complexity)]
pub(super) fn place_handle(
    mut commands: Commands,
    editor: Res<RailEditor>,
    rail: Res<AucklandRail>,
    grids: Grids,
    camera: Query<EditorCamera, With<OrbitalCameraController>>,
    // Without the camera filter Bevy cannot prove that this query and the camera query
    // above never name the same entity, and refuses the system (B0001); the handle is a
    // plain entity and the camera is not, so the filter only states what is already so.
    mut handle: Query<
        (Entity, &mut Transform),
        (With<RailHandle>, Without<OrbitalCameraController>),
    >,
) {
    let existing = handle.single_mut().ok();

    // The handle follows the discs: gone while editing is off or the lines are hidden, and
    // beyond their reach, from where a pixel of drag is tens of metres and the gizmo, which
    // draws over everything, would show through the planet.
    let anchor = editor.anchor.filter(|&(line, index)| {
        editor.editing
            && rail.visible
            && rail
                .network
                .lines
                .get(line)
                .is_some_and(|line| index < line.points.len())
    });

    let Some((line, index)) = anchor else {
        if let Some((entity, _)) = existing {
            commands.entity(entity).despawn();
        }
        return;
    };

    // Dirty, the positions are a mutation behind the points, so the handle keeps its place
    // for this one frame rather than take a position that may belong to another point.
    if rail.dirty {
        return;
    }
    let Ok(camera) = camera.single() else {
        return;
    };
    let Some(grid) = grids.parent_grid(camera.entity) else {
        return;
    };

    let line = &rail.network.lines[line];
    let position = line.positions[index];
    let camera_position = grid.grid_position_double(camera.cell, camera.transform);

    if position.distance(camera_position) > DISC_REACH {
        if let Some((entity, _)) = existing {
            commands.entity(entity).despawn();
        }
        return;
    }

    // As the lines: the subtraction in f64, then the narrowing.
    let cell_origin = grid.cell_to_float(camera.cell);
    let frame = Frame::at_unit(line.points[index].unit());
    let transform = Transform {
        translation: (position - cell_origin).as_vec3(),
        rotation: frame.rotation().as_quat(),
        scale: Vec3::ONE,
    };

    match existing {
        Some((_, mut current)) => *current = transform,
        None => {
            commands.spawn((RailHandle, GizmoTarget::default(), ClaimsPointer, transform));
        }
    }
}

/// Applies the drag the gizmo reports to the selected points, from the snapshot taken as it
/// began. The gizmo runs in Last, so what is read here is the frame before's; the points
/// are then resolved this frame, so the lines keep up with the mouse to within a frame.
///
/// The total comes in the handle's local axes, since the gizmo runs in local orientation,
/// and each point reads it in its own frame. Render space is absolute space less the
/// camera's cell origin, so a displacement is the same in both while the cell stays put.
/// The gizmo measures its totals from where the drag began, in the render space of the cell
/// the camera was in then, and casts its pointer ray in the cell it is in now. The orbital
/// camera cannot move while the handle holds the pointer, but the fly camera's keys can, and
/// once they have carried the camera into another cell every total is off by the width of
/// the move. The drag ends there, with the points where the last total put them.
pub(super) fn apply_drag(
    mut editor: ResMut<RailEditor>,
    mut rail: ResMut<AucklandRail>,
    mut sampler: Option<ResMut<RailSampler>>,
    mut drag: ResMut<MoveDrag>,
    handle: Query<&GizmoTarget, With<RailHandle>>,
    camera: Query<&CellCoord, With<OrbitalCameraController>>,
) {
    let target = handle.single().ok().filter(|target| target.is_active());

    let Some(target) = target else {
        // A press let go without a move applied nothing and has nothing to say.
        if let Some(Drag {
            applied: Some(total),
            points,
            ..
        }) = drag.0.take()
        {
            info!(
                "rail editor: moved {} points {:+.1} m east, {:+.1} m north, {:+.1} m up",
                points.len(),
                total.x,
                -total.z,
                total.y
            );
        }
        return;
    };

    let Ok(&cell) = camera.single() else {
        return;
    };

    if drag.0.is_none() {
        let Some(begun) = Drag::begin(&editor, &rail.network, cell) else {
            return;
        };
        drag.0 = Some(begun);
    }
    let Some(drag) = drag.0.as_mut() else {
        return;
    };

    if !drag.ended && drag.cell != cell {
        drag.ended = true;
        warn!("rail editor: the camera left its cell mid-drag; the drag ends where it is");
    }
    if drag.ended {
        return;
    }

    let Some(GizmoResult::Translation { total, .. }) = target.latest_result() else {
        return;
    };
    let total = DVec3::new(total.x, total.y, total.z);
    if !drag.is_new(total) {
        return;
    }
    if drag.applied.is_none() {
        // Before the first point moves, so undo puts the whole drag back at once; and not at
        // the press, which may be let go without a move and is then no edit.
        editor.record(&rail.network);
    }
    drag.applied = Some(total);

    for snapshot in &drag.points {
        // A point Delete took away mid-drag is left out; the rest still move.
        let Some(point) = rail
            .network
            .lines
            .get_mut(snapshot.line)
            .and_then(|line| line.points.get_mut(snapshot.index))
        else {
            continue;
        };
        *point = displace(snapshot, total);

        // Whatever its mode. A fixed point's height is its own, but the terrain under it
        // goes to the file as the cache the next run compares its sample against, and has
        // to be the terrain under where the point is now.
        match sampler.as_deref_mut() {
            Some(sampler) => sampler.resample(point),
            // The startup sampling has not landed. The point keeps the sample it had where
            // it was, and is put on the ground when the sampler arrives, which samples
            // afresh every point it did not see.
            None => warn_once!(
                "rail editor: the sampler has not landed; a point dragged now keeps the \
                 height it had until the startup sampling is through"
            ),
        }
    }

    rail.dirty = true;
}

#[cfg(test)]
mod tests;
