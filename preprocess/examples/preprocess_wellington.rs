use bevy_terrain::prelude::*;
use bevy_terrain_preprocess::prelude::*;
use gdal::raster::GdalDataType;
use std::env::set_current_dir;

// Preprocesses the high-resolution LINZ open data for Wellington.
// Download the source tiles first: preprocess/download_wellington.sh
//
// The lod count is pinned rather than derived, because the two sources disagree about
// it. Deriving from the 1 m elevation gives 16; the 0.075 m imagery would give 19. A
// terrain has a single lod count, so one of them is always sampled off its native grid.
//
//   lod count   grid     height (1 m)   albedo (0.075 m)
//   16          ~0.6 m   116 GiB        12 GiB
//   17          ~0.3 m   463 GiB        48 GiB
//   18          ~0.15 m  -              190 GiB
//   19          ~0.075 m -              759 GiB
//
// Those are upper bounds from the preprocessor's own estimate; a source with no-data
// regions comes in well under. Height dominates either way, because it covers all three
// Topo50 sheets while the 0.075 m survey only reaches Wellington city. Raising the lod
// count therefore buys albedo detail at height's price, and buys the elevation nothing:
// 16 already oversamples a 1 m source. Hence 16.
const LOD_COUNT: u32 = 16;

// Refuse to start rather than fill the disk, which a run at 18 or 19 would do.
const DISK_BUDGET_GIB: u64 = 300;

fn main() {
    // Run from the workspace root: the terrain must land in the renderer's assets
    // directory and config.path must stay relative to the workspace root.
    set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).unwrap();

    let args = Cli {
        src_path: vec!["preprocess/source_data/wellington/height/1m".into()],
        terrain_path: "assets/terrains/wellington".into(),
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
        src_path: vec!["preprocess/source_data/wellington/albedo/0.075m".into()],
        terrain_path: "assets/terrains/wellington".into(),
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
