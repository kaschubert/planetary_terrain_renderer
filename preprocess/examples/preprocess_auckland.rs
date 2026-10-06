use bevy_terrain::prelude::*;
use bevy_terrain_preprocess::prelude::*;
use gdal::raster::GdalDataType;
use std::env::set_current_dir;

// Preprocesses the high-resolution LINZ open data for Auckland, as one terrain across
// three Topo50 sheets: the city centre, West Auckland and South Auckland.
//
// Download the source tiles first, with both scripts, and colour match the 0.25 m level:
//
//   preprocess/download_auckland.sh              # BA32, the centre sheet
//   preprocess/download_auckland_west_south.sh   # BA31 and BB32, either side
//   preprocess/colour_match.sh source_data/auckland_west_south/albedo/0.25m 1.012 0.942 0.823
//
// The sheets are one terrain rather than one each because a terrain only stitches tile
// borders against neighbours of its own. Split across two, the tiles either side of the
// boundary have no neighbour to stitch from, their borders keep the no data they were
// created with, and bilinear filtering pulls that in as a dark line a couple of texels
// wide along the whole seam. Inside one terrain the boundary is interior and stitches.
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
        src_path: vec![
            "preprocess/source_data/auckland/height/1m".into(),
            "preprocess/source_data/auckland_west_south/height/1m".into(),
        ],
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
        provenance_only: provenance_only(),
    };

    let (src_dataset, mut context) = PreprocessContext::from_cli(args).unwrap();

    preprocess(src_dataset, &mut context);

    let args = Cli {
        // Every level feeds the one attachment, coarsest first so the finest wins wherever
        // it exists: gdalbuildvrt draws later sources on top. The 2010 mosaic covers all
        // three sheets, the 2024 0.25 m survey the outer two where it was flown, and only
        // the centre sheet has 0.075 m. The grid this terrain samples is around 0.6 m, so
        // the coarser levels lose nothing.
        //
        // The 0.25 m level comes in through a .vrt rather than its directory because it is
        // colour matched first: that survey runs blue heavy enough against the 0.075 m one
        // to read as a different season across the boundary. See colour_match.sh.
        src_path: vec![
            "preprocess/source_data/auckland/albedo/0.5m".into(),
            "preprocess/source_data/auckland_west_south/albedo/0.5m".into(),
            "preprocess/source_data/auckland_west_south/albedo/0.25m.vrt".into(),
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
        provenance_only: provenance_only(),
    };

    let (src_dataset, mut context) = PreprocessContext::from_cli(args).unwrap();

    preprocess(src_dataset, &mut context);
}
