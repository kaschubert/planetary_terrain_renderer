use crate::{
    dataset::{FaceInfo, PreprocessContext, available_bytes, create_empty_dataset},
    gdal_extension::{GDALCustomTransformer, ProgressCallback, SuggestedWarpOutput, warp},
    result::{PreprocessError, PreprocessResult},
    transformers::CustomTransformer,
};
use bevy_terrain::prelude::AttachmentLabel;
use gdal::{Dataset, GeoTransform, GeoTransformEx, raster::GdalType};
use glam::{DVec2, IVec2, U64Vec2};
use itertools::Itertools;
use std::{collections::HashMap, fs};

pub struct Transform<'a> {
    pub transformer: GDALCustomTransformer,
    pub face: u32,
    pub lod: u32,
    pub size: U64Vec2,
    pub geo_transform: GeoTransform,
    pub uv_start: DVec2,
    pub uv_end: DVec2,
    pub pixel_start: IVec2,
    pub pixel_end: IVec2,
    pub progress_callback: Option<Box<ProgressCallback<'a>>>,
}

pub fn reproject<T: Copy + GdalType>(
    src_dataset: Dataset,
    context: &mut PreprocessContext,
    progress_callback: Option<&ProgressCallback>,
) -> PreprocessResult<HashMap<u32, FaceInfo>> {
    if let Some(progress_callback) = progress_callback {
        progress_callback(0.0);
    }

    let mut transforms = compute_transforms(&src_dataset, context, progress_callback)?;

    let faces = transforms
        .iter_mut()
        .map(|transform| {
            let dst_path = context.temp_dir.join(format!("face{}.tif", transform.face));
            // Warp into a .partial file and rename once it is complete, so an
            // interrupted run never leaves a truncated raster under the name a
            // later resume would trust.
            let partial_path = dst_path.with_extension("tif.partial");

            // Only reuse a raster that matches what this run would have created.
            // The file name says nothing about the settings it was warped with, so a
            // run with a different size, band count or data type must warp again
            // rather than silently inherit the previous one.
            let existing = (context.resume && dst_path.is_file())
                .then(|| Dataset::open(&dst_path).ok())
                .flatten()
                .filter(|dataset| {
                    dataset.raster_size() == (transform.size.x as usize, transform.size.y as usize)
                        && dataset.raster_count() == context.rasterbands.len()
                        && dataset
                            .rasterband(1)
                            .is_ok_and(|band| band.band_type() == T::datatype())
                });
            let reused = existing.is_some();

            let dst_dataset = if let Some(dst_dataset) = existing {
                if let Some(progress_callback) = transform.progress_callback.as_deref() {
                    progress_callback(1.0);
                }

                dst_dataset
            } else {
                let dst_dataset = create_empty_dataset::<T>(
                    &partial_path,
                    transform.size,
                    Some(transform.geo_transform),
                    &context,
                )?;

                warp(
                    &src_dataset,
                    &dst_dataset,
                    &context,
                    &transform.transformer,
                    transform.progress_callback.as_deref(),
                )?;

                dst_dataset
            };

            if matches!(context.attachment_label, AttachmentLabel::Height) {
                let min_max = dst_dataset
                    .rasterband(1)
                    .unwrap()
                    .compute_raster_min_max(true)
                    .unwrap();

                context.min_height = context.min_height.min(min_max.min as f32);
                context.max_height = context.max_height.max(min_max.max as f32);
            }

            if !reused {
                drop(dst_dataset); // flush to disk before the rename publishes the result
                fs::rename(&partial_path, &dst_path).unwrap();
            }

            Ok((
                transform.face,
                FaceInfo {
                    lod: transform.lod,
                    pixel_start: transform.pixel_start,
                    pixel_end: transform.pixel_end,
                    path: dst_path,
                },
            ))
        })
        .collect::<PreprocessResult<HashMap<_, _>>>()?;

    Ok(faces)
}

/// Refuses to start a run that cannot fit on disk. Both figures are upper bounds: tiles
/// that turn out to be entirely no-data are never written, and a reprojection only
/// allocates the blocks the warp actually touches.
pub(crate) fn check_disk_space<T: Copy + GdalType>(
    src_dataset: &Dataset,
    context: &mut PreprocessContext,
) -> PreprocessResult<()> {
    const GIB: f64 = (1u64 << 30) as f64;

    // cheap next to the warp itself, and it must happen before anything is deleted
    let transforms = compute_transforms(src_dataset, context, None)?;

    let sample_size = (size_of::<T>() * context.rasterbands.len()) as u64;
    let center_size = context.attachment.center_size() as i32;

    let temp_bytes: u64 = transforms
        .iter()
        .map(|transform| transform.size.element_product() * sample_size)
        .sum();

    let tile_count: u64 = transforms
        .iter()
        .map(|transform| {
            let xy_start = transform.pixel_start / center_size;
            let xy_end = (transform.pixel_end - 1) / center_size + 1;
            let xy = (xy_end - xy_start).max(IVec2::ZERO);

            xy.x as u64 * xy.y as u64
        })
        .sum();

    // the coarser lods add roughly a third on top of the finest one
    let tile_bytes =
        tile_count * 4 / 3 * (context.attachment.texture_size as u64).pow(2) * sample_size;

    // A budget given explicitly is enforced; the free space is only reported, because
    // the figures above are upper bounds and a sparse source can come in far under them.
    let (available, enforced) = match context.disk_budget {
        Some(budget) => (budget, true),
        None => (
            available_bytes(&context.terrain_path).unwrap_or(u64::MAX),
            false,
        ),
    };

    println!(
        "Estimated disk usage: at most {:.1} GiB reprojection + {:.1} GiB tiles = {:.1} GiB, {:.1} GiB available",
        temp_bytes as f64 / GIB,
        tile_bytes as f64 / GIB,
        (temp_bytes + tile_bytes) as f64 / GIB,
        available as f64 / GIB,
    );

    if temp_bytes + tile_bytes > available {
        let error = PreprocessError::InsufficientDiskSpace {
            needed_gib: (temp_bytes + tile_bytes) as f64 / GIB,
            temp_gib: temp_bytes as f64 / GIB,
            tile_gib: tile_bytes as f64 / GIB,
            available_gib: available as f64 / GIB,
        };

        if enforced {
            return Err(error);
        }

        println!("Warning: {error}. Sources with no-data regions usually need much less.");
    }

    Ok(())
}

pub fn compute_transforms<'a>(
    src_dataset: &Dataset,
    context: &mut PreprocessContext,
    progress_callback: Option<&'a ProgressCallback>,
) -> PreprocessResult<Vec<Transform<'a>>> {
    let mut transforms = Vec::with_capacity(6);

    let mut total_area = 0.0;

    for face in 0..6 {
        let transformer = CustomTransformer::new(src_dataset, face, None)?;

        let Some(SuggestedWarpOutput {
            size,
            mut geo_transform,
        }) = SuggestedWarpOutput::compute(src_dataset, &transformer)?
        else {
            continue;
        };

        // flip y axis
        geo_transform[3] = geo_transform[3] + geo_transform[5] * size.y as f64;
        geo_transform[5] = -geo_transform[5];

        let uv_start = DVec2::from(geo_transform.apply(0.0, 0.0)).max(DVec2::ZERO);
        let uv_end = DVec2::from(geo_transform.apply(size.x as f64, size.y as f64)).min(DVec2::ONE);

        total_area += (uv_end - uv_start).element_product();

        transforms.push(Transform {
            face,
            size,
            uv_start,
            uv_end,
            lod: 0,
            pixel_start: IVec2::ZERO,
            pixel_end: IVec2::ZERO,
            transformer,
            geo_transform,
            progress_callback: None,
        });
    }

    let max_lod = if let Some(lod_count) = context.lod_count {
        lod_count - 1
    } else {
        let mut max_lod = 0;

        for transform in &mut transforms {
            // GDAL uses a heuristic to compute the output dimensions in pixels by setting
            // the same number of pixels on the diagonal on both the input and output
            // projections. Since we have up to six different output images, this
            // heuristic must be modified a bit. Since the S2 projection with a
            // quadratic mapping is quite area-uniform, we divide the total GDAL based
            // output image into the six output images by their area proportion of the total
            // output.

            let uv_size = transform.uv_end - transform.uv_start;

            let correction = uv_size.element_product().sqrt() / total_area.sqrt();
            let size = (transform.size.as_dvec2() * correction).round();

            max_lod = max_lod.max(
                (size / context.attachment.center_size() as f64 / uv_size)
                    .max_element()
                    .log2()
                    .ceil() as u32,
            );
        }

        context.lod_count = Some(max_lod + 1);

        max_lod
    };

    let pixel_size = 1.0 / ((1 << max_lod) * context.attachment.center_size()) as f64;

    for transform in &mut transforms {
        let pixel_start = (transform.uv_start / pixel_size).floor();
        let pixel_end = (transform.uv_end / pixel_size).ceil();

        // println!(
        //     "Snapping to the quadtree pixel grid caused the size of the reprojected dataset to be adjusted from {} to {}. This is an up-scaling of {:.2}%.",
        //     transform.size,
        //     size,
        //     (size.as_dvec2() / transform.size.as_dvec2()).element_product() * 100.0 - 100.0
        // );

        transform.lod = max_lod;
        transform.size = (pixel_end - pixel_start).as_u64vec2();
        transform.geo_transform = GeoTransform::from([
            pixel_start.x * pixel_size,
            pixel_size,
            0.0,
            pixel_start.y * pixel_size,
            0.0,
            pixel_size,
        ]);
        transform.pixel_start = pixel_start.as_ivec2();
        transform.pixel_end = pixel_end.as_ivec2();
        transform.transformer =
            CustomTransformer::new(src_dataset, transform.face, Some(transform.geo_transform))?;
    }

    let work_portions = transforms
        .iter()
        .map(|transform| transform.size.element_product())
        .collect_vec();
    let total_work = work_portions.iter().sum::<u64>() as f64;
    let callback_intervals = work_portions
        .iter()
        .scan(0, |work_done, &work_portion| {
            *work_done += work_portion;
            Some((
                (*work_done - work_portion) as f64 / total_work,
                work_portion as f64 / total_work,
            ))
        })
        .collect_vec();

    for (transform, (offset, scale)) in transforms.iter_mut().zip(callback_intervals) {
        transform.progress_callback = progress_callback.map(|progress_callback| {
            Box::new(move |completion: f64| {
                progress_callback(completion.clamp(0.0, 1.0).mul_add(scale, offset))
            }) as Box<ProgressCallback>
        })
    }

    Ok(transforms)
}
