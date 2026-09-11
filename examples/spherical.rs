use bevy::math::DVec3;
use bevy::window::WindowResolution;
use bevy::{prelude::*, reflect::TypePath, render::render_resource::*, shader::ShaderRef};
use bevy_terrain::math::Coordinate;
use bevy_terrain::prelude::*;

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
            TerrainDebugPlugin, // enable debug settings and controls
            TerrainPickingPlugin,
        ))
        .insert_resource(TerrainSettings::new(vec!["albedo"]))
        // .insert_resource(ClearColor(Color::WHITE))
        .add_systems(Startup, initialize)
        .run();
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

    commands.spawn_terrain(
        asset_server.load("terrains/earth/config.tc.ron"),
        TerrainViewConfig::default(),
        CustomMaterial {
            gradient: gradient1.clone(),
            gradient_info: GradientInfo { mode: 2 },
        },
        view,
    );

    commands.spawn_terrain(
        asset_server.load("terrains/los/config.tc.ron"),
        TerrainViewConfig {
            order: 1,
            ..default()
        },
        CustomMaterial {
            gradient: gradient2.clone(),
            gradient_info: GradientInfo { mode: 0 },
        },
        view,
    );
    // //
    // commands.spawn_terrain(
    //     asset_server.load("terrains/npd/config.tc.ron"),
    //     TerrainViewConfig {
    //         order: 2,
    //         ..default()
    //     },
    //     CustomMaterial {
    //         gradient: gradient2.clone(),
    //         gradient_info: GradientInfo { mode: 0 },
    //     },
    //     view,
    // );
    //
    // commands.spawn_terrain(
    //     asset_server.load("terrains/utsira/config.tc.ron"),
    //     TerrainViewConfig {
    //         order: 1,
    //         ..default()
    //     },
    //     CustomMaterial {
    //         gradient: gradient2.clone(),
    //         gradient_info: GradientInfo { mode: 0 },
    //     },
    //     view,
    // );
    //
    // commands.spawn_terrain(
    //     asset_server.load("terrains/sas/config.tc.ron"),
    //     TerrainViewConfig {
    //         order: 2,
    //         ..default()
    //     },
    //     CustomMaterial {
    //         gradient: gradient2.clone(),
    //         gradient_info: GradientInfo { mode: 3 },
    //     },
    //     view,
    // );
    //
    // LINZ New Zealand dataset: download and preprocess it first, see
    // preprocess/download_nz.sh and preprocess/examples/preprocess_nz.rs
    commands.spawn_terrain(
        asset_server.load("terrains/nz/config.tc.ron"),
        TerrainViewConfig {
            order: 1,
            ..default()
        },
        CustomMaterial {
            gradient: gradient1.clone(),
            gradient_info: GradientInfo { mode: 2 },
        },
        view,
    );

    // High-resolution Wellington: 1 m LiDAR elevation and 0.075 m aerial colour, see
    // preprocess/download_wellington.sh and preprocess/examples/preprocess_wellington.rs.
    // The 0.075 m survey only flew Wellington city, so colour covers about a quarter of
    // the elevation; the rest has geometry but no imagery.
    commands.spawn_terrain(
        asset_server.load("terrains/wellington/config.tc.ron"),
        TerrainViewConfig {
            // Above the nz terrain it sits inside: the stencil test keeps whichever
            // order is greatest where two terrains cover the same ground.
            order: 2,
            ..default()
        },
        CustomMaterial {
            gradient: gradient1.clone(),
            gradient_info: GradientInfo { mode: 2 },
        },
        view,
    );

    commands.spawn_terrain(
        asset_server.load("terrains/swiss/config.tc.ron"),
        TerrainViewConfig {
            order: 1,
            ..default()
        },
        CustomMaterial {
            gradient: gradient1.clone(),
            gradient_info: GradientInfo { mode: 1 },
        },
        view,
    );
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
