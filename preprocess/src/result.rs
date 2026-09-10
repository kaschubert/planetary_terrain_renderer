use gdal::errors::GdalError;
use std::num::ParseFloatError;
use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum PreprocessError {
    #[error("unknown rasterband data type")]
    UnknownRasterbandDataType,
    #[error("transform operation failed")]
    TransformOperationFailed,
    #[error("The no data value is outside of the datatypes range.")]
    NoDataOutOfRange,
    #[error(
        "the spatial reference does not survive a WKT round trip with its axis mapping intact, \
         so it cannot be rebuilt per thread"
    )]
    AxisMappingNotPreserved,
    #[error("GDAL error")]
    Gdal(#[from] GdalError),
    #[error("Parse error")]
    Parse(#[from] ParseFloatError),
    #[error(
        "not enough disk space: needs about {needed_gib:.1} GiB \
         ({temp_gib:.1} GiB reprojection + {tile_gib:.1} GiB tiles), \
         but only {available_gib:.1} GiB is available"
    )]
    InsufficientDiskSpace {
        needed_gib: f64,
        temp_gib: f64,
        tile_gib: f64,
        available_gib: f64,
    },
}

pub type PreprocessResult<T> = Result<T, PreprocessError>;
