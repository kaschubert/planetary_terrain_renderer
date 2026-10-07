//! A transform every few metres along each rail line, for the track models to stand on, and
//! the F6 view that draws them so they can be checked by eye.
//!
//! A model placer wants a transform every piece length, not a polyline. The points of a
//! line are tens of metres apart and a polyline through them kinks at every one, so a
//! Catmull-Rom spline is run through the resolved positions, every point of the line
//! including those on the chord of a tunnel or a bridge, and resampled at equal distance
//! along it. Every sample is a frame: a position in absolute metres, a rotation that faces
//! along the line and stands up from the ground, the distance along the line and the grade
//! there. The frames are pure geometry over a resolved line, so they are tested with
//! numbers; the view at the end of the file is the only Bevy in it.
//!
//! The rotation follows Bevy's convention for a thing that faces along its path: local -Z
//! is forward along the line in point order, local Y is up and local X is right. Forward is
//! the spline's tangent, right is forward across the ground's normal and so lies level, and
//! up is right across forward: a piece pitches with the grade and never banks. The rail is
//! superelevated on curves in reality, the outer rail raised so that a train leans into the
//! turn, but at the scale these models are seen at a level piece is the right one, and a
//! banked one would need the curve's radius, which nothing here has.
//!
//! The spline is in spline.rs, and TrackSpline wraps it with its measured length so that a
//! frame can be had at any distance along a line, not only at the resampler's steps: the
//! trains in trains.rs drive on it, asking for the frame at where they have got to. The
//! resource TrackSplines keeps one per line, rebuilt whenever the network changes, and the
//! resampled frames are made from it rather than from a spline of their own, so the two
//! agree to the millimetre. A frame is handed to big_space through in_grid, as the cell
//! its position falls in and a Transform within that cell, which is what an entity spawned
//! under the big_space root carries; move_to writes a pose into those two on an entity
//! already spawned, which is how the view entity is moved.

use super::auckland_rail::{AucklandRail, line_colour, resolve_rail_heights};
use super::rail_editor::frame::{Frame, geodetic_normal, unit_under};
use super::shared::rail_network::RailLine;
use super::sheet_grid::IN_FRONT_OF_TERRAIN;
use bevy::{
    math::{DMat3, DQuat, DVec3},
    prelude::*,
};
use bevy_terrain::prelude::*;
use big_space::{
    grid::Grid,
    prelude::{CellCoord, Grids},
};
use spline::{ArcLength, CatmullRom};

mod spline;

/// The distance between frames, in metres: a plausible length for a model of a track piece.
/// 130 km of line is some 5200 of them, and a straight piece this long across a curve of a
/// few hundred metres' radius, as tight as the network gets, leaves the curve by under half
/// a metre midway, which at the distance the models are seen from is not seen. The spawner
/// can pass another.
pub const FRAME_STEP: f64 = 25.0;

/// The distance between the two running lines of a double track, centre to centre, in
/// metres. Auckland's double track sits about four metres between centres; the figure is a
/// placeholder until the models say what they were built to.
pub const TRACK_CENTRES: f64 = 4.0;

/// Pieces each spline segment is cut into to measure its length. The curve between two cuts
/// is taken as its chord, which falls short of the arc by the cube of the piece's length
/// over twenty-four times the square of the curve's radius: nothing where points are tens
/// of metres apart, and on a segment of a kilometre, whose end tangents are set by points
/// far away and which bends gently, a couple of centimetres, or a few decimetres where its
/// neighbours turn it hard, which no model placed to the metre shows. The table only
/// brackets a distance; the parameter within a piece is found by Newton's method, not by
/// interpolation, since the speed along a segment is far from steady where the spacing
/// changes. See ArcLength::parameter.
const SUBDIVISIONS: usize = 16;

/// One place along a line for a model to stand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackFrame {
    /// On the spline through the resolved points, in absolute WGS84 metres: the space the
    /// lines' positions are in, which in_grid splits for big_space.
    pub position: DVec3,
    /// Local -Z forward along the line in point order, local Y up and local X right; see
    /// the module doc for why right lies level.
    pub rotation: DQuat,
    /// Metres along the spline from the line's first point.
    pub distance: f64,
    /// The grade at the sample, rise over run, positive climbing in point order: the climb in
    /// height above the ellipsoid over the distance along the ground, as the editor's segment
    /// grades are, so a stretch the editor colours steep is steep here too.
    pub grade: f64,
}

impl TrackFrame {
    /// Along the line in point order: local -Z.
    pub fn forward(&self) -> DVec3 {
        self.rotation * DVec3::NEG_Z
    }

    /// Local Y: the ground's normal tipped by the grade.
    pub fn up(&self) -> DVec3 {
        self.rotation * DVec3::Y
    }

    /// Local X, to the right of the direction of travel and level.
    pub fn right(&self) -> DVec3 {
        self.rotation * DVec3::X
    }
}

/// A line's spline with its length measured, so that the frame at any distance along the
/// line can be had: what the resampler walks at its step and what a train drives on.
#[derive(Debug, Clone)]
pub struct TrackSpline {
    spline: CatmullRom,
    lengths: ArcLength,
    /// The position at every subdivision the length was measured over, with its distance
    /// along, see nearest_between.
    samples: Vec<(DVec3, f64)>,
}

impl TrackSpline {
    /// The spline through the line's resolved positions, every point of the line including
    /// those on the chord of a tunnel or a bridge.
    ///
    /// None for a line of fewer than two distinct points, which has no direction, and for a
    /// line whose positions are not resolved: on the frame after an insert or a delete the
    /// positions are a point behind the points, until resolve_rail_heights runs, and a
    /// spline through them would be through the line as it was.
    pub fn through(line: &RailLine) -> Option<Self> {
        if line.positions.len() != line.points.len() {
            return None;
        }
        let spline = CatmullRom::through(&line.positions)?;
        let lengths = spline.measure(SUBDIVISIONS);
        let samples = lengths.samples(&spline);

        Some(Self {
            spline,
            lengths,
            samples,
        })
    }

    /// The length of the line along the spline, in metres: the distance of the last point.
    pub fn length(&self) -> f64 {
        self.lengths.total()
    }

    /// The distance along the line of the point on it nearest a position, and how far the
    /// position stands from the line there: nearest_between over the whole line.
    pub fn nearest(&self, position: DVec3) -> (f64, f64) {
        self.nearest_between(position, 0.0, self.length())
            .unwrap_or((0.0, f64::INFINITY))
    }

    /// The distance along the line of the point on it nearest a position, looked for over
    /// the stretch between two distances along the line, and how far the position stands
    /// from the line there, level: the offset with its part along the ground's up taken
    /// out, so a position on the ellipsoid under a track 40 m up the hill is on the line
    /// and not 40 m off it. A train's reported position asks, with the metres of error a
    /// GPS fix carries, and a line that passes a place twice, as the S-C does at Newmarket,
    /// has two answers there, which is what the stretch is for: the caller knows where the
    /// train was.
    ///
    /// The line is walked as the chords between the subdivisions its length was measured
    /// over, a few metres each, widened by one subdivision either side of the stretch so
    /// its ends are covered, and the position is dropped onto each; the distance is the
    /// two subdivisions' interpolated by where it lands, which is the distance the table
    /// would give that point. None for a stretch with no two subdivisions in it, which is
    /// one wholly off either end of the line.
    pub fn nearest_between(&self, position: DVec3, from: f64, to: f64) -> Option<(f64, f64)> {
        let last = self.samples.len().checked_sub(1)?;
        let first = self
            .samples
            .partition_point(|&(_, distance)| distance < from)
            .saturating_sub(1);
        let after = self
            .samples
            .partition_point(|&(_, distance)| distance <= to)
            .min(last);
        if after <= first {
            return None;
        }

        let mut best: Option<(f64, f64)> = None;
        for pair in self.samples[first..=after].windows(2) {
            let ((start, from_distance), (end, to_distance)) = (pair[0], pair[1]);
            let chord = end - start;
            let length_squared = chord.length_squared();
            let fraction = if length_squared > 0.0 {
                ((position - start).dot(chord) / length_squared).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let foot = start + chord * fraction;
            let up = Frame::at_unit(unit_under(foot)).up;
            let off = (position - foot).reject_from(up).length();

            if best.is_none_or(|(_, nearest)| off < nearest) {
                best = Some((
                    from_distance + (to_distance - from_distance) * fraction,
                    off,
                ));
            }
        }

        best
    }

    /// The frame at a distance along the line from its first point, the distance clamped to
    /// the line, so that a train run past either end stands at that end. Found through the
    /// table and Newton's method as the resampler's frames are, from a cursor sought for
    /// this distance alone, so the frame at a resampler's distance is the resampler's frame.
    pub fn frame_at(&self, distance: f64) -> TrackFrame {
        let distance = if distance.is_nan() {
            0.0
        } else {
            distance.clamp(0.0, self.length())
        };
        let mut cursor = self.lengths.seek(distance);
        let t = self.lengths.parameter(&self.spline, distance, &mut cursor);

        frame_at(&self.spline, t, distance)
    }

    /// The frames every step metres from the first point, the first at the first point. The
    /// last regular frame falls short of the last point by whatever the length does not
    /// divide into; when that is more than half a step a final frame is placed at the last
    /// point, so a piece stands at the end of the line unless one already nearly does.
    /// Empty for a step that is not a positive number, rather than a frame at every point
    /// or never ending.
    pub fn resample(&self, step: f64) -> Vec<TrackFrame> {
        if step.is_nan() || step <= 0.0 {
            return Vec::new();
        }
        let total = self.length();

        let mut frames = Vec::with_capacity((total / step) as usize + 2);
        let mut cursor = 0;
        let mut distance = 0.0;
        while distance <= total {
            let t = self.lengths.parameter(&self.spline, distance, &mut cursor);
            frames.push(frame_at(&self.spline, t, distance));
            distance = frames.len() as f64 * step;
        }

        if frames
            .last()
            .is_some_and(|last| total - last.distance > step / 2.0)
        {
            frames.push(frame_at(&self.spline, self.spline.end(), total));
        }

        frames
    }
}

/// The frames along a line, every step metres from its first point, in one call: the line's
/// spline resampled, see TrackSpline::resample, and empty where the line has no spline, see
/// TrackSpline::through. The view resamples the splines the resource holds instead, so only
/// the tests, which build a line and want its frames, call this.
#[cfg(test)]
pub fn track_frames(line: &RailLine, step: f64) -> Vec<TrackFrame> {
    TrackSpline::through(line).map_or_else(Vec::new, |spline| spline.resample(step))
}

/// The frame at a parameter of the spline, as the module doc has it. The ground's frame is
/// taken at the sample itself, not at the nearest point, so a long straight between two
/// points still stands square to the ground along its length. Where the tangent is
/// nothing, which two coincident points could make and the spline leaves none of, or stands
/// straight up, which rail does not, the frame faces east rather than carry a NaN.
///
/// The rotation stands on the frame's up, as the gizmo does, and the grade is measured
/// against the geodetic normal instead, the true vertical: the fifth of a degree between the
/// two, see frame.rs, is nothing in a model's stance but would read as a grade of a third of
/// a per cent on a level stretch running north, where the editor's segment grades, from the
/// heights, read nothing.
fn frame_at(spline: &CatmullRom, t: f64, distance: f64) -> TrackFrame {
    let position = spline.position(t);
    let unit = unit_under(position);
    let ground = Frame::at_unit(unit);

    let forward = spline.tangent(t).try_normalize().unwrap_or(ground.east);
    let right = forward
        .cross(ground.up)
        .try_normalize()
        .unwrap_or(ground.east);
    let up = right.cross(forward);
    let rotation = DQuat::from_mat3(&DMat3::from_cols(right, up, -forward));

    let vertical = geodetic_normal(unit);
    let rise = forward.dot(vertical);
    let run = (forward - vertical * rise).length();
    let grade = if run > 0.0 {
        rise / run
    } else {
        f64::INFINITY.copysign(rise)
    };

    TrackFrame {
        position,
        rotation,
        distance,
        grade,
    }
}

/// The frame shifted sideways along its right axis by the metres given, positive to the
/// right of the direction of travel, with the rotation, distance and grade as they were.
/// The two running lines of a double track are offset(frame, -TRACK_CENTRES / 2.0) and
/// offset(frame, TRACK_CENTRES / 2.0), left and right of the line.
pub fn offset(frame: &TrackFrame, metres: f64) -> TrackFrame {
    TrackFrame {
        position: frame.position + frame.right() * metres,
        ..*frame
    }
}

/// The frame turned to face the other way along the line: the rotation turned half a turn
/// about its up, so forward is back along the line in point order, right is what was left,
/// and up is as it was; the distance stays, and the grade changes sign, since what climbed
/// in point order descends against it. A train running back towards a line's first point
/// stands in this frame, and offset's right then means the other side of the track.
pub fn reversed(frame: &TrackFrame) -> TrackFrame {
    TrackFrame {
        rotation: DQuat::from_axis_angle(frame.up(), std::f64::consts::PI) * frame.rotation,
        grade: -frame.grade,
        ..*frame
    }
}

/// The frame as the pair a spawner puts on an entity spawned under the big_space root: the
/// cell its position falls in and a Transform within that cell, with the rotation and a
/// scale of one. The position is split into cell and remainder in f64 and only the
/// remainder, under a cell's width, is narrowed to f32, so the metre and below survive where
/// the absolute position, six million metres out, would keep half a metre at best. The
/// trains are the caller.
pub fn in_grid(frame: &TrackFrame, grid: &Grid) -> (CellCoord, Transform) {
    let (cell, translation) = grid.translation_to_grid(frame.position);

    (
        cell,
        Transform {
            translation,
            rotation: frame.rotation.as_quat(),
            scale: Vec3::ONE,
        },
    )
}

/// Moves an entity spawned under the big_space root to a pose, writing its cell and the
/// transform within it: the position split in f64 as in_grid splits a frame's, the rotation
/// as given and the scale left alone. The editor's fly-to and the trains' chase camera move
/// the view entity with it, whose orbital camera reads its pose back from these two
/// components every frame and carries on from whatever it finds, so writing them is all a
/// move takes.
pub fn move_to(
    grid: &Grid,
    cell: &mut CellCoord,
    transform: &mut Transform,
    position: DVec3,
    rotation: Quat,
) {
    let (new_cell, translation) = grid.translation_to_grid(position);
    *cell = new_cell;
    transform.translation = translation;
    transform.rotation = rotation;
}

/// The splines of every line, kept current, and the F6 view: the frames of every line, drawn
/// as arrows near the camera.
pub struct TrackFramesPlugin;

/// The frames' own gizmo settings, in front of the terrain as the lines are.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct TrackFrameGizmos;

impl Plugin for TrackFramesPlugin {
    fn build(&self, app: &mut App) {
        app.init_gizmo_group::<TrackFrameGizmos>()
            .init_resource::<TrackSplines>()
            .init_resource::<TrackFrames>()
            .add_systems(Startup, configure_frame_gizmos)
            .add_systems(
                Update,
                // After the positions have been recomputed, so the splines are through this
                // frame's line; see refresh_track_frames for why there and not before. The
                // frames come after the splines, being resampled from them.
                (
                    toggle_track_frames,
                    refresh_track_splines,
                    refresh_track_frames,
                )
                    .chain()
                    .after(resolve_rail_heights),
            )
            .add_systems(
                PostUpdate,
                // After the floating origin has settled on this frame's cell, as the lines.
                draw_track_frames.after(TransformSystems::Propagate),
            );
    }
}

/// How far from the camera a frame is drawn, in metres. 130 km of line at FRAME_STEP is some
/// 5200 frames, each a dozen gizmo lines; within five kilometres, where a ten metre arrow is
/// still pixels wide, there are a few hundred at most, and the rest are too small to read.
/// The reach also keeps a frame from showing through the planet: within it the horizon
/// drops two metres, which a camera on the ground cannot get under.
const DRAW_REACH: f64 = 5_000.0;

/// The arrow along forward, in metres: well short of a step, so neighbours stay apart, and
/// long enough to read from a few hundred metres up.
const ARROW_LENGTH: f64 = 10.0;

/// The tick along up, in metres: a third of the arrow, so the two read as different things
/// and the tick does not reach the next frame's arrow from a camera above, and tall enough
/// that a lean of a few degrees shows against its neighbours from a few hundred metres
/// away. White so it reads against any line's colour.
const UP_TICK: f64 = 3.0;

/// The line across the track joining the two running lines, in a grey that stays behind the
/// colours, so the width between them reads as a width.
const SLEEPER_COLOUR: Color = Color::srgb(0.6, 0.6, 0.6);

/// The spline of every line, for the trains to drive on and the frames to be resampled
/// from. Always current, whether or not anything is shown: the trains drive whether or not
/// they are, and a spline is a few thousand segments, nothing to rebuild on an edit.
#[derive(Resource, Default)]
pub struct TrackSplines {
    /// One per line of the network, in the network's order, None for a line that has no
    /// spline; see TrackSpline::through.
    pub splines: Vec<Option<TrackSpline>>,
}

/// The frames of every line, for the F6 view and for whatever else wants them without
/// computing its own.
#[derive(Resource)]
pub struct TrackFrames {
    /// Toggled by F6. Off, nothing here is computed or drawn. Drawn only while the lines
    /// are, as the discs and the handle are: F4 hides everything on the network.
    pub shown: bool,
    /// The distance between frames, FRAME_STEP.
    pub step: f64,
    /// One Vec per line of the network, in the network's order: the line's spline resampled
    /// at the step, see TrackSpline::resample, and empty for a line that has no spline; see
    /// TrackSpline::through.
    pub frames: Vec<Vec<TrackFrame>>,
    /// Set when the network has changed since the frames were computed, and when the view
    /// comes on; cleared by the recompute.
    pub stale: bool,
}

impl Default for TrackFrames {
    fn default() -> Self {
        Self {
            shown: false,
            step: FRAME_STEP,
            frames: Vec::new(),
            stale: true,
        }
    }
}

fn configure_frame_gizmos(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<TrackFrameGizmos>().0.depth_bias = IN_FRONT_OF_TERRAIN;
}

fn toggle_track_frames(keys: Res<ButtonInput<KeyCode>>, mut frames: ResMut<TrackFrames>) {
    if keys.just_pressed(KeyCode::F6) {
        frames.shown = !frames.shown;
        // Nothing was recomputed while hidden, so the first frame shown recomputes.
        frames.stale = true;
        info!(
            "track frames: {}",
            if frames.shown { "shown" } else { "hidden" }
        );
    }
}

/// Rebuilds the splines when the network changed, on the first frame and after every edit.
///
/// The network is dirty for a frame whenever a point or its sample changes, and the flag
/// is set in several places across two schedules: the editor's keys and mouse in
/// PostUpdate, the drag, the panel's buttons and the sampler landing in Update. A system
/// that watched the flag would have to stand after all of them and before the resolve that
/// clears it. After the resolve there is one place to stand, and change detection on the
/// resource reports every write to it, which is every set of the flag and the resolve
/// itself, plus the F4 toggle, which costs a rebuild of three splines that was not needed
/// and nothing more. Public so the trains can order their driving after it.
pub fn refresh_track_splines(rail: Res<AucklandRail>, mut splines: ResMut<TrackSplines>) {
    if !rail.is_changed() {
        return;
    }
    splines.splines = rail
        .network
        .lines
        .iter()
        .map(TrackSpline::through)
        .collect();
}

/// Marks the frames stale when the network changed, and resamples them from the splines
/// while shown; see refresh_track_splines for why the change is noticed here.
fn refresh_track_frames(
    rail: Res<AucklandRail>,
    splines: Res<TrackSplines>,
    mut frames: ResMut<TrackFrames>,
) {
    if rail.is_changed() && !frames.stale {
        frames.stale = true;
    }
    if !frames.shown || !frames.stale {
        return;
    }

    let step = frames.step;
    let before: usize = frames.frames.iter().map(Vec::len).sum();
    frames.frames = splines
        .splines
        .iter()
        .map(|spline| {
            spline
                .as_ref()
                .map_or_else(Vec::new, |spline| spline.resample(step))
        })
        .collect();
    frames.stale = false;

    // A drag recomputes every frame of the drag and moves the frames without changing
    // their number, so only a change of count is worth a line in the log.
    let after: usize = frames.frames.iter().map(Vec::len).sum();
    if after != before {
        info!(
            "track frames: {after} frames over {} lines, every {step} m",
            frames.frames.len()
        );
    }
}

/// Every frame within DRAW_REACH of the camera: an arrow along forward in its line's colour,
/// a tick up in white, a grey sleeper across the track, and the two running lines at
/// TRACK_CENTRES apart, each run from frame to frame as a chord in the line's colour, so the
/// track reads as two lines and not as marks, and so what is drawn is where the models'
/// rails will meet. Drawn in render space, which is absolute space shifted so the camera's
/// cell is at the origin, with the shift done in f64 before narrowing, as the lines are;
/// nothing is allocated per frame.
fn draw_track_frames(
    mut gizmos: Gizmos<TrackFrameGizmos>,
    frames: Res<TrackFrames>,
    rail: Res<AucklandRail>,
    grids: Grids,
    camera: Query<(Entity, &Transform, &CellCoord), With<OrbitalCameraController>>,
) {
    if !frames.shown || !rail.visible {
        return;
    }
    let Ok((camera, camera_transform, camera_cell)) = camera.single() else {
        return;
    };
    let Some(grid) = grids.parent_grid(camera) else {
        return;
    };
    let cell_origin = grid.cell_to_float(camera_cell);
    let camera_position = grid.grid_position_double(camera_cell, camera_transform);
    let half_gauge = TRACK_CENTRES / 2.0;

    for (line, frames) in rail.network.lines.iter().zip(&frames.frames) {
        let colour = line_colour(&line.name);

        for (index, frame) in frames.iter().enumerate() {
            if frame.position.distance(camera_position) > DRAW_REACH {
                continue;
            }
            let render = |position: DVec3| (position - cell_origin).as_vec3();
            let origin = render(frame.position);

            gizmos.arrow(
                origin,
                render(frame.position + frame.forward() * ARROW_LENGTH),
                colour,
            );
            gizmos.line(
                origin,
                render(frame.position + frame.up() * UP_TICK),
                Color::WHITE,
            );

            // The running lines as the spawner will place them, through offset, each joined
            // to its place at the next frame. A chord of a step across the network's
            // tightest curve leaves it by under half a metre, see FRAME_STEP.
            let left = offset(frame, -half_gauge).position;
            let right = offset(frame, half_gauge).position;
            gizmos.line(render(left), render(right), SLEEPER_COLOUR);
            if let Some(next) = frames.get(index + 1) {
                gizmos.line(
                    render(left),
                    render(offset(next, -half_gauge).position),
                    colour,
                );
                gizmos.line(
                    render(right),
                    render(offset(next, half_gauge).position),
                    colour,
                );
            }
        }
    }
}

// Visible to the trains' tests, which borrow its synthetic lines.
#[cfg(test)]
pub(crate) mod tests;
