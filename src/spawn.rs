use crate::{
    plugin::TerrainSettings,
    terrain::TerrainConfig,
    terrain_data::{TileAtlas, TileTree},
    terrain_view::{TerrainViewComponents, TerrainViewConfig},
};
use bevy::{ecs::system::SystemState, prelude::*, render::storage::ShaderBuffer};
use big_space::floating_origins::BigSpace;

#[derive(Clone)]
pub(crate) struct TerrainToSpawn<M: Material + Clone> {
    /// Reserved when the spawn was requested, so the caller has an entity to hold on to
    /// before the config asset has finished loading.
    terrain: Entity,
    config: Handle<TerrainConfig>,
    view_config: TerrainViewConfig,
    material: M,
    view: Entity,
}

#[derive(Resource)]
pub(crate) struct TerrainsToSpawn<M: Material>(pub(crate) Vec<TerrainToSpawn<M>>);

pub(crate) fn spawn_terrains<M: Material>(
    mut commands: Commands,
    mut terrains: ResMut<TerrainsToSpawn<M>>,
    asset_server: Res<AssetServer>,
) {
    terrains.0.retain(|terrain| {
        if asset_server.is_loaded(&terrain.config) {
            let terrain = terrain.clone();

            commands.queue(move |world: &mut World| {
                let TerrainToSpawn {
                    terrain,
                    config,
                    view_config,
                    material,
                    view,
                } = terrain;

                let mut state = SystemState::<(
                    Commands,
                    Res<Assets<TerrainConfig>>,
                    Query<Entity, With<BigSpace>>,
                    ResMut<Assets<M>>,
                    ResMut<TerrainViewComponents<TileTree>>,
                    ResMut<Assets<ShaderBuffer>>,
                    Res<TerrainSettings>,
                )>::new(world);

                let (
                    mut commands,
                    configs,
                    big_space,
                    mut materials,
                    mut tile_trees,
                    mut buffers,
                    settings,
                ) = state.get_mut(world).unwrap();

                // The caller may have despawned the reserved entity while the config was
                // still loading, which is how a spawn is cancelled. Inserting onto it
                // would be an error, so drop the request instead.
                if commands.get_entity(terrain).is_err() {
                    return;
                }

                let config = configs.get(config.id()).unwrap().clone();

                let root = big_space.single().unwrap();

                commands.entity(terrain).insert((
                    config.shape.transform(),
                    TileAtlas::new(&config, &mut buffers, &settings),
                    MeshMaterial3d(materials.add(material)),
                ));

                commands.entity(root).add_child(terrain);

                tile_trees.insert(
                    (terrain, view),
                    TileTree::new(
                        &config,
                        &view_config,
                        (terrain, view),
                        &mut commands,
                        &mut buffers,
                    ),
                );

                state.apply(world);
            });
            false
        } else {
            true
        }
    });
}

pub trait SpawnTerrainCommandsExt<M: Material> {
    // define a method that we will be able to call on `commands`
    /// Returns the entity the terrain will occupy. It is reserved immediately but stays
    /// empty until the config asset has loaded, so it carries no terrain components yet.
    /// Despawning it before then cancels the spawn.
    fn spawn_terrain(
        &mut self,
        config: Handle<TerrainConfig>,
        view_config: TerrainViewConfig,
        material: M,
        view: Entity,
    ) -> Entity;
}

impl<M: Material> SpawnTerrainCommandsExt<M> for Commands<'_, '_> {
    fn spawn_terrain(
        &mut self,
        config: Handle<TerrainConfig>,
        view_config: TerrainViewConfig,
        material: M,
        view: Entity,
    ) -> Entity {
        let terrain = self.spawn_empty().id();

        self.queue(move |world: &mut World| {
            world
                .resource_mut::<TerrainsToSpawn<M>>()
                .0
                .push(TerrainToSpawn {
                    terrain,
                    config,
                    view_config,
                    material,
                    view,
                });
        });

        terrain
    }
}
