//! Where a terrain's pixels came from.
//!
//! None of this can be recovered from a preprocessed terrain: the tiles carry no metadata
//! and the source paths are expanded into rasters and dropped. It has to be recorded as it
//! passes, so each stage writes down what only it knows. The download scripts know the
//! bucket path a level came from and the survey named in it, the colour matching script
//! knows the gains it applied, and the preprocessor knows which levels fed which
//! attachment.

use crate::terrain_data::AttachmentLabel;
use bevy::{platform::collections::HashMap, prelude::*};
use serde::{Deserialize, Serialize};
use std::{fs, io::ErrorKind, path::Path};

/// The file a download script leaves inside a level directory.
pub const MANIFEST_FILE: &str = "manifest.ron";

/// The file the preprocessor writes beside `config.tc.ron`.
pub const PROVENANCE_FILE: &str = "provenance.tp.ron";

/// What a download script recorded about one level directory.
///
/// Written as `manifest.ron` inside the directory it describes, so it travels with the
/// tiles rather than being orphaned when they move. It describes the directory as it
/// stands and not what any single run fetched: the scripts copy rather than sync, so a
/// later run for another sheet adds to the same directory.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SourceManifest {
    /// The bucket prefix the tiles were copied from, remote prefix and all.
    pub source: String,
    /// The publisher's dataset, exactly as it is spelled in the bucket path.
    pub dataset: String,
    /// The publisher's product under the dataset: `rgb`, `dem_1m`.
    pub product: String,
    /// The projection the tiles are published in.
    pub crs: String,
    /// The attachment these tiles feed: `height`, `albedo`.
    pub attachment: String,
    /// The level directory's name, `0.075m`. The national script has no level segment.
    #[serde(default)]
    pub level: Option<String>,
    /// Ground sample distance, as the publisher writes it.
    pub resolution: String,
    /// Capture year or range, for imagery. The national elevation mosaic is stitched from
    /// surveys spanning years and publishes no date, so its manifests leave this out.
    #[serde(default)]
    pub captured: Option<String>,
    /// Every Topo50 sheet on disk, read back off the filenames rather than taken from the
    /// sheet list a run was invoked with. Kept whole even for the national download, where
    /// it runs to hundreds: it is the coverage record.
    pub sheets: Vec<String>,
    /// Tiles on disk when the manifest was written.
    pub tiles: usize,
    /// Their total size in bytes.
    pub bytes: u64,
    /// The script that wrote this.
    pub script: String,
    /// When it last ran, RFC 3339 in UTC.
    pub updated: String,
    /// Per band gains, when this describes a colour matched virtual raster. They exist
    /// nowhere else: no bucket publishes them, and nothing recovers them from the virtual
    /// raster short of reading its XML.
    #[serde(default)]
    pub gains: Option<[f64; 3]>,
    /// The dataset the gains were measured against.
    #[serde(default)]
    pub matched_to: Option<String>,
}

impl SourceManifest {
    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let encoded = fs::read_to_string(path)?;
        Ok(ron::from_str(&encoded)?)
    }

    pub fn save_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let encoded = ron::ser::to_string_pretty(self, default())?;
        Ok(fs::write(path, encoded)?)
    }
}

/// One source argument as the preprocessor was given it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SourceRecord {
    /// The path as passed to the preprocessor, relative to the workspace root. A colour
    /// matched level is its virtual raster here, while the manifest below belongs to the
    /// directory that raster draws from.
    pub path: String,
    /// Rasters the path expanded to when the terrain was built. A manifest that disagrees
    /// with this has been added to since it was written.
    pub rasters: usize,
    /// Absent when the source predates manifests, as the example terrains do.
    #[serde(default)]
    pub manifest: Option<SourceManifest>,
}

/// Every source that went into a terrain, grouped the way the attachments are.
///
/// Keyed by attachment because the preprocessor runs once per attachment against the same
/// terrain, so each run owns its entry and neither erases the other's.
#[derive(Serialize, Deserialize, Asset, TypePath, Debug, Clone, Default, PartialEq)]
pub struct TerrainProvenance {
    pub sources: HashMap<AttachmentLabel, Vec<SourceRecord>>,
}

impl TerrainProvenance {
    /// A missing file is an empty provenance, the first run for a terrain. A file that is
    /// there and will not parse is an error and never an empty one: this run only fills in
    /// its own attachment, so defaulting here would drop the other run's sources silently.
    pub fn load_or_empty<P: AsRef<Path>>(path: P) -> Result<Self> {
        match fs::read_to_string(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
            Ok(encoded) => Ok(ron::from_str(&encoded)?),
        }
    }

    /// Written through a temporary and renamed, the way the reprojected faces are, so a
    /// crash partway cannot leave a half file that the next run would then refuse.
    pub fn save_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();
        let encoded = ron::ser::to_string_pretty(self, default())?;
        let partial = path.with_extension("ron.partial");

        fs::write(&partial, encoded)?;
        Ok(fs::rename(partial, path)?)
    }
}
