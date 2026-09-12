use bevy_terrain::prelude::*;
use bevy_terrain_preprocess::prelude::*;
use gdal::raster::GdalDataType;
use std::env::set_current_dir;

// Preprocesses the high-resolution LINZ open data for Auckland.
// Download the source tiles first: preprocess/download_auckland.sh
//
// Pinned for the same reason as preprocess_wellington: the elevation and the imagery
// derive different lod counts, a terrain has one, and 16 is the elevation's own value.
// Raising it buys nothing there - 16 already oversamples a 1 m source - and buys albedo
// detail at the elevation's price, since the elevation covers the whole Topo50 sheet
// while the 0.075 m survey reaches under half of it.
const LOD_COUNT: u32 = 16;

// Refuse to start rather than fill the disk.
const DISK_BUDGET_GIB: u64 = 250;

fn main() {
    // Run from the workspace root: the terrain must land in the renderer's assets
    // directory and config.path must stay relative to the workspace root.
    set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).unwrap();

    let args = Cli {
        src_path: vec!["preprocess/source_data/auckland/height/1m".into()],
        terrain_path: "assets/terrains/auckland".into(),
        temp_path: None,
        overwrite: true,
        resume: false,
        disk_budget: Some(DISK_BUDGET_GIB),
        no_data: PreprocessNoData::Source,
        data_type: PreprocessDataType::DataType(GdalDataType::Float32),
        fill_radius: 32.0,
        create_mask: true,
        lod_count: Some(LOD_COUNT),
        attachment_label: AttachmentLabel::Height,
        texture_size: 512,
        border_size: 4,
        mip_level_count: 2,
        format: AttachmentFormat::R32F,
    };

    let (src_dataset, mut context) = PreprocessContext::from_cli(args).unwrap();

    preprocess(src_dataset, &mut context);

    let args = Cli {
        // Both levels feed the one attachment, coarser first so the finer one wins where
        // they overlap: gdalbuildvrt draws later sources on top. The 0.075 m survey covers
        // 394 km2 of the default sheet's 864, the 0.5 m mosaic 691, and the grid this
        // terrain samples is around 0.6 m either way. The coarse imagery is from
        // 2010-2012, so expect a visible seam where the two meet.
        src_path: vec![
            "preprocess/source_data/auckland/albedo/0.5m".into(),
            "preprocess/source_data/auckland/albedo/0.075m".into(),
        ],
        terrain_path: "assets/terrains/auckland".into(),
        temp_path: None,
        overwrite: true,
        resume: false,
        disk_budget: Some(DISK_BUDGET_GIB),
        no_data: PreprocessNoData::NoData(0.0),
        data_type: PreprocessDataType::DataType(GdalDataType::UInt8),
        fill_radius: 0.0,
        create_mask: false,
        lod_count: Some(LOD_COUNT),
        attachment_label: AttachmentLabel::Custom("albedo".to_string()),
        texture_size: 512,
        border_size: 2,
        mip_level_count: 4,
        format: AttachmentFormat::Rgba8U,
    };

    let (src_dataset, mut context) = PreprocessContext::from_cli(args).unwrap();

    preprocess(src_dataset, &mut context);
}
