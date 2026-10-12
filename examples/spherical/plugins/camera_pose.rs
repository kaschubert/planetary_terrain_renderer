//! Where the camera starts, a button to put it back there, and a pose kept on disk.
//!
//! Flying somewhere worth looking at takes a while, and until now a rebuild threw it away:
//! every run opened over Auckland whatever you were working on. So there are two buttons in
//! the F1 list — one that writes down where the camera is, one that puts it back there — and
//! that saved pose is also where the next run opens.
//!
//! Both the button and the startup go through [`home_pose`], which reads the file each time:
//! one place decides where home is, and a pose edited in the file by hand can be jumped to
//! with the button rather than a restart. With no file written yet, home is the start below.
//!
//! The buttons sit in a row the F1 list spawns and leaves empty, which this fills, as the F2
//! panel does with the sheet grid's height controls. They are inert until F1 is up: Bevy's
//! UI picking skips a node that is not rendered, so a hidden list neither takes clicks nor
//! keeps them from the camera.
//!
//! Moving the camera is two components, [`Transform`] and [`CellCoord`], written through
//! [`move_to`]; the orbital controller reads its pose back from them every frame and carries
//! on from whatever it finds, so writing them is all a move takes. The file is the
//! arrangement the photo markers use for theirs: RON beside the plugin in the source tree,
//! written through the same atomic rename.

use bevy::{math::DVec3, prelude::*, text::FontSize};
use bevy_terrain::math::{Coordinate, unit_position};
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::plugins::auckland_rail::write_atomically;
use crate::plugins::photo_markers::panel::{BUTTON_COLOUR, button_colour};
use crate::plugins::shared::rail_network::utc_now;
use crate::plugins::track_frames::move_to;

/// Where the camera starts: over Auckland, looking north over the harbour from the city
/// centre, with West and South Auckland to either side. Where home is until a pose has been
/// written down, and what a rotation the file cannot hold falls back to.
const START_LONGITUDE: f64 = 174.7633;
const START_LATITUDE: f64 = -36.8485;
const START_ALTITUDE: f32 = 1000.0;

/// Where the saved pose lives: beside the plugin, in the source tree, as the photo markers'
/// file does, so that a view found while flying around is still there after a rebuild.
const POSE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/examples/spherical/plugins/camera_pose.ron"
);

/// The pose that looks north over the ground from the start, as a position and a rotation.
///
/// Built the way the rail editor's fly-to builds its pose: up is the camera's direction on
/// the unit sphere and north the meridian's direction there, the part of the planet's axis
/// that is level at up.
fn start_pose() -> (DVec3, Quat) {
    let up = unit_position(START_LONGITUDE, START_LATITUDE);
    let north = (DVec3::Y - up * up.y).normalize();

    let position = Coordinate::from_unit_position(up, true)
        .local_position(TerrainShape::WGS84, START_ALTITUDE);
    // Tilted halfway between straight down and the horizon, facing north over the harbour.
    let rotation = Transform::IDENTITY
        .looking_to((north - up).normalize().as_vec3(), up.as_vec3())
        .rotation;

    (position, rotation)
}

/// Where home is: the saved pose if there is one that reads, and the start otherwise.
///
/// Both the example's setup, before the view is spawned, and the reset button come through
/// here. The file is read on every call rather than kept in a resource, so that the button
/// picks up a pose edited by hand since the run began.
pub fn home_pose() -> (DVec3, Quat) {
    pose_at(Path::new(POSE_PATH))
}

fn pose_at(path: &Path) -> (DVec3, Quat) {
    match load_pose(path) {
        Some(saved) => {
            info!(
                "camera pose: {:.4}, {:.4} from {:.0} m, saved {}",
                saved.longitude, saved.latitude, saved.altitude, saved.saved_at
            );
            saved.pose()
        }
        None => {
            info!("camera pose: nothing written down, so the start over Auckland");
            start_pose()
        }
    }
}

/// The pose as the file holds it.
///
/// Longitude, latitude and altitude rather than the position in metres, because those are
/// the numbers a person can read and edit, as the markers' file keeps them and as the start
/// above is written. The rotation stays a quaternion: it is four numbers either way, and a
/// yaw and pitch pair would have to settle an order and would lose the roll of a rotated
/// view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SavedPose {
    /// For a reader of the file. Not read back.
    saved_at: String,
    longitude: f64,
    latitude: f64,
    /// Metres above the ellipsoid.
    altitude: f64,
    /// x, y, z, w.
    rotation: [f32; 4],
}

impl SavedPose {
    fn of(position: DVec3, rotation: Quat, saved_at: String) -> Self {
        let (unit, altitude) = unit_and_altitude(position);

        Self {
            saved_at,
            // The inverse of unit_position, as the photo markers take theirs.
            longitude: unit.z.atan2(-unit.x).to_degrees(),
            latitude: unit.y.asin().to_degrees(),
            altitude,
            rotation: rotation.to_array(),
        }
    }

    fn pose(&self) -> (DVec3, Quat) {
        let unit = unit_position(self.longitude, self.latitude);
        let position = TerrainShape::WGS84.position_unit_to_local(unit, self.altitude);
        let [x, y, z, w] = self.rotation;
        let written = Quat::from_xyzw(x, y, z, w);
        let length = written.length_squared();

        // A hand-edited file can hold four zeros, or a typo. A rotation that is not a unit
        // quaternion skews everything drawn through the camera, so one that cannot be
        // normalized gives way to the start's.
        let rotation = match length.is_finite() && length > f32::EPSILON {
            true => written.normalize(),
            false => {
                warn!(
                    "camera pose: {:?} is not a rotation, using the start's",
                    self.rotation
                );
                start_pose().1
            }
        };

        (position, rotation)
    }
}

/// Splits a position into the unit position under it and its height above the ellipsoid.
///
/// The exact inverse of [`TerrainShape::position_unit_to_local`], worked out here rather
/// than taken from [`TerrainShape::position_local_to_unit`], which is not it. The forward
/// function adds the height along `normalize(scale * unit)`, while that one projects onto
/// the spheroid along the true geodetic normal, and on an oblate spheroid those differ by
/// about a fifth of a degree — enough to put a saved pose three metres from where it was
/// written at a kilometre up, and further the higher the camera.
///
/// What makes an exact inverse available is that the forward function leaves its result
/// parallel to `scale * unit`: adding `h * normalize(scale * unit)` to `scale * unit` scales
/// it by `1 + h / |scale * unit|` and turns it not at all. So dividing out the scale and
/// normalizing recovers the unit position, and the height is what the length gained. Signed,
/// so a camera below the ellipsoid reads as negative rather than as the same height above it.
fn unit_and_altitude(position: DVec3) -> (DVec3, f64) {
    let scale = TerrainShape::WGS84.scale();
    let unit = (position / scale).normalize();

    (unit, position.length() - (scale * unit).length())
}

/// The pose in the file, if there is one that reads. A file that is not there is no pose and
/// no complaint: nothing has been saved yet.
fn load_pose(path: &Path) -> Option<SavedPose> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            warn!("camera pose: {}: {error}", path.display());
            return None;
        }
    };

    ron::from_str::<SavedPose>(&text)
        .inspect_err(|error| warn!("camera pose: {}: {error}", path.display()))
        .ok()
}

/// Writes the pose down, stamped with the time.
///
/// A file that would not read is written over here, where the markers' save refuses to: this
/// is one pose rather than a collection, so there is nothing in the old file worth keeping
/// that the new one does not say better.
fn save_pose(position: DVec3, rotation: Quat) {
    let path = Path::new(POSE_PATH);
    let pose = SavedPose::of(position, rotation, utc_now());

    let text = match ron::ser::to_string_pretty(&pose, default()) {
        Ok(text) => text,
        Err(error) => {
            warn!("camera pose: {error}");
            return;
        }
    };

    match write_atomically(path, &text) {
        Ok(()) => info!(
            "camera pose: {:.4}, {:.4} from {:.0} m written to {}",
            pose.longitude,
            pose.latitude,
            pose.altitude,
            path.display()
        ),
        Err(error) => warn!("camera pose: {}: {error}", path.display()),
    }
}

/// The row the F1 list leaves for these buttons. The list spawns it, this fills it.
#[derive(Component)]
pub struct CameraButtonRow;

#[derive(Component, Clone, Copy, PartialEq, Debug)]
struct CameraButton(Action);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Action {
    Reset,
    Save,
}

impl Action {
    fn caption(self) -> &'static str {
        match self {
            Self::Reset => "Reset camera",
            Self::Save => "Save this view",
        }
    }
}

/// The two buttons, the moment the F1 list spawns the row.
///
/// Keyed off the row appearing rather than ordered against the list's own startup system, so
/// that neither has to know when the other runs.
fn spawn_buttons(mut commands: Commands, rows: Query<Entity, Added<CameraButtonRow>>) {
    for row in &rows {
        commands.entity(row).with_children(|row| {
            for action in [Action::Reset, Action::Save] {
                row.spawn((
                    CameraButton(action),
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                        ..default()
                    },
                    BackgroundColor(BUTTON_COLOUR),
                    children![(
                        Text::new(action.caption()),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    )],
                ));
            }
        });
    }
}

fn press_buttons(
    grids: Grids,
    mut camera: Query<(Entity, &mut Transform, &mut CellCoord), With<OrbitalCameraController>>,
    buttons: Query<(&CameraButton, &Interaction), Changed<Interaction>>,
) {
    for (button, interaction) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let Ok((entity, mut transform, mut cell)) = camera.single_mut() else {
            return;
        };
        let Some(grid) = grids.parent_grid(entity) else {
            return;
        };

        match button.0 {
            // Back to the saved pose, or to the start while there is none. home_pose logs
            // which it was, so there is nothing to add here.
            Action::Reset => {
                let (position, rotation) = home_pose();
                move_to(grid, &mut cell, &mut transform, position, rotation);
            }
            // Read out of the two components rather than out of the controller, which keeps
            // only the anchors a drag began from.
            Action::Save => save_pose(
                grid.grid_position_double(&cell, &transform),
                transform.rotation,
            ),
        }
    }
}

/// Gives the buttons their hover and press states, in the panels' own colours.
fn colour_buttons(mut buttons: Query<(&Interaction, &mut BackgroundColor), With<CameraButton>>) {
    for (interaction, mut background) in &mut buttons {
        background.set_if_neq(BackgroundColor(button_colour(*interaction, true)));
    }
}

/// The camera's start, the two buttons in the F1 list, and the pose on disk.
pub struct CameraPosePlugin;

impl Plugin for CameraPosePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (spawn_buttons, press_buttons, colour_buttons));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How close two positions have to be to count as the same place, in metres. The file
    /// holds the position as three decimal numbers and rebuilds it through the spheroid, so
    /// a round trip lands within a millimetre rather than exactly.
    const TOLERANCE: f64 = 0.001;

    fn round_trip(position: DVec3, rotation: Quat) -> (DVec3, Quat) {
        let saved = SavedPose::of(position, rotation, "now".to_string());
        let text = ron::ser::to_string_pretty(&saved, default()).unwrap();

        ron::from_str::<SavedPose>(&text).unwrap().pose()
    }

    #[test]
    fn carries_the_start_pose_through_the_file() {
        let (position, rotation) = start_pose();
        let (read, read_rotation) = round_trip(position, rotation);

        assert!(
            read.distance(position) < TOLERANCE,
            "{read} is {} m from {position}",
            read.distance(position)
        );
        assert!(read_rotation.abs_diff_eq(rotation, 1e-6));
    }

    #[test]
    fn carries_a_pose_anywhere_on_the_planet() {
        // A southern and a northern latitude, either side of the prime meridian and of the
        // antimeridian the start sits near, from a metre up and from orbit.
        for (longitude, latitude) in [
            (0.0, 0.0),
            (174.7633, -36.8485),
            (-122.4, 37.8),
            (12.5, 41.9),
            (179.9, -89.0),
            (-179.9, 89.0),
        ] {
            for altitude in [1.0, 1000.0, 400_000.0] {
                let unit = unit_position(longitude, latitude);
                let position = TerrainShape::WGS84.position_unit_to_local(unit, altitude);
                let (read, _) = round_trip(position, Quat::IDENTITY);

                assert!(
                    read.distance(position) < TOLERANCE,
                    "{longitude}, {latitude} at {altitude} m: {} m out",
                    read.distance(position)
                );
            }
        }
    }

    #[test]
    fn splits_a_position_back_into_a_direction_and_a_height() {
        for altitude in [0.0, 1000.0, 400_000.0, -50.0] {
            let unit = unit_position(174.7633, -36.8485);
            let position = TerrainShape::WGS84.position_unit_to_local(unit, altitude);
            let (read_unit, read_altitude) = unit_and_altitude(position);

            assert!((read_altitude - altitude).abs() < TOLERANCE, "{altitude} m");
            assert!(read_unit.distance(unit) < 1e-12, "{altitude} m");
        }
    }

    /// The library's own local-to-unit is the geodetic projection, which the forward
    /// function is not, so it is not the inverse this wants. Pinned because the difference
    /// is invisible at the surface and grows with height: if these ever do agree, the note
    /// on [`unit_and_altitude`] is out of date and it can go.
    #[test]
    fn the_geodetic_projection_is_not_the_inverse() {
        let unit = unit_position(174.7633, -36.8485);
        let position = TerrainShape::WGS84.position_unit_to_local(unit, 1000.0);

        let ours = unit_and_altitude(position).0;
        let geodetic = TerrainShape::WGS84.position_local_to_unit(position);

        assert!(ours.distance(unit) < 1e-12);
        assert!(geodetic.distance(unit) > 1e-9);
    }

    #[test]
    fn falls_back_when_the_rotation_is_not_one() {
        let (_, start) = start_pose();

        for written in [[0.0; 4], [f32::NAN; 4]] {
            let pose = SavedPose {
                saved_at: "now".to_string(),
                longitude: 0.0,
                latitude: 0.0,
                altitude: 0.0,
                rotation: written,
            };

            assert_eq!(pose.pose().1, start, "{written:?}");
        }
    }

    #[test]
    fn normalizes_a_rotation_written_by_hand() {
        let pose = SavedPose {
            saved_at: "now".to_string(),
            longitude: 0.0,
            latitude: 0.0,
            altitude: 0.0,
            // What somebody typing a quarter turn by hand would leave: the right direction,
            // the wrong length.
            rotation: [0.0, 2.0, 0.0, 2.0],
        };

        assert!(pose.pose().1.is_normalized());
    }

    #[test]
    fn no_pose_is_no_complaint() {
        assert_eq!(load_pose(Path::new("/nonexistent/camera_pose.ron")), None);
    }

    /// With no file, home is the start, which is what the reset button falls back to before
    /// anything has been saved.
    #[test]
    fn home_is_the_start_until_a_pose_is_written() {
        let (position, rotation) = pose_at(Path::new("/nonexistent/camera_pose.ron"));
        let (start, start_rotation) = start_pose();

        assert_eq!(position, start);
        assert_eq!(rotation, start_rotation);
    }

    /// And home is the file once there is one, read afresh, which is what lets the button
    /// pick up an edit made while the run is up.
    #[test]
    fn home_is_the_file_once_there_is_one() {
        let path = std::env::temp_dir().join(format!("camera-pose-{}.ron", std::process::id()));
        let unit = unit_position(12.5, 41.9);
        let written = TerrainShape::WGS84.position_unit_to_local(unit, 2500.0);

        let saved = SavedPose::of(written, Quat::IDENTITY, "now".to_string());
        std::fs::write(
            &path,
            ron::ser::to_string_pretty(&saved, default()).unwrap(),
        )
        .unwrap();

        let (read, _) = pose_at(&path);
        assert!(
            read.distance(written) < TOLERANCE,
            "{read} is not {written}"
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn captions_say_which_button_is_which() {
        assert_eq!(Action::Reset.caption(), "Reset camera");
        assert_ne!(Action::Save.caption(), Action::Reset.caption());
    }
}
