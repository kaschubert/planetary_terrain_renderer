use crate::{
    formats::TiffLoader,
    preprocess::{MipPipelines, initialize_mip_pipelines, mip_prepass},
    provenance::TerrainProvenance,
    render::{
        GpuTerrain, GpuTerrainView, TerrainItem, TerrainTilingPrepassPipelines, TilingPrepassItem,
        extract_terrain_phases, initialize_depth_copy_pipeline,
        initialize_terrain_tiling_prepass_pipelines, prepare_terrain_depth_textures,
        queue_tiling_prepass, terrain_pass, tiling_prepass,
    },
    shaders::{InternalShaders, load_terrain_shaders},
    terrain::{TerrainComponents, TerrainConfig},
    terrain_data::{
        AttachmentLabel, GpuTileAtlas, TileAtlas, TileTree, finish_loading, start_loading,
    },
    terrain_view::{CullingCamera, TerrainViewComponents},
};
use bevy::{
    core_pipeline::{Core3d, Core3dSystems, core_3d::main_opaque_pass_3d, schedule::camera_driver},
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        render_phase::{DrawFunctions, ViewSortedRenderPhases, sort_phase_system},
        render_resource::*,
        renderer::{RenderGraph, RenderGraphSystems},
    },
};
use bevy_common_assets::ron::RonAssetPlugin;
use big_space::prelude::*;

#[derive(Resource)]
pub struct TerrainSettings {
    pub attachments: Vec<AttachmentLabel>,
    pub atlas_size: u32,
}

impl Default for TerrainSettings {
    fn default() -> Self {
        Self {
            attachments: vec![AttachmentLabel::Height],
            atlas_size: 1028,
        }
    }
}

impl TerrainSettings {
    pub fn new(custom_attachments: Vec<&str>) -> Self {
        let mut attachments = vec![AttachmentLabel::Height];
        attachments.extend(
            custom_attachments
                .into_iter()
                .map(|name| AttachmentLabel::Custom(name.to_string())),
        );

        Self {
            attachments,
            atlas_size: 1028,
        }
    }
}

/// The plugin for the terrain renderer.
pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(BigSpaceDefaultPlugins);

        app.add_plugins(RonAssetPlugin::<TerrainConfig>::new(&["tc.ron"]))
            .add_plugins(RonAssetPlugin::<TerrainProvenance>::new(&["tp.ron"]))
            .init_asset::<TerrainConfig>()
            .init_asset::<TerrainProvenance>()
            .init_resource::<InternalShaders>()
            .init_resource::<TerrainViewComponents<TileTree>>()
            .init_resource::<CullingCamera>()
            .init_resource::<TerrainSettings>()
            .init_asset_loader::<TiffLoader>()
            .add_systems(
                PostUpdate,
                (
                    // Todo: enable visibility checking again
                    // check_visibility::<With<TileAtlas>>.in_set(VisibilitySystems::CheckVisibility),
                    (
                        TileTree::despawn,
                        TileTree::compute_requests,
                        finish_loading,
                        TileAtlas::update,
                        start_loading,
                        TileTree::adjust_to_tile_atlas,
                        TileTree::generate_surface_approximation,
                        TileTree::update_terrain_view_buffer,
                        TileAtlas::update_terrain_buffer,
                    )
                        .chain()
                        .after(TransformSystems::Propagate),
                ),
            );
        app.sub_app_mut(RenderApp)
            .init_resource::<SpecializedComputePipelines<MipPipelines>>()
            .init_resource::<SpecializedComputePipelines<TerrainTilingPrepassPipelines>>()
            .init_resource::<TerrainComponents<GpuTileAtlas>>()
            .init_resource::<TerrainComponents<GpuTerrain>>()
            .init_resource::<TerrainViewComponents<GpuTerrainView>>()
            .init_resource::<TerrainViewComponents<TilingPrepassItem>>()
            .init_resource::<DrawFunctions<TerrainItem>>()
            .init_resource::<ViewSortedRenderPhases<TerrainItem>>()
            .add_systems(
                ExtractSchedule,
                (
                    extract_terrain_phases,
                    GpuTileAtlas::initialize,
                    GpuTileAtlas::despawn.after(GpuTileAtlas::initialize),
                    GpuTileAtlas::extract.after(GpuTileAtlas::despawn),
                    GpuTerrain::initialize.after(GpuTileAtlas::initialize),
                    GpuTerrainView::initialize,
                ),
            )
            .add_systems(
                Render,
                (
                    (
                        GpuTileAtlas::prepare,
                        GpuTerrain::prepare,
                        GpuTerrainView::prepare_terrain_view,
                        GpuTerrainView::prepare_indirect,
                        GpuTerrainView::prepare_refine_tiles,
                    )
                        .in_set(RenderSystems::Prepare),
                    sort_phase_system::<TerrainItem>.in_set(RenderSystems::PhaseSort),
                    prepare_terrain_depth_textures.in_set(RenderSystems::PrepareResources),
                    (queue_tiling_prepass, GpuTileAtlas::queue).in_set(RenderSystems::Queue),
                    GpuTileAtlas::_cleanup.in_set(RenderSystems::Cleanup),
                ),
            )
            .add_systems(
                Core3d,
                terrain_pass
                    .in_set(Core3dSystems::MainPass)
                    .before(main_opaque_pass_3d),
            )
            .add_systems(
                RenderGraph,
                (mip_prepass, tiling_prepass)
                    .chain()
                    .in_set(RenderGraphSystems::Render)
                    .before(camera_driver),
            );
    }

    fn finish(&self, app: &mut App) {
        let attachments = app
            .world()
            .resource::<TerrainSettings>()
            .attachments
            .clone();

        load_terrain_shaders(app, &attachments);

        app.sub_app_mut(RenderApp).add_systems(
            RenderStartup,
            (
                initialize_terrain_tiling_prepass_pipelines,
                initialize_mip_pipelines,
                initialize_depth_copy_pipeline,
            ),
        );
    }
}
