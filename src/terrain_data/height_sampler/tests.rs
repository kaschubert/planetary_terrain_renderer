//! The pure parts run anywhere. The tests against the terrain data skip, with a note, when
//! the data folders are not on this machine, since they are not in the repository.

use super::*;
use std::{cell::Cell, path::PathBuf};

/// A tile texel's bits with the mask set, as the preprocessor writes real data.
fn valid(height: f32) -> f32 {
    f32::from_bits(height.to_bits() | 1)
}

/// The same with the mask cleared: NoData that the fill interpolated.
fn filled(height: f32) -> f32 {
    f32::from_bits(height.to_bits() & !1)
}

/// An 8 by 8 tile with a one texel border, so a data area of 6, holding 10 * row + column
/// in every texel. Small enough to reason about by hand and large enough for a border on
/// every side.
fn test_layout() -> AttachmentConfig {
    AttachmentConfig {
        texture_size: 8,
        border_size: 1,
        mip_level_count: 1,
        mask: true,
        format: AttachmentFormat::R32F,
    }
}

fn test_tile() -> HeightTile {
    HeightTile {
        size: 8,
        texels: (0..64)
            .map(|index| valid((10 * (index / 8) + index % 8) as f32))
            .collect(),
    }
}

/// The position inside the data area whose texel coordinate is exactly this, so that a
/// whole number lands on a texel centre and a half midway between two.
fn position_at_texel(x: f64, y: f64, attachment: &AttachmentConfig) -> DVec2 {
    (DVec2::new(x, y) + 0.5 - attachment.border_size as f64) / attachment.center_size() as f64
}

#[test]
fn sampler_can_move_into_a_task() {
    fn assert_send<T: Send + 'static>() {}
    assert_send::<TerrainHeightSampler>();
}

#[test]
fn tile_addressing() {
    // Four tiles across at lod 2: 0.3 is a fifth of the way into the second, 0.7 four
    // fifths into the third.
    let (xy, position) = tile_position(DVec2::new(0.3, 0.7), 2);
    assert_eq!(xy, IVec2::new(1, 2));
    assert!((position - DVec2::new(0.2, 0.8)).abs().max_element() < 1e-9);

    // The far edge of the face stays in the last tile rather than addressing a fifth.
    let (xy, position) = tile_position(DVec2::ONE, 2);
    assert_eq!(xy, IVec2::new(3, 3));
    assert!(position.max_element() < 1.0 && position.min_element() > 0.999);

    // Noise below zero stays in the first.
    let (xy, position) = tile_position(DVec2::splat(-1e-12), 2);
    assert_eq!(xy, IVec2::ZERO);
    assert_eq!(position, DVec2::ZERO);

    // At the finest Auckland level the row holds 32768 tiles.
    let (xy, position) = tile_position(DVec2::new(0.5, 0.25), 15);
    assert_eq!(xy, IVec2::new(16384, 8192));
    assert_eq!(position, DVec2::ZERO);
}

#[test]
fn mask_bit() {
    let bits = valid(123.456).to_bits();
    assert_eq!(bits & 1, 1);
    let height = texel_height(bits, true).expect("a set bit is real data");
    // Clearing the bit moves the value by one unit in the last place at most.
    assert!((height - 123.456).abs() < 1e-4);

    assert_eq!(texel_height(filled(123.456).to_bits(), true), None);

    // Without a mask bit 0 is height like the rest, and the value is left alone.
    let bits = filled(123.456).to_bits();
    assert_eq!(texel_height(bits, false), Some(f32::from_bits(bits) as f64));

    // NoData that the fill never reached is missing however its bit is set.
    assert_eq!(texel_height(valid(-9999.0).to_bits(), true), None);
    assert_eq!(texel_height((-9999.0f32).to_bits(), false), None);
    assert_eq!(texel_height(f32::NAN.to_bits(), false), None);
}

#[test]
fn bilinear_weights() {
    let layout = test_layout();
    let tile = test_tile();
    let sample = |x, y| sample_bilinear(&tile, position_at_texel(x, y, &layout), &layout);
    let close = |value: Option<f64>, expected: f64| {
        let value = value.expect("a height");
        assert!((value - expected).abs() < 1e-4, "{value} is not {expected}");
    };

    // On a texel centre the sample is that texel alone.
    close(sample(3.0, 2.0), 23.0);
    // Midway between two columns it is their mean, and between four their mean too.
    close(sample(3.5, 2.0), 23.5);
    close(sample(3.5, 2.5), 28.5);
    // A quarter of the way weights the near texel three to one.
    close(sample(3.25, 2.0), 23.25);

    // The left edge of the data area reads across into the border texel.
    close(sample(0.5, 2.0), 20.5);
    // So does the right edge, into the border on that side.
    close(sample(6.5, 2.0), 26.5);
}

#[test]
fn bilinear_skips_masked_texels() {
    let layout = test_layout();
    let mut tile = test_tile();
    let close = |value: Option<f64>, expected: f64| {
        let value = value.expect("a height");
        assert!((value - expected).abs() < 1e-4, "{value} is not {expected}");
    };

    // Mask the texel at column 4, row 2. Midway between columns 3 and 4 only 3 is left.
    tile.texels[2 * 8 + 4] = filled(24.0);
    close(
        sample_bilinear(&tile, position_at_texel(3.5, 2.0, &layout), &layout),
        23.0,
    );

    // Between four texels with one masked, the three others are renormalised. At the
    // centre they weigh a quarter each, so the mean of 23, 33 and 34.
    close(
        sample_bilinear(&tile, position_at_texel(3.5, 2.5, &layout), &layout),
        30.0,
    );

    // NoData the fill never reached is dropped the same way.
    tile.texels[2 * 8 + 4] = valid(-9999.0);
    close(
        sample_bilinear(&tile, position_at_texel(3.5, 2.0, &layout), &layout),
        23.0,
    );

    // With the whole footprint masked there is no height.
    for index in [2 * 8 + 3, 2 * 8 + 4, 3 * 8 + 3, 3 * 8 + 4] {
        tile.texels[index] = filled(0.0);
    }
    assert_eq!(
        sample_bilinear(&tile, position_at_texel(3.5, 2.5, &layout), &layout),
        None
    );
}

#[test]
fn cache_keeps_the_recently_used() {
    let mut cache = TileCache::default();
    let decodes = Cell::new(0);
    let key = |index: i32| (0, TileCoordinate::new(0, 0, IVec2::new(index, 0)));
    let mut fetch = |index: i32| {
        cache
            .fetch(key(index), || {
                decodes.set(decodes.get() + 1);
                Some(HeightTile {
                    size: 1,
                    texels: vec![index as f32],
                })
            })
            .map(|tile| tile.texels[0])
    };

    assert_eq!(fetch(0), Some(0.0));
    assert_eq!(fetch(0), Some(0.0));
    assert_eq!(decodes.get(), 1, "a hit does not decode");

    // Fill the cache, touching 0 along the way so that it stays recent, then one more.
    for index in 1..CACHE_CAPACITY as i32 {
        fetch(index);
    }
    fetch(0);
    fetch(CACHE_CAPACITY as i32);
    assert_eq!(decodes.get(), CACHE_CAPACITY + 1);

    // 1 was the oldest and went; 0 was used after it and stayed.
    fetch(0);
    assert_eq!(decodes.get(), CACHE_CAPACITY + 1, "0 was still cached");
    fetch(1);
    assert_eq!(decodes.get(), CACHE_CAPACITY + 2, "1 was evicted");
}

#[test]
fn load_skips_a_missing_terrain() {
    let sampler = TerrainHeightSampler::load(["/nowhere/config.tc.ron"]).expect("a sampler");
    assert!(sampler.is_empty());
}

// The rest reads the terrains under assets/terrains, which are downloaded and preprocessed
// on this machine and not in the repository.

fn terrain_config(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("assets/terrains/{name}/config.tc.ron"))
}

/// A sampler over these terrains, or None with a note when any of them is not here.
fn sampler(names: &[&str]) -> Option<TerrainHeightSampler> {
    let paths: Vec<_> = names.iter().map(|name| terrain_config(name)).collect();

    if let Some(missing) = paths.iter().find(|path| !path.exists()) {
        eprintln!("skipped: {} is not on this machine", missing.display());
        return None;
    }

    Some(TerrainHeightSampler::load(paths).expect("the configs parse"))
}

/// Ground truth from the LINZ 1 m DEM the Auckland terrain was built from, read with
/// gdallocationinfo -valonly -wgs84 on the sheet files under preprocess/source_data. The
/// tiles were warped from the DEM with bilinear resampling at 0.6 m, so a sample is a
/// smoothed reading of the 1 m grid and can differ from the point value by up to half a
/// metre of horizontal slip times the gradient. The gradients quoted are what one metre
/// steps east and north in the DEM showed, and set the tolerance: a quarter of a metre on
/// the flat and half a metre on a 35 degree flank, where half a metre of slip is that much
/// height. Measured on 4 October 2026 the largest difference was 0.11 m, on the
/// Maungakiekie summit; the rest were under 0.08 m. A wrong axis or a lost border offset
/// shows here as a wrong hill or a bias on every slope, so the tolerances are kept this
/// tight on purpose.
const ONE_METRE_DEM: &[(f64, f64, f64, f64, &str)] = &[
    // longitude, latitude, GDAL height, tolerance, where.
    (174.7645, -36.8781, 195.656, 0.5, "Maungawhau summit, BA32"),
    (
        174.7832,
        -36.9002,
        180.803,
        0.5,
        "Maungakiekie summit, BA32",
    ),
    (
        174.7800,
        -36.8350,
        -1.138,
        0.25,
        "Waitemata Harbour water, BA32",
    ),
    (
        174.7660,
        -36.8790,
        111.691,
        0.5,
        "Maungawhau flank, gradient 0.7",
    ),
    (174.7700, -36.8600, 67.369, 0.25, "Grafton, gradient 0.14"),
    (
        174.7850,
        -36.9020,
        108.393,
        0.25,
        "Maungakiekie shoulder, gradient 0.07",
    ),
    (174.7400, -36.8900, 42.663, 0.25, "Owairaka, BA31"),
    (174.7200, -36.8700, 16.408, 0.25, "Avondale, BA31"),
    (174.8000, -36.9500, 10.383, 0.25, "Mangere, BB32"),
    (174.9000, -37.0500, -0.444, 0.25, "Pahurehure Inlet, BB32"),
];

#[test]
fn agrees_with_the_one_metre_dem() {
    let Some(mut sampler) = sampler(&["auckland"]) else {
        return;
    };

    for &(longitude, latitude, expected, tolerance, name) in ONE_METRE_DEM {
        let height = sampler
            .height(longitude, latitude)
            .unwrap_or_else(|| panic!("{name}: no height"));
        eprintln!(
            "{name}: {height:.3} m, GDAL {expected:.3} m, difference {:+.3}",
            height - expected
        );
        assert!(
            (height - expected).abs() <= tolerance,
            "{name}: {height:.3} m is more than {tolerance} m from GDAL's {expected:.3} m"
        );
    }
}

/// The steepest point tried, a 44 degree slope where the DEM climbs 0.96 m per metre
/// eastward. Half a metre of resampling slip is half a metre of height here, so this gets
/// a metre and stands apart from the half metre cases. It measured 0.12 m off.
#[test]
fn agrees_on_a_steep_slope() {
    let Some(mut sampler) = sampler(&["auckland"]) else {
        return;
    };

    let height = sampler.height(174.7620, -36.8760).expect("a height");
    eprintln!("steep slope: {height:.3} m, GDAL 102.199 m");
    assert!((height - 102.199).abs() <= 1.0);
}

/// The 8 m national DEM, through the same GDAL call on preprocess/source_data/nz. Its
/// finest tiles are 9.7 m a texel, coarser than the source, so the resampling slip is a
/// texel wide and on a flank that is a metre of height: the three measured 0.36 m low,
/// 0.92 m high and 0.29 m high. Two metres covers that and still tells a wrong hill apart,
/// since the two DEMs differ by 12 m on the summit.
#[test]
fn agrees_with_the_eight_metre_dem() {
    let Some(mut sampler) = sampler(&["nz"]) else {
        return;
    };

    for (longitude, latitude, expected, name) in [
        (174.7645, -36.8781, 183.295, "Maungawhau summit"),
        (174.7660, -36.8790, 110.188, "Maungawhau flank"),
        (174.7850, -36.9020, 114.771, "Maungakiekie shoulder"),
    ] {
        let height = sampler.height(longitude, latitude).expect(name);
        eprintln!(
            "nz {name}: {height:.3} m, GDAL {expected:.3} m, difference {:+.3}",
            height - expected
        );
        assert!(
            (height - expected).abs() <= 2.0,
            "{name}: {height:.3} m is not near {expected:.3} m"
        );
    }
}

#[test]
fn falls_through_to_the_coarser_terrain() {
    let (Some(mut auckland), Some(mut nz), Some(mut all)) = (
        sampler(&["auckland"]),
        sampler(&["nz"]),
        sampler(&["auckland", "nz", "earth"]),
    ) else {
        return;
    };

    // The Hauraki Gulf north of Rangitoto, inside the BA32 sheet. Both DEMs are NoData over
    // water, so only the globe answers. On this dataset the city terrain still has a lod 10
    // tile there, every texel masked, so this is the mask path and not a missing tile; the
    // note below says which it was.
    let (longitude, latitude) = (174.9000, -36.6500);
    let coordinate = Coordinate::from_unit_position(unit_position(longitude, latitude), true);
    let under = auckland.terrains[0].finest_tile(coordinate);
    eprintln!(
        "gulf: auckland tile {:?}",
        under.map(|(tile, _)| tile.to_string())
    );
    assert_eq!(auckland.height(longitude, latitude), None);
    assert_eq!(nz.height(longitude, latitude), None);
    let gulf = all
        .height(longitude, latitude)
        .expect("the globe has data everywhere");
    eprintln!("gulf from the globe: {gulf:.3} m");

    // The harbour has 1 m data at about -1 m, so the stack answers from the city terrain
    // even though the 8 m DEM is NoData there.
    assert_eq!(nz.height(174.7800, -36.8350), None);
    let harbour = all
        .height(174.7800, -36.8350)
        .expect("the 1 m DEM covers the harbour");
    assert!((harbour - -1.138).abs() <= 0.25, "{harbour}");

    // Where the city terrain has data the stack gives its answer, not the national one.
    let summit = all.height(174.7645, -36.8781).expect("a height");
    assert!((summit - 195.656).abs() <= 0.5, "{summit}");
}

#[test]
fn heights_matches_height() {
    let Some(mut sampler) = sampler(&["auckland", "nz", "earth"]) else {
        return;
    };

    let points: Vec<_> = ONE_METRE_DEM
        .iter()
        .map(|&(longitude, latitude, ..)| (longitude, latitude))
        .collect();
    let batch = sampler.heights(&points);
    for (&(longitude, latitude), batch) in points.iter().zip(&batch) {
        assert_eq!(sampler.height(longitude, latitude), *batch);
    }
    assert!(sampler.cache.0.len() <= CACHE_CAPACITY);
}

/// A finest tile on the Waitemata shore that the preprocessor's fill left largely
/// unfilled: a fifth of its texels are real land, the rest masked, many still at -9999.
#[test]
fn masked_texels_are_missing() {
    let Some(sampler) = sampler(&["auckland"]) else {
        return;
    };

    let terrain = &sampler.terrains[0];
    let coordinate = TileCoordinate::new(3, 15, IVec2::new(29975, 18338));
    assert!(terrain.tiles.contains(&coordinate));
    let tile = terrain.decode(coordinate).expect("the tile decodes");
    let attachment = &terrain.attachment;

    let masked = tile
        .texels
        .iter()
        .filter(|texel| texel.to_bits() & 1 == 0)
        .count();
    eprintln!(
        "{coordinate}: {masked} of {} texels masked",
        tile.texels.len()
    );
    assert!(masked > 0 && masked < tile.texels.len());

    // A two by two block of masked texels inside the data area samples to nothing at its
    // centre; a block of real ones samples to a height.
    let size = tile.size as i32;
    let border = attachment.border_size as i32;
    let block_is = |x: i32, y: i32, valid: bool| {
        [(0, 0), (1, 0), (0, 1), (1, 1)]
            .iter()
            .all(|&(dx, dy)| tile.height(IVec2::new(x + dx, y + dy), true).is_some() == valid)
    };
    let find = |valid: bool| {
        (border..size - border - 1)
            .flat_map(|y| (border..size - border - 1).map(move |x| (x, y)))
            .find(|&(x, y)| block_is(x, y, valid))
            .expect("a block")
    };

    let (x, y) = find(false);
    let centre = position_at_texel(x as f64 + 0.5, y as f64 + 0.5, attachment);
    assert_eq!(sample_bilinear(&tile, centre, attachment), None);

    let (x, y) = find(true);
    let centre = position_at_texel(x as f64 + 0.5, y as f64 + 0.5, attachment);
    assert!(sample_bilinear(&tile, centre, attachment).is_some());
}
