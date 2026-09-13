use crate::shaders::DEPTH_COPY_SHADER;
use bevy::{
    core_pipeline::{FullscreenShader, core_3d::CORE_3D_DEPTH_FORMAT},
    ecs::entity::EntityHash,
    prelude::*,
    render::{
        Extract,
        camera::ExtractedCamera,
        render_phase::{
            CachedRenderPipelinePhaseItem, DrawFunctionId, PhaseItem, PhaseItemExtraIndex,
            SortedPhaseItem, ViewSortedRenderPhases,
        },
        render_resource::{binding_types::texture_depth_2d_multisampled, *},
        renderer::{RenderContext, RenderDevice, ViewQuery},
        sync_world::MainEntity,
        texture::{CachedTexture, TextureCache},
        view::{ExtractedView, RetainedViewEntity, ViewDepthTexture, ViewTarget},
    },
};
use indexmap::IndexMap;
use std::ops::Range;

pub(crate) const TERRAIN_DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32FloatStencil8;

pub struct TerrainItem {
    pub representative_entity: (Entity, MainEntity),
    pub draw_function: DrawFunctionId,
    pub pipeline: CachedRenderPipelineId,
    pub batch_range: Range<u32>,
    pub extra_index: PhaseItemExtraIndex,
    pub order: u32,
}

impl PhaseItem for TerrainItem {
    const AUTOMATIC_BATCHING: bool = false;

    #[inline]
    fn entity(&self) -> Entity {
        self.representative_entity.0
    }

    #[inline]
    fn main_entity(&self) -> MainEntity {
        self.representative_entity.1
    }

    #[inline]
    fn draw_function(&self) -> DrawFunctionId {
        self.draw_function
    }

    #[inline]
    fn batch_range(&self) -> &Range<u32> {
        &self.batch_range
    }

    fn batch_range_mut(&mut self) -> &mut Range<u32> {
        &mut self.batch_range
    }

    fn extra_index(&self) -> PhaseItemExtraIndex {
        self.extra_index.clone()
    }

    fn batch_range_and_extra_index_mut(&mut self) -> (&mut Range<u32>, &mut PhaseItemExtraIndex) {
        (&mut self.batch_range, &mut self.extra_index)
    }
}

impl SortedPhaseItem for TerrainItem {
    type SortKey = u32;

    fn sort_key(&self) -> Self::SortKey {
        u32::MAX - self.order
    }

    fn indexed(&self) -> bool {
        false
    }

    fn recalculate_sort_keys(
        _items: &mut IndexMap<(Entity, MainEntity), Self, EntityHash>,
        _view: &ExtractedView,
    ) {
        // The sort key only depends on the terrain view order, which does not change with the view.
    }
}

impl CachedRenderPipelinePhaseItem for TerrainItem {
    fn cached_pipeline(&self) -> CachedRenderPipelineId {
        self.pipeline
    }
}

pub fn extract_terrain_phases(
    mut terrain_phases: ResMut<ViewSortedRenderPhases<TerrainItem>>,
    cameras: Extract<Query<(Entity, &Camera), With<Camera3d>>>,
) {
    terrain_phases.clear();

    for (entity, camera) in &cameras {
        if !camera.is_active {
            continue;
        }

        terrain_phases.insert(
            RetainedViewEntity {
                main_entity: entity.into(),
                auxiliary_entity: Entity::PLACEHOLDER.into(),
                subview_index: 0,
            },
            default(),
        );
    }
}

#[derive(Component)]
pub struct TerrainViewDepthTexture {
    texture: Texture,
    pub view: TextureView,
    pub depth_view: TextureView,
    pub stencil_view: TextureView,
}

impl TerrainViewDepthTexture {
    pub fn new(texture: CachedTexture) -> Self {
        let depth_view = texture.texture.create_view(&TextureViewDescriptor {
            aspect: TextureAspect::DepthOnly,
            ..default()
        });
        let stencil_view = texture.texture.create_view(&TextureViewDescriptor {
            aspect: TextureAspect::StencilOnly,
            ..default()
        });

        Self {
            texture: texture.texture,
            view: texture.default_view,
            depth_view,
            stencil_view,
        }
    }

    pub fn get_attachment(&self) -> RenderPassDepthStencilAttachment<'_> {
        RenderPassDepthStencilAttachment {
            view: &self.view,
            depth_ops: Some(Operations {
                load: LoadOp::Clear(0.0), // Clear depth
                store: StoreOp::Store,
            }),
            stencil_ops: Some(Operations {
                load: LoadOp::Clear(0), // Initialize stencil to 0 (lowest priority)
                store: StoreOp::Store,
            }),
        }
    }
}

pub fn prepare_terrain_depth_textures(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views_3d: Query<(Entity, &ExtractedCamera, &Msaa)>,
) {
    for (view, camera, msaa) in &views_3d {
        let Some(physical_target_size) = camera.physical_target_size else {
            continue;
        };

        let descriptor = TextureDescriptor {
            label: Some("view_depth_texture"),
            size: Extent3d {
                depth_or_array_layers: 1,
                width: physical_target_size.x,
                height: physical_target_size.y,
            },
            mip_level_count: 1,
            sample_count: msaa.samples(),
            dimension: TextureDimension::D2,
            format: TERRAIN_DEPTH_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };

        let cached_texture = texture_cache.get(&device, descriptor);

        commands
            .entity(view)
            .insert(TerrainViewDepthTexture::new(cached_texture));
    }
}

#[derive(Resource)]
pub struct DepthCopyPipeline {
    layout: BindGroupLayoutDescriptor,
    id: CachedRenderPipelineId,
}

pub fn initialize_depth_copy_pipeline(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    fullscreen_shader: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "depth_copy_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (texture_depth_2d_multisampled(),),
        ),
    );

    let id = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: None,
        layout: vec![layout.clone()],
        immediate_size: 0,
        vertex: fullscreen_shader.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: asset_server.load(DEPTH_COPY_SHADER),
            shader_defs: vec![],
            entry_point: Some("fragment".into()),
            targets: vec![],
        }),
        primitive: Default::default(),
        depth_stencil: Some(DepthStencilState {
            format: CORE_3D_DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: MultisampleState {
            count: 4, // Todo: specialize per camera ...
            ..Default::default()
        },
        zero_initialize_workgroup_memory: false,
    });

    commands.insert_resource(DepthCopyPipeline { layout, id });
}

/// Renders the terrain of the current view into its own depth-stencil texture, then copies the
/// terrain depth into the view depth texture so the main passes depth test against it.
pub fn terrain_pass(
    world: &World,
    view: ViewQuery<(
        &ExtractedView,
        &ExtractedCamera,
        &ViewTarget,
        &ViewDepthTexture,
        &TerrainViewDepthTexture,
    )>,
    terrain_phases: Res<ViewSortedRenderPhases<TerrainItem>>,
    pipeline_cache: Res<PipelineCache>,
    depth_copy_pipeline: Res<DepthCopyPipeline>,
    mut ctx: RenderContext,
) {
    let render_view = view.entity();
    let (extracted_view, camera, target, depth, terrain_depth) = view.into_inner();

    let Some(pipeline) = pipeline_cache.get_render_pipeline(depth_copy_pipeline.id) else {
        return;
    };

    let Some(terrain_phase) = terrain_phases.get(&extracted_view.retained_view_entity) else {
        return;
    };

    if terrain_phase.items.is_empty() {
        return;
    }

    // Todo: prepare this in a separate system
    let terrain_depth_view = terrain_depth.texture.create_view(&TextureViewDescriptor {
        aspect: TextureAspect::DepthOnly,
        ..default()
    });
    let depth_copy_bind_group = ctx.render_device().create_bind_group(
        None,
        &pipeline_cache.get_bind_group_layout(&depth_copy_pipeline.layout),
        &BindGroupEntries::single(&terrain_depth_view),
    );

    let color_attachments = [Some(target.get_color_attachment())];

    {
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("terrain_pass"),
            color_attachments: &color_attachments,
            depth_stencil_attachment: Some(terrain_depth.get_attachment()),
            ..default()
        });

        if let Some(viewport) = camera.viewport.as_ref() {
            pass.set_camera_viewport(viewport);
        }

        if let Err(err) = terrain_phase.render(&mut pass, world, render_view) {
            error!("Error encountered while rendering the terrain phase {err:?}");
        }
    }

    {
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("terrain_depth_copy_pass"),
            depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
            ..default()
        });
        pass.set_bind_group(0, &depth_copy_bind_group, &[]);
        pass.set_render_pipeline(pipeline);
        pass.draw(0..3, 0..1);
    }
}
