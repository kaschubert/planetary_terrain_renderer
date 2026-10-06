mod cli;
mod dataset;
mod downsample;
mod fill_no_data;
mod gdal_extension;
mod provenance;
mod reproject;
mod result;
mod split;
mod stitch;
mod transformers;

use crate::{
    cli::PreprocessBar,
    dataset::{PreprocessContext, clear_directory, clear_directory_except, delete_directory},
    downsample::downsample_and_stitch,
    fill_no_data::create_mask_and_fill_no_data,
    provenance::SourceProvenance,
    reproject::{check_disk_space, reproject},
    split::split_and_stitch,
};
use bevy_terrain::prelude::*;
use gdal::{
    Dataset,
    raster::{GdalDataType, GdalType},
};
use num::NumCast;
use std::{fs, time::Instant};

pub mod prelude {
    pub use crate::{
        cli::{Cli, provenance_only},
        dataset::{PreprocessContext, PreprocessDataType, PreprocessNoData},
        preprocess,
    };
}

fn preprocess_gen<T: Copy + GdalType + PartialEq + NumCast>(
    src_dataset: Dataset,
    context: &mut PreprocessContext,
) {
    // Before anything is deleted: refuse a run that cannot fit on disk.
    check_disk_space::<T>(&src_dataset, context).unwrap_or_else(|error| panic!("{error}"));

    if context.overwrite {
        // The temp directory sits inside the tile directory unless one was given
        // explicitly, so a resuming run must wipe the tiles around it.
        if context.resume {
            clear_directory_except(&context.tile_dir, &context.temp_dir);
        } else {
            clear_directory(&context.tile_dir);
        }
    }

    // Resuming keeps the temp directory so a completed reprojection can be reused.
    // Otherwise wipe it, which also clears any .partial left by an interrupted run.
    if context.resume {
        fs::create_dir_all(&context.temp_dir).unwrap();
    } else {
        clear_directory(&context.temp_dir);
    }

    let start_preprocessing = Instant::now();

    let progress_bar = PreprocessBar::new("Reprojecting".to_string());
    let faces = reproject::<T>(src_dataset, context, Some(progress_bar.callback())).unwrap();
    progress_bar.finish();

    let progress_bar = PreprocessBar::new("Splitting".to_string());
    let tiles = split_and_stitch::<T>(faces, context, Some(progress_bar.callback())).unwrap();
    progress_bar.finish();

    let progress_bar = PreprocessBar::new("Downsampling".to_string());
    let tiles = downsample_and_stitch::<T>(&tiles, context, Some(progress_bar.callback())).unwrap();
    progress_bar.finish();

    let progress_bar = PreprocessBar::new("Filling".to_string());
    create_mask_and_fill_no_data(&tiles, context, Some(progress_bar.callback())).unwrap();
    progress_bar.finish();

    delete_directory(&context.temp_dir);

    save_terrain_config(tiles, context);
    save_terrain_provenance(context);

    println!("Preprocessing took: {:?}", start_preprocessing.elapsed());
}

pub fn preprocess(src_dataset: Dataset, context: &mut PreprocessContext) {
    if context.provenance_only {
        // Backfilling a terrain that was built before its sources were documented. The
        // tiles are already correct; only the record of where they came from is missing,
        // and rebuilding them to recover it would cost a day of warping for a few
        // kilobytes of text.
        save_terrain_provenance(context);
        return;
    }

    macro_rules! preprocess_gen {
        ($data_type:ty) => {
            preprocess_gen::<$data_type>(src_dataset, context)
        };
    }

    match context.data_type {
        GdalDataType::Unknown => panic!("Unknown data type!"),
        GdalDataType::UInt8 => preprocess_gen!(u8),
        GdalDataType::UInt16 => preprocess_gen!(u16),
        GdalDataType::UInt32 => preprocess_gen!(u32),
        GdalDataType::UInt64 => preprocess_gen!(u64),
        GdalDataType::Int8 => preprocess_gen!(i8),
        GdalDataType::Int16 => preprocess_gen!(i16),
        GdalDataType::Int32 => preprocess_gen!(i32),
        GdalDataType::Int64 => preprocess_gen!(i64),
        GdalDataType::Float32 => preprocess_gen!(f32),
        GdalDataType::Float64 => preprocess_gen!(f64),
    };
}

fn save_terrain_config(tiles: Vec<TileCoordinate>, context: &PreprocessContext) {
    let file_path = context.terrain_path.join("config.tc.ron");

    // Only a missing file starts from a default. A file that is there and will not parse
    // has to stop the run: this run fills in its own attachment alone, so defaulting here
    // would drop the other run's tiles and lod count on the floor, silently, and the
    // terrain directory is not in version control to restore from.
    let mut config = match TerrainConfig::load_file(&file_path) {
        Ok(config) => config,
        Err(_) if !file_path.exists() => TerrainConfig::default(),
        Err(error) => panic!("{}: {error}", file_path.display()),
    };

    config.shape = TerrainShape::WGS84;
    config.path = context.terrain_path.to_str().unwrap().to_string();
    config.add_attachment(context.attachment_label.clone(), context.attachment.clone());

    if context.attachment_label == AttachmentLabel::Height {
        config.min_height = context.min_height;
        config.max_height = context.max_height;
        config.tiles = tiles;
        config.lod_count = context.lod_count.unwrap();
    }

    config.save_file(&file_path).unwrap();
}

/// Records this run's sources beside the terrain config, keyed by attachment so the height
/// run and the albedo run each own their entry and neither erases the other's.
fn save_terrain_provenance(context: &PreprocessContext) {
    let file_path = context.terrain_path.join(PROVENANCE_FILE);

    let mut provenance = TerrainProvenance::load_or_empty(&file_path)
        .unwrap_or_else(|error| panic!("{}: {error}", file_path.display()));

    provenance.sources.insert(
        context.attachment_label.clone(),
        context
            .sources
            .iter()
            .map(SourceProvenance::record)
            .collect(),
    );

    provenance.save_file(&file_path).unwrap();
}
