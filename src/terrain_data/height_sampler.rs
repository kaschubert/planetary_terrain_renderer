//! Terrain heights read from the height tiles on disk.
//!
//! The renderer streams heights into a gpu atlas, and nothing on the cpu can ask what the
//! ground is under a point: the tile tree reads one height back per view, for the camera,
//! and it only knows the tiles that happen to be resident. This reads the same tiles the
//! atlas streams, straight off the disk, so the answer is a function of the dataset and
//! nothing else. It is there at startup before anything has loaded, it does not change as
//! the camera moves, and when the terrain on disk is swapped for another the answer
//! follows, because it was sampled from whatever is there.
//!
//! The one caveat is the mesh. At distance the rendered terrain is coarser than its data,
//! so a position placed with these heights can sit a few metres into or above the mesh far
//! from the camera. Close in, where the finest tiles are drawn, the two agree.

use crate::{
    math::{Coordinate, TerrainShape, TileCoordinate, unit_position},
    terrain::TerrainConfig,
    terrain_data::{AttachmentConfig, AttachmentFormat, AttachmentLabel},
};
use bevy::{
    math::{DVec2, DVec3, IVec2},
    platform::collections::HashSet,
    prelude::*,
};
use indexmap::IndexMap;
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
use tiff::decoder::{Decoder, DecodingResult};

/// How many decoded tiles to keep. A tile is 512 by 512 floats, a megabyte, so this is
/// 64 MiB at most. A line sampled every few tens of metres stays inside one finest tile
/// for hundreds of points and touches its neighbours only at the border, so a handful would
/// do for one line; 64 leaves room for a caller that hops between several.
const CACHE_CAPACITY: usize = 64;

/// Below this a texel holds NoData, not a height. The preprocessor writes -9999 for NoData
/// and its fill does not reach every texel, so the value survives in tiles that are mostly
/// water or past the edge of the source. The lowest point in the global terrain is -8849 m,
/// so -9000 keeps the two apart with room to spare.
const NO_DATA_LIMIT: f32 = -9000.0;

/// Pulls a uv of exactly 1 back inside the last tile of a face, as TileTree::compute_tree_xy
/// does, so the far edge of a face does not address a tile past the end of the row.
const EDGE_MARGIN: f64 = 0.000001;

/// Samples terrain heights from the height tiles on disk.
///
/// Holds several terrains, tried in the order given, which the caller orders finest first.
/// A point is answered by the first terrain that has real data under it, which is the same
/// stitching the renderer shows: the city terrain where it has data, the national one
/// around it, the globe beyond that.
pub struct TerrainHeightSampler {
    terrains: Vec<Terrain>,
    cache: TileCache,
}

impl TerrainHeightSampler {
    /// Loads the configs at these paths, finest terrain first. A config that cannot be read
    /// is skipped with a warning rather than failing the load, so a caller can list every
    /// terrain it knows and still work when the user has not downloaded one of them. A
    /// config that reads but does not parse, or whose height attachment the sampler cannot
    /// read, is an error, since that is a broken dataset and not a missing one.
    pub fn load(config_paths: impl IntoIterator<Item = impl AsRef<Path>>) -> Result<Self, String> {
        let mut terrains = Vec::new();

        for path in config_paths {
            if let Some(terrain) = Terrain::load(path.as_ref())? {
                terrains.push(terrain);
            }
        }

        Ok(Self {
            terrains,
            cache: TileCache::default(),
        })
    }

    /// True when no config loaded, in which case every query is None.
    pub fn is_empty(&self) -> bool {
        self.terrains.is_empty()
    }

    /// The height in metres above the ellipsoid under a longitude and latitude in degrees,
    /// or None where no terrain has data.
    pub fn height(&mut self, longitude: f64, latitude: f64) -> Option<f64> {
        self.height_at_unit(unit_position(longitude, latitude))
    }

    /// The height in metres under a direction on the unit sphere, for a caller that already
    /// has one. The convention is the preprocessor's, see [`unit_position`].
    pub fn height_at_unit(&mut self, unit: DVec3) -> Option<f64> {
        (0..self.terrains.len()).find_map(|index| self.sample_terrain(index, unit))
    }

    /// [`Self::height`] for each (longitude, latitude), in order. A convenience only: the
    /// tiles that neighbouring points share come from the cache either way, so a loop over
    /// [`Self::height`] costs the same.
    pub fn heights(&mut self, points: &[(f64, f64)]) -> Vec<Option<f64>> {
        points
            .iter()
            .map(|&(longitude, latitude)| self.height(longitude, latitude))
            .collect()
    }

    fn sample_terrain(&mut self, index: usize, unit: DVec3) -> Option<f64> {
        let terrain = &self.terrains[index];
        let coordinate = Coordinate::from_unit_position(unit, terrain.shape.is_spherical());
        let (tile, position) = terrain.finest_tile(coordinate)?;
        let texels = self.cache.fetch((index, tile), || terrain.decode(tile))?;

        sample_bilinear(texels, position, &terrain.attachment)
    }
}

/// One terrain the sampler reads from.
struct Terrain {
    /// The folder the config sits in; the height tiles are in height/ inside it. The
    /// config's own path is relative to the asset root the renderer was started from,
    /// which this code has no way to know.
    directory: PathBuf,
    shape: TerrainShape,
    /// The height attachment's layout: where the border ends and whether bit 0 is a mask.
    attachment: AttachmentConfig,
    lod_count: u32,
    /// Every tile the terrain has, at every lod, so that the finest tile under a point is
    /// one lookup per lod rather than a stat on the disk.
    tiles: HashSet<TileCoordinate>,
}

impl Terrain {
    /// None, with a warning, when the config cannot be read.
    fn load(path: &Path) -> Result<Option<Self>, String> {
        let encoded = match fs::read_to_string(path) {
            Ok(encoded) => encoded,
            Err(error) => {
                warn!("height sampler: skipping {}: {error}", path.display());
                return Ok(None);
            }
        };

        let config: TerrainConfig =
            ron::from_str(&encoded).map_err(|error| format!("{}: {error}", path.display()))?;

        let attachment = config
            .attachments
            .get(&AttachmentLabel::Height)
            .ok_or_else(|| format!("{}: no height attachment", path.display()))?
            .clone();

        if attachment.format != AttachmentFormat::R32F {
            return Err(format!(
                "{}: the height attachment is {:?}, and only R32F can be sampled",
                path.display(),
                attachment.format
            ));
        }

        let directory = path.parent().unwrap_or(Path::new("")).to_path_buf();

        Ok(Some(Self {
            directory,
            shape: config.shape,
            attachment,
            lod_count: config.lod_count,
            tiles: config.tiles.into_iter().collect(),
        }))
    }

    /// The finest tile the terrain has under the coordinate, and where inside its data area
    /// the coordinate falls, in [0, 1) along each axis.
    fn finest_tile(&self, coordinate: Coordinate) -> Option<(TileCoordinate, DVec2)> {
        (0..self.lod_count).rev().find_map(|lod| {
            let (xy, position) = tile_position(coordinate.uv, lod);
            let tile = TileCoordinate::new(coordinate.face, lod, xy);

            self.tiles.contains(&tile).then_some((tile, position))
        })
    }

    /// Reads and decodes one tile. A tile the config lists but the disk does not have, or
    /// one that is not the float image the config promised, is logged and treated as
    /// missing, so that one bad file does not take the terrain down with it.
    fn decode(&self, tile: TileCoordinate) -> Option<HeightTile> {
        let height = String::from(&AttachmentLabel::Height);
        let path = tile.path(&self.directory.join(height));

        match HeightTile::read(&path, self.attachment.texture_size) {
            Ok(tile) => Some(tile),
            Err(error) => {
                warn!("height sampler: {}: {error}", path.display());
                None
            }
        }
    }
}

/// The tile at this lod under a face uv, and the position inside it in tile widths.
fn tile_position(uv: DVec2, lod: u32) -> (IVec2, DVec2) {
    let tile_count = (lod as f64).exp2();
    let tree_xy = (uv * tile_count).clamp(DVec2::ZERO, DVec2::splat(tile_count - EDGE_MARGIN));
    let xy = tree_xy.floor();

    (xy.as_ivec2(), tree_xy - xy)
}

/// A decoded tile as the gpu holds it: texture_size squared floats, row major, border
/// included, mask bit still in place.
struct HeightTile {
    size: usize,
    texels: Vec<f32>,
}

impl HeightTile {
    fn read(path: &Path, texture_size: u32) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        let mut decoder = Decoder::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
        let (width, height) = decoder.dimensions().map_err(|error| error.to_string())?;

        if width != texture_size || height != texture_size {
            return Err(format!(
                "expected {texture_size} by {texture_size} texels, found {width} by {height}"
            ));
        }

        match decoder.read_image().map_err(|error| error.to_string())? {
            DecodingResult::F32(texels) => Ok(Self {
                size: texture_size as usize,
                texels,
            }),
            _ => Err("not a 32 bit float image".to_string()),
        }
    }

    /// The height one texel holds, or None where it holds NoData. Clamps to the edge as the
    /// gpu's sampler does, though a position inside the data area never reaches past a
    /// border of more than one texel.
    fn height(&self, texel: IVec2, mask: bool) -> Option<f64> {
        let limit = IVec2::splat(self.size as i32 - 1);
        let texel = texel.clamp(IVec2::ZERO, limit);
        let bits = self.texels[texel.y as usize * self.size + texel.x as usize].to_bits();

        texel_height(bits, mask)
    }
}

/// In a masked attachment bit 0 of each float is validity, not height: the preprocessor
/// sets it where the source had data and clears it where it filled NoData by interpolation
/// so the gpu has something to blend across, see fill_no_data.rs. Clearing it costs one
/// unit in the last place, a hundredth of a millimetre at the height of a hill.
fn texel_height(bits: u32, mask: bool) -> Option<f64> {
    if mask && bits & 1 == 0 {
        return None;
    }

    let height = f32::from_bits(if mask { bits & !1 } else { bits });

    (height > NO_DATA_LIMIT).then_some(height as f64)
}

/// The height at a position inside a tile, sampled as the gpu's linear sampler does. The
/// shader maps the position into the texture past the border, uv * center_size /
/// texture_size + border_size / texture_size, and the sampler blends the four texels whose
/// centres surround it, centres sitting half a texel in. So the texel column under a
/// position is uv * center_size + border_size - 0.5, and the fraction left over weights
/// the next column.
///
/// A masked texel carries no height. The shader discards the whole footprint when any of
/// the four is masked, which is right for a surface, but for a height query the ground just
/// inside the edge of the data is real, so the masked texels are dropped and the rest
/// renormalised. The answer leans towards the real side, which is where a point within a
/// texel of the edge sits in any case. With all four masked there is no height here.
fn sample_bilinear(
    tile: &HeightTile,
    position: DVec2,
    attachment: &AttachmentConfig,
) -> Option<f64> {
    let texel = position * attachment.center_size() as f64 + attachment.border_size as f64 - 0.5;
    let corner = texel.floor();
    let fraction = texel - corner;
    let corner = corner.as_ivec2();

    let mut weighted = 0.0;
    let mut total = 0.0;

    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let weight_x = if dx == 0 {
            1.0 - fraction.x
        } else {
            fraction.x
        };
        let weight_y = if dy == 0 {
            1.0 - fraction.y
        } else {
            fraction.y
        };

        if let Some(height) = tile.height(corner + IVec2::new(dx, dy), attachment.mask) {
            weighted += weight_x * weight_y * height;
            total += weight_x * weight_y;
        }
    }

    (total > 0.0).then(|| weighted / total)
}

/// Decoded tiles, keyed by terrain and coordinate, in order of use. A failed decode is
/// kept as None so that a bad file is read and warned about once rather than at every
/// point over it; it is retried once it has been evicted.
#[derive(Default)]
struct TileCache(IndexMap<(usize, TileCoordinate), Option<HeightTile>>);

impl TileCache {
    /// The cached tile, or the one `decode` produces. A hit moves the entry to the back and
    /// a miss evicts the front when the cache is full. Shifting 64 entries along is nothing
    /// next to decoding a megabyte.
    fn fetch(
        &mut self,
        key: (usize, TileCoordinate),
        decode: impl FnOnce() -> Option<HeightTile>,
    ) -> Option<&HeightTile> {
        if let Some(index) = self.0.get_index_of(&key) {
            self.0.move_index(index, self.0.len() - 1);
        } else {
            if self.0.len() >= CACHE_CAPACITY {
                self.0.shift_remove_index(0);
            }

            self.0.insert(key, decode());
        }

        self.0.get(&key)?.as_ref()
    }
}

#[cfg(test)]
mod tests;
