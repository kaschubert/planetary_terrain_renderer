use bevy::asset::LoadState;
use bevy::dev_tools::fps_overlay::{FPS_OVERLAY_ZINDEX, FpsOverlayPlugin};
use bevy::ecs::relationship::RelatedSpawnerCommands;
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

// Where the camera starts: over Auckland, looking north over the harbour from the city
// centre, with West and South Auckland to either side.
const CAMERA_LONGITUDE: f64 = 174.7633;
const CAMERA_LATITUDE: f64 = -36.8485;
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
            ProvenancePlugin,            // where each terrain's pixels came from, top right
            TerrainPickingPlugin,
        ))
        // A terrain costs roughly 2.6 MiB per atlas slot, for height and albedo together,
        // and the atlas is allocated whole however few slots are in use. With loading
        // culled to the view frustum the resident terrains hold about a hundred slots
        // each, against 1028 at the default size, so 256 leaves better than twice that in
        // hand for the burst a fast turn requests before released slots cycle back. Four
        // are resident at once at most - the globe, the country and a city - which is
        // about 2.0 GiB. Running out no longer panics either: the
        // finest tiles just go missing until the tree re-requests them, with a warning.
        .insert_resource(TerrainSettings {
            atlas_size: 256,
            ..TerrainSettings::new(vec!["albedo"])
        })
        // .insert_resource(ClearColor(Color::WHITE))
        .add_systems(Startup, (initialize, spawn_hotkey_list))
        .add_systems(Update, (stream_terrains, toggle_hotkey_list))
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

/// The README is the one place the controls are written down, so the in-app list is
/// rendered from it rather than kept as a second copy that could drift.
const README: &str = include_str!("../README.md");

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

fn spawn_hotkey_list(mut commands: Commands) {
    commands.spawn((
        HotkeyList,
        Text::new(hotkey_list_text()),
        TextFont {
            font_size: FontSize::Px(13.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            // Below the fps overlay, whose graph puts its bottom edge around 120 px.
            top: Val::Px(150.0),
            left: Val::Px(6.0),
            // Bounded so the longer lines wrap instead of running across the screen.
            width: Val::Px(560.0),
            padding: UiRect::all(Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
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

#[derive(Component)]
struct VramText;

#[derive(Component)]
struct CopyButton;

/// The "Copied!" note next to the button, gone again once the timer runs out.
#[derive(Component)]
struct CopiedToast(Timer);

struct VramUsagePlugin;

impl Plugin for VramUsagePlugin {
    fn build(&self, app: &mut App) {
        let usage = VramUsage::default();

        app.insert_resource(usage.clone())
            .add_systems(Startup, spawn_vram_text)
            .add_systems(
                Update,
                (
                    update_vram_text,
                    offset_fps_overlay,
                    copy_stats_to_clipboard,
                    highlight_copy_button,
                    expire_copied_toast,
                ),
            );

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
    commands
        .spawn((
            CopyButton,
            Button,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(6.0),
                left: Val::Px(420.0),
                padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
        ))
        .with_child((
            Text::new("copy"),
            TextFont {
                font_size: FontSize::Px(14.0),
                ..default()
            },
        ));

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

/// Puts the overlay text on the clipboard, so the numbers can be pasted somewhere.
///
/// On X11 whoever sets the clipboard has to keep serving it until another application
/// claims it, which is what wait() does - hence the thread, since it blocks.
fn copy_stats_to_clipboard(
    mut commands: Commands,
    button: Query<&Interaction, (Changed<Interaction>, With<CopyButton>)>,
    text: Single<&Text, With<VramText>>,
    mut toast: Query<&mut CopiedToast>,
) {
    use arboard::SetExtLinux;

    for interaction in &button {
        if *interaction != Interaction::Pressed {
            continue;
        }

        // One toast at a time: a second press while it is showing just restarts the clock.
        if let Ok(mut toast) = toast.single_mut() {
            toast.0.reset();
        } else {
            commands.spawn((
                CopiedToast(Timer::from_seconds(3.0, TimerMode::Once)),
                Text::new("Copied!"),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(8.0),
                    // Just right of the button.
                    left: Val::Px(476.0),
                    ..default()
                },
            ));
        }

        let stats = text.0.clone();

        std::thread::spawn(move || {
            match arboard::Clipboard::new() {
                Ok(mut clipboard) => {
                    let _ = clipboard.set().wait().text(stats);
                }
                Err(error) => error!("could not reach the clipboard: {error}"),
            };
        });
    }
}

fn expire_copied_toast(
    mut commands: Commands,
    time: Res<Time>,
    mut toasts: Query<(Entity, &mut CopiedToast)>,
) {
    for (entity, mut toast) in &mut toasts {
        if toast.0.tick(time.delta()).is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

/// Gives the copy button a hover and a press state, so it reads as a button.
fn highlight_copy_button(
    mut button: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<CopyButton>),
    >,
) {
    for (interaction, mut background) in &mut button {
        background.0 = match interaction {
            Interaction::None => Color::srgba(0.0, 0.0, 0.0, 0.5),
            Interaction::Hovered => Color::srgba(0.35, 0.35, 0.35, 0.8),
            Interaction::Pressed => Color::srgba(0.6, 0.6, 0.6, 0.9),
        };
    }
}

/// Moves the fps overlay down so the vram line can have the top left corner.
///
/// The overlay spawns at the origin and exposes no position setting, but its root is the
/// node carrying FPS_OVERLAY_ZINDEX, which is public.
fn offset_fps_overlay(mut overlay: Query<(&mut Node, &GlobalZIndex), Added<GlobalZIndex>>) {
    for (mut node, z_index) in &mut overlay {
        if z_index.0 == FPS_OVERLAY_ZINDEX {
            node.top = Val::Px(52.0);
            node.left = Val::Px(6.0);
        }
    }
}

fn update_vram_text(
    usage: Res<VramUsage>,
    atlases: Query<&TileAtlas>,
    mut text: Single<&mut Text, With<VramText>>,
) {
    const GIB: f64 = (1u64 << 30) as f64;

    let allocated = usage.allocated.load(Ordering::Relaxed) as f64 / GIB;
    let reserved = usage.reserved.load(Ordering::Relaxed) as f64 / GIB;

    // Slots run out before memory does, and unlike the figures above they respond to
    // frustum culling: the atlas texture is allocated whole, however little of it is used.
    let (used_slots, total_slots) = atlases
        .iter()
        .map(TileAtlas::slot_usage)
        .fold((0, 0), |(used, total), (u, t)| (used + u, total + t));

    text.0 = format!(
        "VRAM used / alloc {allocated:.2} / {reserved:.2} GiB\n\
         atlas slots {used_slots} / {total_slots} ({} terrains)",
        atlases.iter().len()
    );
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

/// The table of where each terrain's pixels came from, toggled with F2.
///
/// It loads its own files rather than reading the terrains', because the terrains are not
/// there to read. They stream in by camera distance, so Wellington's config is never loaded
/// at all unless you fly to it, and one that does load is dropped as soon as the atlas has
/// copied out of it. A provenance file is a few kilobytes, so this holds all of them for
/// the life of the process and the table is complete from the first frame.
struct ProvenancePlugin;

impl Plugin for ProvenancePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProvenanceTable>()
            .add_systems(Startup, (load_provenance, spawn_provenance_table))
            .add_systems(
                Update,
                (
                    settle_provenance.run_if(provenance_pending),
                    rebuild_provenance_table.run_if(provenance_dirty),
                    toggle_provenance_table,
                )
                    .chain(),
            );
    }
}

#[derive(Component)]
struct ProvenancePanel;

#[derive(Resource, Default)]
struct ProvenanceTable {
    entries: Vec<ProvenanceEntry>,
    /// Set when an entry settles, cleared when the rows are rebuilt.
    dirty: bool,
}

struct ProvenanceEntry {
    name: String,
    /// Held for the life of the app. Dropping it would unload the asset, which is exactly
    /// how the streamed terrain configs disappear.
    handle: Handle<TerrainProvenance>,
    state: ProvenanceState,
}

#[derive(PartialEq)]
enum ProvenanceState {
    Pending,
    Loaded,
    /// The terrain was preprocessed before its sources were recorded. Backfill it with
    /// `preprocess_<name> --provenance-only`.
    Missing,
}

/// The columns, in order. The first is the terrain, left blank on continuation rows so the
/// name reads as a heading without needing a spanning row.
const PROVENANCE_COLUMNS: [&str; 8] = [
    "terrain", "layer", "dataset", "res", "captured", "sheets", "tiles", "size",
];

fn load_provenance(asset_server: Res<AssetServer>, mut table: ResMut<ProvenanceTable>) {
    table.entries = STREAMED_TERRAINS
        .iter()
        .map(|terrain| {
            // "terrains/auckland/config.tc.ron" -> "terrains/auckland" -> "auckland"
            let directory = terrain.path.rsplit_once('/').map_or("", |(dir, _)| dir);

            ProvenanceEntry {
                name: directory
                    .rsplit('/')
                    .next()
                    .unwrap_or(directory)
                    .to_string(),
                handle: asset_server.load(format!("{directory}/provenance.tp.ron")),
                state: ProvenanceState::Pending,
            }
        })
        .collect();

    table.dirty = true;
}

fn provenance_pending(table: Res<ProvenanceTable>) -> bool {
    table
        .entries
        .iter()
        .any(|entry| entry.state == ProvenanceState::Pending)
}

fn provenance_dirty(table: Res<ProvenanceTable>) -> bool {
    table.dirty
}

/// Moves entries out of Pending as the asset server finishes with them.
///
/// Polled rather than driven by AssetEvent because a file that is not there raises no
/// event at all, and a terrain built before provenance existed is the ordinary case. The
/// polling stops for good once every entry has settled.
fn settle_provenance(asset_server: Res<AssetServer>, mut table: ResMut<ProvenanceTable>) {
    let mut settled = false;

    for entry in &mut table.entries {
        entry.state = match asset_server.load_state(&entry.handle) {
            LoadState::Loaded => ProvenanceState::Loaded,
            LoadState::Failed(_) => ProvenanceState::Missing,
            _ => continue,
        };

        settled = true;
    }

    table.dirty |= settled;
}

fn spawn_provenance_table(mut commands: Commands) {
    commands.spawn((
        ProvenancePanel,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(6.0),
            // Opposite side to the hotkey list, which is long enough to run most of the
            // way down the left.
            right: Val::Px(6.0),
            display: Display::Grid,
            grid_template_columns: RepeatedGridTrack::auto(PROVENANCE_COLUMNS.len() as u16),
            column_gap: Val::Px(12.0),
            row_gap: Val::Px(2.0),
            padding: UiRect::all(Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
        Visibility::Hidden,
    ));
}

fn rebuild_provenance_table(
    mut commands: Commands,
    mut table: ResMut<ProvenanceTable>,
    provenance: Res<Assets<TerrainProvenance>>,
    panel: Single<Entity, With<ProvenancePanel>>,
) {
    table.dirty = false;

    let mut panel = commands.entity(*panel);
    panel.despawn_children();

    panel.with_children(|panel| {
        for column in PROVENANCE_COLUMNS {
            provenance_cell(panel, column, Color::srgb(0.65, 0.8, 1.0));
        }

        // A rule under the headings, spanning every column. Grid lines are 1 indexed.
        panel.spawn((
            Node {
                grid_column: GridPlacement::start_span(1, PROVENANCE_COLUMNS.len() as u16),
                height: Val::Px(1.0),
                ..default()
            },
            BackgroundColor(Color::srgb(0.4, 0.4, 0.4)),
        ));

        for entry in &table.entries {
            let sources = match entry.state {
                ProvenanceState::Loaded => provenance.get(&entry.handle),
                _ => None,
            };

            let Some(sources) = sources else {
                let note = match entry.state {
                    ProvenanceState::Pending => "loading",
                    // Not an error: every terrain built before this existed has none.
                    _ => "no provenance recorded",
                };

                provenance_cell(panel, &entry.name, Color::WHITE);
                provenance_cell(panel, "", Color::WHITE);
                provenance_cell(panel, note, Color::srgb(0.6, 0.6, 0.6));

                for _ in 3..PROVENANCE_COLUMNS.len() {
                    provenance_cell(panel, "", Color::WHITE);
                }

                continue;
            };

            // A HashMap has no order of its own, and a table that reshuffles between
            // rebuilds is unreadable. Height first, it being the layer the rest sits on.
            let mut layers = sources.sources.iter().collect::<Vec<_>>();
            layers.sort_by_key(|(label, _)| {
                (**label != AttachmentLabel::Height, String::from(*label))
            });

            let mut first = true;

            for (label, records) in layers {
                for record in records {
                    // The terrain names itself once, on its first row.
                    provenance_cell(panel, if first { &entry.name } else { "" }, Color::WHITE);
                    first = false;

                    provenance_cell(panel, &String::from(label), Color::WHITE);

                    let Some(manifest) = &record.manifest else {
                        // A source no download script fetched, as the example terrains use.
                        provenance_cell(panel, &record.path, Color::srgb(0.6, 0.6, 0.6));

                        for _ in 3..PROVENANCE_COLUMNS.len() {
                            provenance_cell(panel, "", Color::WHITE);
                        }

                        continue;
                    };

                    let colour = if manifest.gains.is_some() {
                        // Colour matched, so what you are looking at is not quite what the
                        // survey published.
                        Color::srgb(1.0, 0.85, 0.5)
                    } else {
                        Color::WHITE
                    };

                    provenance_cell(panel, &manifest.dataset, colour);
                    provenance_cell(panel, &manifest.resolution, Color::WHITE);
                    // The national elevation mosaic is stitched from surveys spanning
                    // years and publishes no single date.
                    provenance_cell(
                        panel,
                        manifest.captured.as_deref().unwrap_or("-"),
                        Color::WHITE,
                    );
                    provenance_cell(panel, &manifest.sheets.len().to_string(), Color::WHITE);
                    provenance_cell(panel, &manifest.tiles.to_string(), Color::WHITE);
                    provenance_cell(panel, &gibibytes(manifest.bytes), Color::WHITE);
                }
            }
        }
    });
}

fn provenance_cell(panel: &mut RelatedSpawnerCommands<ChildOf>, text: &str, colour: Color) {
    panel.spawn((
        // Columns are sized to their content, so a cell that wrapped would size its column
        // to a word instead of the whole string.
        TextLayout::new(Justify::Left, LineBreak::NoWrap),
        Text::new(ascii(text)),
        TextFont {
            font_size: FontSize::Px(12.0),
            ..default()
        },
        TextColor(colour),
    ));
}

/// The bundled font covers printable ASCII and nothing else, so anything outside it would
/// render as a gap. These strings come from data files rather than from this source, so
/// fold rather than trust.
fn ascii(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\u{2013}' | '\u{2014}' | '\u{2022}' | '\u{00b7}' => '-',
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            '\u{00a0}' => ' ',
            character if character.is_ascii_graphic() || character == ' ' => character,
            _ => '?',
        })
        .collect()
}

fn gibibytes(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / (1u64 << 30) as f64)
}

fn toggle_provenance_table(
    input: Res<ButtonInput<KeyCode>>,
    mut panel: Query<&mut Visibility, With<ProvenancePanel>>,
) {
    if !input.just_pressed(KeyCode::F2) {
        return;
    }

    for mut visibility in &mut panel {
        *visibility = match *visibility {
            Visibility::Hidden => Visibility::Visible,
            _ => Visibility::Hidden,
        };
    }
}
