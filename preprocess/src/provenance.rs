//! Carrying a source's manifest through to the terrain it builds.
//!
//! The source paths used to be expanded into rasters and dropped, so by the time anything
//! was written down nothing could say where the pixels had come from. Here each argument
//! keeps its own identity through the expansion, paired with the manifest the download
//! left in its directory.

use crate::{
    dataset::iter_directory,
    result::{PreprocessError, PreprocessResult},
};
use bevy_terrain::prelude::{MANIFEST_FILE, SourceManifest, SourceRecord};
use itertools::Itertools;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// One source argument, the rasters it expanded to, and what documents them.
pub(crate) struct SourceProvenance {
    src_path: PathBuf,
    pub(crate) rasters: Vec<PathBuf>,
    manifest: Option<SourceManifest>,
}

impl SourceProvenance {
    /// Expands one source argument and finds its manifest.
    ///
    /// A source without one is not an error. The example terrains point straight at loose
    /// rasters that no download script ever fetched, and they have to keep preprocessing.
    pub(crate) fn resolve(src_path: PathBuf) -> PreprocessResult<Self> {
        let expanded = src_path.is_dir();
        let rasters = if expanded {
            iter_directory(&src_path)
                .filter(|path| is_raster(path))
                .collect_vec()
        } else {
            vec![src_path.clone()]
        };

        let manifest = match manifest_path(&src_path) {
            None => {
                println!(
                    "no {MANIFEST_FILE} for {}, recording the path alone",
                    src_path.display()
                );
                None
            }
            Some(path) => {
                let manifest = SourceManifest::load_file(&path).map_err(|error| {
                    // Fatal on purpose, and early: a manifest that will not parse means a
                    // download wrote something wrong, and quietly losing the provenance is
                    // the one failure this is all here to prevent.
                    PreprocessError::Manifest {
                        path: path.clone(),
                        message: error.to_string(),
                    }
                })?;

                // Only worth comparing when the argument was a directory. A virtual
                // raster is a single file standing for all of them, so the counts are
                // meant to differ.
                if expanded && manifest.tiles != rasters.len() {
                    println!(
                        "{}: {MANIFEST_FILE} counts {} tiles but {} are on disk, so it \
                         predates a later download into the same directory",
                        src_path.display(),
                        manifest.tiles,
                        rasters.len()
                    );
                }

                Some(manifest)
            }
        };

        Ok(Self {
            src_path,
            rasters,
            manifest,
        })
    }

    pub(crate) fn record(&self) -> SourceRecord {
        SourceRecord {
            path: self.src_path.to_str().unwrap().to_string(),
            rasters: self.rasters.len(),
            manifest: self.manifest.clone(),
        }
    }
}

/// The rasters a source can be built from, virtual ones included.
pub(crate) fn is_raster(path: &Path) -> bool {
    let path = path.to_str().unwrap();
    // .vrt as well as the rasters themselves, so a source can be a virtual one: a level
    // whose colour has been matched to another, say.
    path.ends_with(".tif") || path.ends_with(".tiff") || path.ends_with(".vrt")
}

/// What documents a source argument.
///
/// A directory holds its own manifest. A virtual raster gets one of its own beside it when
/// the colour matching script built it, because the gains it applied exist nowhere else.
/// Failing that it falls back to the directory it draws from, which is the right answer for
/// a plain virtual raster: that is a lens on those tiles rather than a new source.
fn manifest_path(src_path: &Path) -> Option<PathBuf> {
    fn in_dir(dir: &Path) -> Option<PathBuf> {
        let path = dir.join(MANIFEST_FILE);
        path.is_file().then_some(path)
    }

    if src_path.is_dir() {
        return in_dir(src_path);
    }

    if src_path.extension().is_some_and(|ext| ext == "vrt") {
        let own = PathBuf::from(format!("{}.{MANIFEST_FILE}", src_path.display()));

        return own
            .is_file()
            .then_some(own)
            // The colour matching script writes <dir>.vrt beside <dir>, so stripping the
            // extension finds the tiles it was built over; reading the raster's own source
            // list covers a virtual raster built some other way.
            .or_else(|| in_dir(&src_path.with_extension("")))
            .or_else(|| vrt_source_dirs(src_path).iter().find_map(|dir| in_dir(dir)));
    }

    src_path.parent().and_then(in_dir)
}

/// The directories a virtual raster draws from, in the order it lists them.
///
/// Read as text rather than through GDAL: the safe bindings expose no file list, and all
/// that is wanted here is a parent directory.
fn vrt_source_dirs(vrt_path: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(vrt_path) else {
        return vec![];
    };
    let base = vrt_path.parent().unwrap_or(Path::new("."));

    text.split("<SourceFilename")
        .skip(1)
        .filter_map(|chunk| {
            let (attributes, rest) = chunk.split_once('>')?;
            let (filename, _) = rest.split_once("</SourceFilename>")?;

            let path = if attributes.contains("relativeToVRT=\"1\"") {
                base.join(filename)
            } else {
                PathBuf::from(filename)
            };

            Some(path.parent()?.to_path_buf())
        })
        .dedup()
        .collect_vec()
}
