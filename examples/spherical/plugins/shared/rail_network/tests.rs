use super::*;

/// The file as committed, so the format the example compiles in is the one the tests read.
const COMMITTED: &str = include_str!("../../auckland_rail/auckland_rail.csv");

const V1: &str = "\
# Auckland passenger rail lines, from Auckland Transport's GTFS feed
# Written by preprocess/download_auckland_rail.sh on 2026-10-03
line,latitude,longitude
E-W,-36.99388,174.87739
E-W,-36.99404,174.87725
S-C,-37.20246,174.90939
";

const V2: &str = "\
# Auckland passenger rail lines, from Auckland Transport's GTFS feed
# Edited by hand in the spherical example, last saved 2026-10-12 14:03 UTC
line,latitude,longitude,mode,height,terrain
E-W,-36.99388,174.87739,ground,0.0,21.4
S-C,-36.85090,174.76550,fixed,-12.0,38.9
S-C,-36.85340,174.76310,between,,41.2
S-C,-36.85400,174.76300,ground,1.5,
";

/// A point on the equator, where chords between longitudes are in proportion to the
/// longitudes, so the interpolation can be checked by arithmetic.
fn point(longitude: f64, mode: PointMode, height: Option<f64>) -> RailPoint {
    RailPoint {
        longitude,
        latitude: 0.0,
        mode,
        height,
        terrain: None,
        sampled: None,
    }
}

fn line(points: Vec<RailPoint>) -> RailLine {
    RailLine {
        name: "test".to_string(),
        points,
        positions: Vec::new(),
    }
}

fn assert_heights(line: &RailLine, expected: &[f64]) {
    let heights = line.resolved_heights();
    assert_eq!(heights.len(), expected.len());
    for (index, (height, expected)) in heights.iter().zip(expected).enumerate() {
        assert!(
            (height - expected).abs() < 1e-6,
            "point {index}: {height} vs {expected}"
        );
    }
}

#[test]
fn reads_the_first_format_as_ground_points() {
    let network = RailNetwork::parse(V1).unwrap();

    assert_eq!(network.comments.len(), 2);
    assert_eq!(network.lines.len(), 2);
    assert_eq!(network.lines[0].name, "E-W");
    assert_eq!(network.lines[0].points.len(), 2);
    assert_eq!(network.lines[1].name, "S-C");
    assert_eq!(network.point_count(), 3);

    for point in network.points() {
        assert_eq!(point.mode, PointMode::Ground);
        assert_eq!(point.height, Some(0.0));
        assert_eq!(point.terrain, None);
        assert_eq!(point.sampled, None);
    }
    assert_eq!(network.lines[0].points[1].latitude, -36.99404);
    assert_eq!(network.lines[0].points[1].longitude, 174.87725);
}

#[test]
fn reads_the_second_format_with_blanks_as_none() {
    let network = RailNetwork::parse(V2).unwrap();

    // The edited-by line is not a comment to keep: a save writes its own.
    assert_eq!(network.comments.len(), 1);
    assert!(network.comments[0].starts_with("# Auckland"));

    let points: Vec<&RailPoint> = network.points().collect();
    assert_eq!(points.len(), 4);

    assert_eq!(points[0].mode, PointMode::Ground);
    assert_eq!(points[0].height, Some(0.0));
    assert_eq!(points[0].terrain, Some(21.4));

    assert_eq!(points[1].mode, PointMode::Fixed);
    assert_eq!(points[1].height, Some(-12.0));
    assert_eq!(points[1].terrain, Some(38.9));

    assert_eq!(points[2].mode, PointMode::Between);
    assert_eq!(points[2].height, None);
    assert_eq!(points[2].terrain, Some(41.2));

    assert_eq!(points[3].mode, PointMode::Ground);
    assert_eq!(points[3].height, Some(1.5));
    assert_eq!(points[3].terrain, None);
}

#[test]
fn the_committed_file_reads() {
    let network = RailNetwork::parse(COMMITTED).unwrap();

    let names: Vec<&str> = network
        .lines
        .iter()
        .map(|line| line.name.as_str())
        .collect();
    assert_eq!(names, ["E-W", "O-W", "S-C"]);
    assert!(network.point_count() > 1000, "{}", network.point_count());
    assert!(network.comments.len() >= 3);
}

#[test]
fn writes_what_it_read() {
    let mut network = RailNetwork::parse(V2).unwrap();
    // A fresh sample replaces the cached terrain height in the file; a point the sampler
    // had no answer for keeps what the file had.
    network.lines[0].points[0].sampled = Some(22.75);

    let csv = network.to_csv("2026-10-13 09:00 UTC");
    let rows: Vec<&str> = csv.lines().collect();

    assert_eq!(
        rows[0],
        "# Auckland passenger rail lines, from Auckland Transport's GTFS feed"
    );
    assert_eq!(
        rows[1],
        "# Edited by hand in the spherical example, last saved 2026-10-13 09:00 UTC"
    );
    assert_eq!(rows[2], HEADER_V2);
    assert_eq!(rows[3], "E-W,-36.9938800,174.8773900,ground,0.00,22.75");
    assert_eq!(rows[4], "S-C,-36.8509000,174.7655000,fixed,-12.00,38.90");
    assert_eq!(rows[5], "S-C,-36.8534000,174.7631000,between,,41.20");
    assert_eq!(rows[6], "S-C,-36.8540000,174.7630000,ground,1.50,");
    assert_eq!(rows.len(), 7);

    let again = RailNetwork::parse(&csv).unwrap();
    assert_eq!(again.comments, network.comments);
    assert_eq!(again.lines.len(), network.lines.len());

    for (before, after) in network.points().zip(again.points()) {
        assert_eq!(after.longitude, before.longitude);
        assert_eq!(after.latitude, before.latitude);
        assert_eq!(after.mode, before.mode);
        assert_eq!(after.height, before.height);
        assert_eq!(after.terrain, before.sampled.or(before.terrain));
        assert_eq!(after.sampled, None);
    }

    // Saving twice stamps one edited-by line, not two.
    let twice = again.to_csv("2026-10-14 09:00 UTC");
    assert_eq!(twice.matches(EDITED_BY).count(), 1);

    // A point placed with the gizmo is saved to the centimetre, which is seven decimals.
    let mut placed = RailNetwork::parse(V1).unwrap();
    placed.lines[0].points[0].latitude = -36.993_881_234_5;
    placed.lines[0].points[0].longitude = 174.877_394_321_9;
    let saved = placed.to_csv("2026-10-14 09:00 UTC");
    assert!(saved.contains("E-W,-36.9938812,174.8773943,"), "{saved}");
}

#[test]
fn the_committed_file_survives_a_round_trip() {
    let network = RailNetwork::parse(COMMITTED).unwrap();
    let again = RailNetwork::parse(&network.to_csv("2026-10-13 09:00 UTC")).unwrap();

    assert_eq!(again.point_count(), network.point_count());
    assert_eq!(again.comments, network.comments);
    for (before, after) in network.points().zip(again.points()) {
        assert_eq!(after, before);
    }
}

#[test]
fn rejects_what_it_cannot_read() {
    let bad = |rows: &str| RailNetwork::parse(&format!("{HEADER_V2}\n{rows}\n")).unwrap_err();

    assert!(bad("E-W,-36.9,174.8,ground,0.0").contains("expected 6 fields"));
    assert!(bad("E-W,south,174.8,ground,0.0,").contains("latitude"));
    assert!(bad("E-W,-36.9,174.8,floating,0.0,").contains("not a mode"));
    assert!(bad("E-W,-36.9,174.8,fixed,,").contains("needs a height"));
    assert!(bad("E-W,-96.9,174.8,ground,0.0,").contains("off the planet"));
    assert!(bad("E-W,-36.9,174.8,ground,0.0,deep").contains("terrain"));

    assert!(
        RailNetwork::parse("E-W,-36.9,174.8\n")
            .unwrap_err()
            .contains("before the header")
    );
    assert!(
        RailNetwork::parse("line,lat,lon\n")
            .unwrap_err()
            .contains("unknown header")
    );

    // The line number is the file's, comments included.
    let error = RailNetwork::parse("# one\n# two\nline,latitude,longitude\nE-W,x,y\n").unwrap_err();
    assert!(error.starts_with("line 4:"), "{error}");
}

#[test]
fn ground_points_follow_the_terrain() {
    let mut fresh = point(0.0, PointMode::Ground, Some(2.0));
    fresh.terrain = Some(30.0);
    fresh.sampled = Some(31.0);

    let mut cached = point(0.001, PointMode::Ground, Some(0.0));
    cached.terrain = Some(30.0);

    let unknown = point(0.002, PointMode::Ground, Some(-1.0));

    // The sample first, the file's height while the sampler is out, the ellipsoid before
    // the first save; the offset on top of whichever it is.
    assert_heights(&line(vec![fresh, cached, unknown]), &[33.0, 30.0, -1.0]);
}

#[test]
fn fixed_points_hold_their_height() {
    let mut portal = point(0.0, PointMode::Fixed, Some(-12.0));
    portal.terrain = Some(38.9);
    portal.sampled = Some(40.0);

    assert_heights(&line(vec![portal]), &[-12.0]);
}

#[test]
fn between_points_lie_on_the_chord() {
    let tunnel = line(vec![
        point(0.0, PointMode::Fixed, Some(0.0)),
        point(0.001, PointMode::Between, None),
        point(0.003, PointMode::Between, None),
        point(0.004, PointMode::Fixed, Some(100.0)),
    ]);

    // By distance along the line, not by count: the points are a quarter and three
    // quarters of the way.
    assert_heights(&tunnel, &[0.0, 25.0, 75.0, 100.0]);
}

#[test]
fn between_points_fall_back_to_ground_anchors() {
    let mut low = point(0.0, PointMode::Ground, Some(0.0));
    low.sampled = Some(10.0);
    let mut high = point(0.004, PointMode::Ground, Some(2.0));
    high.sampled = Some(48.0);

    let bridge = line(vec![
        low,
        point(0.001, PointMode::Between, None),
        point(0.003, PointMode::Between, None),
        high,
    ]);

    assert_heights(&bridge, &[10.0, 20.0, 40.0, 50.0]);
}

#[test]
fn between_points_prefer_a_fixed_anchor_over_a_nearer_ground_one() {
    let mut ground = point(0.001, PointMode::Ground, Some(0.0));
    ground.sampled = Some(500.0);

    let run = line(vec![
        point(0.0, PointMode::Fixed, Some(0.0)),
        ground,
        point(0.002, PointMode::Between, None),
        point(0.004, PointMode::Fixed, Some(100.0)),
    ]);

    // Half way from the fixed point at 0 to the one at 0.004, whatever the ground does.
    assert_heights(&run, &[0.0, 500.0, 50.0, 100.0]);
}

#[test]
fn between_points_with_one_anchor_hold_its_height() {
    let open_end = line(vec![
        point(0.0, PointMode::Fixed, Some(40.0)),
        point(0.001, PointMode::Between, None),
        point(0.002, PointMode::Between, None),
    ]);
    assert_heights(&open_end, &[40.0, 40.0, 40.0]);

    let open_start = line(vec![
        point(0.0, PointMode::Between, None),
        point(0.001, PointMode::Fixed, Some(40.0)),
    ]);
    assert_heights(&open_start, &[40.0, 40.0]);
}

#[test]
fn between_points_with_no_anchor_follow_the_ground() {
    let mut one = point(0.0, PointMode::Between, None);
    one.sampled = Some(12.0);
    let mut two = point(0.001, PointMode::Between, None);
    two.terrain = Some(13.0);
    let three = point(0.002, PointMode::Between, None);

    assert_heights(&line(vec![one, two, three]), &[12.0, 13.0, 0.0]);
}

#[test]
fn resolved_positions_stand_at_the_height() {
    let mut track = line(vec![
        point(0.0, PointMode::Fixed, Some(0.0)),
        point(0.002, PointMode::Between, None),
        point(0.004, PointMode::Fixed, Some(100.0)),
    ]);
    track.resolve();

    // On the equator the normal is radial, so the position's length is the semi major axis
    // plus the height.
    let TerrainShape::Spheroid { major_axis, .. } = TerrainShape::WGS84 else {
        unreachable!()
    };
    for (position, height) in track.positions.iter().zip([0.0, 50.0, 100.0]) {
        assert!(
            (position.length() - major_axis - height).abs() < 1e-6,
            "{position} should stand {height} m up"
        );
    }

    let network = RailNetwork {
        comments: Vec::new(),
        lines: vec![track],
    };
    let centre = network.centre();
    assert!(centre.length() < major_axis + 100.0 && centre.length() > major_axis - 1.0);
}

#[test]
fn grades_are_rise_over_the_run_along_the_ground() {
    // A kilometre along the equator is this many degrees of longitude.
    let kilometre = (1000.0 / 6_378_137.0_f64).to_degrees();

    // Level, whatever the heights are made of.
    let mut sampled = point(kilometre, PointMode::Ground, Some(2.0));
    sampled.sampled = Some(18.0);
    let flat = line(vec![
        point(0.0, PointMode::Fixed, Some(20.0)),
        sampled,
        point(2.0 * kilometre, PointMode::Fixed, Some(20.0)),
    ]);
    assert_eq!(flat.segment_grades(), [0.0, 0.0]);

    // The City Rail Link's limit: 35 m up over a kilometre, and the same down again.
    let climb = line(vec![
        point(0.0, PointMode::Fixed, Some(0.0)),
        point(kilometre, PointMode::Fixed, Some(35.0)),
        point(2.0 * kilometre, PointMode::Fixed, Some(0.0)),
    ]);
    let grades = climb.segment_grades();
    assert_eq!(grades.len(), 2);
    for grade in &grades {
        assert!((grade - MAX_GRADE).abs() < 1e-4, "{grade}");
    }

    // The run is measured on the ground, so a between point on the chord has the chord's
    // grade on both sides of it.
    let chord = line(vec![
        point(0.0, PointMode::Fixed, Some(10.0)),
        point(kilometre, PointMode::Between, None),
        point(3.0 * kilometre, PointMode::Fixed, Some(40.0)),
    ]);
    for grade in chord.segment_grades() {
        assert!((grade - 0.01).abs() < 1e-6, "{grade}");
    }

    // Two points at one place: no run, so a step is infinitely steep and no step is level.
    let doubled = line(vec![
        point(0.0, PointMode::Fixed, Some(0.0)),
        point(0.0, PointMode::Fixed, Some(0.0)),
        point(0.0, PointMode::Fixed, Some(5.0)),
    ]);
    assert_eq!(doubled.segment_grades(), [0.0, f64::INFINITY]);

    // A point alone has no segment.
    assert!(
        line(vec![point(0.0, PointMode::Fixed, Some(0.0))])
            .segment_grades()
            .is_empty()
    );
}

#[test]
fn longitude_and_latitude_come_back_from_the_unit_sphere() {
    // A dozen places: the origin of the convention and its axes, both poles, the date line
    // from both sides, the network, and the far corners.
    let places = [
        (0.0, 0.0),
        (90.0, 0.0),
        (-90.0, 0.0),
        (0.0, 90.0),
        (0.0, -90.0),
        (180.0, 0.0),
        (-180.0, 0.0),
        (180.0, -36.8),
        (174.7633, -36.8485),
        (-74.0060, 40.7128),
        (-179.99, 89.9),
        (0.01, -89.9),
    ];

    for (longitude, latitude) in places {
        let unit = unit_position(longitude, latitude);
        let (back_longitude, back_latitude) = lon_lat_from_unit(unit);

        // The direction is what matters and is unambiguous everywhere, including at a pole
        // and at 180 against -180.
        let again = unit_position(back_longitude, back_latitude);
        assert!(
            again.distance(unit) < 1e-12,
            "{longitude}, {latitude} came back as {back_longitude}, {back_latitude}"
        );
        assert!(
            (back_latitude - latitude).abs() < 1e-9,
            "{latitude} came back as {back_latitude}"
        );
        // Away from the poles the longitude is unambiguous too, up to the sign at the date
        // line.
        if latitude.abs() < 89.0 {
            let difference = (back_longitude - longitude).abs() % 360.0;
            assert!(
                difference < 1e-9 || (difference - 360.0).abs() < 1e-9,
                "{longitude} came back as {back_longitude}"
            );
        }
    }

    let point = RailPoint::ground_at_unit(unit_position(174.7633, -36.8485));
    assert_eq!(point.mode, PointMode::Ground);
    assert_eq!(point.height, Some(0.0));
    assert!((point.longitude - 174.7633).abs() < 1e-9);
    assert!((point.latitude + 36.8485).abs() < 1e-9);
}

#[test]
fn formats_the_time_in_utc() {
    // Checked against date -u.
    assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
    assert_eq!(format_utc(1_000_000_000), "2001-09-09 01:46 UTC");
    assert_eq!(format_utc(951_782_400), "2000-02-29 00:00 UTC");
    assert_eq!(format_utc(1_791_117_240), "2026-10-04 12:34 UTC");
    assert_eq!(format_utc(4_102_444_799), "2099-12-31 23:59 UTC");
}
