use crate::{
    gdal_extension::{GDALCustomTransformer, GDALTransformerInfo, Transformer},
    result::{PreprocessError, PreprocessResult},
};
use bevy_terrain::math::Coordinate;
use gdal::spatial_ref::AxisMappingStrategy;
use gdal::{Dataset, GeoTransform, GeoTransformEx, errors::GdalError, spatial_ref::SpatialRef};
use gdal_sys::{
    GDALCreateReprojectionTransformerEx, GDALDestroyReprojectionTransformer,
    GDALReprojectionTransform, OSRGetDataAxisToSRSAxisMapping, OSRSetDataAxisToSRSAxisMapping,
};
use glam::{DVec2, DVec3};
use itertools::izip;
use std::ffi::{c_int, c_void};
use std::ptr;
use std::slice;
use std::sync::Arc;
use thread_local::ThreadLocal;

impl Transformer for GeoTransform {
    fn transform(
        &self,
        dst_to_src: bool,
        x: &mut [f64],
        y: &mut [f64],
        _: &mut [f64],
        _: &mut [c_int],
    ) -> PreprocessResult<()> {
        let transform = if dst_to_src { *self } else { self.invert()? };

        for (x, y) in x.iter_mut().zip(y.iter_mut()) {
            (*x, *y) = transform.apply(*x, *y);
        }

        Ok(())
    }
}

pub struct ReprojectionTransformer {
    ptr: *mut c_void,
}

// PROJ rebinds the underlying PJ to the calling thread's context before every transform,
// and again before destroying it, so one of these may be used from, and dropped on, a
// thread other than the one that built it. It is deliberately NOT Sync: two threads
// sharing a single one is exactly the data race this whole change removes.
unsafe impl Send for ReprojectionTransformer {}

impl ReprojectionTransformer {
    fn new(src_spatial_ref: &SpatialRef, dst_spatial_ref: &SpatialRef) -> PreprocessResult<Self> {
        let ptr = unsafe {
            GDALCreateReprojectionTransformerEx(
                src_spatial_ref.to_c_hsrs(),
                dst_spatial_ref.to_c_hsrs(),
                ptr::null(),
            )
        };
        if ptr.is_null() {
            return Err(GdalError::NullPointer {
                method_name: "GDALCreateReprojectionTransformerEx",
                msg: "Creating the transformer failed".to_string(),
            }
            .into());
        }

        Ok(Self { ptr })
    }
}

impl Drop for ReprojectionTransformer {
    fn drop(&mut self) {
        unsafe { GDALDestroyReprojectionTransformer(self.ptr) }
    }
}

// Deliberately an inherent method rather than a Transformer impl: that trait requires
// Sync, and this type must never be shared between threads. Only the pool implements it.
impl ReprojectionTransformer {
    fn transform(
        &self,
        dst_to_src: bool,
        x: &mut [f64],
        y: &mut [f64],
        z: &mut [f64],
        success: &mut [c_int],
    ) -> PreprocessResult<()> {
        let mut success_int = vec![0; x.len()];

        let return_value = unsafe {
            GDALReprojectionTransform(
                self.ptr,
                dst_to_src.into(),
                x.len().try_into().unwrap(),
                x.as_mut_ptr(),
                y.as_mut_ptr(),
                z.as_mut_ptr(),
                success_int.as_mut_ptr(),
            )
        };

        if return_value == 0 {
            return Err(PreprocessError::TransformOperationFailed);
        }

        for (success, &returned) in success.iter_mut().zip(success_int.iter()) {
            *success = (*success != 0 && returned != 0) as c_int;
        }

        Ok(())
    }
}

/// Enough of a SpatialRef to rebuild an equivalent one on another thread.
///
/// The axis mapping is the load-bearing part. A raster driver hands out its SRS in
/// traditional GIS order, while SpatialRef::from_wkt returns an authority-compliant one,
/// which transposes the axes for a CRS whose authority order is northing first - EPSG:2193,
/// the New Zealand source, is exactly such a CRS. Capturing the strategy alone is not
/// enough either, because a custom mapping reports its strategy faithfully while silently
/// reverting to the default order.
struct SrsSpec {
    wkt: String,
    strategy: AxisMappingStrategy,
    mapping: Vec<c_int>,
}

fn data_axis_mapping(srs: &SpatialRef) -> Vec<c_int> {
    let mut count: c_int = 0;
    let ptr = unsafe { OSRGetDataAxisToSRSAxisMapping(srs.to_c_hsrs(), &mut count) };

    if ptr.is_null() || count <= 0 {
        return Vec::new();
    }

    unsafe { slice::from_raw_parts(ptr, count as usize) }.to_vec()
}

impl SrsSpec {
    fn capture(srs: &SpatialRef) -> PreprocessResult<Self> {
        let spec = Self {
            wkt: srs.to_wkt()?,
            strategy: srs.axis_mapping_strategy(),
            mapping: data_axis_mapping(srs),
        };

        // Checked here, once, on the main thread rather than assumed: a mapping that does
        // not survive the round trip would transpose every coordinate of the output with
        // nothing to show for it but wrong geography.
        if data_axis_mapping(&spec.build()?) != spec.mapping {
            return Err(PreprocessError::AxisMappingNotPreserved);
        }

        Ok(spec)
    }

    fn build(&self) -> PreprocessResult<SpatialRef> {
        let mut srs = SpatialRef::from_wkt(&self.wkt)?;
        srs.set_axis_mapping_strategy(self.strategy);

        if !self.mapping.is_empty() {
            unsafe {
                OSRSetDataAxisToSRSAxisMapping(
                    srs.to_c_hsrs(),
                    self.mapping.len() as c_int,
                    self.mapping.as_ptr(),
                )
            };
        }

        Ok(srs)
    }
}

/// One reprojection transformer per thread, built on first use from immutable specs.
///
/// This mirrors SharedReadOnlyDataset, which already hands out a Dataset per thread for
/// the same reason. ThreadLocal<T> is Sync as long as T is Send, which is what lets the
/// whole transformer be shared across GDAL's warp workers.
pub struct ReprojectionTransformerPool {
    src: SrsSpec,
    dst: SrsSpec,
    pool: ThreadLocal<ReprojectionTransformer>,
}

impl ReprojectionTransformerPool {
    fn new(src: &SpatialRef, dst: &SpatialRef) -> PreprocessResult<Self> {
        let pool = Self {
            src: SrsSpec::capture(src)?,
            dst: SrsSpec::capture(dst)?,
            pool: ThreadLocal::new(),
        };

        // Build one now so a spec that cannot be rebuilt fails here, rather than inside a
        // warp worker whose error the kernel would discard.
        pool.get()?;

        Ok(pool)
    }

    fn get(&self) -> PreprocessResult<&ReprojectionTransformer> {
        self.pool
            .get_or_try(|| ReprojectionTransformer::new(&self.src.build()?, &self.dst.build()?))
    }
}

impl Transformer for ReprojectionTransformerPool {
    fn transform(
        &self,
        dst_to_src: bool,
        x: &mut [f64],
        y: &mut [f64],
        z: &mut [f64],
        success: &mut [c_int],
    ) -> PreprocessResult<()> {
        self.get()?.transform(dst_to_src, x, y, z, success)
    }
}

struct CubeTransformer {
    face: u32,
}

impl CubeTransformer {
    fn new(face: u32) -> Self {
        Self { face }
    }
}

impl Transformer for CubeTransformer {
    fn transform(
        &self,
        dst_to_src: bool,
        lon_or_u: &mut [f64],
        lat_or_v: &mut [f64],
        _: &mut [f64],
        success: &mut [c_int],
    ) -> PreprocessResult<()> {
        // Todo: convert to and from spherical to ellipsoidal lat/lon
        // Todo: check unit <--> lat/lon

        if dst_to_src {
            for (lon_or_u, lat_or_v, success) in
                izip!(lon_or_u.iter_mut(), lat_or_v.iter_mut(), success.iter_mut())
            {
                let coordinate = Coordinate::new(self.face, DVec2::new(*lon_or_u, *lat_or_v));
                let unit_position = coordinate.unit_position(true);

                let lon = unit_position.z.atan2(-unit_position.x);
                let lat = unit_position.y.asin();

                *success = (*success != 0 && !lat.is_nan()) as c_int;
                *lon_or_u = lon.to_degrees();
                *lat_or_v = lat.to_degrees();
            }
        } else {
            for (lon_or_u, lat_or_v, success) in
                izip!(lon_or_u.iter_mut(), lat_or_v.iter_mut(), success.iter_mut())
            {
                let lon = lon_or_u.to_radians();
                let lat = lat_or_v.to_radians();

                let unit_position =
                    DVec3::new(-lat.cos() * lon.cos(), lat.sin(), lat.cos() * lon.sin());

                let coordinate = Coordinate::from_unit_position(unit_position, true);

                *success = (*success != 0
                    && (unit_position.length() - 1.0).abs() < 0.00001
                    && coordinate.face == self.face) as c_int;
                *lon_or_u = coordinate.uv.x;
                *lat_or_v = coordinate.uv.y;
            }
        }
        Ok(())
    }
}

/// GDAL clones the transformer once per warp worker. The clone shares the same inner
/// state through an Arc - the per-thread work happens inside ReprojectionTransformerPool -
/// and exists so that each entry in GDAL's thread-to-transformer map owns a distinct
/// allocation it may free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clone_custom_transformer(
    arg: *mut c_void,
    src_ratio_x: f64,
    src_ratio_y: f64,
) -> *mut c_void {
    // The warp path always clones at 1:1; only VRT overview building asks for anything
    // else, and nothing here does. Returning null on an unexpected ratio would hang rather
    // than fail, because the thread that records the failure never signals the progress
    // condition variable the main thread waits on, so this aborts instead.
    assert!(
        src_ratio_x == 1.0 && src_ratio_y == 1.0,
        "cloning at a reduced resolution is not supported"
    );

    // A shared read, which is what makes this legal to run while another worker is inside
    // transformer_c on the same original.
    let source = unsafe { &*arg.cast::<GDALCustomTransformer>() };

    Box::into_raw(Box::new(GDALCustomTransformer {
        info: GDALTransformerInfo::gdal_owned_clone(
            clone_custom_transformer,
            destroy_cloned_transformer,
        ),
        inner: Arc::clone(&source.inner),
    }))
    .cast()
}

/// Frees a clone. GDAL calls this from GWKThreadsEnd, on the thread that ends the warp
/// rather than the one that used the clone.
unsafe extern "C" fn destroy_cloned_transformer(arg: *mut c_void) {
    drop(unsafe { Box::from_raw(arg.cast::<GDALCustomTransformer>()) });
}

#[repr(C)]
pub struct CustomTransformer {
    src_inverse_geo_transform: GeoTransform,
    dst_geo_transform: Option<GeoTransform>,
    lon_lat_transformer: ReprojectionTransformerPool,
    cube_transformer: CubeTransformer,
}

impl CustomTransformer {
    pub fn new(
        src: &Dataset,
        face: u32,
        dst_geo_transform: Option<GeoTransform>,
    ) -> PreprocessResult<GDALCustomTransformer> {
        Ok(GDALCustomTransformer {
            info: GDALTransformerInfo::owner(clone_custom_transformer),
            inner: Arc::new(Self {
                src_inverse_geo_transform: src.geo_transform()?.invert()?,
                dst_geo_transform,
                lon_lat_transformer: ReprojectionTransformerPool::new(
                    &src.spatial_ref()?,
                    &SpatialRef::from_proj4("+proj=lonlat +ellps=WGS84 +datum=WGS84")?,
                )?,
                cube_transformer: CubeTransformer::new(face),
            }),
        })
    }
}

impl Transformer for CustomTransformer {
    fn transform(
        &self,
        dst_to_src: bool,
        x: &mut [f64],
        y: &mut [f64],
        z: &mut [f64],
        success: &mut [c_int],
    ) -> PreprocessResult<()> {
        // gdal suggest requires a bidirectional transformer from src pixel space, to destination uv space
        // gdal warp requires a unidirectional transformer from destination pixel space, to src pixel space

        // for some strange reason success is not correctly initialized
        for success in success.iter_mut() {
            *success = 1;
        }

        if dst_to_src {
            if let Some(geo_transform) = self.dst_geo_transform {
                geo_transform.transform(dst_to_src, x, y, z, success)?;
            }

            self.cube_transformer
                .transform(dst_to_src, x, y, z, success)?;
            self.lon_lat_transformer
                .transform(dst_to_src, x, y, z, success)?;
            self.src_inverse_geo_transform
                .transform(dst_to_src, x, y, z, success)?;
        } else {
            // this only runs during the suggest phase
            // here we output uv coordinates directly, without applying a geo transform (we want to compute this)
            self.src_inverse_geo_transform
                .transform(dst_to_src, x, y, z, success)?;
            self.lon_lat_transformer
                .transform(dst_to_src, x, y, z, success)?;
            self.cube_transformer
                .transform(dst_to_src, x, y, z, success)?;
        }

        Ok(())
    }
}
