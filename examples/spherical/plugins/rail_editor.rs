//! Editing the rail lines with the mouse: selecting points, adding and removing them, and
//! undoing any of it.
//!
//! While editing is on every point draws as a small disc on its line, sized in pixels so it
//! reads the same at any zoom, and the mouse picks among them in screen space: the nearest
//! disc within a few pixels of a click is the one meant, which needs no mesh picking and
//! works at any distance. A click selects one point, a Shift-click the run along the line
//! from the last point clicked, which is how a tunnel gets selected, one portal then the
//! other. A double-click on the terrain beside a line adds a point to it there, from the
//! terrain hit the picking pass already provides. Delete removes the selection.
//!
//! The network is twelve hundred points, so undo is a stack of snapshots of the point lists
//! rather than a log of changes: every mutation here, and the move gizmo's drags, push one
//! first. The gizmo is the other half of the editor and sits on top of this module: it reads
//! the selection and the anchor, the point clicked last and where it should sit, records a
//! snapshot when a drag starts, and resamples through RailSampler as the drag goes, so a
//! ground point slides over the terrain. It is in move_gizmo.rs.
//!
//! The systems run in PostUpdate after the transforms have propagated, as the lines draw,
//! so the camera's global transform used for the hit test is this frame's and not the last.
//! A mutation marks the network dirty and the positions follow next frame; the mouse, the
//! discs and the handle skip that one frame rather than work from positions the point list
//! has moved on from. The click rules are in clicks.rs, the disc drawing in discs.rs, the
//! mode operations, which turn a run of points into a tunnel or a bridge, in modes.rs, and
//! the F5 panel that reads out the anchor and has the buttons for those operations, and for
//! saving, in panel.rs; the selection, insert and delete, and the undo stack are here.

use super::auckland_rail::{AucklandRail, RailSampler, collect_samples, resolve_rail_heights};
use super::shared::rail_network::{RailLine, RailNetwork, RailPoint};
use super::sheet_grid::IN_FRONT_OF_TERRAIN;
use bevy::{ecs::query::QueryData, math::DVec3, prelude::*, window::PrimaryWindow};
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};
use clicks::{Click, ClickDetector};
use discs::draw_point_discs;
use move_gizmo::{MoveDrag, apply_drag, configure_move_gizmo, place_handle};
use std::collections::HashMap;

mod clicks;
mod discs;
/// The east, north and up frame the gizmo and the fly-to stand on; other plugins' tests
/// borrow it too.
pub(crate) mod frame;
mod modes;
mod move_gizmo;
mod panel;

pub struct RailEditorPlugin;

/// The editor's own gizmo settings: the discs beat the terrain's depth as the lines do, or a
/// point on a ridge would be hidden by the ground it stands on.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct RailEditorGizmos;

/// The editor's per-frame systems, which the handle and the panel's readout order themselves
/// after: the keys, then the mouse, then the discs, in PostUpdate after
/// TransformSystems::Propagate.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct RailEditorSystems;

impl Plugin for RailEditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_gizmo_group::<RailEditorGizmos>()
            .init_resource::<RailEditor>()
            .init_resource::<MoveDrag>()
            .add_plugins(panel::RailPanelPlugin)
            .add_systems(Startup, (configure_editor_gizmos, configure_move_gizmo))
            // Before the startup samples land, so a drag applied the frame they do is seen
            // by them as a point moved while the task ran and sampled afresh, rather than
            // putting the snapshot's stale sample back over the fresh one; and before the
            // positions are recomputed, so a drag is drawn the frame it is read rather than
            // the one after.
            .add_systems(
                Update,
                apply_drag
                    .before(collect_samples)
                    .before(resolve_rail_heights),
            )
            .add_systems(
                PostUpdate,
                (
                    (edit_with_keys, edit_with_mouse, draw_point_discs)
                        .chain()
                        .in_set(RailEditorSystems),
                    // After the mouse has had its say on the anchor, and still before Last,
                    // where the gizmo reads the handle.
                    place_handle.after(RailEditorSystems),
                )
                    .after(TransformSystems::Propagate),
            );
    }
}

/// Points further than this from the camera draw no disc and take no click, in metres. The
/// network is 130 km of line and some 1200 points; from further out than this the discs
/// would merge into the line, and the cap keeps the per-frame projection to the points that
/// could be told apart.
const DISC_REACH: f64 = 30_000.0;

/// How near the cursor a disc has to be to be the one clicked, in pixels. Wider than the
/// disc itself, so the click does not have to be exact.
const PICK_PIXELS: f32 = 8.0;

/// How far from a line the terrain under a double-click may be and still add a point to
/// it, in metres on the ground. The lines sit on the ground and a tunnel a few tens of
/// metres below it, so a double-click on the line as drawn is always within reach, and one
/// in open country far from any line does nothing.
const INSERT_REACH: f64 = 200.0;

/// Segments whose first point is further than this from the hit are not measured. The
/// longest segment on the network is under two kilometres, see chord_lengths in
/// rail_network.rs, so no segment within INSERT_REACH of the hit begins outside this.
const SEGMENT_SEARCH: f64 = 30_000.0;

/// How many edits undo remembers. A snapshot is the point list, about 1200 points of a
/// hundred bytes, so this is some 25 MiB at the deepest and a long session's worth.
const UNDO_DEPTH: usize = 200;

/// A line is a line of at least this many points; Delete stops short of taking more.
const MIN_POINTS: usize = 2;

/// The camera as the mouse and the discs need it: to project a position onto the screen,
/// to shift positions into its cell, to size a disc in pixels, and to read the terrain hit
/// under the cursor.
#[derive(QueryData)]
struct EditorCamera {
    entity: Entity,
    camera: &'static Camera,
    global: &'static GlobalTransform,
    transform: &'static Transform,
    cell: &'static CellCoord,
    projection: &'static Projection,
    picking: &'static PickingData,
}

/// The state of the network at one moment, for undo.
#[derive(Clone)]
struct Snapshot {
    /// Every line's points, line by line. The lines themselves are never added or removed
    /// by the editor, so their names and order need not be kept. The samples the points
    /// carry are not what is restored, see restore.
    points: Vec<Vec<RailPoint>>,
    selection: Vec<(usize, usize)>,
    anchor: Option<(usize, usize)>,
}

/// What is being edited and how to take it back.
#[derive(Resource, Default)]
pub struct RailEditor {
    /// Toggled by F5, in panel.rs, which shows the panel exactly while this is on. Off,
    /// nothing here draws or reacts to the mouse, and the selection is kept for when it comes
    /// back on.
    pub editing: bool,
    /// The selected points as (line index, point index), in the order they were selected.
    pub selection: Vec<(usize, usize)>,
    /// The point clicked last, where the gizmo sits and where a Shift-click's run starts.
    /// Always a member of the selection when it is Some.
    pub anchor: Option<(usize, usize)>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    clicks: ClickDetector,
}

impl RailEditor {
    /// A click on a point. Plain, the selection becomes that point. Extended, with Shift,
    /// and the anchor on the same line, the selection becomes the run of points from the
    /// anchor to this one inclusive; on another line, or with no anchor, the point joins
    /// the selection. Either way the point is the new anchor.
    pub fn select(&mut self, point: (usize, usize), extend: bool) {
        match self.anchor {
            Some(anchor) if extend && anchor.0 == point.0 => {
                self.selection = run_between(anchor, point);
            }
            Some(_) if extend => {
                if !self.selection.contains(&point) {
                    self.selection.push(point);
                }
            }
            _ => self.selection = vec![point],
        }
        self.anchor = Some(point);
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.anchor = None;
    }

    /// Remembers the network and the selection as they are, to be undone to. Call before a
    /// mutation, not after: the gizmo calls it when a drag starts. A new edit forgets what
    /// had been undone, as every editor does.
    pub fn record(&mut self, network: &RailNetwork) {
        let snapshot = self.snapshot(network);
        push_capped(&mut self.undo, snapshot);
        self.redo.clear();
    }

    /// Back to the state before the last recorded edit, if there was one. The state being
    /// left goes on the redo stack. The run's terrain samples are carried over rather than
    /// restored, and the sampler, when there is one, reads the ground under the few points
    /// the edit had moved; see restore.
    pub fn undo(&mut self, network: &mut RailNetwork, sampler: Option<&mut RailSampler>) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        let current = self.snapshot(network);
        push_capped(&mut self.redo, current);
        self.restore(network, snapshot, sampler);
        true
    }

    /// Forward again to a state that was undone, if there is one, with the samples carried
    /// over as undo carries them.
    pub fn redo(&mut self, network: &mut RailNetwork, sampler: Option<&mut RailSampler>) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        let current = self.snapshot(network);
        push_capped(&mut self.undo, current);
        self.restore(network, snapshot, sampler);
        true
    }

    /// Whether there is an edit to undo, or one to redo, for the panel to dim its buttons by.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Removes the selected points and clears the selection. A line keeps its last
    /// MIN_POINTS whatever is selected, with a warning. False when nothing was selected, or
    /// when nothing could go for that reason, in which case nothing is recorded either and
    /// the selection stays, as for any edit that would change nothing.
    pub fn delete_selection(&mut self, network: &mut RailNetwork) -> bool {
        clamp_selection(&mut self.selection, network);
        if self.selection.is_empty() {
            self.anchor = None;
            return false;
        }

        let selected = selected_by_line(&self.selection, network);
        let (removing, kept) = removal_counts(network, &selected);
        if kept > 0 {
            warn!(
                "rail editor: keeping {kept} of the {} selected points, because a line keeps at \
                 least {MIN_POINTS}",
                removing + kept
            );
        }
        if removing == 0 {
            return false;
        }

        self.record(network);
        remove_points(network, &selected);
        info!("rail editor: removed {removing} points");
        self.clear_selection();
        true
    }

    /// Adds a ground point to the line nearest the hit, on the segment nearest it, and
    /// selects the new point. None when no line comes within INSERT_REACH of the hit, in
    /// which case nothing is recorded. The point's height is not sampled here; the caller
    /// does that through RailSampler when there is one.
    pub fn insert_point(
        &mut self,
        network: &mut RailNetwork,
        hit: DVec3,
    ) -> Option<(usize, usize)> {
        let (line, segment, distance) = nearest_segment(&network.lines, hit)?;
        if distance > INSERT_REACH {
            return None;
        }

        self.record(network);
        let index = segment + 1;
        let unit = TerrainShape::WGS84.position_local_to_unit(hit);
        network.lines[line]
            .points
            .insert(index, RailPoint::ground_at_unit(unit));
        self.select((line, index), false);

        Some((line, index))
    }

    fn snapshot(&self, network: &RailNetwork) -> Snapshot {
        Snapshot {
            points: network
                .lines
                .iter()
                .map(|line| line.points.clone())
                .collect(),
            selection: self.selection.clone(),
            anchor: self.anchor,
        }
    }

    /// Puts the network and the selection back as the snapshot had them. The selection is
    /// clamped to the points that exist, which only matters if the lines themselves have
    /// changed under the stack, which nothing here does.
    ///
    /// The terrain samples are the network's and not the snapshot's: a sample is a fact
    /// about the ground at a place and not an edit, and a snapshot taken before the startup
    /// sampling landed has none, so restoring it as it is would drop every point's sample and
    /// put the lines back on the ellipsoid. A restored point at a place the network has a
    /// point at now takes that point's sample and cached terrain, which is all but the few
    /// the edit moved, and those are sampled afresh through the sampler when there is one.
    /// Without one the snapshot's stand, and the startup sampling puts every point on the
    /// ground when it lands.
    fn restore(
        &mut self,
        network: &mut RailNetwork,
        snapshot: Snapshot,
        mut sampler: Option<&mut RailSampler>,
    ) {
        if snapshot.points.len() != network.lines.len() {
            warn!(
                "rail editor: a snapshot of {} lines restored over {}",
                snapshot.points.len(),
                network.lines.len()
            );
        }

        // By the bits, as the sampler's landing matches its samples to the points.
        let samples: HashMap<(u64, u64), (Option<f64>, Option<f64>)> = network
            .points()
            .map(|point| (point.place_bits(), (point.sampled, point.terrain)))
            .collect();

        for (line, points) in network.lines.iter_mut().zip(snapshot.points) {
            line.points = points;
            for point in &mut line.points {
                match (samples.get(&point.place_bits()), sampler.as_deref_mut()) {
                    (Some(&(sampled, terrain)), _) => {
                        point.sampled = sampled;
                        point.terrain = terrain;
                    }
                    (None, Some(sampler)) => sampler.resample(point),
                    (None, None) => {}
                }
            }
        }

        self.selection = snapshot.selection;
        self.anchor = snapshot.anchor;
        clamp_selection(&mut self.selection, network);
        if self
            .anchor
            .is_some_and(|anchor| !self.selection.contains(&anchor))
        {
            self.anchor = None;
        }
    }
}

/// Pushes, dropping the oldest when the stack is full.
fn push_capped(stack: &mut Vec<Snapshot>, snapshot: Snapshot) {
    if stack.len() >= UNDO_DEPTH {
        stack.remove(0);
    }
    stack.push(snapshot);
}

/// The points of one line from one index to another inclusive, in that direction, so the
/// run reads from the anchor towards the point clicked.
fn run_between(from: (usize, usize), to: (usize, usize)) -> Vec<(usize, usize)> {
    let line = from.0;
    if from.1 <= to.1 {
        (from.1..=to.1).map(|index| (line, index)).collect()
    } else {
        (to.1..=from.1).rev().map(|index| (line, index)).collect()
    }
}

/// Drops the entries that no longer name a point.
fn clamp_selection(selection: &mut Vec<(usize, usize)>, network: &RailNetwork) {
    selection.retain(|&(line, index)| {
        network
            .lines
            .get(line)
            .is_some_and(|line| index < line.points.len())
    });
}

/// The selected points as each line sees them: one list per line of the network, in order
/// along the line and each index once, without the entries that name no point. What a
/// delete, a drag and the mode operations work from.
fn selected_by_line(selection: &[(usize, usize)], network: &RailNetwork) -> Vec<Vec<usize>> {
    network
        .lines
        .iter()
        .enumerate()
        .map(|(line_index, line)| {
            let mut indices: Vec<usize> = selection
                .iter()
                .filter(|&&(selected_line, index)| {
                    selected_line == line_index && index < line.points.len()
                })
                .map(|&(_, index)| index)
                .collect();
            indices.sort_unstable();
            indices.dedup();
            indices
        })
        .collect()
}

/// How many of the points selected, as selected_by_line lists them, a removal would take
/// and how many it would keep, since a line keeps its last MIN_POINTS.
fn removal_counts(network: &RailNetwork, selected: &[Vec<usize>]) -> (usize, usize) {
    let (mut removing, mut kept) = (0, 0);

    for (line, indices) in network.lines.iter().zip(selected) {
        let room = line.points.len().saturating_sub(MIN_POINTS);
        removing += indices.len().min(room);
        kept += indices.len().saturating_sub(room);
    }

    (removing, kept)
}

/// Removes the points selected on each line, highest index first so the lower indices stay
/// good, stopping at MIN_POINTS on a line: the points removal_counts says go.
fn remove_points(network: &mut RailNetwork, selected: &[Vec<usize>]) {
    for (line, indices) in network.lines.iter_mut().zip(selected) {
        for &index in indices.iter().rev() {
            if line.points.len() <= MIN_POINTS {
                break;
            }
            line.points.remove(index);
        }
    }
}

/// The segment of any line nearest the position, as (line index, index of the segment's
/// first point, distance in metres), measured in 3D against the resolved positions. None
/// when there are no segments within SEGMENT_SEARCH.
fn nearest_segment(lines: &[RailLine], hit: DVec3) -> Option<(usize, usize, f64)> {
    let mut nearest: Option<(usize, usize, f64)> = None;

    for (line_index, line) in lines.iter().enumerate() {
        for (index, pair) in line.positions.windows(2).enumerate() {
            if pair[0].distance(hit) > SEGMENT_SEARCH {
                continue;
            }
            let distance = distance_to_segment(hit, pair[0], pair[1]);
            if nearest.is_none_or(|(_, _, best)| distance < best) {
                nearest = Some((line_index, index, distance));
            }
        }
    }

    nearest
}

/// The distance from a point to the nearest point of the segment from a to b.
fn distance_to_segment(point: DVec3, a: DVec3, b: DVec3) -> f64 {
    let ab = b - a;
    let length_squared = ab.length_squared();
    let along = if length_squared > 0.0 {
        ((point - a).dot(ab) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };

    point.distance(a + ab * along)
}

fn configure_editor_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<RailEditorGizmos>();
    config.depth_bias = IN_FRONT_OF_TERRAIN;
}

/// The editing keys. The letters under Control are chords the debug plugin stands aside
/// for; Delete and Escape nothing else uses. F5, which turns editing on, is read in panel.rs
/// beside the panel it shows, and Ctrl+S there too, beside the toast it raises. Nothing is
/// read while F4 hides the lines, as the mouse reads nothing then: an edit to lines that are
/// not drawn would be a surprise.
fn edit_with_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut editor: ResMut<RailEditor>,
    mut rail: ResMut<AucklandRail>,
    mut sampler: Option<ResMut<RailSampler>>,
    drag: Res<MoveDrag>,
) {
    if !editor.editing || !rail.visible {
        return;
    }

    let control = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);

    if keys.just_pressed(KeyCode::Escape) {
        editor.clear_selection();
    }
    if keys.just_pressed(KeyCode::Delete) && editor.delete_selection(&mut rail.network) {
        rail.dirty = true;
    }

    // Undo and redo wait for a drag to end. Mid-drag an undo would pop the drag's own
    // snapshot, and the next movement of the mouse would put the dragged points back over
    // the restored ones with nothing left on the stack to take them off again.
    if drag.in_progress() {
        return;
    }
    if control && keys.just_pressed(KeyCode::KeyZ) {
        if editor.undo(&mut rail.network, sampler.as_deref_mut()) {
            rail.dirty = true;
            info!("rail editor: undone");
        } else {
            info!("rail editor: nothing to undo");
        }
    }
    if control && keys.just_pressed(KeyCode::KeyY) {
        if editor.redo(&mut rail.network, sampler.as_deref_mut()) {
            rail.dirty = true;
            info!("rail editor: redone");
        } else {
            info!("rail editor: nothing to redo");
        }
    }
}

/// Clicks on discs select, double-clicks on the terrain insert. The mouse is read only
/// while the lines show: a selection made on lines that are hidden would be a surprise. Nor
/// on a frame the network is dirty, after a key edit earlier in the chain: the positions a
/// hit is tested against are then a mutation behind the points, and the hit would name the
/// wrong one. The discs and the handle skip such a frame too.
#[allow(clippy::too_many_arguments)]
fn edit_with_mouse(
    mut editor: ResMut<RailEditor>,
    mut rail: ResMut<AucklandRail>,
    mut sampler: Option<ResMut<RailSampler>>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    capture: Res<PointerCapture>,
    time: Res<Time<Real>>,
    window: Query<&Window, With<PrimaryWindow>>,
    grids: Grids,
    camera: Query<EditorCamera, With<OrbitalCameraController>>,
) {
    if !editor.editing || !rail.visible || rail.dirty {
        return;
    }
    let Ok(window) = window.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let now = time.elapsed_secs_f64();

    if buttons.just_pressed(MouseButton::Left) {
        editor.clicks.press(cursor, now, capture.blocks_pointer());
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let Some(click) = editor.clicks.release(cursor, now) else {
        return;
    };

    let Ok(camera) = camera.single() else {
        return;
    };
    let Some(grid) = grids.parent_grid(camera.entity) else {
        return;
    };
    let cell_origin = grid.cell_to_float(camera.cell);
    let camera_position = grid.grid_position_double(camera.cell, camera.transform);

    let picked = nearest_point_on_screen(
        &rail.network,
        camera.camera,
        camera.global,
        cell_origin,
        camera_position,
        click.position(),
    );

    if let Some(point) = picked {
        let extend = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        editor.select(point, extend);
        return;
    }

    // A click on the terrain leaves the selection alone; a double-click there adds a point
    // to the line beside it, if there is one.
    let Click::Double(_) = click else {
        return;
    };
    let Some(translation) = camera.picking.translation else {
        return;
    };
    // The hit is in render space relative to the cell the readback was taken in, which the
    // orbital camera turns into an absolute position the same way.
    let hit = grid.grid_position_double(
        &camera.picking.cell,
        &Transform::from_translation(translation),
    );

    let Some((line, index)) = editor.insert_point(&mut rail.network, hit) else {
        return;
    };

    let point = &mut rail.network.lines[line].points[index];
    match sampler.as_deref_mut() {
        Some(sampler) => sampler.resample(point),
        // The startup sampling has not landed yet. The point resolves from its cached
        // terrain height, which a new point has none of, so it sits on the ellipsoid
        // until it is moved or the file is reloaded.
        None => warn_once!(
            "rail editor: the sampler has not landed; a point added now has no height until \
             the startup sampling is through"
        ),
    }
    info!(
        "rail editor: added point {index} to {} at {:.5}, {:.5}",
        rail.network.lines[line].name,
        rail.network.lines[line].points[index].latitude,
        rail.network.lines[line].points[index].longitude,
    );
    rail.dirty = true;
}

/// The point nearest the cursor on screen, among those within DISC_REACH and in front of
/// the camera, if it is within PICK_PIXELS. The positions are the resolved ones, so this
/// is only right while the network is not dirty, which the caller sees to.
fn nearest_point_on_screen(
    network: &RailNetwork,
    camera: &Camera,
    camera_global: &GlobalTransform,
    cell_origin: DVec3,
    camera_position: DVec3,
    cursor: Vec2,
) -> Option<(usize, usize)> {
    let mut nearest: Option<((usize, usize), f32)> = None;

    for (line_index, line) in network.lines.iter().enumerate() {
        for (index, &position) in line.positions.iter().enumerate() {
            if position.distance(camera_position) > DISC_REACH {
                continue;
            }
            // Err for a point behind the camera, which is the test for that.
            let Ok(on_screen) =
                camera.world_to_viewport(camera_global, (position - cell_origin).as_vec3())
            else {
                continue;
            };
            let distance = on_screen.distance(cursor);
            if distance <= PICK_PIXELS && nearest.is_none_or(|(_, best)| distance < best) {
                nearest = Some(((line_index, index), distance));
            }
        }
    }

    nearest.map(|(point, _)| point)
}

#[cfg(test)]
mod tests;
