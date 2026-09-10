use crate::{
    dataset::PreprocessContext,
    result::{PreprocessError, PreprocessResult},
};
use gag::Gag;
use gdal::{
    Dataset, GeoTransform,
    errors::{GdalError, Result as GdalResult},
};
use gdal_sys::{
    CPLErr, CPLErrorReset, CPLGetLastErrorMsg, CPLGetLastErrorNo, CSLSetNameValue,
    GDALAccess::GA_Update, GDALChunkAndWarpImage, GDALCreateWarpOptions, GDALDestroyWarpOperation,
    GDALDestroyWarpOptions, GDALDummyProgress, GDALFillNodata, GDALOpenShared, GDALResampleAlg,
    GDALSuggestedWarpOutput,
};
use glam::U64Vec2;
use itertools::Itertools;
use std::{
    env,
    ffi::{CStr, CString, c_char, c_double, c_int, c_void},
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr, slice,
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
};
use thread_local::ThreadLocal;

// The slots below mirror GDALTransformerInfo from gdal_alg.h. Their signatures have to
// match exactly: GDALDestroyTransformer calls pfn_cleanup with no null check, and
// GDALCloneTransformer null-checks pfn_serialize before using it.
type TransformFunc = unsafe extern "C" fn(
    *mut c_void,
    c_int,
    c_int,
    *mut f64,
    *mut f64,
    *mut f64,
    *mut c_int,
) -> c_int;
type CleanupFunc = unsafe extern "C" fn(*mut c_void);
type SerializeFunc = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type CreateSimilarFunc = unsafe extern "C" fn(
    transformer_arg: *mut c_void,
    src_ratio_x: f64,
    src_ratio_y: f64,
) -> *mut c_void;

/// Cleanup for the transformer this process owns. GDAL calls it on every entry of its
/// thread-to-transformer map, including the original it only borrowed, so the original
/// must not free itself.
unsafe extern "C" fn no_cleanup(_: *mut c_void) {}

#[repr(C)]
pub struct GDALTransformerInfo {
    aby_signature: [u8; 4],
    psz_class_name: *const c_char,
    pfn_transform: TransformFunc,
    pfn_cleanup: CleanupFunc,
    pfn_serialize: Option<SerializeFunc>,
    pfn_create_similar: Option<CreateSimilarFunc>,
}

// Every field is a function pointer or a 'static C string that GDAL only ever reads.
unsafe impl Send for GDALTransformerInfo {}
unsafe impl Sync for GDALTransformerInfo {}

impl GDALTransformerInfo {
    /// The transformer owned by this process, which GDAL borrows but must never free.
    pub(crate) fn owner(similar_func: CreateSimilarFunc) -> Self {
        Self::with_cleanup(similar_func, no_cleanup)
    }

    /// A clone handed to GDAL, which frees it through pfn_cleanup in GWKThreadsEnd.
    pub(crate) fn gdal_owned_clone(
        similar_func: CreateSimilarFunc,
        cleanup_func: CleanupFunc,
    ) -> Self {
        Self::with_cleanup(similar_func, cleanup_func)
    }

    fn with_cleanup(similar_func: CreateSimilarFunc, cleanup_func: CleanupFunc) -> Self {
        Self {
            aby_signature: *b"GTI2",
            psz_class_name: c"BevyTerrainCustomTransformer".as_ptr(),
            pfn_transform: transformer_c,
            pfn_cleanup: cleanup_func,
            pfn_serialize: None,
            pfn_create_similar: Some(similar_func),
        }
    }
}

/// repr(C) with info first is load-bearing: GDALDestroyTransformer and
/// GDALCloneTransformer check the leading "GTI2" signature before touching anything else.
#[repr(C)]
pub struct GDALCustomTransformer {
    pub(crate) info: GDALTransformerInfo,
    pub(crate) inner: Arc<dyn Transformer>,
}

const _: () = {
    // A clone is boxed on a GDAL worker and dropped on the thread that ends the warp, and
    // the original is shared across workers, so both bounds have to hold.
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<GDALCustomTransformer>();
};

pub fn warp(
    src: &Dataset,
    dst: &Dataset,
    context: &PreprocessContext,
    transformer: &GDALCustomTransformer,
    mut progress_callback: Option<&ProgressCallback>,
) -> PreprocessResult<()> {
    let (width, height) = dst.raster_size();

    // make sure, that these outlive the warp operation
    let band_count = context.rasterbands.len() as c_int;
    let mut bands = (1..=band_count).collect_vec();
    let mut src_no_data = src
        .rasterband(1)?
        .no_data_value()
        .map(|value| vec![value; band_count as usize])
        .unwrap_or_default();
    let mut dst_no_data = context
        .no_data_value
        .map(|value| vec![value; band_count as usize])
        .unwrap_or_default();

    let options = unsafe { &mut *GDALCreateWarpOptions() };
    options.hSrcDS = src.c_dataset();
    options.hDstDS = dst.c_dataset();
    // options.eResampleAlg = GDALResampleAlg::GRA_NearestNeighbour;
    options.eResampleAlg = GDALResampleAlg::GRA_Bilinear;
    options.dfWarpMemoryLimit = 1024f64.powi(2) * 8.0; // Todo: figure out, why this affects reprojection at the poles

    // for some reason this is not automatically recognized, so we have to set it manually
    options.eWorkingDataType = context.data_type as u32;
    options.nBandCount = band_count;
    options.panSrcBands = bands.as_mut_ptr();
    options.panDstBands = bands.as_mut_ptr();
    options.padfSrcNoDataReal = if !src_no_data.is_empty() {
        src_no_data.as_mut_ptr()
    } else {
        ptr::null_mut()
    };
    options.padfDstNoDataReal = if !dst_no_data.is_empty() {
        dst_no_data.as_mut_ptr()
    } else {
        ptr::null_mut()
    };

    options.pfnTransformer = Some(transformer_c);
    options.pTransformerArg = ptr::from_ref(transformer).cast_mut().cast();

    (options.pfnProgress, options.pProgressArg) = match progress_callback.as_mut() {
        None => (Some(GDALDummyProgress as _), ptr::null_mut()),
        Some(callback) => (Some(progress_c as _), ptr::addr_of_mut!(*callback).cast()),
    };

    // Threading is switched on per warp rather than through GDAL_NUM_THREADS, which would
    // apply process-wide. GWKThreadsCreate reads this option first and only then falls back
    // to the config option, and it runs inside GDALCreateWarpOperation, so it has to be set
    // before that call. The list is freed by GDALDestroyWarpOptions below.
    //
    // Setting GDAL_NUM_THREADS explicitly suppresses the option, so that variable still
    // works as an override - which is what makes a single-threaded comparison run possible.
    if env::var_os("GDAL_NUM_THREADS").is_none() {
        options.papszWarpOptions = unsafe {
            CSLSetNameValue(
                options.papszWarpOptions,
                c"NUM_THREADS".as_ptr(),
                c"ALL_CPUS".as_ptr(),
            )
        };
    }

    unsafe {
        let operation = gdal_sys::GDALCreateWarpOperation(options);
        let rv = GDALChunkAndWarpImage(operation, 0, 0, width as c_int, height as c_int);

        options.panSrcBands = ptr::null_mut();
        options.panDstBands = ptr::null_mut();
        options.padfSrcNoDataReal = ptr::null_mut();
        options.padfDstNoDataReal = ptr::null_mut();
        GDALDestroyWarpOptions(options);
        GDALDestroyWarpOperation(operation);

        if rv != CPLErr::CE_None {
            return Err(PreprocessError::Gdal(last_cpl_err(rv)));
        }
    }

    Ok(())
}

pub fn fill_no_data(src: &Dataset, fill_radius: f64) -> PreprocessResult<()> {
    for raster_band in src.rasterbands() {
        unsafe {
            let rv = GDALFillNodata(
                raster_band?.c_rasterband(),
                ptr::null_mut(),
                fill_radius,
                0,
                0,
                ptr::null_mut(),
                Some(GDALDummyProgress as _),
                ptr::null_mut(),
            );

            if rv != CPLErr::CE_None {
                return Err(PreprocessError::Gdal(last_cpl_err(rv)));
            }
        }
    }

    Ok(())
}

pub type ProgressCallback<'a> = dyn Fn(f64) -> bool + Sync + 'a;

#[unsafe(no_mangle)]
extern "C" fn progress_c(complete: c_double, _message: *const c_char, arg: *mut c_void) -> c_int {
    assert!(!arg.is_null());
    let progress_callback = unsafe { arg.cast::<&ProgressCallback<'_>>().as_mut().unwrap() };
    progress_callback(complete as _) as i32
}

pub(crate) struct CountingProgressCallback<'a> {
    count: f64,
    counter: AtomicU64,
    progress_callback: Option<&'a ProgressCallback<'a>>,
}

impl<'a> CountingProgressCallback<'a> {
    pub(crate) fn new(count: u64, progress_callback: Option<&'a ProgressCallback<'a>>) -> Self {
        Self {
            count: count as f64,
            counter: AtomicU64::new(1),
            progress_callback,
        }
    }

    pub(crate) fn increment(&self) {
        if let Some(progress_callback) = self.progress_callback {
            progress_callback(self.counter.fetch_add(1, Ordering::Relaxed) as f64 / self.count);
        }
    }
}

/// Takes &self, not &mut self: GDAL calls this concurrently from its warp workers, and
/// one of them always runs on the original transformer rather than a clone.
pub trait Transformer: Send + Sync {
    fn transform(
        &self,
        dst_to_src: bool,
        x: &mut [f64],
        y: &mut [f64],
        z: &mut [f64],
        success: &mut [c_int],
    ) -> PreprocessResult<()>;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn transformer_c(
    arg: *mut c_void,
    dst_to_src: c_int,
    n_point_count: c_int,
    x: *mut f64,
    y: *mut f64,
    z: *mut f64,
    pan_success: *mut c_int,
) -> c_int {
    assert!(!arg.is_null());
    // as_ref, not as_mut: several threads hold this pointer at once.
    let transformer = unsafe { arg.cast::<GDALCustomTransformer>().as_ref().unwrap() };
    let n_point_count = n_point_count as usize;

    // GDAL hands us the array it allocated with CPLMalloc and never initialised, so it is
    // borrowed as [c_int] rather than [bool] - uninitialised bytes are not valid bools.
    let success = unsafe { slice::from_raw_parts_mut(pan_success, n_point_count) };

    match transformer.inner.transform(
        dst_to_src != 0,
        unsafe { slice::from_raw_parts_mut(x, n_point_count) },
        unsafe { slice::from_raw_parts_mut(y, n_point_count) },
        unsafe { slice::from_raw_parts_mut(z, n_point_count) },
        success,
    ) {
        Ok(()) => 1,
        Err(error) => {
            // The warp kernel discards this return value and reads only pan_success, so a
            // failure has to be reported there or it is silently taken for success.
            success.fill(0);
            // GWKRun only propagates a worker failure as a flag, and CPL's error state is
            // thread local, so the message would otherwise never reach the caller.
            eprintln!("transform failed: {error}");
            0
        }
    }
}

pub struct SuggestedWarpOutput {
    pub size: U64Vec2,
    pub geo_transform: GeoTransform,
}

impl SuggestedWarpOutput {
    pub fn compute(
        src: &Dataset,
        transformer: &GDALCustomTransformer,
    ) -> GdalResult<Option<SuggestedWarpOutput>> {
        let _gag_stderr = Gag::stderr();

        let mut geo_transform = GeoTransform::default();
        let (mut width, mut height) = (0, 0);

        let rv = unsafe {
            GDALSuggestedWarpOutput(
                src.c_dataset(),
                Some(transformer_c),
                ptr::from_ref(transformer).cast_mut().cast(),
                geo_transform.as_mut_ptr(),
                &mut width,
                &mut height,
            )
        };

        if rv != CPLErr::CE_None {
            let error = last_cpl_err(rv);

            return match error {
                GdalError::CplError {
                    class: CPLErr::CE_Failure,
                    number: 1,
                    ..
                } => Ok(None),
                _ => Err(error),
            };
        }

        Ok(Some(SuggestedWarpOutput {
            size: U64Vec2::new(width as u64, height as u64),
            geo_transform,
        }))
    }
}

fn last_cpl_err(cpl_err_class: CPLErr::Type) -> GdalError {
    let last_err_no = unsafe { CPLGetLastErrorNo() };
    let last_err_msg = unsafe { CStr::from_ptr(CPLGetLastErrorMsg()) }
        .to_string_lossy()
        .into_owned();
    unsafe { CPLErrorReset() };

    GdalError::CplError {
        class: cpl_err_class,
        number: last_err_no,
        msg: last_err_msg,
    }
}

pub struct SharedReadOnlyDataset {
    path: CString,
    pool: ThreadLocal<Dataset>,
}

impl SharedReadOnlyDataset {
    pub fn new(path: &Path) -> Self {
        Self {
            path: CString::new(path.as_os_str().as_bytes()).unwrap(),
            pool: ThreadLocal::new(),
        }
    }
    pub fn get(&self) -> &Dataset {
        self.pool.get_or(|| unsafe {
            Dataset::from_c_dataset(GDALOpenShared(self.path.as_ptr(), GA_Update))
        })
    }
}

#[cfg(test)]
mod test {
    use gdal_sys::{GDALProgressFunc, GDALTransformerFunc};

    use super::*;

    fn accept_transformer_c(_transformer: GDALTransformerFunc) {}

    #[test]
    fn transformer_c_signature_is_correct() {
        accept_transformer_c(Some(transformer_c));
    }

    fn accept_progress_c(_progress: GDALProgressFunc) {}

    #[test]
    fn progress_c_signature_is_correct() {
        accept_progress_c(Some(progress_c));
    }
}
