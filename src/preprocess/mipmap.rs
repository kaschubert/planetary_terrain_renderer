use crate::{
    shaders::MIP_SHADER,
    terrain::TerrainComponents,
    terrain_data::{AttachmentFormat, GpuTileAtlas},
};
use bevy::{
    asset::{AssetServer, Handle},
    platform::collections::HashMap,
    prelude::*,
    render::{
        render_resource::{binding_types::*, *},
        renderer::RenderContext,
    },
    shader::ShaderDefVal,
};
use strum::IntoEnumIterator;

pub(crate) fn create_mip_layout(format: AttachmentFormat) -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new(
        "mip_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<u32>(false), // atlas_index
                texture_2d_array(TextureSampleType::Float { filterable: true }), // parent
                texture_storage_2d_array(
                    format.processing_format(),
                    StorageTextureAccess::WriteOnly,
                ), // child
            ),
        ),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MipPipelineKey {
    pub(crate) format: AttachmentFormat,
}

impl MipPipelineKey {
    pub fn shader_defs(&self) -> Vec<ShaderDefVal> {
        let mut shader_defs = Vec::new();

        let format = match self.format {
            AttachmentFormat::Rgb8U => "RGB8U",
            AttachmentFormat::Rgba8U => "RGBA8U",
            AttachmentFormat::R16U => "R16U",
            AttachmentFormat::R16I => "R16I",
            AttachmentFormat::Rg16U => "RG16U",
            AttachmentFormat::R32F => "R32F",
        };

        shader_defs.push(format.into());

        shader_defs
    }
}

#[derive(Resource)]
pub struct MipPipelines {
    pub(crate) mip_layouts: HashMap<AttachmentFormat, BindGroupLayoutDescriptor>,
    mip_shader: Handle<Shader>,
}

pub fn initialize_mip_pipelines(mut commands: Commands, asset_server: Res<AssetServer>) {
    let mip_layouts = AttachmentFormat::iter()
        .map(|format| (format, create_mip_layout(format)))
        .collect();
    let mip_shader = asset_server.load(MIP_SHADER);

    commands.insert_resource(MipPipelines {
        mip_layouts,
        mip_shader,
    });
}

impl SpecializedComputePipeline for MipPipelines {
    type Key = MipPipelineKey;

    fn specialize(&self, key: Self::Key) -> ComputePipelineDescriptor {
        ComputePipelineDescriptor {
            label: Some("mip_pipeline".into()),
            layout: vec![self.mip_layouts[&key.format].clone()],
            immediate_size: 0,
            shader: self.mip_shader.clone(),
            shader_defs: key.shader_defs(),
            entry_point: Some("main".into()),
            zero_initialize_workgroup_memory: false,
        }
    }
}

/// Generates the mip levels of all atlas tiles uploaded this frame, before any camera renders.
pub(crate) fn mip_prepass(
    pipeline_cache: Res<PipelineCache>,
    gpu_tile_atlases: Res<TerrainComponents<GpuTileAtlas>>,
    mut ctx: RenderContext,
) {
    let mut pass = ctx
        .command_encoder()
        .begin_compute_pass(&ComputePassDescriptor::default());

    for gpu_tile_atlas in gpu_tile_atlases.values() {
        gpu_tile_atlas.generate_mip(&mut pass, &pipeline_cache);
    }
}
