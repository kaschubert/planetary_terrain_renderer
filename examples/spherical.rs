use bevy::dev_tools::fps_overlay::{FPS_OVERLAY_ZINDEX, FpsOverlayPlugin};
use bevy::math::DVec3;
use bevy::render::{Render, RenderApp, RenderSystems, renderer::RenderDevice};
use bevy::text::FontSize;
use bevy::window::WindowResolution;
use bevy::{prelude::*, reflect::TypePath, render::render_resource::*, shader::ShaderRef};
use bevy_terrain::math::Coordinate;
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const RADIUS: f64 = 6371000.0;

// Where the camera starts: over Wellington, the finest terrain in the scene.
const CAMERA_LONGITUDE: f64 = 174.7762;
const CAMERA_LATITUDE: f64 = -41.2866;
const CAMERA_ALTITUDE: f32 = 1000.0;

#[derive(ShaderType, Clone)]
struct GradientInfo {
    mode: u32,
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct CustomMaterial {
    #[texture(0)]
    #[sampler(1)]
    gradient: Handle<Image>,
    #[uniform(2)]
    gradient_info: GradientInfo,
}

impl Material for CustomMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/spherical.wgsl".into()
    }
}

/// A terrain that is only kept resident while the camera is near it.
///
/// Each terrain allocates its atlas textures up front, whether or not it is on screen, so
/// holding all of them costs the same as looking at all of them. Spawning by distance
/// trades a load pause on approach for the memory of the ones you are nowhere near.
struct StreamedTerrain {
    path: &'static str,
    order: u32,
    gradient_mode: u32,
    longitude: f64,
    latitude: f64,
    /// Spawned when the camera comes within this distance of the terrain's centre, and
    /// despawned once it passes DESPAWN_MARGIN beyond it. None means always resident,
    /// which is what the base globe wants.
    ///
    /// Spawning is not instant - the terrain still has to stream its tiles in - so this
    /// wants to be comfortably further out than the distance at which the detail becomes
    /// visible, or it pops in. It also has to be small enough that two neighbours are
    /// never resident at once: wellington and auckland are 493 km apart, so with the
    /// margin below the pair can never both be live.
    radius: Option<f64>,
}

/// How much further than its radius a terrain is kept before being despawned. Without it
/// a terrain sitting exactly on the boundary spawns and despawns every frame.
const DESPAWN_MARGIN: f64 = 40_000.0;

const STREAMED_TERRAINS: &[StreamedTerrain] = &[
    StreamedTerrain {
        path: "terrains/earth/config.tc.ron",
        order: 0,
        gradient_mode: 2,
        longitude: 0.0,
        latitude: 0.0,
        radius: None,
    },
    StreamedTerrain {
        path: "terrains/nz/config.tc.ron",
        order: 1,
        gradient_mode: 2,
        longitude: 173.0,
        latitude: -41.0,
        radius: Some(2_000_000.0),
    },
    StreamedTerrain {
        path: "terrains/wellington/config.tc.ron",
        order: 2,
        gradient_mode: 2,
        longitude: 174.7762,
        latitude: -41.2866,
        radius: Some(200_000.0),
    },
    StreamedTerrain {
        path: "terrains/auckland/config.tc.ron",
        order: 2,
        gradient_mode: 2,
        longitude: 174.7633,
        latitude: -36.8485,
        radius: Some(200_000.0),
    },
];

#[derive(Resource)]
struct TerrainStreaming {
    view: Entity,
    gradient: Handle<Image>,
    /// The spawned entity per entry of STREAMED_TERRAINS.
    active: Vec<Option<Entity>>,
}

/// Converts longitude and latitude to a position on the spheroid. Matches the unit sphere
/// convention the preprocessor warps with, see CubeTransformer in transformers.rs.
fn unit_position(longitude: f64, latitude: f64) -> DVec3 {
    let (longitude, latitude) = (longitude.to_radians(), latitude.to_radians());

    DVec3::new(
        -latitude.cos() * longitude.cos(),
        latitude.sin(),
        latitude.cos() * longitude.sin(),
    )
}

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        resolution: WindowResolution::new(1920, 1080),
                        ..default()
                    }),
                    ..default()
                })
                .build()
                .disable::<TransformPlugin>(),
            TerrainPlugin,
            TerrainMaterialPlugin::<CustomMaterial>::default(),
            TerrainDebugPlugin,          // enable debug settings and controls
            FpsOverlayPlugin::default(), // frame rate and frame time graph, top left
            VramUsagePlugin,             // gpu allocator usage, below the fps overlay
            TerrainPickingPlugin,
        ))
        // A terrain costs roughly 2.6 MiB per atlas slot, for height and albedo together,
        // so the default atlas size is about 2.6 GiB each. Four at once does not fit a
        // 12 GiB card, which is what stream_terrains is for: the two cities are 500 km
        // apart and never resident together, so the most that is ever live is the globe,
        // the country and one city.
        //
        // The atlas cannot be shrunk much instead of this: the tile tree asks for
        // lod_count x tree_size squared tiles per terrain, and the city terrains are 16
        // lods deep, so halving it runs the atlas out of indices.
        .insert_resource(TerrainSettings::new(vec!["albedo"]))
        // .insert_resource(ClearColor(Color::WHITE))
        .add_systems(Startup, initialize)
        .add_systems(Update, stream_terrains)
        .run();
}

/// Reports how much gpu memory wgpu's allocator is holding.
///
/// This is what the allocator has handed out and what it has reserved from the driver, not
/// the size of the card: wgpu exposes no portable way to ask for total or free video
/// memory. It is still the number that moves when a terrain streams in or out, since a
/// terrain's atlas textures dwarf everything else in this example.
#[derive(Resource, Clone, Default)]
struct VramUsage {
    allocated: Arc<AtomicU64>,
    reserved: Arc<AtomicU64>,
}

#[derive(Component)]
struct VramText;

struct VramUsagePlugin;

impl Plugin for VramUsagePlugin {
    fn build(&self, app: &mut App) {
        let usage = VramUsage::default();

        app.insert_resource(usage.clone())
            .add_systems(Startup, spawn_vram_text)
            .add_systems(Update, (update_vram_text, offset_fps_overlay));

        // The allocator lives in the render world, so the counters are shared across the
        // two rather than extracted: extraction only runs main to render.
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(usage)
                .add_systems(Render, sample_vram.in_set(RenderSystems::Cleanup));
        }
    }
}

fn sample_vram(device: Res<RenderDevice>, usage: Res<VramUsage>) {
    let Some(report) = device.wgpu_device().generate_allocator_report() else {
        return;
    };

    usage
        .allocated
        .store(report.total_allocated_bytes, Ordering::Relaxed);
    usage
        .reserved
        .store(report.total_reserved_bytes, Ordering::Relaxed);
}

fn spawn_vram_text(mut commands: Commands) {
    commands.spawn((
        VramText,
        Text::default(),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(6.0),
            left: Val::Px(6.0),
            ..default()
        },
    ));
}

/// Moves the fps overlay down so the vram line can have the top left corner.
///
/// The overlay spawns at the origin and exposes no position setting, but its root is the
/// node carrying FPS_OVERLAY_ZINDEX, which is public.
fn offset_fps_overlay(mut overlay: Query<(&mut Node, &GlobalZIndex), Added<GlobalZIndex>>) {
    for (mut node, z_index) in &mut overlay {
        if z_index.0 == FPS_OVERLAY_ZINDEX {
            node.top = Val::Px(30.0);
            node.left = Val::Px(6.0);
        }
    }
}

fn update_vram_text(usage: Res<VramUsage>, mut text: Single<&mut Text, With<VramText>>) {
    const GIB: f64 = (1u64 << 30) as f64;

    let allocated = usage.allocated.load(Ordering::Relaxed) as f64 / GIB;
    let reserved = usage.reserved.load(Ordering::Relaxed) as f64 / GIB;

    text.0 = format!("VRAM used / alloc {allocated:.2} / {reserved:.2} GiB");
}

#[allow(clippy::too_many_arguments)]
fn initialize(
    mut commands: Commands,
    mut images: ResMut<LoadingImages>,
    asset_server: Res<AssetServer>,
) {
    let gradient1 = asset_server.load("textures/gradient1.png");
    images.load_image(
        &gradient1,
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
    );

    let gradient2 = asset_server.load("textures/gradient2.png");
    images.load_image(
        &gradient2,
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
    );

    let mut view = Entity::PLACEHOLDER;

    // Longitude and latitude to a position on the spheroid. The unit sphere convention
    // matches the one the preprocessor warps with, see CubeTransformer in transformers.rs.
    let (longitude, latitude) = (CAMERA_LONGITUDE.to_radians(), CAMERA_LATITUDE.to_radians());
    let up = DVec3::new(
        -latitude.cos() * longitude.cos(),
        latitude.sin(),
        latitude.cos() * longitude.sin(),
    );
    let north = DVec3::new(
        latitude.sin() * longitude.cos(),
        latitude.cos(),
        -latitude.sin() * longitude.sin(),
    );

    let camera_position = Coordinate::from_unit_position(up, true)
        .local_position(TerrainShape::WGS84, CAMERA_ALTITUDE);
    // Tilted halfway between straight down and the horizon, facing north over the harbour.
    let camera_direction = (north - up).normalize();

    commands.spawn_big_space(Grid::default(), |root| {
        view = root
            .spawn_spatial((
                Transform::from_translation(camera_position.as_vec3())
                    .looking_to(camera_direction.as_vec3(), up.as_vec3()),
                DebugCameraController::new(RADIUS),
                OrbitalCameraController::default(),
            ))
            .id();
    });

    // Terrains are no longer spawned here: stream_terrains spawns and despawns them as
    // the camera moves, so only the ones nearby hold atlas memory.
    commands.insert_resource(TerrainStreaming {
        view,
        gradient: gradient1.clone(),
        active: vec![None; STREAMED_TERRAINS.len()],
    });
}

/// Keeps the terrains near the camera resident and drops the rest.
fn stream_terrains(
    mut commands: Commands,
    mut streaming: ResMut<TerrainStreaming>,
    grids: Grids,
    camera: Query<(Entity, &Transform, &CellCoord), With<OrbitalCameraController>>,
    asset_server: Res<AssetServer>,
) {
    let Ok((camera, camera_transform, camera_cell)) = camera.single() else {
        return;
    };
    let Some(grid) = grids.parent_grid(camera) else {
        return;
    };

    let camera_position = grid.grid_position_double(camera_cell, camera_transform);

    for (index, terrain) in STREAMED_TERRAINS.iter().enumerate() {
        let wanted = match terrain.radius {
            None => true,
            Some(radius) => {
                let centre = Coordinate::from_unit_position(
                    unit_position(terrain.longitude, terrain.latitude),
                    true,
                )
                .local_position(TerrainShape::WGS84, 0.0);

                let threshold = if streaming.active[index].is_some() {
                    radius + DESPAWN_MARGIN
                } else {
                    radius
                };

                camera_position.distance(centre) < threshold
            }
        };

        match (wanted, streaming.active[index]) {
            (true, None) => {
                let terrain = commands.spawn_terrain(
                    asset_server.load(terrain.path),
                    TerrainViewConfig {
                        order: terrain.order,
                        ..default()
                    },
                    CustomMaterial {
                        gradient: streaming.gradient.clone(),
                        gradient_info: GradientInfo {
                            mode: terrain.gradient_mode,
                        },
                    },
                    streaming.view,
                );

                streaming.active[index] = Some(terrain);
            }
            (false, Some(entity)) => {
                // TileTree::despawn and GpuTileAtlas::despawn drop the data that hangs off
                // this entity, on the main and render worlds respectively.
                commands.entity(entity).despawn();
                streaming.active[index] = None;
            }
            _ => {}
        }
    }

    // commands.spawn_terrain(
    //     asset_server.load("terrains/swiss/config.tc.ron"),
    //     TerrainViewConfig {
    //         order: 1,
    //         ..default()
    //     },
    //     CustomMaterial {
    //         gradient: gradient1.clone(),
    //         gradient_info: GradientInfo { mode: 1 },
    //     },
    //     view,
    // );

    //
    // commands.spawn_terrain(
    //     asset_server.load("terrains/hartenstein/config.tc.ron"),
    //     TerrainViewConfig {
    //         order: 1,
    //         ..default()
    //     },
    //     CustomMaterial {
    //         gradient: gradient2.clone(),
    //         gradient_info: GradientInfo { mode: 2 },
    //     },
    //     view,
    // );
}
