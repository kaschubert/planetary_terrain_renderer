//! Auckland's passenger rail lines drawn over the city, one colour per line.
//!
//! The geometry comes from Auckland Transport's GTFS feed. preprocess/download_auckland_rail.sh
//! extracts it into auckland_rail.csv next to this file, and that is compiled in, so the
//! example needs no download of its own. Each line is the shape its trips most often run:
//! the full line end to end, which for S-C includes the city loop. The three together
//! cover every passenger track on the network.
//!
//! The lines float at one height and are drawn in front of the terrain, as the sheet grid
//! is. The terrain's heights live on the gpu, so there is no putting a point on the
//! ground, and the City Rail Link is a tunnel anyway.

use super::unit_position;
use bevy::{math::DVec3, prelude::*};
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};

pub struct AucklandRailPlugin;

/// The lines' own gizmo settings, so drawing them in front of the terrain and wider than
/// the default leaves the other debug gizmos alone.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct RailGizmos;

impl Plugin for AucklandRailPlugin {
    fn build(&self, app: &mut App) {
        app.init_gizmo_group::<RailGizmos>()
            .insert_resource(RailNetwork::parse(RAIL_CSV))
            .add_systems(Startup, configure_rail_gizmos)
            .add_systems(Update, toggle_rail_lines)
            .add_systems(
                PostUpdate,
                // After the floating origin has settled on this frame's cell, so the lines
                // land where the camera is now rather than where it was a frame ago.
                draw_rail_lines.after(TransformSystems::Propagate),
            );
    }
}

/// Comment lines, a header, then line,latitude,longitude rows grouped by line and in order
/// along it. The comments name the feed version and the shape each line came from.
const RAIL_CSV: &str = include_str!("auckland_rail.csv");

/// How far above the ellipsoid the lines are drawn, in metres. The network runs from sea
/// level at Waitematā to about 70 m at Pukekohe. The lines are drawn in front of the terrain
/// whatever its height, so this only sets where they appear to sit when the view is tilted.
const TRACK_HEIGHT: f64 = 30.0;

/// Beyond this distance from the network the lines are not drawn. The Auckland terrain
/// streams in at 150 km, so the lines arrive with the city and not before.
const MAX_DISTANCE: f64 = 200_000.0;

/// In pixels. The default 2 reads as a hairline against the imagery.
const LINE_WIDTH: f32 = 3.0;

/// AT's colours for its lines, route_color in the feed's routes.txt, so they match the
/// network map. A line the file names that is not here draws white.
fn line_colour(name: &str) -> Color {
    match name {
        "E-W" => Color::srgb_u8(0x97, 0xC9, 0x3D),
        "S-C" => Color::srgb_u8(0xD5, 0x29, 0x23),
        "O-W" => Color::srgb_u8(0x00, 0xAE, 0xEF),
        _ => Color::WHITE,
    }
}

struct RailLine {
    name: String,
    colour: Color,
    /// Along the line, on the spheroid at TRACK_HEIGHT, in absolute metres. Projected once
    /// here rather than every frame: a frame only has to shift them into the camera's cell.
    points: Vec<DVec3>,
}

#[derive(Resource)]
struct RailNetwork {
    lines: Vec<RailLine>,
    /// The mean of every point, for the distance and horizon tests. It sits a few tens of
    /// metres inside the spheroid, which neither test minds.
    centre: DVec3,
    visible: bool,
}

impl RailNetwork {
    /// Panics on a malformed row: the file is compiled in, so a bad one is a bug in the
    /// script that wrote it, not a condition to recover from.
    fn parse(csv: &str) -> Self {
        let mut lines: Vec<RailLine> = Vec::new();

        for row in csv.lines() {
            if row.is_empty() || row.starts_with('#') || row.starts_with("line,") {
                continue;
            }

            let mut fields = row.split(',');
            let (Some(name), Some(latitude), Some(longitude)) =
                (fields.next(), fields.next(), fields.next())
            else {
                panic!("auckland_rail.csv: expected line,latitude,longitude, got {row:?}");
            };
            let degrees = |field: &str| {
                field
                    .parse::<f64>()
                    .unwrap_or_else(|_| panic!("auckland_rail.csv: {field:?} is not a number"))
            };

            let position = TerrainShape::WGS84.position_unit_to_local(
                unit_position(degrees(longitude), degrees(latitude)),
                TRACK_HEIGHT,
            );

            match lines.last_mut() {
                Some(line) if line.name == name => line.points.push(position),
                _ => lines.push(RailLine {
                    name: name.to_string(),
                    colour: line_colour(name),
                    points: vec![position],
                }),
            }
        }

        let points = lines.iter().flat_map(|line| &line.points);
        let count = points.clone().count();
        let centre = points.sum::<DVec3>() / count as f64;

        info!("auckland rail: {} lines, {count} points", lines.len());

        Self {
            lines,
            centre,
            visible: true,
        }
    }
}

/// See draw_grid_over_terrain in sheet_grid.rs for why -0.9 and not -1.
fn configure_rail_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<RailGizmos>();
    config.depth_bias = -0.9;
    config.line.width = LINE_WIDTH;
}

fn toggle_rail_lines(input: Res<ButtonInput<KeyCode>>, mut rail: ResMut<RailNetwork>) {
    if input.just_pressed(KeyCode::F4) {
        rail.visible = !rail.visible;
    }
}

fn draw_rail_lines(
    mut gizmos: Gizmos<RailGizmos>,
    rail: Res<RailNetwork>,
    grids: Grids,
    camera: Query<(Entity, &Transform, &CellCoord), With<OrbitalCameraController>>,
) {
    if !rail.visible {
        return;
    }

    let Ok((camera, camera_transform, camera_cell)) = camera.single() else {
        return;
    };
    let Some(grid) = grids.parent_grid(camera) else {
        return;
    };

    // Positions on the spheroid are absolute and large. Gizmos want render space, which is
    // absolute space shifted so the camera's cell is at the origin. Subtract in f64 before
    // narrowing, or the metre and below is gone.
    let cell_origin = grid.cell_to_float(camera_cell);
    let camera_position = grid.grid_position_double(camera_cell, camera_transform);

    // Out of reach, or over the horizon. The lines beat the terrain's depth, so from a
    // camera below the plane tangent to the spheroid at the network they would show
    // through the planet. The test is on the centre, so the far end of a line can still
    // smear along the horizon from a low camera far away; closer in it is exact enough.
    if camera_position.distance(rail.centre) > MAX_DISTANCE
        || (camera_position - rail.centre).dot(rail.centre) < 0.0
    {
        return;
    }

    for line in &rail.lines {
        gizmos.linestrip(
            line.points
                .iter()
                .map(|&point| (point - cell_origin).as_vec3()),
            line.colour,
        );
    }
}
