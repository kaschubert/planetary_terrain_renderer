use bevy_terrain_preprocess::prelude::*;
use clap::Parser;

fn main() {
    // GDAL_NUM_THREADS is deliberately left alone. Setting it to ALL_CPUS makes the
    // warper call pfnTransformer from several threads at once, and transformer_c hands
    // each of them a &mut to the same GDALCustomTransformer, which segfaults. Warping
    // in parallel needs a transformer per thread first.
    let args = Cli::parse();
    let (src_dataset, mut context) = PreprocessContext::from_cli(args).unwrap();

    preprocess(src_dataset, &mut context);
}
