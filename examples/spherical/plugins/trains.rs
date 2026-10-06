//! One carriage on each rail line, driving itself along the drawn track end to end and back,
//! so that trains are seen moving over the city before the live positions arrive.
//!
//! A train is a distance along its line's spline and a direction, and nothing more. The
//! driver moves the distance on at a steady speed and turns the train round at either end;
//! the placer asks the spline for the frame at that distance and stands the carriage there,
//! on the left-hand running line as New Zealand's trains keep, facing the way it is going.
//! The two are kept apart so that the driver can be replaced by one that heads for the
//! position Auckland Transport's feed last reported, which is a distance along the line too,
//! without the placing changing.
//!
//! The carriage is a glTF model of one car, spawned once per train as a spatial entity under
//! the big_space root, so that it is placed as the lines are, by a cell and a transform
//! within it, with the model as a child carrying the fixed transform that turns it to face
//! along the frame, scales it to a car's length and lifts it onto the rail. A label with
//! the line's name stands above each carriage on screen, placed as the sheet grid's labels
//! are. F7 hides the lot; the trains drive on hidden, so they come back where they would
//! have been.
//!
//! A table in the bottom left corner lists the trains, where each is along its line and
//! which way it is running, with a camera icon on every row that puts a chase camera behind
//! that train; the table is in table.rs and the camera in chase.rs. Both read the trains
//! and place by carriage_frame, so the table, the camera and the carriage agree.

use super::auckland_rail::{AucklandRail, line_colour};
use super::shared::rail_network::RailLine;
use super::sheet_grid::label_size;
use super::track_frames::{
    TRACK_CENTRES, TrackFrame, TrackSpline, TrackSplines, in_grid, offset, refresh_track_splines,
    reversed,
};
use bevy::{prelude::*, text::FontSize, ui::UiSystems};
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};
use std::f32::consts::FRAC_PI_2;

mod chase;
mod table;

/// How fast a train runs, in metres per second: 72 km/h, a fair average for Auckland's
/// electric units between stops, which are allowed 110 and spend much of a run braking for
/// the next station. This driver has no stops, so the average stands in for the whole run.
pub const TRAIN_SPEED: f64 = 20.0;

/// The length a carriage is drawn at, in metres: an AM class car is about 24 m over its
/// body, and a three-car set 72.
pub const CARRIAGE_LENGTH: f32 = 24.0;

/// The model's bounding box, measured from the file, in its own units: x along the carriage,
/// y up and z across, the origin at its centre. The scale and the lift are derived from it
/// rather than typed in, so a model with another box needs only these two lines changed.
const MODEL_MIN: Vec3 = Vec3::new(-0.955, -0.285, -0.136);
const MODEL_MAX: Vec3 = Vec3::new(0.951, 0.282, 0.132);

/// The one scale on all three axes that makes the model CARRIAGE_LENGTH long. The model is
/// chunkier than the real car, so at a car's length it comes out some seven metres tall and
/// three and a half wide against the real 3.9 and 2.8; a scale per axis would thin it to
/// true and distort whatever the modeller drew on it. Accepted as it is.
const MODEL_SCALE: f32 = CARRIAGE_LENGTH / (MODEL_MAX.x - MODEL_MIN.x);

/// How far the model is lifted along the frame's up, in metres. The model's origin is at its
/// centre, so unlifted its floor would be as far below the rail as its roof is above, and the
/// lift puts its lowest point, the wheels, on the rail.
const MODEL_LIFT: f32 = -MODEL_MIN.y * MODEL_SCALE;

/// How tall the carriage stands above the rail, in metres, which is where its label sits.
const CARRIAGE_HEIGHT: f32 = (MODEL_MAX.y - MODEL_MIN.y) * MODEL_SCALE;

/// The model, under the asset root. One carriage, one mesh, 8 MB, untracked in git.
const MODEL_PATH: &str = "models/Meshy_AI_Auckland_Metro_Glide_0929051957_texture.glb";

pub struct TrainsPlugin;

impl Plugin for TrainsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Trains>()
            .add_plugins((table::TrainsTablePlugin, chase::ChaseCameraPlugin))
            .add_systems(Startup, start_trains)
            .add_systems(
                Update,
                (
                    toggle_trains,
                    spawn_carriages.run_if(carriages_missing),
                    // After the splines are through this frame's line, so a train on a line
                    // just edited drives on the line as it is.
                    drive_trains.after(refresh_track_splines),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    // Before the transforms propagate, so the carriage's global transform
                    // this frame is from where it is this frame: the cell and the transform
                    // written here are absolute, and want nothing of the camera's cell.
                    place_trains.before(TransformSystems::Propagate),
                    // After, as the sheet labels are: the projection reads the camera's
                    // global transform, which propagation has just written. And before the
                    // labels are laid out, as the sheet labels are too, so a label lands
                    // where its carriage is this frame rather than a frame behind it.
                    label_trains
                        .after(TransformSystems::Propagate)
                        .before(UiSystems::Prepare),
                ),
            );
    }
}

/// The trains and the switch that shows them.
#[derive(Resource)]
pub struct Trains {
    /// Toggled by F7. Off, the carriages and labels are hidden and the trains drive on.
    /// Shown only while the lines are, as the frames and the discs are: F4 hides everything
    /// on the network.
    pub shown: bool,
    /// Metres per second, for every train: TRAIN_SPEED.
    pub speed: f64,
    /// One per line, in the network's order, from start_trains.
    pub trains: Vec<Train>,
}

impl Default for Trains {
    fn default() -> Self {
        Self {
            shown: true,
            speed: TRAIN_SPEED,
            trains: Vec::new(),
        }
    }
}

impl Trains {
    /// Whether the carriages are drawn: shown by F7 and the network they run on not hidden
    /// by F4. The labels, the table and the chase camera go with them.
    pub fn drawn(&self, rail: &AucklandRail) -> bool {
        self.shown && rail.visible
    }
}

/// One train: where it is on its line and which way it is going. What a driver sets, this
/// one or a live feed's, and what the placer reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Train {
    /// Index into the network's lines, which the editor never reorders.
    pub line: usize,
    /// Metres along the line's spline from its first point, see TrackSpline::frame_at.
    pub distance: f64,
    /// +1 running in point order, towards the line's last point; -1 running back.
    pub direction: f64,
    /// The carriage, once spawned: a spatial entity under the big_space root, whose cell and
    /// transform the placer writes. None until spawn_carriages has run, and for good where
    /// there is no camera to find the root by or no asset server, as in the headless test.
    pub entity: Option<Entity>,
    /// The label above the carriage, a UI node, spawned with the carriage.
    pub label: Option<Entity>,
}

impl Train {
    /// The line the train runs on, or None should the index name none, which the editor
    /// never arranges: it neither adds lines nor removes them.
    pub fn line<'a>(&self, rail: &'a AucklandRail) -> Option<&'a RailLine> {
        rail.network.lines.get(self.line)
    }

    /// The line's name, for the label and the table, and nothing when there is no line.
    pub fn line_name<'a>(&self, rail: &'a AucklandRail) -> &'a str {
        self.line(rail).map_or("", |line| line.name.as_str())
    }
}

/// The carriage entity, for the placer's query.
#[derive(Component)]
struct Carriage;

/// The label entity, for the labeller's query.
#[derive(Component)]
struct TrainLabel;

/// The model's transform under the carriage: turned so that its +X long axis runs along the
/// frame's -Z forward, scaled to a car's length and lifted onto the rail. A quarter turn
/// about Y takes +X to -Z. Which end of the model is its front the file does not say, so
/// whether the carriage runs nose first is for the eye to settle, and a half turn here is
/// the fix.
fn model_transform() -> Transform {
    Transform {
        translation: Vec3::new(0.0, MODEL_LIFT, 0.0),
        rotation: Quat::from_rotation_y(FRAC_PI_2),
        scale: Vec3::splat(MODEL_SCALE),
    }
}

/// Where a train is after a step: its distance moved on by its speed over the time, in its
/// direction, and reflected at either end of the line so that it never leaves it. The
/// overshoot past an end is folded back, so a train that would have run three metres past
/// the end stands three metres short of it heading back, having gone the whole way. A step
/// longer than the line, as a long pause between frames could give, folds as many times as
/// it must. A distance off the line, as an edit that shortened the line leaves, is brought
/// onto it first, so what comes out is within the line whatever went in. A line of no
/// length, or none, has one place, and the train stands there as it was heading.
pub fn advance(distance: f64, direction: f64, speed: f64, dt: f64, length: f64) -> (f64, f64) {
    let heading = if direction < 0.0 { -1.0 } else { 1.0 };
    if !length.is_finite() || length <= 0.0 {
        return (0.0, heading);
    }
    let distance = if distance.is_nan() {
        0.0
    } else {
        distance.clamp(0.0, length)
    };
    let step = speed * dt;
    if !step.is_finite() {
        return (distance, heading);
    }

    // Unfolded: the run out and back as one lap of twice the length, the way out as the
    // distance itself and the way back as its reflection past the end. The lap wraps, and
    // where in it the train lands says which way it is going.
    let lap = 2.0 * length;
    let along = if heading > 0.0 {
        distance
    } else {
        lap - distance
    };
    let along = (along + step).rem_euclid(lap);

    if along < length {
        (along, 1.0)
    } else {
        (lap - along, -1.0)
    }
}

/// The frame the carriage stands in: the line's frame at the train's distance, turned round
/// when the train is running back so that forward is the way it is going, and moved half
/// the track centres to the left of that, onto the left-hand running line. Turned round,
/// left is the other side of the centre line from the one it ran out on, as a train
/// returning on the other track is.
pub fn carriage_frame(spline: &TrackSpline, train: &Train) -> TrackFrame {
    let frame = spline.frame_at(train.distance);
    let facing = if train.direction < 0.0 {
        reversed(&frame)
    } else {
        frame
    };

    offset(&facing, -TRACK_CENTRES / 2.0)
}

fn toggle_trains(keys: Res<ButtonInput<KeyCode>>, mut trains: ResMut<Trains>) {
    if keys.just_pressed(KeyCode::F7) {
        trains.shown = !trains.shown;
        info!("trains: {}", if trains.shown { "shown" } else { "hidden" });
    }
}

/// One train per line, at the line's first point heading along it. The lines are never
/// added or removed by the editor, so once is enough.
fn start_trains(rail: Res<AucklandRail>, mut trains: ResMut<Trains>) {
    trains.trains = (0..rail.network.lines.len())
        .map(|line| Train {
            line,
            distance: 0.0,
            direction: 1.0,
            entity: None,
            label: None,
        })
        .collect();
}

fn carriages_missing(trains: Res<Trains>) -> bool {
    trains.trains.iter().any(|train| train.entity.is_none())
}

/// Spawns a carriage and a label for every train that has none: on the first frame, once
/// the big_space root the camera was spawned under is there to be found, and for any train
/// a later driver adds. The carriage is a spatial entity in the root grid with the model as
/// its child, hidden until the placer has stood it somewhere; the label is a UI node with
/// the line's name in the line's colour, hidden until it is placed too.
fn spawn_carriages(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    rail: Res<AucklandRail>,
    mut trains: ResMut<Trains>,
    grids: Grids,
    camera: Query<Entity, With<OrbitalCameraController>>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let Some(root) = grids.parent_grid_entity(camera) else {
        return;
    };
    let scene = asset_server.load(GltfAssetLabel::Scene(0).from_asset(MODEL_PATH));

    let mut spawned = 0;
    for train in trains
        .trains
        .iter_mut()
        .filter(|train| train.entity.is_none())
    {
        let Some(line) = rail.network.lines.get(train.line) else {
            continue;
        };

        let carriage = commands
            .spawn((
                Carriage,
                CellCoord::default(),
                Transform::default(),
                Visibility::Hidden,
                ChildOf(root),
            ))
            .with_child((WorldAssetRoot(scene.clone()), model_transform()))
            .id();
        let label = commands
            .spawn((
                TrainLabel,
                Text::new(line.name.clone()),
                TextColor(line_colour(&line.name)),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
                Node {
                    position_type: PositionType::Absolute,
                    padding: UiRect::axes(Val::Px(4.0), Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                Visibility::Hidden,
            ))
            .id();

        train.entity = Some(carriage);
        train.label = Some(label);
        spawned += 1;
    }

    if spawned > 0 {
        info!("trains: {spawned} carriages spawned, {CARRIAGE_LENGTH} m long at {TRAIN_SPEED} m/s");
    }
}

/// Moves every train on by the frame's time at its speed, turning it round at either end of
/// its line. A train whose line has no spline this frame, as on the frame after an insert
/// or a delete, stays where it is; one whose line has shortened under it is brought back
/// onto it, see advance.
fn drive_trains(time: Res<Time>, splines: Res<TrackSplines>, mut trains: ResMut<Trains>) {
    let speed = trains.speed;
    let dt = time.delta_secs_f64();

    for train in &mut trains.trains {
        let Some(spline) = splines.splines.get(train.line).and_then(Option::as_ref) else {
            continue;
        };
        (train.distance, train.direction) =
            advance(train.distance, train.direction, speed, dt, spline.length());
    }
}

/// Stands every carriage at its train's place on the line, see carriage_frame, as the cell
/// and the transform within it that big_space places it by. Hidden while the trains are off
/// or the lines are, and while its line has no spline to stand on.
fn place_trains(
    trains: Res<Trains>,
    rail: Res<AucklandRail>,
    splines: Res<TrackSplines>,
    grids: Grids,
    mut carriages: Query<(&mut CellCoord, &mut Transform, &mut Visibility), With<Carriage>>,
) {
    let shown = trains.drawn(&rail);

    for train in &trains.trains {
        let Some(entity) = train.entity else {
            continue;
        };
        let Ok((mut cell, mut transform, mut visibility)) = carriages.get_mut(entity) else {
            continue;
        };
        let spline = splines.splines.get(train.line).and_then(Option::as_ref);
        let (Some(spline), Some(grid), true) = (spline, grids.parent_grid(entity), shown) else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };

        let (placed_cell, placed_transform) = in_grid(&carriage_frame(spline, train), grid);
        cell.set_if_neq(placed_cell);
        transform.set_if_neq(placed_transform);
        visibility.set_if_neq(Visibility::Visible);
    }
}

/// Puts each train's label above its carriage on screen, as the sheet grid's labels are
/// placed: the middle of the roof projected through the camera, and the node's bottom edge
/// at it. Hidden when the carriage is, when the roof is off the screen or behind the camera,
/// and when it is over the horizon, where the planet hides the carriage but a node would
/// show regardless.
fn label_trains(
    trains: Res<Trains>,
    rail: Res<AucklandRail>,
    splines: Res<TrackSplines>,
    grids: Grids,
    camera: Query<
        (Entity, &Camera, &GlobalTransform, &Transform, &CellCoord),
        With<OrbitalCameraController>,
    >,
    mut labels: Query<(&mut Node, &mut Visibility), With<TrainLabel>>,
) {
    let shown = trains.drawn(&rail);

    // The camera as the projection wants it, and None for a frame or two at startup, before
    // the window has told the camera its size. As the lines: positions are absolute and
    // large, the projection wants them relative to the camera's cell, and the subtraction
    // has to happen in f64.
    let view = camera
        .single()
        .ok()
        .and_then(|(entity, camera, global, transform, cell)| {
            let grid = grids.parent_grid(entity)?;
            let viewport = camera.logical_viewport_size()?;
            let cell_origin = grid.cell_to_float(cell);
            let camera_position = grid.grid_position_double(cell, transform);
            Some((camera, global, viewport, cell_origin, camera_position))
        });

    for train in &trains.trains {
        let Some(label) = train.label else {
            continue;
        };
        let Ok((mut node, mut visibility)) = labels.get_mut(label) else {
            continue;
        };
        let spline = splines.splines.get(train.line).and_then(Option::as_ref);
        let (Some((camera, global, viewport, cell_origin, camera_position)), Some(spline), true) =
            (view, spline, shown)
        else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };

        let frame = carriage_frame(spline, train);
        let roof = frame.position + frame.up() * CARRIAGE_HEIGHT as f64;
        // Over the horizon when the camera is below the plane tangent to the spheroid at the
        // roof, as the lines test their centre.
        let over_horizon = (camera_position - roof).dot(roof) < 0.0;
        let on_screen = camera
            .world_to_viewport(global, (roof - cell_origin).as_vec3())
            .ok()
            .filter(|at| at.cmpge(Vec2::ZERO).all() && at.cmple(viewport).all());

        match on_screen.filter(|_| !over_horizon) {
            Some(at) => {
                let size = label_size(train.line_name(&rail));
                node.left = Val::Px(at.x - size.x / 2.0);
                node.top = Val::Px(at.y - size.y);
                visibility.set_if_neq(Visibility::Visible);
            }
            None => {
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
    }
}

#[cfg(test)]
mod tests;
