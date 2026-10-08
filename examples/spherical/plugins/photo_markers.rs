//! Markers pinned to the terrain, each holding a photo on its screen.
//!
//! A marker is a place on the ground, a colour and a photo, and nothing more. Ctrl and the left
//! button stand one where the cursor is: the picking pass already reads the terrain depth under
//! the cursor back as a position, so the hit costs nothing here, and Control says the press is
//! this plugin's rather than the camera's, which pans on the same button. The modifier is what
//! makes it safe, and it is the same one the rail editor's Ctrl chords use; the library stands
//! aside for it, in toggle_debug for the keys and in the orbital camera for the left button.
//!
//! A marker remembers where it is as a direction on the unit sphere and the terrain height under
//! it, not as a position, so its position is recomputed every frame and a saved marker comes back
//! where it was. It floats HOVER_HEIGHT above that ground, measured to the plate's base, and yaws
//! about the ground's up each frame so its screen faces the camera: the plate's recess has no
//! back, so there has to be no behind.
//!
//! Each marker is a holder entity under the big_space root, carrying the cell and the transform
//! within it that place it, with the model's two meshes as children of its own materials. The
//! meshes are loaded by name out of the glb rather than by spawning its scene, because the file
//! has no materials and a scene spawn would hand both meshes one shared material, so colouring
//! the plate would colour the photo too.
//!
//! What is where: the measured numbers and the pure geometry in model.rs, the dropped photos in
//! photo.rs, the F10 panel in panel.rs, the saved file in file.rs. The placing systems run in
//! PostUpdate before the transforms propagate, as the trains' do, and the ones that read the
//! mouse after them, as the editor's do, so a click is tested against where things are this frame.

use bevy::ecs::query::QueryData;
use bevy::gltf::GltfMesh;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};
use std::path::PathBuf;

use super::rail_editor::clicks::ClickDetector;
use super::rail_editor::frame::{Frame, unit_under};
use super::sheet_grid::IN_FRONT_OF_TERRAIN;
use model::{
    HOVER_HEIGHT, MARKER_HEIGHT, MARKER_MESH, MODEL_PATH, SCREEN_MESH, apparent_pixels,
    billboard_rotation, model_transform, pixel_floor_scale,
};

mod file;
mod model;
mod panel;
mod photo;

/// How near the cursor a marker has to be on screen to be the one a plain click selects, in
/// pixels. The editor's discs use 10 for points a few pixels across; a marker is larger and
/// stands alone, so its own projected height is used where that is bigger, see nearest_marker.
const PICK_PIXELS: f32 = 12.0;

/// The colour a marker takes when nothing has said otherwise: a very light grey. Held as hue,
/// saturation and value rather than as a colour, so that the panel's hue slider still means
/// something at zero saturation, where a grey has no hue to read back.
const DEFAULT_COLOUR: Hsva = Hsva::hsv(0.0, 0.0, 0.9);

/// The screen before a photo lands on it: near enough to black to read as a screen that is off,
/// and dark enough that a light plate still reads as the marker's colour.
const BLANK_SCREEN: Color = Color::srgb(0.08, 0.09, 0.10);

/// The ring round the selected marker: white, as the editor's selected discs are, which no
/// marker colour can be mistaken for since a white plate is still ringed in white on the dark
/// terrain around it.
const SELECTION_COLOUR: Color = Color::WHITE;

/// The selection ring's own gizmo group, so that it can be drawn in front of the terrain the way
/// the rail overlays are without the default group's settings following it.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct PhotoMarkerGizmos;

/// A marker's identity, which outlives its index in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerId(u32);

/// How the selected marker stands on screen at this moment: what it would take to judge whether
/// it is the right size, and what the panel's sample button writes to marker_sizes.csv.
///
/// The distance is what the size has to answer to; the altitude is there because it is what a
/// person means by how far out they are zoomed, and the two come apart on a tilted view. The
/// field of view and the viewport are recorded so that the pixels can be recomputed from the
/// metres afterwards, whatever the window was.
#[derive(Resource, Default)]
pub struct MarkerView(pub Option<SizeSample>);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizeSample {
    /// Metres from the camera to the marker's base.
    pub distance: f64,
    /// The camera's height above the ellipsoid, in metres.
    pub altitude: f64,
    /// How tall the marker stands on screen, in pixels, at the height the slider is on.
    pub pixels: f32,
    /// The viewport's height in pixels and the camera's vertical field of view in degrees.
    pub viewport: f32,
    pub fov: f32,
}

/// One marker: where it is, what colour it is, and what is on its screen.
#[derive(Debug, Clone)]
pub struct PhotoMarker {
    pub id: MarkerId,
    /// The direction on the unit sphere the marker stands over, which with the height below
    /// gives its position; see model.rs for why this and not a position.
    pub unit: DVec3,
    /// The terrain height under it above the ellipsoid, in metres, from the pick that placed it.
    pub ground: f64,
    pub colour: Hsva,
    /// The file the photo on its screen came from, if any. Saved with the marker and read back
    /// at startup, which is what puts a picture on a screen again without it being dropped twice.
    pub photo: Option<PathBuf>,
    /// Set when that file would not read, so the panel can say so. Not saved: whether a file is
    /// there is a fact about the disk now, not about the marker, and a drive plugged back in
    /// should not have to argue with the file about it.
    pub missing: bool,
    /// The holder entity, once spawned, and the two materials that are this marker's own.
    pub entity: Option<Entity>,
    pub body_material: Option<Handle<StandardMaterial>>,
    pub screen_material: Option<Handle<StandardMaterial>>,
}

impl PhotoMarker {
    /// Where the plate's base floats: the ground under the marker plus the hover height.
    pub fn position(&self) -> DVec3 {
        TerrainShape::WGS84.position_unit_to_local(self.unit, self.ground + HOVER_HEIGHT)
    }

    /// Longitude and latitude in degrees, for the readout and the file: the inverse of
    /// unit_position, whose x runs the other way, so the longitude is measured against -x.
    pub fn longitude(&self) -> f64 {
        self.unit.z.atan2(-self.unit.x).to_degrees()
    }

    pub fn latitude(&self) -> f64 {
        self.unit.y.asin().to_degrees()
    }
}

/// The markers, the selection, and the switch that shows the panel.
#[derive(Resource, Default)]
pub struct PhotoMarkers {
    /// Toggled by F10, in panel.rs, which shows the panel exactly while this is on. It gates the
    /// panel and the keys that destroy something, Delete and Escape, and nothing else: placing
    /// and selecting both work with it off, since neither can be done by accident.
    pub editing: bool,
    pub markers: Vec<PhotoMarker>,
    /// The marker last placed or clicked, as an index into markers. A dropped photo goes to it.
    pub selected: Option<usize>,
    /// The colour the next marker takes, which the panel's sliders set while nothing is
    /// selected. Selecting a marker brings its colour here, so the sliders always read as the
    /// colour they would apply.
    pub colour: Hsva,
    /// How tall every marker is drawn, base to top, in metres. One number for all of them rather
    /// than one each: what is being settled is how big a marker should be against the terrain, and
    /// the panel's slider moves this while the example runs so it can be judged by eye.
    pub height: f32,
    /// The pixel floor every marker is held to when far away, in pixels, or zero for none. Off
    /// while the height is being settled, see MIN_PIXELS.
    pub min_pixels: f32,
    /// Edits not yet written to the file, which dims the panel's save button when there are none.
    pub dirty: bool,
    /// Set when there is a file that would not read, which blocks saving over it; see file.rs.
    pub file_unreadable: bool,
    clicks: ClickDetector,
    /// Whether Control was down when the press began, so that the release knows which it was.
    chord: bool,
    next_id: u32,
}

impl PhotoMarkers {
    /// A marker on the ground at a direction on the unit sphere, in the current colour,
    /// selected. The entity and the materials follow on the next frame, see spawn_markers.
    pub fn place(&mut self, unit: DVec3, ground: f64) -> MarkerId {
        let id = MarkerId(self.next_id);
        self.next_id += 1;

        self.markers.push(PhotoMarker {
            id,
            unit,
            ground,
            colour: self.colour,
            photo: None,
            missing: false,
            entity: None,
            body_material: None,
            screen_material: None,
        });
        self.selected = Some(self.markers.len() - 1);
        self.dirty = true;

        id
    }

    /// Selects a marker, and brings its colour to the panel with it, so that the sliders read as
    /// the colour they would apply rather than as whatever was set before. Without this a drag
    /// after a selection would start from the colour of the marker selected before it.
    pub fn select(&mut self, index: usize) {
        let Some(marker) = self.markers.get(index) else {
            return;
        };

        self.colour = marker.colour;
        self.selected = Some(index);
    }

    /// How tall a marker is actually drawn at this moment, in metres: the size slider's height,
    /// scaled up by the pixel floor when it is far enough away for that to bite. The hit test and
    /// the selection ring both want this rather than the model's own height, or they would
    /// describe a marker of a size nothing on screen has.
    pub fn drawn_height(
        &self,
        marker: &PhotoMarker,
        camera_position: DVec3,
        focal_pixels: Option<f32>,
    ) -> f32 {
        let distance = camera_position.distance(marker.position());
        let floor = focal_pixels
            .map(|focal| pixel_floor_scale(self.height, distance, focal, self.min_pixels))
            .unwrap_or(1.0);

        self.height * floor
    }

    /// The marker the panel reads out and a photo lands on.
    pub fn selected(&self) -> Option<&PhotoMarker> {
        self.selected.and_then(|index| self.markers.get(index))
    }

    pub fn selected_mut(&mut self) -> Option<&mut PhotoMarker> {
        match self.selected {
            Some(index) => self.markers.get_mut(index),
            None => None,
        }
    }

    /// Removes the selection, returning its holder entity for despawning. The selection moves
    /// to nothing rather than to a neighbour, so nothing is edited by surprise afterwards.
    pub fn remove_selected(&mut self) -> Option<Entity> {
        let index = self.selected.take()?;
        if index >= self.markers.len() {
            return None;
        }

        let marker = self.markers.remove(index);
        self.dirty = true;

        marker.entity
    }
}

/// The holder entity of a marker. Which marker it is comes from the list, which holds the
/// entity, so the component is a label and nothing more.
#[derive(Component)]
pub struct Marker;

/// The glb's two meshes, once the asset has loaded.
#[derive(Resource, Default)]
struct MarkerModel {
    gltf: Handle<Gltf>,
    meshes: Option<MarkerMeshes>,
}

struct MarkerMeshes {
    marker: Handle<Mesh>,
    screen: Handle<Mesh>,
}

/// The mouse systems, which the panel's readout orders itself after so that it reads the marker
/// clicked this frame.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhotoMarkerSystems;

pub struct PhotoMarkersPlugin;

impl Plugin for PhotoMarkersPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PhotoMarkers::load())
            .init_resource::<MarkerModel>()
            .init_resource::<MarkerView>()
            .init_gizmo_group::<PhotoMarkerGizmos>()
            .add_systems(Startup, configure_marker_gizmos)
            .add_plugins(panel::PhotoPanelPlugin)
            .add_systems(Startup, load_marker_model)
            .add_systems(
                Update,
                (
                    resolve_marker_model,
                    spawn_markers.run_if(markers_unspawned),
                    // After the spawn, so a photo dropped on the frame a marker was placed finds
                    // the material it goes on rather than waiting for the next one.
                    (
                        photo::tint_on_hover,
                        photo::receive_drops,
                        photo::poll_photos,
                    ),
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                (
                    // Before the transforms propagate, as the carriages are placed: the cell and
                    // the transform written here are absolute and want nothing of the camera's.
                    place_markers.before(TransformSystems::Propagate),
                    // After, as the editor's mouse is: a click is tested against where the
                    // markers are on screen this frame, which propagation has just settled.
                    (edit_with_mouse, ring_selection)
                        .chain()
                        .in_set(PhotoMarkerSystems)
                        .after(TransformSystems::Propagate),
                ),
            );
    }
}

/// Every marker's holder, as the placing writes it. Without the camera, which carries the same
/// three components and would otherwise be reachable two ways, which Bevy refuses.
type MarkerHolders<'world, 'state> = Query<
    'world,
    'state,
    (
        &'static mut CellCoord,
        &'static mut Transform,
        &'static mut Visibility,
    ),
    (With<Marker>, Without<OrbitalCameraController>),
>;

/// The camera as the mouse and the placing need it: to project a marker onto the screen, to
/// shift positions into its cell, to size a marker in pixels, and to read the terrain hit under
/// the cursor.
#[derive(QueryData)]
struct MarkerCamera {
    entity: Entity,
    camera: &'static Camera,
    global: &'static GlobalTransform,
    transform: &'static Transform,
    cell: &'static CellCoord,
    projection: &'static Projection,
    picking: &'static PickingData,
}

fn configure_marker_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<PhotoMarkerGizmos>();
    config.depth_bias = IN_FRONT_OF_TERRAIN;
}

fn load_marker_model(mut model: ResMut<MarkerModel>, asset_server: Res<AssetServer>) {
    model.gltf = asset_server.load(MODEL_PATH);
}

/// Takes the two meshes out of the glb once it has loaded, by name. Not by index: a re-export
/// from Blender can renumber them, and a name that has gone is worth saying out loud rather
/// than silently drawing the wrong mesh.
fn resolve_marker_model(
    mut model: ResMut<MarkerModel>,
    gltfs: Res<Assets<Gltf>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
) {
    if model.meshes.is_some() {
        return;
    }
    let Some(gltf) = gltfs.get(&model.gltf) else {
        return;
    };

    let mesh = |name: &str| -> Option<Handle<Mesh>> {
        let mesh = gltf.named_meshes.get(name)?;
        Some(gltf_meshes.get(mesh)?.primitives.first()?.mesh.clone())
    };

    match (mesh(MARKER_MESH), mesh(SCREEN_MESH)) {
        (Some(marker), Some(screen)) => {
            model.meshes = Some(MarkerMeshes { marker, screen });
            info!("photo markers: {MODEL_PATH} loaded");
        }
        _ => error_once!(
            "photo markers: {MODEL_PATH} has no meshes named {MARKER_MESH} and {SCREEN_MESH}; \
             the markers will not draw"
        ),
    }
}

fn markers_unspawned(markers: Res<PhotoMarkers>) -> bool {
    markers.markers.iter().any(|marker| marker.entity.is_none())
}

/// Spawns the holder and the two mesh children for every marker that has none: the ones a click
/// just placed, and the ones the file brought back once the big_space root and the model are
/// both there to be found. Each marker gets materials of its own, so recolouring one leaves the
/// rest alone, and both are unlit: the scene's one light points in a fixed direction that has
/// nothing to do with the ground under a marker, so a lit plate would not be the colour picked.
fn spawn_markers(
    mut commands: Commands,
    mut markers: ResMut<PhotoMarkers>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    model: Res<MarkerModel>,
    grids: Grids,
    camera: Query<Entity, With<OrbitalCameraController>>,
) {
    let Some(meshes) = &model.meshes else {
        return;
    };
    let Ok(camera) = camera.single() else {
        return;
    };
    let Some(root) = grids.parent_grid_entity(camera) else {
        return;
    };

    let (mut spawned, mut loading) = (0, 0);
    for marker in &mut markers.markers {
        if marker.entity.is_some() {
            continue;
        }

        let body = materials.add(StandardMaterial {
            base_color: marker.colour.into(),
            unlit: true,
            ..default()
        });
        // The quad is one sided and the recess around it has no back, so a marker caught
        // side-on while it turns would show through. Double sided costs nothing on two
        // triangles.
        let screen = materials.add(StandardMaterial {
            base_color: BLANK_SCREEN,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        });

        let entity = commands
            .spawn((
                Marker,
                CellCoord::default(),
                Transform::default(),
                // Until place_markers has stood it somewhere, which is the frame after this.
                Visibility::Hidden,
                ChildOf(root),
            ))
            .with_children(|holder| {
                holder.spawn((
                    Mesh3d(meshes.marker.clone()),
                    MeshMaterial3d(body.clone()),
                    model_transform(),
                ));
                holder.spawn((
                    Mesh3d(meshes.screen.clone()),
                    MeshMaterial3d(screen.clone()),
                    model_transform(),
                ));
            })
            .id();

        // A marker read back from the file knows which photo was on it but has no texture yet,
        // so the decode starts here, once, as the screen it goes on comes into being. A marker
        // just placed has no photo and skips it.
        if let Some(path) = marker.photo.clone() {
            photo::start_decode(&mut commands, marker.id, path);
            loading += 1;
        }

        marker.entity = Some(entity);
        marker.body_material = Some(body);
        marker.screen_material = Some(screen);
        spawned += 1;
    }

    if spawned > 0 {
        info!(
            "photo markers: {spawned} placed, {} m tall{}",
            markers.height,
            match loading {
                0 => String::new(),
                loading => format!(", {loading} reading their photos"),
            }
        );
    }
}

/// Stands every marker over its ground, facing the camera, as the cell and the transform within
/// it that big_space places it by, at the height the panel's slider is on. A marker too far away
/// to read is scaled up to the pixel floor, when there is one.
///
/// Along the way it notes how the selected marker stands on screen, which is what the panel reads
/// out and what its sample button writes down.
fn place_markers(
    markers: Res<PhotoMarkers>,
    mut view_sample: ResMut<MarkerView>,
    grids: Grids,
    camera: Query<MarkerCamera, With<OrbitalCameraController>>,
    mut holders: MarkerHolders,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let Some(grid) = grids.parent_grid(camera.entity) else {
        return;
    };
    let camera_position = grid.grid_position_double(camera.cell, camera.transform);

    // The focal length in pixels, as the editor's discs compute it: a metre at a metre's
    // distance covers this many pixels. None for a frame or two at startup, before the window
    // has told the camera its size, and then the floor is simply not applied.
    let view = match (camera.camera.logical_viewport_size(), camera.projection) {
        (Some(viewport), Projection::Perspective(perspective)) => Some((
            viewport.y / (2.0 * (perspective.fov / 2.0).tan()),
            viewport.y,
            perspective.fov.to_degrees(),
        )),
        _ => None,
    };
    let focal_pixels = view.map(|(focal, _, _)| focal);

    // How high the camera itself is, which is what a person means by how far out they are: the
    // climb along the direction heights run in, as a marker's own ground is measured.
    let camera_unit = unit_under(camera_position);
    let altitude = (camera_position - TerrainShape::WGS84.scale() * camera_unit)
        .dot(Frame::at_unit(camera_unit).up);

    let height = markers.height;
    let min_pixels = markers.min_pixels;
    let selected = markers.selected;
    let mut sample = None;

    for (index, marker) in markers.markers.iter().enumerate() {
        let Some(entity) = marker.entity else {
            continue;
        };
        let Ok((mut cell, mut transform, mut visibility)) = holders.get_mut(entity) else {
            continue;
        };

        let position = marker.position();
        let distance = camera_position.distance(position);
        let frame = Frame::at_unit(marker.unit);
        let rotation = billboard_rotation(frame.up, camera_position - position);

        // The model is built MARKER_HEIGHT tall, so the slider's height is a scale on top of it,
        // and the floor a scale on top of that.
        let floor = focal_pixels
            .map(|focal| pixel_floor_scale(height, distance, focal, min_pixels))
            .unwrap_or(1.0);
        let scale = floor * height / MARKER_HEIGHT;

        if Some(index) == selected
            && let Some((focal, viewport, fov)) = view
        {
            sample = Some(SizeSample {
                distance,
                altitude,
                pixels: apparent_pixels(height * floor, distance, focal),
                viewport,
                fov,
            });
        }

        let (placed_cell, translation) = grid.translation_to_grid(position);
        cell.set_if_neq(placed_cell);
        transform.set_if_neq(Transform {
            translation,
            rotation: rotation.as_quat(),
            scale: Vec3::splat(scale),
        });
        visibility.set_if_neq(Visibility::Visible);
    }

    // A resource of its own rather than a field on the markers: the distance changes every frame
    // the camera moves, so writing it there would mark the markers edited every frame and the
    // systems that watch them for a real change would all run for nothing.
    view_sample.0 = sample;
}

/// Ctrl+click on the terrain places a marker, a plain click selects one, Delete removes the
/// selection and Escape clears it. The press and the release are told apart by the editor's
/// click rules, so a drag does neither: a Ctrl-drag places nothing, and a plain drag is the
/// camera's pan as it always was.
///
/// Placing and selecting are not gated on the panel being open. A photo is dropped onto whichever
/// marker is selected, so needing the panel up to choose one would be a hidden step between
/// having a marker and putting a picture on it. Delete and Escape are gated, since those lose
/// work and the rail editor wants the same two keys.
#[allow(clippy::too_many_arguments)]
fn edit_with_mouse(
    mut commands: Commands,
    mut markers: ResMut<PhotoMarkers>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    capture: Res<PointerCapture>,
    time: Res<Time<Real>>,
    window: Query<&Window, With<PrimaryWindow>>,
    grids: Grids,
    camera: Query<MarkerCamera, With<OrbitalCameraController>>,
) {
    if markers.editing {
        if keys.just_pressed(KeyCode::Escape) {
            markers.selected = None;
        }
        if keys.just_pressed(KeyCode::Delete)
            && let Some(entity) = markers.remove_selected()
        {
            commands.entity(entity).despawn();
        }
    }

    let Ok(window) = window.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let now = time.elapsed_secs_f64();

    if buttons.just_pressed(MouseButton::Left) {
        markers.chord = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
        let blocked = capture.blocks_pointer();
        markers.clicks.press(cursor, now, blocked);
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let chord = markers.chord;
    let Some(click) = markers.clicks.release(cursor, now) else {
        return;
    };

    let Ok(camera) = camera.single() else {
        return;
    };
    let Some(grid) = grids.parent_grid(camera.entity) else {
        return;
    };

    // A plain click selects, whether or not the panel is open: a photo is dropped on the
    // selected marker, and wanting to drop one is no reason to have the panel up. A click that
    // lands on no marker leaves the selection alone, so nothing is lost by missing.
    if !chord {
        let cell_origin = grid.cell_to_float(camera.cell);
        let camera_position = grid.grid_position_double(camera.cell, camera.transform);
        let focal_pixels = match (camera.camera.logical_viewport_size(), camera.projection) {
            (Some(viewport), Projection::Perspective(perspective)) => {
                Some(viewport.y / (2.0 * (perspective.fov / 2.0).tan()))
            }
            _ => None,
        };

        if let Some(index) = nearest_marker(
            &markers,
            camera_position,
            focal_pixels,
            camera.camera,
            camera.global,
            cell_origin,
            click.position(),
        ) {
            markers.select(index);
            let marker = &markers.markers[index];
            info!(
                "photo markers: marker {} of {} selected, {}",
                index + 1,
                markers.markers.len(),
                match &marker.photo {
                    Some(path) => format!(
                        "showing {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    None => "no photo yet: drop one on the window".to_string(),
                }
            );
        }
        return;
    }

    // The hit is in render space relative to the cell the readback was taken in, which is not
    // always the camera's current one, so it is that cell the position is resolved against.
    let Some(translation) = camera.picking.translation else {
        return;
    };
    let hit = grid.grid_position_double(
        &camera.picking.cell,
        &Transform::from_translation(translation),
    );

    // The height the hit stands at above the ellipsoid, along the direction heights run in, so
    // that position() puts the marker back exactly here plus the hover.
    let unit = unit_under(hit);
    let ground = (hit - TerrainShape::WGS84.scale() * unit).dot(Frame::at_unit(unit).up);

    markers.place(unit, ground);
    let marker = markers.markers.last().expect("just placed");
    info!(
        "photo markers: placed at {:.5}, {:.5}, ground {:.1} m",
        marker.latitude(),
        marker.longitude(),
        marker.ground,
    );
}

/// Where a marker's middle falls on screen and how far from it a click still counts, in pixels.
///
/// The reach is the larger of PICK_PIXELS and half the marker's own height on screen, so a marker
/// drawn large can be clicked anywhere on it and a distant speck is still catchable. The height
/// is the one being drawn, the slider's and the floor's together, and not the model's own: a
/// marker scaled up to 200 m whose reach was still the model's 12 would have a clickable spot the
/// size of a full stop somewhere near its foot, which is no way to select anything.
fn marker_on_screen(
    marker: &PhotoMarker,
    drawn_height: f32,
    camera: &Camera,
    camera_global: &GlobalTransform,
    cell_origin: DVec3,
) -> Option<(Vec2, f32)> {
    let position = marker.position();
    let up = Frame::at_unit(marker.unit).up;
    let centre = position + up * (drawn_height as f64 / 2.0);

    // Err for a point behind the camera, which is the test for that. Both are projected rather
    // than the height being worked out from the distance, so that a marker seen from almost
    // overhead, foreshortened to nearly nothing, has the small reach it looks like it has.
    let (Ok(on_screen), Ok(base)) = (
        camera.world_to_viewport(camera_global, (centre - cell_origin).as_vec3()),
        camera.world_to_viewport(camera_global, (position - cell_origin).as_vec3()),
    ) else {
        return None;
    };

    Some((on_screen, PICK_PIXELS.max(on_screen.distance(base))))
}

/// The marker nearest the cursor on screen, if the cursor is within its reach.
fn nearest_marker(
    markers: &PhotoMarkers,
    camera_position: DVec3,
    focal_pixels: Option<f32>,
    camera: &Camera,
    camera_global: &GlobalTransform,
    cell_origin: DVec3,
    cursor: Vec2,
) -> Option<usize> {
    let mut nearest: Option<(usize, f32)> = None;

    for (index, marker) in markers.markers.iter().enumerate() {
        let drawn_height = markers.drawn_height(marker, camera_position, focal_pixels);
        let Some((on_screen, reach)) =
            marker_on_screen(marker, drawn_height, camera, camera_global, cell_origin)
        else {
            continue;
        };

        let distance = on_screen.distance(cursor);
        if distance <= reach && nearest.is_none_or(|(_, best)| distance < best) {
            nearest = Some((index, distance));
        }
    }

    nearest.map(|(index, _)| index)
}

/// A ring round the selected marker, so that a click can be seen to have taken. Drawn in front of
/// the terrain, as the rail overlays are, and sized to the marker so it hugs whatever the size
/// slider is on. Nothing is drawn when nothing is selected.
fn ring_selection(
    mut gizmos: Gizmos<PhotoMarkerGizmos>,
    markers: Res<PhotoMarkers>,
    grids: Grids,
    camera: Query<MarkerCamera, With<OrbitalCameraController>>,
) {
    let Some(marker) = markers.selected() else {
        return;
    };
    let Ok(camera) = camera.single() else {
        return;
    };
    let Some(grid) = grids.parent_grid(camera.entity) else {
        return;
    };

    let cell_origin = grid.cell_to_float(camera.cell);
    let camera_position = grid.grid_position_double(camera.cell, camera.transform);
    let focal_pixels = match (camera.camera.logical_viewport_size(), camera.projection) {
        (Some(viewport), Projection::Perspective(perspective)) => {
            Some(viewport.y / (2.0 * (perspective.fov / 2.0).tan()))
        }
        _ => None,
    };

    let drawn_height = markers.drawn_height(marker, camera_position, focal_pixels);
    let position = marker.position();
    let up = Frame::at_unit(marker.unit).up;
    let centre = position + up * (drawn_height as f64 / 2.0);
    // Wide enough to sit clear of the plate, which is about three quarters as wide as it is tall.
    let radius = drawn_height * 0.7;
    let facing = (camera_position - centre).normalize().as_vec3();

    gizmos.circle(
        Isometry3d::new(
            (centre - cell_origin).as_vec3(),
            Quat::from_rotation_arc(Vec3::Z, facing),
        ),
        radius,
        SELECTION_COLOUR,
    );
}

#[cfg(test)]
mod tests;
