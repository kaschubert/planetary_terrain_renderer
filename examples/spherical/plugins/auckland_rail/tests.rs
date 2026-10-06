//! The sampler order and the drape run against the terrains on disk, and skip with a note
//! when they are not here. The file handling runs in a scratch directory of its own, so
//! nothing here touches the committed file.

use super::*;
use crate::plugins::shared::rail_network::EDITED_BY;
use std::{env, process};

#[test]
fn the_sampler_reads_the_finest_terrain_first() {
    let configs = sampler_configs();
    let names: Vec<&str> = configs
        .iter()
        .map(|path| {
            path.parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .unwrap()
        })
        .collect();

    // The city terrains in either order, then the national one, then the globe.
    assert_eq!(names.len(), STREAMED_TERRAINS.len());
    assert_eq!(&names[names.len() - 2..], ["nz", "earth"]);
    assert!(names[..names.len() - 2].contains(&"auckland"));
    assert!(configs[0].starts_with(ASSET_ROOT));
}

/// Needs every terrain downloaded; says so and passes when one is not, as the library's
/// own data tests do. Every one, because the count below relies on the globe answering
/// where the city and national terrains are NoData, over the harbour crossings, and the
/// sampler itself skips a missing terrain without a word.
#[test]
fn the_terrain_on_disk_drapes_the_network() {
    let configs = sampler_configs();
    if let Some(missing) = configs.iter().find(|path| !path.exists()) {
        eprintln!("skipped: {} is not on this machine", missing.display());
        return;
    }

    let network = RailNetwork::parse(RAIL_CSV).unwrap();
    let points: Vec<(f64, f64)> = network
        .points()
        .map(|point| (point.longitude, point.latitude))
        .collect();

    let mut sampler = TerrainHeightSampler::load(&configs).unwrap();
    let heights = sampler.heights(&points);

    // With the globe among the terrains there is data under every point, and the
    // network runs from the harbour's edge to about 70 m at Pukekohe, with the highest
    // ground over the City Rail Link near 80 m.
    let found: Vec<f64> = heights.iter().flatten().copied().collect();
    assert_eq!(found.len(), points.len());
    let (lowest, highest) = found.iter().fold((f64::MAX, f64::MIN), |(low, high), &h| {
        (low.min(h), high.max(h))
    });
    eprintln!(
        "{} points sampled, {lowest:.1} m to {highest:.1} m",
        found.len()
    );
    assert!(lowest > -30.0 && highest < 200.0, "{lowest} to {highest}");
}

/// A directory of the test's own under the system's temporary directory, named for the
/// test and the process so the tests can run in parallel, and removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = env::temp_dir().join(format!("auckland_rail_{name}_{}", process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// Where the file would be, whether or not it is there yet.
    fn rail_csv(&self) -> PathBuf {
        self.0.join("auckland_rail.csv")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The file a write goes through on its way to the target.
fn temporary_beside(path: &Path) -> PathBuf {
    path.with_extension("csv.tmp")
}

/// One point in the first format, which reads as a ground point.
const ONE_POINT: &str = "line,latitude,longitude\nE-W,-36.99388,174.87739\n";

/// The same point with a typo in its mode, the hand edit that a save must not punish.
const MALFORMED: &str =
    "line,latitude,longitude,mode,height,terrain\nE-W,-36.99388,174.87739,fixd,12.0,\n";

#[test]
fn writes_through_a_temporary_and_leaves_none_behind() {
    let scratch = Scratch::new("writes");
    let path = scratch.rail_csv();

    write_atomically(&path, "first\n").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "first\n");

    // Over the file that is there, as every save after the first is.
    write_atomically(&path, "second\n").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "second\n");
    assert!(!temporary_beside(&path).exists());
}

#[test]
fn a_failed_write_removes_its_temporary() {
    let scratch = Scratch::new("fails");
    // A directory where the file should be: the temporary writes, and the rename over a
    // directory fails, which is the failure after the write that the cleanup is for.
    let path = scratch.rail_csv();
    fs::create_dir(&path).unwrap();

    assert!(write_atomically(&path, "anything\n").is_err());
    assert!(path.is_dir());
    assert!(!temporary_beside(&path).exists());
}

#[test]
fn loads_the_file_when_it_reads() {
    let scratch = Scratch::new("reads");
    fs::write(scratch.rail_csv(), ONE_POINT).unwrap();

    let rail = AucklandRail::load_from(&scratch.rail_csv());
    assert_eq!(rail.network.point_count(), 1);
    assert!(!rail.file_unreadable);
}

#[test]
fn loads_the_compiled_in_copy_when_there_is_no_file() {
    let scratch = Scratch::new("absent");
    let compiled_in = RailNetwork::parse(RAIL_CSV).unwrap();

    let rail = AucklandRail::load_from(&scratch.rail_csv());
    assert_eq!(rail.network.point_count(), compiled_in.point_count());
    assert!(!rail.file_unreadable);
}

#[test]
fn a_file_that_does_not_parse_is_stood_in_for_and_not_saved_over() {
    let scratch = Scratch::new("malformed");
    let path = scratch.rail_csv();
    fs::write(&path, MALFORMED).unwrap();
    let compiled_in = RailNetwork::parse(RAIL_CSV).unwrap();

    let rail = AucklandRail::load_from(&path);
    assert_eq!(rail.network.point_count(), compiled_in.point_count());
    assert!(rail.file_unreadable);

    let refused = rail.save_to(&path).unwrap_err();
    assert!(refused.contains("fix or move the file"), "{refused}");
    assert_eq!(fs::read_to_string(&path).unwrap(), MALFORMED);
    assert!(!temporary_beside(&path).exists());
}

/// Three ground points in the second format, with the terrain the file cached under them.
const THREE_POINTS: &str = "line,latitude,longitude,mode,height,terrain\n\
    E-W,-36.99388,174.87739,ground,0.0,20.0\n\
    E-W,-36.99300,174.87800,ground,0.0,21.0\n\
    E-W,-36.99200,174.87900,ground,0.0,22.0\n";

#[test]
fn samples_are_matched_by_place_so_an_edit_during_the_task_loses_nothing() {
    let scratch = Scratch::new("matched");
    fs::write(scratch.rail_csv(), THREE_POINTS).unwrap();
    let mut rail = AucklandRail::load_from(&scratch.rail_csv());

    // What the task read: the three points as they were, the ground under the second 3 m
    // higher than the file had it, and no data under the third.
    let samples: Vec<((f64, f64), Option<f64>)> = rail
        .network
        .points()
        .zip([Some(20.2), Some(24.0), None])
        .map(|(point, sample)| ((point.longitude, point.latitude), sample))
        .collect();

    // Meanwhile the first point was deleted, the third dragged a little east with the
    // file's terrain still on it, and a point added after it: the count and the order are
    // both off, and one cache describes a place the point has left.
    let points = &mut rail.network.lines[0].points;
    points.remove(0);
    points[1].longitude += 0.0001;
    points.push(RailPoint::ground(174.88, -36.991));

    // A sampler with no terrain on it answers None everywhere, which tells a point sampled
    // now from one that kept its sample from the task.
    let mut sampler = TerrainHeightSampler::load(std::iter::empty::<&Path>()).unwrap();
    rail.apply_samples(&mut sampler, &samples);

    let points = &rail.network.lines[0].points;
    assert_eq!(points[0].sampled, Some(24.0));
    assert_eq!(points[0].terrain, Some(21.0));
    assert_eq!(points[1].sampled, None);
    assert_eq!(points[1].terrain, None);
    assert_eq!(points[2].sampled, None);
    assert_eq!(points[2].terrain, None);
    assert!(rail.dirty);

    // The ground moved under the matched point alone, by what the sample said.
    assert_eq!(rail.moved_ground.len(), 1);
    let moved = &rail.moved_ground[0];
    assert_eq!((moved.line, moved.index_in(&rail.network)), (0, Some(0)));
    assert!((moved.delta - 3.0).abs() < 1e-9, "{}", moved.delta);

    // The entry names the point by place, so a point added before it on the line does not
    // shift it onto its neighbour, and once the point itself is moved the entry names nothing.
    rail.network.lines[0]
        .points
        .insert(0, RailPoint::ground(174.87, -36.995));
    assert_eq!(moved.index_in(&rail.network), Some(1));
    rail.network.lines[0].points[1].longitude += 0.0001;
    assert_eq!(moved.index_in(&rail.network), None);
}

#[test]
fn saves_over_a_file_that_read() {
    let scratch = Scratch::new("saves");
    let path = scratch.rail_csv();
    fs::write(&path, ONE_POINT).unwrap();

    AucklandRail::load_from(&path).save_to(&path).unwrap();

    let saved = fs::read_to_string(&path).unwrap();
    assert!(
        saved.lines().any(|line| line.starts_with(EDITED_BY)),
        "{saved}"
    );
    assert_eq!(RailNetwork::parse(&saved).unwrap().point_count(), 1);
    assert!(!temporary_beside(&path).exists());
}
