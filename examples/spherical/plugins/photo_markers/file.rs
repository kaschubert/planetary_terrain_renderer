//! The markers on disk, so that they come back next run.
//!
//! RON rather than the CSV the rail lines use: a marker has a colour and an optional path in it,
//! which quote and nest badly in a flat file, and the renderer already writes its terrain
//! configurations in RON so nothing new is pulled in. The file sits beside the plugin, as the rail
//! network's does, and is written through the same atomic rename, so a crash mid-write leaves the
//! old file whole rather than half of a new one.
//!
//! A file that will not parse is not written over. The markers come up empty and saving is refused
//! until the file is fixed or moved aside, which is what the rail lines do with theirs: a save
//! would otherwise replace a file whose contents are still wanted with an empty one.

use bevy::prelude::*;
use bevy_terrain::math::unit_position;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::{MarkerId, PhotoMarker, PhotoMarkers, SizeSample};
use crate::plugins::auckland_rail::write_atomically;
use crate::plugins::shared::rail_network::utc_now;

/// Where the markers live: beside the plugin, in the source tree, as the rail network's CSV does,
/// so that a marker placed while flying around is still there after a rebuild.
pub(super) const MARKERS_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/examples/spherical/plugins/photo_markers/markers.ron"
);

/// The file's shape. The time is for a reader of the file and is not read back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MarkerFile {
    pub saved_at: String,
    pub markers: Vec<SavedMarker>,
}

/// One marker as the file holds it. Longitude and latitude rather than the direction on the unit
/// sphere, because those are the numbers a person can read and edit; the colour as hue,
/// saturation and value, which is what the marker holds and what the sliders set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct SavedMarker {
    pub longitude: f64,
    pub latitude: f64,
    /// The terrain height under the marker when it was placed, in metres above the ellipsoid.
    pub ground: f64,
    pub hue: f32,
    pub saturation: f32,
    pub value: f32,
    pub photo: Option<PathBuf>,
}

impl SavedMarker {
    fn of(marker: &PhotoMarker) -> Self {
        Self {
            longitude: marker.longitude(),
            latitude: marker.latitude(),
            ground: marker.ground,
            hue: marker.colour.hue,
            saturation: marker.colour.saturation,
            value: marker.colour.value,
            photo: marker.photo.clone(),
        }
    }

    /// The marker this stands for, with no entity or materials yet: spawn_markers gives it those
    /// once the big_space root and the model are there.
    fn marker(&self, id: MarkerId) -> PhotoMarker {
        PhotoMarker {
            id,
            unit: unit_position(self.longitude, self.latitude),
            ground: self.ground,
            colour: Hsva::hsv(self.hue, self.saturation, self.value),
            photo: self.photo.clone(),
            // Whether the file is still there is found out by trying to read it, see
            // spawn_markers, not by anything the saved file could have said.
            missing: false,
            entity: None,
            body_material: None,
            screen_material: None,
        }
    }
}

/// The markers in the file, in the order it holds them, and whether it would not read. A file
/// that is not there is no markers and no complaint: nothing has been placed yet.
pub(super) fn load_markers(path: &Path) -> (Vec<PhotoMarker>, bool) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), false),
        Err(error) => {
            warn!("photo markers: {}: {error}", path.display());
            return (Vec::new(), true);
        }
    };

    match ron::from_str::<MarkerFile>(&text) {
        Ok(file) => {
            let markers = file
                .markers
                .iter()
                .enumerate()
                .map(|(index, saved)| saved.marker(MarkerId(index as u32)))
                .collect();
            (markers, false)
        }
        Err(error) => {
            warn!("photo markers: {}: {error}", path.display());
            (Vec::new(), true)
        }
    }
}

/// The file as it would be written for these markers.
pub(super) fn to_ron(markers: &[PhotoMarker], saved_at: &str) -> Result<String, String> {
    let file = MarkerFile {
        saved_at: saved_at.to_string(),
        markers: markers.iter().map(SavedMarker::of).collect(),
    };

    ron::ser::to_string_pretty(&file, default()).map_err(|error| error.to_string())
}

impl PhotoMarkers {
    /// The markers from the file, at startup. Everything else starts at its default.
    pub fn load() -> Self {
        let (markers, file_unreadable) = load_markers(Path::new(MARKERS_PATH));
        let next_id = markers.len() as u32;

        if !markers.is_empty() {
            info!("photo markers: {} loaded", markers.len());
        }

        Self {
            markers,
            next_id,
            file_unreadable,
            colour: super::DEFAULT_COLOUR,
            height: super::model::MARKER_HEIGHT,
            min_pixels: super::model::MIN_PIXELS,
            ..default()
        }
    }

    /// Whether a save would be allowed to write, which dims the panel's button. False while the
    /// file would not read, and false with nothing to write.
    pub fn can_save(&self) -> bool {
        !self.file_unreadable && self.dirty
    }

    /// Writes the markers back, stamped with the time. The panel's button and Ctrl+S call it and
    /// turn the result into a line on the console.
    pub fn save(&mut self) -> Result<usize, String> {
        let path = Path::new(MARKERS_PATH);

        if self.file_unreadable {
            return Err(format!(
                "{} did not read at startup, so the markers are empty; saving would write that \
                 emptiness over the file, so fix or move the file and start again",
                path.display()
            ));
        }

        let text = to_ron(&self.markers, &utc_now())?;
        write_atomically(path, &text).map_err(|error| format!("{}: {error}", path.display()))?;
        self.dirty = false;

        Ok(self.markers.len())
    }
}

/// Saves and says how it went on the console, which the panel's button calls.
///
/// There is no key for this, and deliberately. Ctrl+S is the rail editor's, which writes its CSV
/// whether or not its panel is open, so a second claim on the chord would write that file every
/// time the markers were saved, giving a tracked file a new timestamp nobody asked to change.
pub(super) fn save_markers(markers: &mut PhotoMarkers) {
    match markers.save() {
        Ok(count) => info!("photo markers: {count} saved"),
        Err(message) => error!("photo markers: {message}"),
    }
}

/// Where the size samples go: one row per press of the panel's `note size`, appended, so that a
/// session of flying about and sizing markers by eye leaves a table to read afterwards.
pub(super) const SIZES_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/examples/spherical/plugins/photo_markers/marker_sizes.csv"
);

/// The columns, written once when the file is new.
const SIZES_HEADER: &str = "# How big a marker looked right, one row per press of `note size` in the F10 panel.\n\
                            # height_m is what the size slider was on, floor_px the pixel floor beside it,\n\
                            # distance_m how far the camera was from the marker and altitude_m how high it was,\n\
                            # pixels how tall the marker stood on screen, and the last two what the window was.\n\
                            saved_at,height_m,floor_px,distance_m,altitude_m,pixels,viewport_px,fov_deg\n";

/// One row of the size table, as it is written.
pub(super) fn size_row(saved_at: &str, height: f32, floor: f32, sample: &SizeSample) -> String {
    format!(
        "{saved_at},{height:.1},{floor:.0},{:.0},{:.0},{:.1},{:.0},{:.1}\n",
        sample.distance, sample.altitude, sample.pixels, sample.viewport, sample.fov,
    )
}

/// Appends a size sample, writing the header first when the file is new. Appended rather than
/// written whole, because the point of it is a session's worth of judgements and losing the
/// earlier ones to a crash or a restart would waste the flying.
pub(super) fn note_size(markers: &PhotoMarkers, sample: &SizeSample) {
    use std::io::Write;

    let path = Path::new(SIZES_PATH);
    let fresh = !path.exists();

    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| {
            if fresh {
                file.write_all(SIZES_HEADER.as_bytes())?;
            }
            file.write_all(
                size_row(&utc_now(), markers.height, markers.min_pixels, sample).as_bytes(),
            )
        });

    match written {
        Ok(()) => info!(
            "photo markers: noted {:.0} m at {:.0} m away, {:.0} px on screen",
            markers.height, sample.distance, sample.pixels
        ),
        Err(error) => error!("photo markers: {}: {error}", path.display()),
    }
}
