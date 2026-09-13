use crate::{
    math::{TerrainShape, TileCoordinate},
    plugin::TerrainSettings,
    render::TerrainUniform,
    terrain::TerrainConfig,
    terrain_data::{
        Attachment, AttachmentData, AttachmentLabel, AttachmentTile, AttachmentTileWithData,
        DefaultLoader, TileTree, TileTreeEntry,
    },
    terrain_view::TerrainViewComponents,
};
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::{VisibilityClass, add_visibility_class},
    platform::collections::{HashMap, HashSet},
    prelude::*,
    render::{render_resource::*, storage::ShaderBuffer},
    tasks::Task,
};
use big_space::prelude::CellCoord;
use std::collections::VecDeque;

/// The current state of a tile of a [`TileAtlas`].
///
/// This indicates, whether the tile is loading or loaded and ready to be used.
#[derive(Clone, Copy, Debug)]
enum LoadingState {
    /// The tile is loading, but can not be used yet.
    Loading(u32),
    /// The tile is loaded and can be used.
    Loaded,
}

/// The internal representation of a present tile in a [`TileAtlas`].
struct TileState {
    /// Indicates whether or not the tile is loading or loaded.
    state: LoadingState,
    /// The index of the tile inside the atlas.
    atlas_index: u32,
    /// The count of [`TileTrees`] that have requested this tile.
    requests: u32,
    /// Distinguishes this allocation of the coordinate from earlier ones whose loads may
    /// still be in flight.
    generation: u32,
}

// Todo: rename to terrain?
// Todo: consider turning this into an asset

/// A sparse storage of all terrain attachments, which streams data in and out of memory
/// depending on the decisions of the corresponding [`TileTree`]s.
///
/// A tile is considered present and assigned an [`u32`] as soon as it is
/// requested by any tile_tree. Then the tile atlas will start loading all of its attachments
/// by storing the [`TileCoordinate`] (for one frame) in `load_events` for which
/// attachment-loading-systems can listen.
/// Tiles that are not being used by any tile_tree anymore are cached (LRU),
/// until new atlas indices are required.
///
/// The [`u32`] can be used for accessing the attached data in systems by the CPU
/// and in shaders by the GPU.
#[derive(Component)]
#[require(Transform, CellCoord, Visibility, VisibilityClass, DefaultLoader)]
#[component(on_add = add_visibility_class::<TileAtlas>)]
pub struct TileAtlas {
    pub(crate) attachments: HashMap<AttachmentLabel, Attachment>, // stores the attachment data
    tile_states: HashMap<TileCoordinate, TileState>,
    unused_indices: VecDeque<u32>,
    /// The slot count the atlas was created with, kept so usage can be reported.
    atlas_size: u32,
    /// Bumped on every slot allocation, see request_tile.
    generation: u32,
    existing_tiles: HashSet<TileCoordinate>,
    pub(crate) uploading_tiles: Vec<AttachmentTileWithData>,
    pub(crate) downloading_tiles: Vec<Task<AttachmentTileWithData>>,
    pub(crate) to_load: Vec<AttachmentTile>,

    pub(crate) lod_count: u32,
    pub(crate) min_height: f32,
    pub(crate) max_height: f32,
    pub(crate) height_scale: f32,
    pub(crate) shape: TerrainShape,

    pub(crate) terrain_buffer: Handle<ShaderBuffer>,
}

impl TileAtlas {
    /// Creates a new tile_tree from a terrain config.
    pub fn new(
        config: &TerrainConfig,
        buffers: &mut Assets<ShaderBuffer>,
        settings: &TerrainSettings,
    ) -> Self {
        let attachments = config
            .attachments
            .iter()
            .map(|(label, attachment)| (label.clone(), Attachment::new(attachment, &config.path)))
            .collect();

        let terrain_buffer = buffers.add(ShaderBuffer::with_size(
            TerrainUniform::min_size().get() as usize,
            RenderAssetUsages::all(),
        ));

        Self {
            attachments,
            tile_states: default(),
            unused_indices: (0..settings.atlas_size).collect(),
            atlas_size: settings.atlas_size,
            generation: 0,
            existing_tiles: HashSet::from_iter(config.tiles.clone()),
            to_load: default(),
            uploading_tiles: default(),
            downloading_tiles: default(),
            lod_count: config.lod_count,
            min_height: config.min_height,
            max_height: config.max_height,
            height_scale: 1.0,
            shape: config.shape,
            terrain_buffer,
        }
    }

    /// The atlas slots currently held by tiles, and the total the atlas was built with.
    ///
    /// Slots are what bound how much of a terrain can be resident at once, and they are
    /// what runs out before memory does, so this is the number to watch when deciding
    /// whether the atlas could be smaller.
    pub fn slot_usage(&self) -> (u32, u32) {
        (
            self.atlas_size - self.unused_indices.len() as u32,
            self.atlas_size,
        )
    }

    pub(crate) fn get_best_tile(&self, tile_coordinate: TileCoordinate) -> TileTreeEntry {
        let mut best_tile_coordinate = tile_coordinate;

        if !self.existing_tiles.contains(&tile_coordinate) {
            return TileTreeEntry::default();
        }

        loop {
            if best_tile_coordinate == TileCoordinate::INVALID {
                // highest lod is not loaded
                return TileTreeEntry::default();
            }

            if let Some(tile) = self.tile_states.get(&best_tile_coordinate) {
                if matches!(tile.state, LoadingState::Loaded) {
                    // found best loaded tile
                    return TileTreeEntry {
                        atlas_index: tile.atlas_index,
                        atlas_lod: best_tile_coordinate.lod,
                    };
                }
            }

            best_tile_coordinate = best_tile_coordinate
                .parent()
                .unwrap_or(TileCoordinate::INVALID);
        }
    }

    pub(crate) fn tile_loaded(&mut self, tile: AttachmentTile, data: AttachmentData) {
        // A load is stale if its coordinate was evicted while it was in flight, whether
        // or not the coordinate has since been requested again: a new request is a new
        // generation, and counting an old load against it would overflow its attachment
        // count and upload data into whatever slot the coordinate holds now.
        let Some(tile_state) = self
            .tile_states
            .get_mut(&tile.coordinate)
            .filter(|tile_state| tile_state.generation == tile.generation)
        else {
            return;
        };

        tile_state.state = match tile_state.state {
            LoadingState::Loading(1) => LoadingState::Loaded,
            LoadingState::Loading(n) => LoadingState::Loading(n - 1),
            LoadingState::Loaded => {
                panic!("Loaded more attachments, than registered with the tile atlas.")
            }
        };

        self.uploading_tiles.push(AttachmentTileWithData {
            atlas_index: tile_state.atlas_index,
            label: tile.label,
            data,
        });
    }

    /// Updates the tile atlas according to all corresponding tile_trees.
    pub(crate) fn update(
        mut tile_trees: ResMut<TerrainViewComponents<TileTree>>,
        mut tile_atlases: Query<&mut TileAtlas>,
    ) {
        for (&(terrain, _view), tile_tree) in tile_trees.iter_mut() {
            let mut tile_atlas = tile_atlases.get_mut(terrain).unwrap();

            for tile_coordinate in tile_tree.released_tiles.drain(..) {
                tile_atlas.release_tile(tile_coordinate);
            }

            for tile_coordinate in tile_tree.requested_tiles.drain(..) {
                tile_atlas.request_tile(tile_coordinate);
            }
        }
    }

    pub fn update_terrain_buffer(
        mut tile_atlases: Query<(&mut TileAtlas, &GlobalTransform)>,
        mut buffers: ResMut<Assets<ShaderBuffer>>,
    ) {
        for (tile_atlas, global_transform) in &mut tile_atlases {
            let mut terrain_buffer = buffers.get_mut(&tile_atlas.terrain_buffer).unwrap();
            terrain_buffer.set_data(TerrainUniform::new(&tile_atlas, global_transform));
        }
    }

    fn request_tile(&mut self, tile_coordinate: TileCoordinate) {
        if !self.existing_tiles.contains(&tile_coordinate) {
            return;
        }

        // check if the tile is already present else start loading it
        if let Some(tile) = self.tile_states.get_mut(&tile_coordinate) {
            if tile.requests == 0 {
                // the tile is now used again
                self.unused_indices
                    .retain(|&atlas_index| tile.atlas_index != atlas_index);
            }

            tile.requests += 1;
        } else {
            // With every slot held by a tile in use, this one is left unloaded rather
            // than evicting one that is: the tile tree shows the finest loaded parent
            // instead, so the picture degrades where the process used to panic. The
            // tree asks again the next time it releases and re-requests the tile.
            let Some(atlas_index) = self.unused_indices.pop_front() else {
                warn_once!("atlas full, skipping tiles: consider a larger atlas_size");
                return;
            };

            self.tile_states
                .retain(|_, tile| tile.atlas_index != atlas_index); // remove tile if it is still cached

            // Loads for an earlier occupant of this coordinate may still be in flight;
            // the generation is what lets tile_loaded tell them apart from these.
            self.generation += 1;
            let generation = self.generation;

            self.tile_states.insert(
                tile_coordinate,
                TileState {
                    requests: 1,
                    state: LoadingState::Loading(self.attachments.len() as u32),
                    atlas_index,
                    generation,
                },
            );

            for label in self.attachments.keys() {
                self.to_load.push(AttachmentTile {
                    coordinate: tile_coordinate,
                    label: label.clone(),
                    generation,
                });
            }
        }
    }

    fn release_tile(&mut self, tile_coordinate: TileCoordinate) {
        if !self.existing_tiles.contains(&tile_coordinate) {
            return;
        }

        // Absent if the atlas was full when it was requested, see request_tile.
        let Some(tile) = self.tile_states.get_mut(&tile_coordinate) else {
            return;
        };
        tile.requests -= 1;

        if tile.requests == 0 {
            self.unused_indices.push_back(tile.atlas_index);

            // Todo: we should cancel loading tiles, that have not yet started loading and a no longer requested
        }
    }
}
