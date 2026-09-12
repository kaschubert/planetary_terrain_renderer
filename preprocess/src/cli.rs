use crate::{
    dataset::{PreprocessDataType, PreprocessNoData},
    gdal_extension::ProgressCallback,
};
use bevy_terrain::prelude::*;
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;

const BAR_SIZE: u64 = 10000;

#[derive(Parser, Debug)]
#[command(name = "btpp", author, version, about)]
pub struct Cli {
    // could be optional and default to the current directory, but that would be
    // risky in combination with overwrite
    #[arg(required = true)]
    pub terrain_path: PathBuf,
    /// The GeoTIFFs to preprocess, or directories containing them.
    // Variadic, so it has to come last: clap cannot tell where it starts unless every
    // positional in front of it is required, which would make all the options below
    // mandatory too.
    #[arg(required = true)]
    pub src_path: Vec<PathBuf>,

    #[arg(short, long, default_value_t = false)]
    pub overwrite: bool,
    /// Reuse a completed reprojection in the temp directory instead of redoing it.
    #[arg(short, long, default_value_t = false)]
    pub resume: bool,
    /// Disk budget in GiB the run must fit into. Defaults to the free space on the target device.
    #[arg(long, default_value = None)]
    pub disk_budget: Option<u64>,
    /// Where to reproject into. Defaults to a temp directory inside the attachment.
    #[arg(long, default_value = None)]
    pub temp_path: Option<PathBuf>,
    #[arg(long, default_value = "source")]
    pub no_data: PreprocessNoData,
    #[arg(long, default_value = "source")]
    pub data_type: PreprocessDataType,
    #[arg(long, default_value_t = 16.0)]
    pub fill_radius: f32,
    #[arg(long, default_value_t = false)]
    pub create_mask: bool,
    #[arg(long, default_value = None)]
    pub lod_count: Option<u32>,
    #[arg(long, default_value = "height")]
    pub attachment_label: AttachmentLabel,
    #[arg(short, long = "ts", default_value_t = 512)]
    pub texture_size: u32,
    #[arg(short, long = "bs", default_value_t = 1)]
    pub border_size: u32,
    #[arg(short, long = "m", default_value_t = 1)]
    pub mip_level_count: u32,
    #[arg(long, default_value = "r16u")]
    pub format: AttachmentFormat,
}

pub(crate) struct PreprocessBar<'a> {
    name: String,
    bar: ProgressBar,
    callback: Box<ProgressCallback<'a>>,
}

impl PreprocessBar<'_> {
    pub(crate) fn new(name: String) -> Self {
        let bar = ProgressBar::new(BAR_SIZE).with_style(
            ProgressStyle::with_template(
                &(name.clone() + " dataset: {wide_bar} {percent} % [{elapsed}/{duration}])"),
            )
            .unwrap(),
        );

        let callback = Box::new({
            let progress_bar = bar.clone();
            move |completion| {
                progress_bar.set_position((completion * BAR_SIZE as f64) as u64);
                true
            }
        });

        Self {
            name,
            bar,
            callback,
        }
    }

    pub(crate) fn callback(&self) -> &ProgressCallback<'_> {
        self.callback.as_ref()
    }

    pub(crate) fn finish(&self) {
        self.bar.finish_and_clear();
        println!("{} took: {:?}", self.name, self.bar.elapsed());
    }
}
