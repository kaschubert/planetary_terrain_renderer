//! The stations of the network named on the map, on F9: a label over each, placed as the
//! trains' and the sheet grid's labels are, and the stations themselves for whatever else
//! wants a stop id turned into a name, which the live trains' next stop does.
//!
//! The stations come from the CSV the download script writes beside the rail lines, see
//! shared/stations.rs, compiled in since it is small and never edited. A station is
//! anchored to the track: its position is dropped onto the nearest line, see
//! TrackSpline::nearest, and the label stands LABEL_LIFT above the rail there, so a station
//! in the City Rail Link's tunnel is labelled above the ground over it and one on a bridge
//! over the deck. A station more than STATION_REACH from every line, which none of
//! Auckland's is, has no anchor and no label. The anchors are recomputed whenever the
//! splines are, so an edit to a line moves its stations with it.
//!
//! Every station has a node of its own, placed each frame by projecting its anchor through
//! the camera: hidden off screen, over the horizon or beyond LABEL_REACH, and with the
//! nearest stations placed first, none lands on another, so from far enough out only the
//! stations that fit are named and the rest return as the camera comes in. Shown only
//! while the lines are, as everything on the network is: F4 hides them too.

use super::auckland_rail::AucklandRail;
use super::provenance::ascii;
use super::rail_editor::frame::{Frame, unit_under};
use super::shared::stations::Stations;
use super::sheet_grid::label_size;
use super::track_frames::{TrackSplines, refresh_track_splines};
use bevy::{math::DVec3, prelude::*, text::FontSize, ui::UiSystems};
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};
use std::collections::HashMap;

/// The file, compiled in: 45 stations and their platforms, a few kilobytes.
const STATIONS_CSV: &str = include_str!("auckland_rail/auckland_stations.csv");

/// How far a station may stand from the nearest line and still be anchored to it, level,
/// in metres. A station's GTFS position is on its platforms, a few tens of metres from the
/// service shape at most; a station further off than this is not on the network drawn.
pub const STATION_REACH: f64 = 500.0;

/// How far above the rail a station's label stands, in metres: over the roof of a carriage
/// standing there, so the two labels do not fight, and under the eye of a chase camera.
pub const LABEL_LIFT: f64 = 20.0;

/// How far from the camera a station is still named, in metres: the whole network from
/// over the city, and nothing from orbit, where 45 labels would be a smear.
pub const LABEL_REACH: f64 = 60_000.0;

/// The clear space between two labels, in pixels, as the sheet grid keeps.
const LABEL_GAP: f32 = 6.0;

/// The labels' colour: off-white, so a station reads as a place and not as a line.
const LABEL_COLOUR: Color = Color::srgb(0.92, 0.92, 0.88);

pub struct StationsPlugin;

impl Plugin for StationsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RailStations::load())
            .add_systems(Startup, spawn_station_labels)
            .add_systems(
                Update,
                (
                    toggle_stations,
                    // After the splines are through this frame's line, so the anchors are
                    // on the line as it is.
                    anchor_stations.after(refresh_track_splines),
                ),
            )
            .add_systems(
                PostUpdate,
                // After the transforms propagate and before the nodes are laid out, as the
                // trains' labels are placed, and for the same reasons.
                place_station_labels
                    .after(TransformSystems::Propagate)
                    .before(UiSystems::Prepare),
            );
    }
}

/// The stations, their anchors on the track and the switch that names them.
#[derive(Resource)]
pub struct RailStations {
    /// Toggled by F9. Shown only while the lines are: F4 hides everything on the network.
    pub shown: bool,
    pub stations: Stations,
    /// One per stop, in the file's order: where its label stands, see anchor_for, and None
    /// for a platform, for a station off every line, and for every stop until the splines
    /// are first through.
    anchors: Vec<Option<DVec3>>,
}

impl RailStations {
    fn load() -> Self {
        let stations =
            Stations::parse(STATIONS_CSV).expect("the compiled-in auckland_stations.csv parses");

        Self {
            shown: true,
            stations,
            anchors: Vec::new(),
        }
    }

    /// Whether the labels are drawn: shown by F9 and the network not hidden by F4.
    pub fn drawn(&self, rail: &AucklandRail) -> bool {
        self.shown && rail.visible
    }
}

/// Where a station's label stands: LABEL_LIFT above the rail at the point of the nearest
/// line to the station's position, the nearest by its level offset, see
/// TrackSpline::nearest, and None when no line comes within STATION_REACH.
pub fn anchor_for(position: DVec3, splines: &TrackSplines) -> Option<DVec3> {
    let (spline, distance, _) = splines
        .splines
        .iter()
        .flatten()
        .map(|spline| {
            let (distance, off) = spline.nearest(position);
            (spline, distance, off)
        })
        .filter(|&(_, _, off)| off <= STATION_REACH)
        .min_by(|a, b| a.2.total_cmp(&b.2))?;
    let frame = spline.frame_at(distance);
    let up = Frame::at_unit(unit_under(frame.position)).up;

    Some(frame.position + up * LABEL_LIFT)
}

/// A station's label, by the station's index into the stops.
#[derive(Component)]
struct StationLabel(usize);

fn toggle_stations(keys: Res<ButtonInput<KeyCode>>, mut stations: ResMut<RailStations>) {
    if keys.just_pressed(KeyCode::F9) {
        stations.shown = !stations.shown;
        info!(
            "stations: {}",
            if stations.shown { "shown" } else { "hidden" }
        );
    }
}

/// A node per station, in the style of the trains' labels, hidden until placed.
fn spawn_station_labels(mut commands: Commands, stations: Res<RailStations>) {
    for (index, station) in stations.stations.stops.iter().enumerate() {
        if !station.is_station() {
            continue;
        }
        commands.spawn((
            StationLabel(index),
            Text::new(ascii(station.short_name())),
            TextColor(LABEL_COLOUR),
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

/// Anchors every station to the track whenever the splines are rebuilt, which is on the
/// first frame and after every edit; see refresh_track_splines for why the change is
/// noticed here.
fn anchor_stations(splines: Res<TrackSplines>, mut stations: ResMut<RailStations>) {
    if !splines.is_changed() {
        return;
    }
    let before = stations.anchors.iter().flatten().count();
    stations.anchors = stations
        .stations
        .stops
        .iter()
        .map(|stop| {
            if !stop.is_station() {
                return None;
            }
            let position = TerrainShape::WGS84.position_unit_to_local(stop.unit(), 0.0);
            anchor_for(position, &splines)
        })
        .collect();

    let after = stations.anchors.iter().flatten().count();
    if after != before {
        info!(
            "stations: {after} of {} anchored to the lines",
            stations.stations.stations().count()
        );
    }
}

/// Places each station's label above its anchor on screen, the nearest stations first and
/// none on top of another, see the module doc.
fn place_station_labels(
    stations: Res<RailStations>,
    rail: Res<AucklandRail>,
    grids: Grids,
    camera: Query<
        (Entity, &Camera, &GlobalTransform, &Transform, &CellCoord),
        With<OrbitalCameraController>,
    >,
    mut labels: Query<(&StationLabel, &mut Node, &mut Visibility)>,
) {
    // The camera as the projection wants it, as the trains' labels have it.
    let view = camera
        .single()
        .ok()
        .and_then(|(entity, camera, global, transform, cell)| {
            let grid = grids.parent_grid(entity)?;
            let viewport = camera.logical_viewport_size()?;
            let cell_origin = grid.cell_to_float(cell);
            let camera_position = grid.grid_position_double(cell, transform);
            Some((camera, global, viewport, cell_origin, camera_position))
        });
    let Some((camera, global, viewport, cell_origin, camera_position)) =
        view.filter(|_| stations.drawn(&rail))
    else {
        for (_, _, mut visibility) in &mut labels {
            visibility.set_if_neq(Visibility::Hidden);
        }
        return;
    };

    // Every station on screen with where its label would go, nearest first.
    let mut candidates: Vec<(f64, usize, Vec2, Vec2)> = Vec::new();
    for (label, _, _) in &labels {
        let Some(anchor) = stations.anchors.get(label.0).copied().flatten() else {
            continue;
        };
        let distance = anchor.distance(camera_position);
        if distance > LABEL_REACH || (camera_position - anchor).dot(anchor) < 0.0 {
            continue;
        }
        let Some(at) = camera
            .world_to_viewport(global, (anchor - cell_origin).as_vec3())
            .ok()
            .filter(|at| at.cmpge(Vec2::ZERO).all() && at.cmple(viewport).all())
        else {
            continue;
        };
        let size = label_size(stations.stations.stops[label.0].short_name());
        candidates.push((distance, label.0, at, size));
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));

    // Centre and size of every label placed so far, as the sheet grid keeps them.
    let mut placed: Vec<(Vec2, Vec2)> = Vec::new();
    let mut wanted: HashMap<usize, (Vec2, Vec2)> = HashMap::new();
    for (_, index, at, size) in candidates {
        let centre = Vec2::new(at.x, at.y - size.y / 2.0);
        let overlaps = |&(other_centre, other): &(Vec2, Vec2)| {
            let reach = (size + other) / 2.0 + LABEL_GAP;
            (centre - other_centre).abs().cmplt(reach).all()
        };
        if placed.iter().any(overlaps) {
            continue;
        }
        placed.push((centre, size));
        wanted.insert(index, (at, size));
    }

    for (label, mut node, mut visibility) in &mut labels {
        match wanted.get(&label.0) {
            Some(&(at, size)) => {
                node.left = Val::Px(at.x - size.x / 2.0);
                node.top = Val::Px(at.y - size.y);
                visibility.set_if_neq(Visibility::Visible);
            }
            None => {
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
    }
}

#[cfg(test)]
mod tests;
