//! Auckland's passenger rail lines drawn over the city, one colour per line, on the ground.
//!
//! The geometry comes from Auckland Transport's GTFS feed. preprocess/download_auckland_rail.sh
//! extracts it into auckland_rail/auckland_rail.csv beside this file. The file is compiled in so the
//! example runs without it, but the copy on disk is read in preference when there is one,
//! so a save from inside the example is picked up on the next run without a rebuild. Each
//! line is the shape its trips most often run: the full line end to end, which for S-C
//! includes the city loop. The three together cover every passenger track on the network.
//!
//! The heights come from the height tiles on disk through the library's sampler, read on a
//! background task at startup because the network crosses a few hundred megabytes of tiles.
//! Until it lands the lines draw from the terrain heights the file cached at its last save,
//! or on the ellipsoid if it has never been saved, as the committed file has not. Once it
//! has, the sampler stays as a resource for the points the editor adds or moves.
//!
//! A stretch between two anchors, a tunnel or a bridge, draws dashed so that it reads as
//! one, and while the editor is on a stretch of ground too steep for rail draws in a warning
//! colour, since that is where a tunnel or a bridge is wanted; strokes.rs decides which is
//! which. The point model, the file format and the height resolution are in rail_network.rs,
//! and the editing in rail_editor.rs, whose panel is where a save is asked for; this is the
//! plugin that loads, samples, draws and writes the file.

use super::rail_editor::RailEditor;
use super::shared::rail_network::{PointMode, RailNetwork, RailPoint, utc_now};
use super::sheet_grid::IN_FRONT_OF_TERRAIN;
use crate::STREAMED_TERRAINS;
use bevy::{
    math::DVec3,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};
use std::{
    cmp::Reverse,
    collections::HashMap,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};
use strokes::split_runs;

mod strokes;

// The one warning colour, for the editor's panel as well as the lines.
pub use strokes::WARNING_COLOUR;

pub struct AucklandRailPlugin;

/// The lines' own gizmo settings, so drawing them in front of the terrain and wider than
/// the default leaves the other debug gizmos alone.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct RailGizmos;

/// The between stretches' own settings: as the lines, but dashed. A line style belongs to a
/// group and not to a linestrip, which is why there are two.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct RailSpanGizmos;

impl Plugin for AucklandRailPlugin {
    fn build(&self, app: &mut App) {
        app.init_gizmo_group::<RailGizmos>()
            .init_gizmo_group::<RailSpanGizmos>()
            .insert_resource(AucklandRail::load())
            .add_systems(Startup, (configure_rail_gizmos, start_sampling))
            .add_systems(
                Update,
                (
                    toggle_rail_lines,
                    collect_samples.run_if(resource_exists::<SamplingTask>),
                    // Last, so that samples landing this frame are drawn this frame.
                    resolve_rail_heights.run_if(rail_dirty),
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                // After the floating origin has settled on this frame's cell, so the lines
                // land where the camera is now rather than where it was a frame ago.
                draw_rail_lines.after(TransformSystems::Propagate),
            );
    }
}

/// The compiled-in copy of the file, for when there is none on disk.
const RAIL_CSV: &str = include_str!("auckland_rail/auckland_rail.csv");

/// The file itself, read in preference to the copy at startup and written by a save.
/// Through the manifest directory, as the asset root is, so it is the same file whichever
/// directory the example is run from.
pub const RAIL_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/examples/spherical/plugins/auckland_rail/auckland_rail.csv"
);

/// Where the terrain configs are. STREAMED_TERRAINS names them relative to the asset root,
/// which is assets/ beside the manifest when the example runs through cargo.
const ASSET_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets");

/// Beyond this distance from the network the lines are not drawn. The Auckland terrain
/// streams in at 150 km, so the lines arrive with the city and not before.
const MAX_DISTANCE: f64 = 200_000.0;

/// In pixels. The default 2 reads as a hairline against the imagery.
const LINE_WIDTH: f32 = 3.0;

/// A dash and the gap after it, in line widths on screen, so 12 px and 6 px at LINE_WIDTH. A
/// dash four widths long reads as a piece of line rather than a dot, and a gap of two widths
/// stays open after the anti-aliased ends either side of it have taken half a pixel each.
/// The pattern starts afresh at every point, so from far enough away that a segment is
/// shorter than a dash the stretch reads as solid, which is all a tunnel at that distance
/// deserves.
const DASH_SCALE: f32 = 4.0;
const GAP_SCALE: f32 = 2.0;

/// How far the ground may differ from the height the file cached before the point is
/// reported as moved. The 1 m DEM is good to a few tenths of a metre, and re-sampling a
/// re-processed copy of the same data moves a point by less than that, so a metre separates
/// a changed dataset from noise. The editor's panel quotes it.
pub const MOVED_LIMIT: f64 = 1.0;

/// AT's colours for its lines, route_color in the feed's routes.txt, so they match the
/// network map. A line the file names that is not here draws white.
pub fn line_colour(name: &str) -> Color {
    match name {
        "E-W" => Color::srgb_u8(0x97, 0xC9, 0x3D),
        "S-C" => Color::srgb_u8(0xD5, 0x29, 0x23),
        "O-W" => Color::srgb_u8(0x00, 0xAE, 0xEF),
        _ => Color::WHITE,
    }
}

/// A point whose ground is not where the file said it was. Named by its place and not by its
/// index: the list is made when the sampling lands, and every insert and delete the editor
/// makes on the line after that shifts the indices behind it, while the place is the point's
/// own until it is dragged, and then it is not the point the sampling saw.
pub struct MovedGround {
    /// Index into the network's lines, which the editor never reorders.
    pub line: usize,
    pub longitude: f64,
    pub latitude: f64,
    /// This run's sample less the cached terrain height, in metres.
    pub delta: f64,
}

impl MovedGround {
    /// Where the point is on its line now, by the bits of its place, or None once the editor
    /// has moved or removed it.
    pub fn index_in(&self, network: &RailNetwork) -> Option<usize> {
        let place = (self.longitude.to_bits(), self.latitude.to_bits());

        network
            .lines
            .get(self.line)?
            .points
            .iter()
            .position(|point| point.place_bits() == place)
    }
}

/// The network and what this run knows about it.
#[derive(Resource)]
pub struct AucklandRail {
    pub network: RailNetwork,
    pub visible: bool,
    /// Set by whatever changes a point or its sample, cleared once the positions have been
    /// recomputed from the points. An edit sets it; nothing else need be done.
    pub dirty: bool,
    /// Points whose ground moved by more than MOVED_LIMIT since the file was saved, filled
    /// when the sampler lands. What "the dataset changed" looks like in practice: nothing
    /// breaks, and this says where to look.
    pub moved_ground: Vec<MovedGround>,
    /// The mean of every resolved position, for the distance and horizon tests.
    centre: DVec3,
    /// Set when there is a file at RAIL_PATH that would not read or parse, so the network
    /// is the compiled-in copy standing in for it. A save would write that copy over the
    /// file, which may be weeks of edits behind one typo in a mode, so save refuses until
    /// the file is fixed or moved aside; the error says so.
    file_unreadable: bool,
}

impl AucklandRail {
    fn load() -> Self {
        Self::load_from(Path::new(RAIL_PATH))
    }

    /// The network in the file at this path, else the compiled-in copy, resolved.
    fn load_from(path: &Path) -> Self {
        let (mut network, file_unreadable) = load_network(path);
        network.resolve();
        let centre = network.centre();

        Self {
            network,
            visible: true,
            dirty: false,
            moved_ground: Vec::new(),
            centre,
            file_unreadable,
        }
    }

    /// Writes the network back to RAIL_PATH in the second format, stamped with the time. The
    /// editor panel's save button and Ctrl+S call it, and turn the result into a toast.
    pub fn save(&self) -> Result<(), String> {
        self.save_to(Path::new(RAIL_PATH))
    }

    /// Whether a save would be allowed to write, for the panel to dim its button by. False
    /// while the compiled-in copy stands in for a file that would not read, see
    /// [`Self::file_unreadable`].
    pub fn can_save(&self) -> bool {
        !self.file_unreadable
    }

    /// Refuses while the compiled-in copy is standing in for a file that would not read,
    /// since the write would replace that file, and the way out is to fix the file.
    fn save_to(&self, path: &Path) -> Result<(), String> {
        if self.file_unreadable {
            return Err(format!(
                "{} did not read at startup and the lines are the compiled-in copy; saving would \
                 write that copy over the file, so fix or move the file and start again",
                path.display()
            ));
        }

        let csv = self.network.to_csv(&utc_now());

        write_atomically(path, &csv).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// Takes the sampler's answer, each sample with the longitude and latitude it was read
    /// at, and notes where the ground has moved since the file was saved. The samples are
    /// matched to the points by place and not by position in the list, because the editor
    /// can add, remove and move points in the seconds the task takes. A point the task did
    /// not see is sampled now, which the sampler's cache makes a lookup for the few there
    /// can be, and its cached terrain is dropped: a moved point's is another place's, and
    /// no witness.
    fn apply_samples(
        &mut self,
        sampler: &mut TerrainHeightSampler,
        samples: &[((f64, f64), Option<f64>)],
    ) {
        // By the bits: the places are the points' own values copied out, and a point the
        // editor has not touched still has them exactly.
        let by_place: HashMap<(u64, u64), Option<f64>> = samples
            .iter()
            .map(|&((longitude, latitude), sample)| {
                ((longitude.to_bits(), latitude.to_bits()), sample)
            })
            .collect();

        let (mut found, mut missing, mut late) = (0, 0, 0);
        self.moved_ground.clear();

        for (line_index, line) in self.network.lines.iter_mut().enumerate() {
            for point in &mut line.points {
                let sample = match by_place.get(&point.place_bits()) {
                    Some(&sample) => sample,
                    None => {
                        late += 1;
                        point.terrain = None;
                        sampler.height_at_unit(point.unit())
                    }
                };
                point.sampled = sample;

                match sample {
                    Some(_) => found += 1,
                    None => missing += 1,
                }
                if let (Some(sampled), Some(cached)) = (sample, point.terrain) {
                    let delta = sampled - cached;
                    if delta.abs() > MOVED_LIMIT {
                        self.moved_ground.push(MovedGround {
                            line: line_index,
                            longitude: point.longitude,
                            latitude: point.latitude,
                            delta,
                        });
                    }
                }
            }
        }

        self.dirty = true;

        let edited = if late > 0 {
            format!(", {late} of them edited while it ran")
        } else {
            String::new()
        };
        info!(
            "auckland rail: sampled the terrain under {found} points, {missing} without \
             data{edited}; the ground moved more than {MOVED_LIMIT} m under {} since the file \
             was saved{}",
            self.moved_ground.len(),
            self.largest_move()
        );
    }

    /// The moved point that moved furthest, for the log, as ", the most under S-C 312 (4.3 m)".
    fn largest_move(&self) -> String {
        self.moved_ground
            .iter()
            .max_by(|a, b| a.delta.abs().total_cmp(&b.delta.abs()))
            .map_or_else(String::new, |moved| {
                format!(
                    ", the most under {} {} ({:+.1} m)",
                    self.network.lines[moved.line].name,
                    // By place, as the panel finds it; the list was just made from the
                    // points, so it is there.
                    moved.index_in(&self.network).unwrap_or_default(),
                    moved.delta
                )
            })
    }
}

/// The file at the path when there is one, else the compiled-in copy, saying which. The
/// flag is true when there is a file and it is not what was loaded, see
/// AucklandRail::file_unreadable.
fn load_network(path: &Path) -> (RailNetwork, bool) {
    let file_unreadable = match fs::read_to_string(path) {
        Ok(csv) => match RailNetwork::parse(&csv) {
            Ok(network) => {
                info!(
                    "auckland rail: {} lines, {} points from {}",
                    network.lines.len(),
                    network.point_count(),
                    path.display()
                );
                return (network, false);
            }
            Err(error) => {
                error!(
                    "auckland rail: {}: {error}; using the compiled-in copy, and refusing to \
                     save over the file until it is fixed",
                    path.display()
                );
                true
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            info!(
                "auckland rail: no file at {}; using the compiled-in copy",
                path.display()
            );
            false
        }
        Err(error) => {
            error!(
                "auckland rail: {}: {error}; using the compiled-in copy, and refusing to save \
                 over the file until it reads",
                path.display()
            );
            true
        }
    };

    // The copy is compiled in, so a copy that does not parse is a bug in whatever wrote it,
    // not a condition to recover from.
    let network = RailNetwork::parse(RAIL_CSV).expect("the compiled-in auckland_rail.csv parses");
    info!(
        "auckland rail: {} lines, {} points from the compiled-in copy",
        network.lines.len(),
        network.point_count()
    );
    (network, file_unreadable)
}

/// Writes through a temporary file in the same directory, renamed over the target once it
/// is complete and on disk, so a crash mid-write leaves the old file whole rather than half
/// of a new one. The rename is atomic because the two are on the same filesystem, which the
/// same directory guarantees.
fn write_atomically(path: &Path, contents: &str) -> io::Result<()> {
    let temporary = path.with_extension("csv.tmp");

    let written = File::create(&temporary).and_then(|mut file| {
        file.write_all(contents.as_bytes())?;
        file.sync_all()
    });

    written
        .and_then(|()| fs::rename(&temporary, path))
        .inspect_err(|_| {
            // Best effort: the error being reported is the one that matters.
            let _ = fs::remove_file(&temporary);
        })
}

/// What the startup sampling produces: the sampler, and the terrain under every point that
/// was there when it began, each with the longitude and latitude it was read at, since the
/// editor may have changed the points by the time it lands.
type Sampled = Result<(TerrainHeightSampler, Vec<((f64, f64), Option<f64>)>), String>;

/// The terrain under every point, being read on the compute pool, together with the sampler
/// that read it. Removed once it lands. Visible because collect_samples, which the editor
/// orders against, names it.
#[derive(Resource)]
pub struct SamplingTask(Task<Sampled>);

/// The sampler that read the heights at startup, kept once that is done so a point the
/// editor adds or moves can be put on the ground straight away. Its cache still holds the
/// tiles under the network, so one more point is a lookup and not a read. Absent until the
/// startup task lands, which the editor allows for.
#[derive(Resource)]
pub struct RailSampler(pub TerrainHeightSampler);

impl RailSampler {
    /// Reads the terrain under the point's longitude and latitude into its sample, as the
    /// startup pass did for every point. None where no terrain has data. The terrain the
    /// file cached goes with it: the point may have been moved, and a cache that may be
    /// another place's is worse than none, as a height to fall back on and as the witness
    /// the next run compares its sample against.
    pub fn resample(&mut self, point: &mut RailPoint) {
        point.sampled = self.0.height_at_unit(point.unit());
        point.terrain = None;
    }
}

/// Starts reading the terrain under every point. The sampler decodes a megabyte per tile and
/// the network crosses a few hundred of them, so this is seconds of disk work that does not
/// belong on the main thread.
fn start_sampling(mut commands: Commands, rail: Res<AucklandRail>) {
    let points: Vec<(f64, f64)> = rail
        .network
        .points()
        .map(|point| (point.longitude, point.latitude))
        .collect();
    let configs = sampler_configs();

    let task = AsyncComputeTaskPool::get().spawn(async move {
        let mut sampler = TerrainHeightSampler::load(&configs)?;
        if sampler.is_empty() {
            warn!("auckland rail: no terrain on disk to sample; the lines keep the heights the file has");
        }
        let heights = sampler.heights(&points);
        Ok((sampler, points.into_iter().zip(heights).collect()))
    });

    commands.insert_resource(SamplingTask(task));
}

/// The terrain configs the sampler reads, finest first, which is the reverse of the order
/// the terrains draw in: the city terrains, then the national one, then the globe. A terrain
/// that has not been downloaded is skipped by the sampler.
fn sampler_configs() -> Vec<PathBuf> {
    let mut terrains: Vec<_> = STREAMED_TERRAINS.iter().collect();
    terrains.sort_by_key(|terrain| Reverse(terrain.order));

    terrains
        .iter()
        .map(|terrain| Path::new(ASSET_ROOT).join(terrain.path))
        .collect()
}

/// Takes the startup sampling when it lands. The editor orders its drag before this, see
/// RailEditorPlugin.
pub fn collect_samples(
    mut commands: Commands,
    mut task: ResMut<SamplingTask>,
    mut rail: ResMut<AucklandRail>,
) {
    let Some(result) = block_on(poll_once(&mut task.0)) else {
        return;
    };
    commands.remove_resource::<SamplingTask>();

    match result {
        Ok((mut sampler, samples)) => {
            rail.apply_samples(&mut sampler, &samples);
            commands.insert_resource(RailSampler(sampler));
        }
        Err(error) => error!("auckland rail: the height sampler failed: {error}"),
    }
}

fn rail_dirty(rail: Res<AucklandRail>) -> bool {
    rail.dirty
}

/// Recomputes the positions from the points, once per change rather than every frame. An
/// editing system that wants its change drawn the same frame orders itself before this.
pub fn resolve_rail_heights(mut rail: ResMut<AucklandRail>) {
    rail.network.resolve();
    rail.centre = rail.network.centre();
    rail.dirty = false;
}

fn configure_rail_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<RailGizmos>();
    config.depth_bias = IN_FRONT_OF_TERRAIN;
    config.line.width = LINE_WIDTH;

    let (config, _) = store.config_mut::<RailSpanGizmos>();
    config.depth_bias = IN_FRONT_OF_TERRAIN;
    config.line.width = LINE_WIDTH;

    config.line.style = GizmoLineStyle::Dashed {
        gap_scale: GAP_SCALE,
        line_scale: DASH_SCALE,
    };
}

fn toggle_rail_lines(input: Res<ButtonInput<KeyCode>>, mut rail: ResMut<AucklandRail>) {
    if input.just_pressed(KeyCode::F4) {
        rail.visible = !rail.visible;
    }
}

/// Each line in runs: solid track in the line's colour, the spans dashed, and while the
/// editor is on the steep track in the warning colour. The editor is optional so that the
/// lines draw without the editor plugin, and then as if it were off.
fn draw_rail_lines(
    mut gizmos: Gizmos<RailGizmos>,
    mut span_gizmos: Gizmos<RailSpanGizmos>,
    rail: Res<AucklandRail>,
    editor: Option<Res<RailEditor>>,
    grids: Grids,
    camera: Query<(Entity, &Transform, &CellCoord), With<OrbitalCameraController>>,
) {
    if !rail.visible {
        return;
    }
    let editing = editor.is_some_and(|editor| editor.editing);

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

    for line in &rail.network.lines {
        let colour = line_colour(&line.name);
        let render = |positions: &[DVec3]| -> Vec<Vec3> {
            positions
                .iter()
                .map(|&position| (position - cell_origin).as_vec3())
                .collect()
        };

        // On the frame after an insert or a delete the positions are a point behind the
        // points, and the modes would be matched to the wrong positions; the line draws plain
        // for that one frame, as the discs skip it. A change of mode alone keeps the count,
        // and draws in its new style the frame before the positions move to match.
        if line.points.len() != line.positions.len() {
            gizmos.linestrip(render(&line.positions), colour);
            continue;
        }

        let modes: Vec<PointMode> = line.points.iter().map(|point| point.mode).collect();
        for run in split_runs(&modes, &line.segment_grades(), editing) {
            let points = render(&line.positions[run.first..=run.last]);
            if run.stroke.dashed() {
                span_gizmos.linestrip(points, colour);
            } else {
                gizmos.linestrip(points, run.stroke.colour(colour));
            }
        }
    }
}

#[cfg(test)]
mod tests;
