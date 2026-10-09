//! A test affordance: a folder of photographs, a count, and a button that scatters that many
//! markers across the view with a picture each.
//!
//! Placing markers by hand is fine for two or three, and useless for judging a layout engine that
//! only starts to mean anything at a dozen. This fills the view in one press. Nothing here is
//! part of what a marker is: it writes markers through the same `place_riding` and
//! `photo::start_decode` a Ctrl+click and a drop go through, so what comes out is indistinguishable
//! from markers placed by hand, and saving the file keeps them like any others.
//!
//! The folder chooser is `zenity`, or `kdialog` where that is what is installed, run as a command
//! in a task the way the photo decodes are. Not a crate: the ones that do this properly pull a
//! portal client or a GTK binding in behind them, which is a great deal of dependency for a
//! button that exists to make testing quicker. The cost is that on a machine with neither
//! installed the button says so and nothing else happens.
//!
//! The markers land in a disc around the point under the camera, sized from how high it is, so
//! that a press fills the view rather than the planet. Their ground is the ellipsoid rather than
//! the terrain: there is no height to sample away from the cursor, and a dot a few metres under a
//! hillside still draws, since the gizmos are biased in front of the terrain anyway.

use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use bevy_terrain::prelude::TerrainShape;
use rand::Rng;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::photo;
use crate::plugins::rail_editor::frame::{Frame, unit_under};

/// How many markers a press makes, and the range the panel's slider covers. A dozen is where a
/// layout starts to have something to do; a hundred is more than the columns can hold, which is
/// the other thing worth being able to see.
pub(super) const SCATTER_COUNT: f32 = 12.0;
pub(super) const MIN_COUNT: f32 = 1.0;
pub(super) const MAX_COUNT: f32 = 100.0;

/// How wide the scatter is, as a share of the camera's height above the ellipsoid. A 45 degree
/// view covers about eight tenths of its altitude across, so four tenths of it either way fills
/// the screen and a little past its edges.
const SPREAD: f64 = 0.4;

/// The smallest scatter, in metres, so that a press from ground level does not stack every marker
/// on one spot.
const CLOSEST: f64 = 200.0;

/// The chosen folder and the photographs in it.
#[derive(Resource, Default)]
pub(super) struct Scatter {
    pub folder: Option<PathBuf>,
    /// The readable images in that folder, in name order. Read once when the folder is chosen
    /// rather than at every press: a press should not go to the disk.
    pub photos: Vec<PathBuf>,
    /// Set when the chooser could not be run at all, so the panel can say why.
    pub no_chooser: bool,
}

impl Scatter {
    /// What the panel says about the folder: its name and how many pictures are in it.
    pub(super) fn caption(&self) -> String {
        if self.no_chooser {
            return "no zenity or kdialog to choose a folder with".to_string();
        }
        let Some(folder) = &self.folder else {
            return "no folder chosen".to_string();
        };
        let name = folder
            .file_name()
            .unwrap_or(folder.as_os_str())
            .to_string_lossy();

        match self.photos.len() {
            0 => format!("{name}: no photos this build can read"),
            1 => format!("{name}: 1 photo"),
            photos => format!("{name}: {photos} photos"),
        }
    }

    pub(super) fn ready(&self) -> bool {
        !self.photos.is_empty()
    }
}

/// A folder chooser waiting on the person, on a throwaway entity as the photo decodes are.
#[derive(Component)]
pub(super) struct FolderTask(Task<Choice>);

/// What the chooser came back with. The three are told apart because the panel says something
/// different about each: a folder, a cancelled dialog, or no dialog to be had.
enum Choice {
    Folder(PathBuf),
    Cancelled,
    NoChooser,
}

/// Runs the folder chooser off the main thread, so the window keeps drawing while the dialog is
/// open rather than freezing behind it.
pub(super) fn choose_folder(commands: &mut Commands) {
    let task = AsyncComputeTaskPool::get().spawn(async move { ask() });

    commands.spawn(FolderTask(task));
}

/// Asks whichever of the two chooser is installed. Each prints the path on standard output and
/// leaves with a failure when the dialog is cancelled, so an empty answer is a cancel either way.
fn ask() -> Choice {
    let chooser = [
        ("zenity", vec!["--file-selection", "--directory"]),
        ("kdialog", vec!["--getexistingdirectory", "."]),
    ];

    for (program, arguments) in chooser {
        let Ok(output) = Command::new(program).args(arguments).output() else {
            continue;
        };
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();

        return match path.is_empty() {
            true => Choice::Cancelled,
            false => Choice::Folder(PathBuf::from(path)),
        };
    }

    Choice::NoChooser
}

/// Takes the chosen folder when the dialog closes, and reads the photographs out of it.
pub(super) fn poll_folder(
    mut commands: Commands,
    mut scatter: ResMut<Scatter>,
    mut tasks: Query<(Entity, &mut FolderTask)>,
) {
    for (entity, mut task) in &mut tasks {
        let Some(choice) = block_on(poll_once(&mut task.0)) else {
            continue;
        };
        commands.entity(entity).despawn();

        match choice {
            Choice::Cancelled => {}
            Choice::NoChooser => {
                scatter.no_chooser = true;
                warn!("photo markers: no zenity or kdialog to choose a folder with");
            }
            Choice::Folder(folder) => {
                scatter.photos = photos_in(&folder);
                info!(
                    "photo markers: {} has {} photos this build can read",
                    folder.display(),
                    scatter.photos.len()
                );
                scatter.folder = Some(folder);
                scatter.no_chooser = false;
            }
        }
    }
}

/// The images in a folder this build can decode, in name order so that a scatter is the same
/// twice over but for the choosing. Not recursive: a folder of photographs is what is wanted, and
/// walking a whole drive from a button is a surprise nobody asked for.
pub(super) fn photos_in(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };

    let mut photos: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && photo::readable(path).is_ok())
        .collect();
    photos.sort();

    photos
}

/// Where one marker of a scatter lands: a point on the ellipsoid at `radius` metres from the
/// one under the camera, in the direction `angle` round the local frame.
///
/// Pure, and the only part of the scatter that is geometry rather than plumbing. The offset is
/// laid in the tangent plane and the result dropped back onto the sphere, so a scatter a few
/// kilometres wide is right and one a thousand is near enough for test markers.
pub(super) fn placed_unit(ground: DVec3, frame: &Frame, radius: f64, angle: f64) -> DVec3 {
    let offset = frame.east * (radius * angle.cos()) + frame.north * (radius * angle.sin());

    unit_under(ground + offset)
}

/// How wide a scatter is from this high up: a share of the altitude, never closer than CLOSEST.
pub(super) fn spread_at(altitude: f64) -> f64 {
    (altitude * SPREAD).max(CLOSEST)
}

/// Where a scatter from the panel's button goes, and how high the camera is over it: the panel
/// has no place of its own to point at, so it takes the ground under the camera. A Ctrl+Shift
/// click has a place, and passes the ground it landed on instead.
pub(super) fn over_the_camera(camera_position: DVec3) -> (DVec3, f64) {
    let unit = unit_under(camera_position);
    let ground = TerrainShape::WGS84.position_unit_to_local(unit, 0.0);
    let altitude = (camera_position - ground).dot(Frame::at_unit(unit).up);

    (ground, altitude)
}

/// Scatters `count` markers in a disc about a place on the ground, each with a photograph from
/// the folder.
///
/// The disc is as wide as the camera's height makes sensible, so a scatter fills the screen from
/// wherever it is asked for. The colours are spread round the wheel rather than taken from the
/// panel: what these are for is seeing a dozen tethers at once, and a dozen of the same colour
/// says nothing.
pub(super) fn scatter_markers(
    markers: &mut super::PhotoMarkers,
    scatter: &Scatter,
    around: DVec3,
    altitude: f64,
    commands: &mut Commands,
) -> usize {
    if scatter.photos.is_empty() {
        return 0;
    }

    let unit = unit_under(around);
    let frame = Frame::at_unit(unit);
    let ground = TerrainShape::WGS84.position_unit_to_local(unit, 0.0);
    let spread = spread_at(altitude);

    let count = markers.scatter_count.round().max(0.0) as usize;
    let mut rng = rand::rng();

    for index in 0..count {
        // The square root spreads them evenly over the disc rather than crowding the middle,
        // which is what a plain radius would do.
        let angle = rng.random_range(0.0..std::f64::consts::TAU);
        let radius = spread * rng.random_range(0.0..1.0f64).sqrt();
        let placed = placed_unit(ground, &frame, radius, angle);

        let id = markers.place_riding(placed, 0.0, None);

        // Round the wheel rather than at random, so no two of a dozen come out near enough to be
        // mistaken for each other.
        if let Some(marker) = markers.markers.last_mut() {
            marker.colour = Hsva::hsv(index as f32 * 360.0 / count.max(1) as f32, 0.8, 0.95);
        }

        let photo = scatter.photos[rng.random_range(0..scatter.photos.len())].clone();
        photo::start_decode(commands, id, photo);
    }

    // The scatter leaves the last of them selected, as placing one by hand does, and the markers
    // are unsaved until the panel's save button is pressed.
    info!("photo markers: {count} scattered within {spread:.0} m");

    count
}
