use bevy::dev_tools::fps_overlay::FpsOverlayPlugin;
use bevy::feathers::theme::ThemedText;
use bevy::feathers::{FeathersPlugins, dark_theme::create_dark_theme, theme::UiTheme};
use bevy::ui::Overflow;
use bevy::ui_widgets::ScrollArea;
use bevy::window::WindowResolution;
use bevy::{prelude::*, reflect::TypePath, render::render_resource::*, shader::ShaderRef};
use bevy_terrain::math::{Coordinate, unit_position};
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};
use transform_gizmo_bevy::{GizmoCamera, TransformGizmoPlugin};

mod plugins;
use plugins::auckland_rail::AucklandRailPlugin;
use plugins::camera_pose::home_pose;
use plugins::live_trains::LiveTrainsPlugin;
use plugins::photo_markers::PhotoMarkersPlugin;
use plugins::provenance::ProvenancePlugin;
use plugins::rail_editor::RailEditorPlugin;
use plugins::sheet_grid::SheetGridPlugin;
use plugins::stations::StationsPlugin;
use plugins::tab_panel::{SubTab, SubTabBody, TabPanelPlugin};
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
                TabPanelPlugin, // the tabbed panel of settings and readouts, on F11
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
        .add_systems(Startup, initialize)
        .add_systems(Update, (stream_terrains, fill_help_tab))
        .run();
}

/// The README is the one place the controls are written down, so the in-app list is
/// rendered from it rather than kept as a second copy that could drift.
const README: &str = include_str!("../../README.md");

/// Which of the README's headings each Help page carries.
///
/// The section has seven and the panel has four, so the small ones travel with the big one
/// they belong beside: the two cameras together, the lines and their editor with the trains on
/// them, and Quality Adjustments with the toggles it is the cost of. The markers, being a tool
/// of their own with a tab of their own, have a page of their own.
///
/// Named rather than taken in order, so that a heading renamed in the README shows up as a page
/// that has lost its lines rather than as lines quietly moved to another page. The pages are
/// `tab_panel`'s, the headings are the README's, and this is the one place the two meet.
fn help_headings(page: SubTab) -> &'static [&'static str] {
    match page {
        SubTab::HelpCamera => &["Fly Camera", "Orbital Camera"],
        SubTab::HelpVisu => &["Visualization Toggles", "Quality Adjustments"],
        SubTab::HelpMarkers => &["Photo Markers"],
        SubTab::HelpTrain => &["Rail Editor", "Trains"],
        _ => &[],
    }
}

/// How many characters a line of the help runs to before it is broken.
///
/// The lines come from the README, where each bullet is one line however long it runs. A
/// hundred characters is about what the Terrain tab's width gives this tab, and breaking them
/// here rather than leaving it to the layout puts the break at a word and in the same place
/// whichever tab has set the panel's width.
const HELP_COLUMNS: usize = 100;

/// A line of the help, broken at the last word that fits in [`HELP_COLUMNS`].
///
/// Greedy, and on whitespace only: a word longer than the whole column — a path, in practice —
/// is left to run over rather than cut in the middle, where it could not be read back.
fn wrapped(line: &str) -> String {
    let mut wrapped = String::with_capacity(line.len() + line.len() / HELP_COLUMNS);
    let mut column = 0;

    for word in line.split_whitespace() {
        let width = word.chars().count();

        match column {
            0 => {}
            _ if column + 1 + width > HELP_COLUMNS => {
                wrapped.push('\n');
                column = 0;
            }
            _ => {
                wrapped.push(' ');
                column += 1;
            }
        }

        wrapped.push_str(word);
        column += width;
    }

    wrapped
}

/// The named headings of the README's Debug Controls section as plain text: their bullets,
/// without the markdown or the prose around them, one string per heading and per bullet.
///
/// A string each rather than one with blank lines in it: a bullet that wraps to three lines
/// wants the next bullet a few pixels below it and not a whole empty line, and a few pixels is
/// a gap in the layout rather than anything the text itself can say.
fn hotkey_list_text(headings: &[&str]) -> Vec<String> {
    let section = README
        .split_once("## Debug Controls")
        .map_or("", |(_, rest)| rest);
    let section = &section[..section.find("\n## ").unwrap_or(section.len())];

    let mut taking = false;

    section
        .lines()
        .filter_map(|line| {
            if let Some(heading) = line.strip_prefix("### ") {
                // A heading this page does not carry takes its bullets with it, until the
                // next heading says otherwise.
                taking = headings.contains(&heading);
                taking.then(|| heading.to_string())
            } else if taking && line.starts_with("- ") {
                Some(wrapped(&line.replace('`', "")))
            } else {
                None
            }
        })
        .collect()
}

/// Fills each page of the F11 panel's Help tab with the controls as the README writes them.
///
/// The list was a panel of its own down the left, which F1 showed and hid. It is a tab now,
/// three pages of it, and F1 asks for that tab. The camera's two buttons used to sit in a row
/// above the text, filled in by camera_pose; the Camera tab has both of them as feathers
/// buttons, so the row has gone rather than be drawn twice.
///
/// Each page still scrolls: Visu alone is twenty-nine bullets, which is taller than the window
/// at a font a person would read. The wheel over it scrolls it, a node under the pointer being
/// the interface's rather than the camera's.
///
/// The text is the README's whole Debug Controls section, seventy-odd lines and taller than the
/// window at any font a person would read, so the body scrolls rather than running off the top:
/// bounded to two thirds of the window's height, and the wheel over it scrolls it, a node under
/// the pointer being the interface's rather than the camera's.
fn fill_help_tab(mut commands: Commands, bodies: Query<(Entity, &SubTabBody), Added<SubTabBody>>) {
    for (body, page) in &bodies {
        let headings = help_headings(page.0);
        if headings.is_empty() {
            continue;
        }

        // The scroll is a child rather than the body itself: the body arrives with the
        // `display` its page is laid out by, and an inserted `Node` would replace the lot.
        let page = commands
            .spawn((
                ScrollArea,
                Node {
                    flex_direction: FlexDirection::Column,
                    // What a blank line between the entries used to do, at a few pixels
                    // rather than a whole line of them. The panel holds this tab to the
                    // Terrain tab's width, so the longer bullets wrap to two and three, and
                    // without any gap a page of them reads as one paragraph of unrelated
                    // sentences.
                    row_gap: Val::Px(4.0),
                    // No height of its own: the tab body is bounded to the room the window
                    // has and scrolls, so this one only keeps the page's strip in place while
                    // the lines under it move.
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
                ChildOf(body),
            ))
            .id();

        for line in hotkey_list_text(headings) {
            // No size of its own: the panel's root carries the one every line in it is
            // written at, and a `ThemedText` takes it from there.
            commands.spawn((Text::new(line), ThemedText, ChildOf(page)));
        }
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
