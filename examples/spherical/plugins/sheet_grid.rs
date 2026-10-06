//! The Topo50 sheet grid drawn over the terrain, coloured by what has been downloaded.
//!
//! The download scripts take sheet names, and the provenance records which sheets each
//! level reached, but nothing connected a place on the ground to either. This does: fly
//! over the edge of the good imagery, and the border and its label say which sheet the
//! coarse side is, which is the name the script wants.
//!
//! A sheet is a fixed 24 by 36 km rectangle on the national grid, so its corners are
//! arithmetic on its name. That arithmetic and the projection out of the grid were checked
//! against the georeferencing of every whole sheet file on disk, 964 of them, and against
//! PROJ at the corners.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::{
    color::palettes::{basic, css},
    input::mouse::{MouseScrollUnit, MouseWheel},
    math::{DVec2, DVec3},
    prelude::*,
    text::FontSize,
    ui::{Checked, UiSystems},
    ui_widgets::{
        Checkbox, SetSliderValue, Slider, SliderRange, SliderThumb, SliderValue, SliderValueChange,
        TrackClick, ValueChange, observe,
    },
};
use bevy_terrain::{
    math::{Coordinate, unit_position},
    prelude::*,
};
use big_space::prelude::{CellCoord, Grids};
use std::f64::consts::PI;

pub struct SheetGridPlugin;

/// The grid's own gizmo settings, so that lifting it clear of the terrain does not lift the
/// other debug gizmos with it.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct SheetGizmos;

impl Plugin for SheetGridPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SheetGrid>()
            .init_gizmo_group::<SheetGizmos>()
            .add_systems(Startup, draw_grid_over_terrain)
            .add_systems(
                Startup,
                (
                    spawn_sheet_labels,
                    // Into the row the provenance panel leaves for it.
                    spawn_height_controls.after(super::provenance::spawn_provenance_table),
                ),
            )
            .add_systems(
                Update,
                (
                    toggle_sheet_grid,
                    collect_sheets,
                    wheel_adjusts_height,
                    yield_wheel,
                    (style_height_slider, sync_checkboxes, show_height),
                ),
            )
            .add_systems(
                PostUpdate,
                // After the floating origin has settled on this frame's cell, and before
                // the labels are laid out, so they land where the camera is now rather
                // than where it was a frame ago.
                draw_sheet_grid
                    .after(TransformSystems::Propagate)
                    .before(UiSystems::Prepare),
            );
    }
}

// The national grid: NZTM2000, EPSG:2193. Transverse Mercator on GRS80.
const SEMI_MAJOR: f64 = 6378137.0;
const INVERSE_FLATTENING: f64 = 298.257222101;
const SCALE: f64 = 0.9996;
const CENTRAL_MERIDIAN: f64 = 173.0;
const FALSE_EASTING: f64 = 1_600_000.0;
const FALSE_NORTHING: f64 = 10_000_000.0;

/// The Topo50 grid over it. Two letters count rows southward from the top, two digits
/// count columns eastward, and both skip nothing except I and O in the letters.
const ROW_LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
const SHEET_WIDTH: f64 = 24_000.0;
const SHEET_HEIGHT: f64 = 36_000.0;
const GRID_EASTING: f64 = 988_000.0;
const GRID_NORTHING: f64 = 6_810_000.0;

/// A sheet's extent on the grid, in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SheetExtent {
    pub east: f64,
    pub north: f64,
}

impl SheetExtent {
    /// The south west corner of the sheet with this name.
    ///
    /// Errors rather than guessing on anything off the pattern. That matters for the
    /// Chatham Islands, whose sheets are named the same way but sit on another projection
    /// entirely: their I is not a row letter, so they fail here instead of landing in the
    /// Tasman Sea.
    pub fn parse(name: &str) -> Result<Self, String> {
        let bytes = name.as_bytes();

        if bytes.len() != 4 {
            return Err(format!(
                "{name}: a sheet name is two letters and two digits"
            ));
        }

        let row = |letter: u8| {
            ROW_LETTERS
                .iter()
                .position(|&candidate| candidate == letter)
                .ok_or_else(|| format!("{name}: {} is not a row letter", letter as char))
        };
        let row = 24 * row(bytes[0])? + row(bytes[1])?;

        let column = name[2..]
            .parse::<usize>()
            .map_err(|_| format!("{name}: the last two characters are not digits"))?;

        Ok(Self {
            east: GRID_EASTING + SHEET_WIDTH * column as f64,
            north: GRID_NORTHING - SHEET_HEIGHT * (row + 1) as f64,
        })
    }
}

/// Grid coordinates to longitude and latitude, in degrees.
///
/// Kruger's series to the sixth power of the third flattening, which is exact to
/// nanometres this close to the central meridian. The constants are the standard ones.
pub fn grid_to_lonlat(east: f64, north: f64) -> (f64, f64) {
    let f = 1.0 / INVERSE_FLATTENING;
    let n = f / (2.0 - f);
    let (n2, n3, n4, n5, n6) = (n * n, n * n * n, n * n * n * n, n.powi(5), n.powi(6));

    let radius = SEMI_MAJOR / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0 + n6 / 256.0);

    let beta = [
        n / 2.0 - 2.0 * n2 / 3.0 + 37.0 * n3 / 96.0 - n4 / 360.0 - 81.0 * n5 / 512.0
            + 96199.0 * n6 / 604800.0,
        n2 / 48.0 + n3 / 15.0 - 437.0 * n4 / 1440.0 + 46.0 * n5 / 105.0
            - 1118711.0 * n6 / 3870720.0,
        17.0 * n3 / 480.0 - 37.0 * n4 / 840.0 - 209.0 * n5 / 4480.0 + 5569.0 * n6 / 90720.0,
        4397.0 * n4 / 161280.0 - 11.0 * n5 / 504.0 - 830251.0 * n6 / 7257600.0,
        4583.0 * n5 / 161280.0 - 108847.0 * n6 / 3991680.0,
        20648693.0 * n6 / 638668800.0,
    ];

    let delta = [
        2.0 * n - 2.0 * n2 / 3.0 - 2.0 * n3 + 116.0 * n4 / 45.0 + 26.0 * n5 / 45.0
            - 2854.0 * n6 / 675.0,
        7.0 * n2 / 3.0 - 8.0 * n3 / 5.0 - 227.0 * n4 / 45.0
            + 2704.0 * n5 / 315.0
            + 2323.0 * n6 / 945.0,
        56.0 * n3 / 15.0 - 136.0 * n4 / 35.0 - 1262.0 * n5 / 105.0 + 73814.0 * n6 / 2835.0,
        4279.0 * n4 / 630.0 - 332.0 * n5 / 35.0 - 399572.0 * n6 / 14175.0,
        4174.0 * n5 / 315.0 - 144838.0 * n6 / 6237.0,
        601676.0 * n6 / 22275.0,
    ];

    let xi = (north - FALSE_NORTHING) / (SCALE * radius);
    let eta = (east - FALSE_EASTING) / (SCALE * radius);

    let (mut xi_prime, mut eta_prime) = (xi, eta);

    for (j, coefficient) in beta.iter().enumerate() {
        let j = 2.0 * (j + 1) as f64;
        xi_prime -= coefficient * (j * xi).sin() * (j * eta).cosh();
        eta_prime -= coefficient * (j * xi).cos() * (j * eta).sinh();
    }

    let chi = (xi_prime.sin() / eta_prime.cosh()).asin();

    let latitude = delta
        .iter()
        .enumerate()
        .fold(chi, |latitude, (j, coefficient)| {
            latitude + coefficient * (2.0 * (j + 1) as f64 * chi).sin()
        });
    let longitude = eta_prime.sinh().atan2(xi_prime.cos());

    (
        CENTRAL_MERIDIAN + longitude * 180.0 / PI,
        latitude * 180.0 / PI,
    )
}

/// How many pieces each edge is cut into. Straight across, a 36 km edge sinks 25 m below
/// the surface at its middle; in eight it sinks under half a metre.
const EDGE_SEGMENTS: usize = 8;

/// How far each sheet's outline is drawn inside its true edge. Neighbours would otherwise
/// draw the same line twice in two colours, and the gap makes each sheet read as its own.
const INSET: f64 = 120.0;

/// A sheet ready to draw.
struct SheetOutline {
    name: String,
    /// On the unit sphere, for culling and for placing the label.
    centre: DVec3,
    /// The inset border on the unit sphere, closed, EDGE_SEGMENTS points per edge.
    ring: Vec<DVec3>,
    /// The finest imagery downloaded for it, as a ground sample distance in metres.
    finest: Option<f64>,
}

#[derive(Resource)]
pub struct SheetGrid {
    sheets: Vec<SheetOutline>,
    visible: bool,
    /// How far above the ellipsoid the grid floats, in metres. One value for the whole
    /// grid, so it reads as a single surface however the view is angled. It cannot sit on
    /// the ground, since the heights live on the gpu and there is no way to ask for the
    /// one under a point, so it is set by hand instead: the slider in the F2 panel, or the
    /// mouse wheel when that is switched on there.
    pub height: f32,
    pub wheel_adjusts_height: bool,
}

impl Default for SheetGrid {
    fn default() -> Self {
        Self {
            sheets: default(),
            visible: true,
            height: 100.0,
            wheel_adjusts_height: false,
        }
    }
}

/// The slider's reach. The highest point in the national elevation data is 2920 m, so the
/// upper end clears every hill in the country.
const HEIGHT_RANGE: (f32, f32) = (0.0, 3000.0);

/// Metres per notch of the wheel.
const WHEEL_STEP: f32 = 25.0;

/// The depth bias that draws a gizmo group in front of the terrain: the sheet grid here, and
/// the rail lines, the editor's discs and the track frames, which take it from here so that a
/// retune reaches them all.
///
/// Not the full -1, though. The gizmo shader maps a line's depth to (z/w)^(1 + bias), and
/// at -1 that is the near plane for every vertex: the whole grid on one depth, with depth
/// writes on, so crossings, joints and the paired edges of neighbouring cells fought each
/// other, and at long range the rounding put vertices a hair past the plane and clipped
/// them. At -0.9 the map is (z/w)^0.1, still ordered and still distinct across the grid,
/// while anything short of geometry touching the lens is beaten.
pub const IN_FRONT_OF_TERRAIN: f32 = -0.9;

/// The lines are drawn in front of the terrain, the way the labels already are. They float
/// at one height, and terrain higher than that would otherwise hide them: a ridge between
/// the camera and a cell took its outline while its label, being screen space, stayed.
/// Cells on the far side of the planet are still dropped by the horizon test.
fn draw_grid_over_terrain(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<SheetGizmos>().0.depth_bias = IN_FRONT_OF_TERRAIN;
}

fn toggle_sheet_grid(input: Res<ButtonInput<KeyCode>>, mut grid: ResMut<SheetGrid>) {
    if input.just_pressed(KeyCode::F3) {
        grid.visible = !grid.visible;
    }
}

/// Every sheet any provenance names, with the finest imagery that reached it.
///
/// Rebuilt when a provenance file arrives. Not on the Assets resource changing, since the
/// asset server touches that every frame whether anything happened or not.
fn collect_sheets(
    mut arrivals: MessageReader<AssetEvent<TerrainProvenance>>,
    provenance: Res<Assets<TerrainProvenance>>,
    mut grid: ResMut<SheetGrid>,
) {
    if arrivals.is_empty() {
        return;
    }
    arrivals.clear();

    let mut finest = bevy::platform::collections::HashMap::<String, Option<f64>>::new();

    for (_, provenance) in provenance.iter() {
        for (label, records) in &provenance.sources {
            let imagery = matches!(label, AttachmentLabel::Custom(name) if name == "albedo");

            for manifest in records.iter().filter_map(|record| record.manifest.as_ref()) {
                // "0.075m" -> 0.075. Only imagery counts; the height carries no colour.
                let resolution = imagery
                    .then(|| {
                        manifest
                            .resolution
                            .trim_end_matches('m')
                            .parse::<f64>()
                            .ok()
                    })
                    .flatten();

                for sheet in &manifest.sheets {
                    let entry = finest.entry(sheet.clone()).or_default();

                    *entry = match (*entry, resolution) {
                        (Some(known), Some(new)) => Some(known.min(new)),
                        (known, new) => known.or(new),
                    };
                }
            }
        }
    }

    let mut sheets = finest
        .into_iter()
        .filter_map(|(name, finest)| {
            let extent = SheetExtent::parse(&name)
                .inspect_err(|error| warn!("skipping sheet {error}"))
                .ok()?;

            Some(SheetOutline {
                centre: unit_at(
                    extent.east + SHEET_WIDTH / 2.0,
                    extent.north + SHEET_HEIGHT / 2.0,
                ),
                ring: ring(extent),
                name,
                finest,
            })
        })
        .collect::<Vec<_>>();

    sheets.sort_by(|a, b| a.name.cmp(&b.name));

    info!("sheet grid: {} sheets", sheets.len());

    grid.sheets = sheets;
}

fn unit_at(east: f64, north: f64) -> DVec3 {
    let (longitude, latitude) = grid_to_lonlat(east, north);
    unit_position(longitude, latitude)
}

/// The inset border, walked in grid space and projected point by point. A sheet edge is a
/// straight line on the grid, which is neither a parallel nor a great circle, so the
/// points in between have to come from the grid too.
fn ring(extent: SheetExtent) -> Vec<DVec3> {
    let corners = [
        DVec2::new(extent.east + INSET, extent.north + INSET),
        DVec2::new(extent.east + SHEET_WIDTH - INSET, extent.north + INSET),
        DVec2::new(
            extent.east + SHEET_WIDTH - INSET,
            extent.north + SHEET_HEIGHT - INSET,
        ),
        DVec2::new(extent.east + INSET, extent.north + SHEET_HEIGHT - INSET),
    ];

    let mut ring = Vec::with_capacity(4 * EDGE_SEGMENTS + 1);

    for edge in 0..4 {
        let (from, to) = (corners[edge], corners[(edge + 1) % 4]);

        for step in 0..EDGE_SEGMENTS {
            let point = from.lerp(to, step as f64 / EDGE_SEGMENTS as f64);
            ring.push(unit_at(point.x, point.y));
        }
    }

    ring.push(ring[0]);
    ring
}

fn coverage_colour(finest: Option<f64>) -> Color {
    match finest {
        Some(metres) if metres <= 0.1 => basic::LIME.into(),
        Some(metres) if metres <= 0.3 => basic::YELLOW.into(),
        Some(metres) if metres <= 1.0 => css::ORANGE.into(),
        Some(_) => Color::srgb(0.45, 0.55, 0.7),
        None => Color::srgb(0.4, 0.4, 0.4),
    }
}

/// Beyond this the grid is not drawn at all. The national terrain streams in at this
/// distance too, so the grid appears with the ground it describes and not before.
const MAX_DISTANCE: f64 = 2_000_000.0;

/// From further than this an edge in two pieces is indistinguishable from eight.
const COARSE_DISTANCE: f64 = 150_000.0;

/// How many labels there are to hand out. A cell gets one only when the label fits inside
/// it, so they thin out as the cells shrink and the grid carries on alone; among those
/// that fit, the larger cell wins where two labels would overlap.
const LABEL_POOL: usize = 96;

/// A label's footprint on screen, estimated rather than measured: the bundled font is
/// monospaced at six tenths of an em, and the estimate only has to be right to within the
/// gap. Two labels closer than this in both axes overlap, and the smaller sheet's is dropped.
const LABEL_CHAR_WIDTH: f32 = 7.8;
const LABEL_HEIGHT: f32 = 20.0;
const LABEL_GAP: f32 = 6.0;

#[derive(Component)]
struct SheetLabel;

fn spawn_sheet_labels(mut commands: Commands) {
    for _ in 0..LABEL_POOL {
        commands.spawn((
            SheetLabel,
            Text::new(""),
            TextFont {
                font_size: FontSize::Px(13.0),
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                padding: UiRect::axes(Val::Px(4.0), Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            Visibility::Hidden,
        ));
    }
}

#[allow(clippy::type_complexity)]
fn draw_sheet_grid(
    mut gizmos: Gizmos<SheetGizmos>,
    grid: Res<SheetGrid>,
    grids: Grids,
    camera: Query<
        (Entity, &Camera, &GlobalTransform, &Transform, &CellCoord),
        With<OrbitalCameraController>,
    >,
    mut labels: Query<(&mut Node, &mut Text, &mut TextColor, &mut Visibility), With<SheetLabel>>,
) {
    if !grid.visible {
        // The lines are drawn afresh each frame and just stop, but the labels are nodes
        // that stay wherever they were last put, so they have to be put away.
        for (_, _, _, mut visibility) in &mut labels {
            visibility.set_if_neq(Visibility::Hidden);
        }
        return;
    }

    let Ok((camera_entity, camera, camera_global, camera_transform, camera_cell)) = camera.single()
    else {
        return;
    };
    let Some(big_grid) = grids.parent_grid(camera_entity) else {
        return;
    };

    // Positions on the spheroid are absolute and large. Gizmos and the projection want
    // render space, which is absolute space shifted so the camera's cell is at the origin.
    // Subtract in f64 before narrowing, or the metre and below is gone.
    let cell_origin = big_grid.cell_to_float(camera_cell);
    let camera_position = big_grid.grid_position_double(camera_cell, camera_transform);
    let camera_up = camera_position.normalize();
    let camera_radius = camera_position.length();

    // A sheet below the horizon is on the far side of the planet.
    let horizon = TerrainShape::WGS84.scale().x / camera_radius;

    let height = grid.height;

    // None for a frame or two at startup, before the window has told the camera its size.
    let viewport = camera.logical_viewport_size();
    let mut candidates = Vec::new();

    for sheet in &grid.sheets {
        if sheet.centre.dot(camera_up) < horizon {
            continue;
        }

        let centre = Coordinate::from_unit_position(sheet.centre, true)
            .local_position(TerrainShape::WGS84, 0.0);
        let distance = centre.distance(camera_position);

        if distance > MAX_DISTANCE {
            continue;
        }

        let stride = if distance > COARSE_DISTANCE {
            EDGE_SEGMENTS / 2
        } else {
            1
        };
        let colour = coverage_colour(sheet.finest);

        let to_render = |unit: DVec3| {
            let position = Coordinate::from_unit_position(unit, true)
                .local_position(TerrainShape::WGS84, height);
            (position - cell_origin).as_vec3()
        };

        gizmos.linestrip(
            sheet.ring.iter().step_by(stride).copied().map(to_render),
            colour,
        );

        // Where the cell lands on screen, from its four corners. A label is offered only
        // when it fits inside the cell with a margin: one wider than its cell would sit over
        // the neighbours, and from far enough out that is every label at once. The grid
        // carries on regardless, and the labels return as the cells grow.
        let project = |unit: DVec3| {
            camera
                .world_to_viewport(camera_global, to_render(unit))
                .ok()
        };

        let Some(viewport) = viewport else {
            continue;
        };
        let Some(at) = project(sheet.centre) else {
            continue;
        };
        if !(at.cmpge(Vec2::ZERO).all() && at.cmple(viewport).all()) {
            // Off the screen: placed out of sight it would still hold a label that an
            // on-screen cell could have used.
            continue;
        }
        let [Some(a), Some(b), Some(c), Some(d)] =
            [0, 1, 2, 3].map(|corner| project(sheet.ring[corner * EDGE_SEGMENTS]))
        else {
            continue;
        };
        let cell = a.max(b).max(c).max(d) - a.min(b).min(c).min(d);

        let caption = match sheet.finest {
            Some(metres) => format!("{} {metres}m", sheet.name),
            None => sheet.name.clone(),
        };
        let size = Vec2::new(LABEL_CHAR_WIDTH * caption.len() as f32 + 8.0, LABEL_HEIGHT);

        if cell.cmplt(size + 2.0 * LABEL_GAP).any() {
            continue;
        }

        candidates.push((cell.x * cell.y, at, size, caption, colour));
    }

    // The biggest cells on screen get labels first, and none lands on top of another.
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));

    // Centre and size of every label placed so far.
    let mut placed: Vec<(Vec2, Vec2)> = Vec::new();
    let mut labels = labels.iter_mut();

    for (_, at, size, caption, colour) in candidates {
        let overlaps = |&(centre, other): &(Vec2, Vec2)| {
            let reach = (size + other) / 2.0 + LABEL_GAP;
            (at - centre).abs().cmplt(reach).all()
        };

        if placed.iter().any(overlaps) {
            continue;
        }
        let Some((mut node, mut text, mut text_colour, mut visibility)) = labels.next() else {
            break;
        };

        placed.push((at, size));

        node.left = Val::Px(at.x - size.x / 2.0);
        node.top = Val::Px(at.y - size.y / 2.0);
        text.0 = caption;
        text_colour.0 = colour;
        *visibility = Visibility::Visible;
    }

    for (_, _, _, mut visibility) in labels {
        *visibility = Visibility::Hidden;
    }
}

/// The row in the F2 panel the height controls go into. The panel spawns it, this fills it.
#[derive(Component)]
pub struct SheetGridControls;

#[derive(Component)]
struct HeightSlider;

#[derive(Component)]
struct HeightThumb;

#[derive(Component)]
struct HeightReadout;

/// Which switch on the grid a checkbox drives. The resource is the one source of truth,
/// so a checkbox and its hotkey can never disagree: both write the field, and the box
/// redraws from it.
#[derive(Component, Clone, Copy)]
struct GridCheckbox(GridSetting);

#[derive(Clone, Copy)]
enum GridSetting {
    Visible,
    WheelAdjustsHeight,
}

impl GridSetting {
    fn get(self, grid: &SheetGrid) -> bool {
        match self {
            Self::Visible => grid.visible,
            Self::WheelAdjustsHeight => grid.wheel_adjusts_height,
        }
    }

    fn set(self, grid: &mut SheetGrid, value: bool) {
        match self {
            Self::Visible => grid.visible = value,
            Self::WheelAdjustsHeight => grid.wheel_adjusts_height = value,
        }
    }
}

/// The filled square inside a checkbox, shown when it is on.
#[derive(Component)]
struct Checkmark;

const SLIDER_WIDTH: f32 = 160.0;
const THUMB_SIZE: f32 = 12.0;

fn spawn_height_controls(
    mut commands: Commands,
    row: Single<Entity, With<SheetGridControls>>,
    grid: Res<SheetGrid>,
) {
    let font = TextFont {
        font_size: FontSize::Px(12.0),
        ..default()
    };

    commands.entity(*row).with_children(|row| {
        spawn_checkbox(row, GridSetting::Visible, "show grid", &font);

        row.spawn((Text::new("sheet grid height"), font.clone()));

        // The headless slider handles the pointer and keeps the value; the track and the
        // thumb are ours to draw, and the thumb is moved to match by style_height_slider.
        row.spawn((
            HeightSlider,
            Slider {
                track_click: TrackClick::Snap,
                ..default()
            },
            SliderValue(grid.height),
            SliderRange::new(HEIGHT_RANGE.0, HEIGHT_RANGE.1),
            Node {
                width: Val::Px(SLIDER_WIDTH),
                height: Val::Px(THUMB_SIZE),
                justify_content: JustifyContent::Center,
                flex_direction: FlexDirection::Column,
                ..default()
            },
            observe(
                |change: On<ValueChange<f32>>,
                 mut grid: ResMut<SheetGrid>,
                 mut commands: Commands| {
                    grid.height = change.value;
                    commands
                        .entity(change.source)
                        .insert(SliderValue(change.value));
                },
            ),
            children![
                (
                    Node {
                        height: Val::Px(4.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.35, 0.35, 0.35)),
                ),
                (
                    // The thumb's travel is the track less the thumb, so its centre lands
                    // exactly under the pointer at both ends.
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        right: Val::Px(THUMB_SIZE),
                        top: Val::Px(0.0),
                        bottom: Val::Px(0.0),
                        ..default()
                    },
                    children![(
                        HeightThumb,
                        SliderThumb,
                        Node {
                            position_type: PositionType::Absolute,
                            width: Val::Px(THUMB_SIZE),
                            height: Val::Px(THUMB_SIZE),
                            left: Val::Percent(0.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.8, 0.8, 0.8)),
                    )],
                ),
            ],
        ));

        row.spawn((
            HeightReadout,
            Text::new(format!("{} m", grid.height as i32)),
            font.clone(),
            Node {
                min_width: Val::Px(52.0),
                ..default()
            },
        ));

        spawn_checkbox(
            row,
            GridSetting::WheelAdjustsHeight,
            "wheel sets height",
            &font,
        );
    });
}

fn spawn_checkbox(
    row: &mut RelatedSpawnerCommands<ChildOf>,
    setting: GridSetting,
    caption: &str,
    font: &TextFont,
) {
    row.spawn((
        GridCheckbox(setting),
        Checkbox,
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(6.0),
            margin: UiRect::horizontal(Val::Px(6.0)),
            ..default()
        },
        observe(on_checkbox_change),
        children![
            (
                Node {
                    width: Val::Px(14.0),
                    height: Val::Px(14.0),
                    border: UiRect::all(Val::Px(1.0)),
                    padding: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BorderColor::all(Color::srgb(0.7, 0.7, 0.7)),
                children![(
                    Checkmark,
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.8, 0.8, 0.8)),
                    Visibility::Hidden,
                )],
            ),
            (Text::new(caption), font.clone()),
        ],
    ));
}

/// A click writes the setting. The box itself catches up in sync_checkboxes, the same way
/// it does when the hotkey changes the setting instead.
fn on_checkbox_change(
    change: On<ValueChange<bool>>,
    checkboxes: Query<&GridCheckbox>,
    mut grid: ResMut<SheetGrid>,
) {
    if let Ok(checkbox) = checkboxes.get(change.source) {
        checkbox.0.set(&mut grid, change.value);
    }
}

/// Keeps the thumb under the value. The slider moves the value, not the thumb.
#[allow(clippy::type_complexity)]
fn style_height_slider(
    slider: Query<(&SliderValue, &SliderRange), (With<HeightSlider>, Changed<SliderValue>)>,
    mut thumb: Single<&mut Node, With<HeightThumb>>,
) {
    let Ok((value, range)) = slider.single() else {
        return;
    };

    thumb.left = Val::Percent(range.thumb_position(value.0) * 100.0);
}

/// Redraws every checkbox from the resource whenever it changes, and keeps the widget's
/// own Checked marker in step, since that is what it reads to decide what the next click
/// means.
fn sync_checkboxes(
    grid: Res<SheetGrid>,
    checkboxes: Query<(Entity, &GridCheckbox, Has<Checked>)>,
    children: Query<&Children>,
    mut marks: Query<&mut Visibility, With<Checkmark>>,
    mut commands: Commands,
) {
    if !grid.is_changed() {
        return;
    }

    for (entity, checkbox, checked) in &checkboxes {
        let on = checkbox.0.get(&grid);

        match (on, checked) {
            (true, false) => {
                commands.entity(entity).insert(Checked);
            }
            (false, true) => {
                commands.entity(entity).remove::<Checked>();
            }
            _ => {}
        }

        for descendant in children.iter_descendants(entity) {
            if let Ok(mut mark) = marks.get_mut(descendant) {
                *mark = if on {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
        }
    }
}

fn show_height(grid: Res<SheetGrid>, mut readout: Single<&mut Text, With<HeightReadout>>) {
    if grid.is_changed() {
        readout.0 = format!("{} m", grid.height as i32);
    }
}

/// Moves the slider rather than the height directly, so the two can never disagree: the
/// slider's own observer above is the one place the height is set.
/// Hands the wheel to the grid's height while the box is ticked and back to the camera's
/// zoom when it is not, since the orbital camera zooms on the wheel otherwise and the two
/// acting at once would be a surprise. The grid resource changes rarely, so the camera is
/// written only then.
fn yield_wheel(grid: Res<SheetGrid>, mut cameras: Query<&mut OrbitalCameraController>) {
    if !grid.is_changed() {
        return;
    }
    for mut camera in &mut cameras {
        camera.wheel_zooms = !grid.wheel_adjusts_height;
    }
}

fn wheel_adjusts_height(
    mut wheel: MessageReader<MouseWheel>,
    grid: Res<SheetGrid>,
    slider: Single<Entity, With<HeightSlider>>,
    mut commands: Commands,
) {
    if !grid.wheel_adjusts_height {
        wheel.clear();
        return;
    }

    let notches: f32 = wheel
        .read()
        .map(|scroll| match scroll.unit {
            MouseScrollUnit::Line => scroll.y,
            // A touchpad reports pixels; a line is about twenty of them.
            MouseScrollUnit::Pixel => scroll.y / 20.0,
        })
        .sum();

    if notches != 0.0 {
        commands.trigger(SetSliderValue {
            entity: *slider,
            change: SliderValueChange::Relative(notches * WHEEL_STEP),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_names_place_on_the_grid() {
        // South west corners read off the files' own georeferencing.
        let cases = [
            ("AS21", 1_492_000.0, 6_198_000.0),
            ("BA32", 1_756_000.0, 5_910_000.0),
            ("BQ31", 1_732_000.0, 5_406_000.0),
            ("CK09", 1_204_000.0, 4_722_000.0),
            ("BH46", 2_092_000.0, 5_658_000.0),
        ];

        for (name, east, north) in cases {
            assert_eq!(
                SheetExtent::parse(name).unwrap(),
                SheetExtent { east, north },
                "{name}"
            );
        }
    }

    #[test]
    fn the_row_letters_skip_i_and_o() {
        // BH and BJ are neighbours: one sheet apart, not two.
        let bh = SheetExtent::parse("BH46").unwrap();
        let bj = SheetExtent::parse("BJ46").unwrap();
        assert_eq!(bh.north - bj.north, SHEET_HEIGHT);

        // Which is exactly what keeps the Chatham Islands off this grid.
        assert!(SheetExtent::parse("CI01").is_err());
        assert!(SheetExtent::parse("BO12").is_err());
        assert!(SheetExtent::parse("BA4").is_err());
        assert!(SheetExtent::parse("BA3X").is_err());
    }

    #[test]
    fn grid_corners_project_where_proj_puts_them() {
        // EPSG:2193 to 4326 through PROJ, at sheet corners across the whole country.
        let cases = [
            (1492000.0, 6234000.0, 171.830140051, -34.029103619),
            (1516000.0, 6198000.0, 172.086582291, -34.355931670),
            (1756000.0, 5946000.0, 174.744530967, -36.618774366),
            (1780000.0, 5910000.0, 175.021281437, -36.938873789),
            (1732000.0, 5442000.0, 174.573377575, -41.162595138),
            (1756000.0, 5406000.0, 174.868550940, -41.482439879),
            (1204000.0, 4758000.0, 167.769950461, -47.211955195),
            (1228000.0, 4722000.0, 168.055524057, -47.548702668),
            (2092000.0, 5694000.0, 178.661186600, -38.765489694),
            (2116000.0, 5658000.0, 178.963032072, -39.074471659),
            (1084000.0, 5082000.0, 166.539121804, -44.232185585),
            (1108000.0, 5046000.0, 166.803804190, -44.570975028),
        ];

        for (east, north, longitude, latitude) in cases {
            let (lon, lat) = grid_to_lonlat(east, north);
            // A billionth of a degree is a tenth of a millimetre.
            assert!(
                (lon - longitude).abs() < 1e-9,
                "{east} {north}: lon {lon} vs {longitude}"
            );
            assert!(
                (lat - latitude).abs() < 1e-9,
                "{east} {north}: lat {lat} vs {latitude}"
            );
        }
    }
}
