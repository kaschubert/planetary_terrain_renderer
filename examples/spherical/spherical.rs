use bevy::dev_tools::fps_overlay::FpsOverlayPlugin;
use bevy::feathers::{FeathersPlugins, dark_theme::create_dark_theme, theme::UiTheme};
use bevy::text::FontSize;
use bevy::window::WindowResolution;
use bevy::{prelude::*, reflect::TypePath, render::render_resource::*, shader::ShaderRef};
use bevy_terrain::math::{Coordinate, unit_position};
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};
use transform_gizmo_bevy::{GizmoCamera, TransformGizmoPlugin};

mod plugins;
use plugins::auckland_rail::AucklandRailPlugin;
use plugins::camera_pose::{CameraButtonRow, CameraPosePlugin, home_pose};
use plugins::live_trains::LiveTrainsPlugin;
use plugins::photo_markers::PhotoMarkersPlugin;
use plugins::provenance::ProvenancePlugin;
use plugins::rail_editor::RailEditorPlugin;
use plugins::sheet_grid::SheetGridPlugin;
use plugins::stations::StationsPlugin;
use plugins::tab_panel::TabPanelPlugin;
use plugins::track_frames::TrackFramesPlugin;
use plugins::trains::TrainsPlugin;
use plugins::vram_usage::VramUsagePlugin;

const RADIUS: f64 = 6371000.0;

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
    /// never resident at once: wellington and auckland are 483 km apart, so with the
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
    // Three Topo50 sheets - the centre, West and South Auckland - as one terrain, so the
    // boundaries between them are interior and stitch. Centred on the middle of the three
    // rather than on the city. The radius stays under the 200 the others use to keep well
    // clear of Wellington 483 km away, which at 200 either side would be a 3 km margin.
    StreamedTerrain {
        path: "terrains/auckland/config.tc.ron",
        order: 2,
        gradient_mode: 2,
        longitude: 174.7550,
        latitude: -36.9450,
        radius: Some(150_000.0),
    },
];

#[derive(Resource)]
struct TerrainStreaming {
    view: Entity,
    gradient: Handle<Image>,
    /// The spawned entity per entry of STREAMED_TERRAINS.
    active: Vec<Option<Entity>>,
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
            // The example's own plugins, as a tuple of their own: add_plugins takes fifteen
            // plugins at most, and with these there are more.
            (
                VramUsagePlugin,    // gpu allocator usage, below the fps overlay
                CameraPosePlugin,   // the camera's start, and the F1 list's two buttons
                ProvenancePlugin,   // where each terrain's pixels came from, top right
                SheetGridPlugin,    // the Topo50 sheets over the terrain, coloured by coverage
                AucklandRailPlugin, // the rail lines over Auckland, in AT's colours
                RailEditorPlugin,   // selecting, adding and removing the lines' points, on F5
                TrackFramesPlugin,  // the track frames the models stand on, every 25 m, on F6
                StationsPlugin,     // the station names over the lines, on F9
                PhotoMarkersPlugin, // photos pinned over the terrain, on Ctrl+click and F10
                TrainsPlugin,       // one carriage per line, driving along the track, on F7
                // The carriages where Auckland Transport's feed puts the trains, on F8, with
                // the key from AT_API_KEY; tests.rs adds it without one, so it never fetches.
                LiveTrainsPlugin::from_env(),
                TabPanelPlugin,      // the tabbed panel of settings and readouts, on F11
            ),
            // Bevy's own widget set and theme, which the F11 panel is built from. The
            // group is TabNavigationPlugin and FeathersCorePlugin; InputDispatchPlugin,
            // which the widgets' focus needs, is in DefaultPlugins already.
            FeathersPlugins,
            TerrainPickingPlugin,
            // The move gizmo's arrows, see rail_editor/move_gizmo.rs. Added here and not by
            // RailEditorPlugin so that tests.rs can run the editor without a renderer, which
            // the gizmo crate's plugin wants.
            TransformGizmoPlugin,
        ))
        // A terrain costs roughly 2.6 MiB per atlas slot, for height and albedo together,
        // and the atlas is allocated whole however few slots are in use. With loading
        // culled to the view frustum the resident terrains hold about a hundred slots
        // each, against 1028 at the default size, but 256 still ran out, so 384 leaves
        // more room for the burst a fast turn requests before released slots cycle back.
        // Three are resident at once at most - the globe, the country and a city - which
        // is about 2.9 GiB. Running out no longer panics either: the finest tiles just go
        // missing until the tree re-requests them, with a warning.
        .insert_resource(TerrainSettings {
            atlas_size: 384,
            ..TerrainSettings::new(vec!["albedo"])
        })
        // .insert_resource(ClearColor(Color::WHITE))
        // Feathers has no theme of its own until one is given: UiTheme is empty by
        // default and every token in it resolves to a warning and a fuchsia, so the
        // dark theme goes in beside the plugins rather than being left to a widget to
        // notice. Dark because the F11 panel sits over the terrain with the rest.
        .insert_resource(UiTheme(create_dark_theme()))
        .add_systems(Startup, (initialize, spawn_hotkey_list))
        .add_systems(Update, (stream_terrains, toggle_hotkey_list))
        .run();
}

/// The README is the one place the controls are written down, so the in-app list is
/// rendered from it rather than kept as a second copy that could drift.
const README: &str = include_str!("../../README.md");

#[derive(Component)]
struct HotkeyList;

/// The Debug Controls section of the README as plain text: its headings and bullets,
/// without the markdown or the prose around them.
fn hotkey_list_text() -> String {
    let section = README
        .split_once("## Debug Controls")
        .map_or("", |(_, rest)| rest);
    let section = &section[..section.find("\n## ").unwrap_or(section.len())];

    section
        .lines()
        .filter_map(|line| {
            if let Some(heading) = line.strip_prefix("### ") {
                Some(format!("\n{heading}"))
            } else if line.starts_with("- ") {
                Some(line.replace('`', ""))
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The F1 list: a row of buttons, then the hotkeys themselves.
///
/// A column now rather than one text node, so that the camera buttons have somewhere to sit.
/// The row is left empty here and filled by camera_pose, which owns what goes in it. Both the
/// row and the text inherit the list's visibility, so F1 still shows and hides the lot, and
/// the buttons cannot be clicked while it is down: Bevy's UI picking skips what is not drawn.
fn spawn_hotkey_list(mut commands: Commands) {
    commands.spawn((
        HotkeyList,
        // Off until asked for; F1 brings it up.
        Visibility::Hidden,
        Node {
            position_type: PositionType::Absolute,
            // Below the fps overlay, whose graph puts its bottom edge around 120 px.
            top: Val::Px(150.0),
            left: Val::Px(6.0),
            // Bounded so the longer lines wrap instead of running across the screen.
            width: Val::Px(560.0),
            padding: UiRect::all(Val::Px(6.0)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
        children![
            (
                CameraButtonRow,
                Node {
                    flex_direction: FlexDirection::Row,
                    // Wrapping rather than overflowing, as the marker panel's rows do.
                    flex_wrap: FlexWrap::Wrap,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    row_gap: Val::Px(4.0),
                    ..default()
                },
            ),
            (
                Text::new(hotkey_list_text()),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
            ),
        ],
    ));
}

fn toggle_hotkey_list(
    input: Res<ButtonInput<KeyCode>>,
    mut list: Query<&mut Visibility, With<HotkeyList>>,
) {
    if !input.just_pressed(KeyCode::F1) {
        return;
    }

    for mut visibility in &mut list {
        *visibility = match *visibility {
            Visibility::Hidden => Visibility::Inherited,
            _ => Visibility::Hidden,
        };
    }
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

    // Wherever the F1 list's save button last left the camera, or the start over Auckland
    // with nothing saved yet. Written straight into the Transform below; the grid splits it
    // on the frame after, as a spatial spawn is placed in cell zero and moved from there.
    let (camera_position, camera_rotation) = home_pose();

    commands.spawn_big_space(Grid::default(), |root| {
        // big_space is built without its camera feature, so the root bundle leaves out the
        // Visibility it would otherwise carry, and every child that has one warns B0004:
        // the chain visibility inherits along has no start. Inherited visibility falls back
        // to visible when the parent has none, so nothing was hidden by it, but hiding the
        // whole space by hiding its root would not have worked. Visibility requires
        // InheritedVisibility and ViewVisibility, so this one insert gives the root all three.
        root.insert(Visibility::default());

        view = root
            .spawn_spatial((
                Transform::from_translation(camera_position.as_vec3())
                    .with_rotation(camera_rotation),
                DebugCameraController::new(RADIUS),
                OrbitalCameraController::default(),
                // The move gizmo draws through this camera and casts its pointer rays
                // from it. It reads the camera's GlobalTransform, which big_space keeps
                // in render space, so the handle it targets is placed in render space too.
                GizmoCamera,
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

#[cfg(test)]
mod tests;
