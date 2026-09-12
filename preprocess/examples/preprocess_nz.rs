use bevy_terrain::prelude::*;
use bevy_terrain_preprocess::prelude::*;
use gdal::raster::GdalDataType;
use std::env::set_current_dir;

// Preprocesses the LINZ open data for New Zealand (https://github.com/linz/imagery).
// Download the source tiles first: preprocess/download_nz.sh
fn main() {
    // Run from the workspace root: the terrain must land in the renderer's assets
    // directory and config.path must stay relative to the workspace root.
    set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).unwrap();

    let args = Cli {
        src_path: vec!["preprocess/source_data/nz/height".into()],
        terrain_path: "assets/terrains/nz".into(),
        temp_path: None,
        overwrite: true,
        resume: false,
        disk_budget: None,
        no_data: PreprocessNoData::Source,
        data_type: PreprocessDataType::DataType(GdalDataType::Float32),
        fill_radius: 32.0,
        create_mask: true,
        lod_count: None,
        attachment_label: AttachmentLabel::Height,
        texture_size: 512,
        border_size: 4,
        mip_level_count: 2,
        format: AttachmentFormat::R32F,
    };

    let (src_dataset, mut context) = PreprocessContext::from_cli(args).unwrap();

    preprocess(src_dataset, &mut context);

    // Match the albedo depth to the lod count derived from the height dataset.
    let lod_count = TerrainConfig::load_file("assets/terrains/nz/config.tc.ron")
        .unwrap()
        .lod_count;

    let args = Cli {
        src_path: vec!["preprocess/source_data/nz/albedo".into()],
        terrain_path: "assets/terrains/nz".into(),
        temp_path: None,
        overwrite: true,
        resume: false,
        disk_budget: None,
        no_data: PreprocessNoData::NoData(0.0),
        data_type: PreprocessDataType::DataType(GdalDataType::UInt8),
        fill_radius: 0.0,
        create_mask: false,
        lod_count: Some(lod_count),
        attachment_label: AttachmentLabel::Custom("albedo".to_string()),
        texture_size: 512,
        border_size: 2,
        mip_level_count: 4,
        format: AttachmentFormat::Rgba8U,
    };

    let (src_dataset, mut context) = PreprocessContext::from_cli(args).unwrap();

    preprocess(src_dataset, &mut context);
}
