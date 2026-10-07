//! The driver and the placing checked with numbers: the reflection at the ends of a line,
//! the model's transform against its measured box, and the carriage on the left of the
//! track whichever way the train runs, on the synthetic lines the track frames are tested
//! on.

use super::*;
use crate::plugins::sheet_grid::label_size;
use crate::plugins::track_frames::tests::{eastward, quarter_circle};

/// A train on the first line at a distance, heading as given.
fn train(distance: f64, direction: f64) -> Train {
    Train {
        distance,
        direction,
        ..Train::stand_in(0)
    }
}

#[test]
fn a_train_runs_on_at_its_speed_and_keeps_its_heading() {
    assert_eq!(advance(100.0, 1.0, 20.0, 0.5, 1000.0), (110.0, 1.0));
    assert_eq!(advance(100.0, -1.0, 20.0, 0.5, 1000.0), (90.0, -1.0));

    // Standing still is standing still, and a heading of nought is taken as forward.
    assert_eq!(advance(100.0, 1.0, 20.0, 0.0, 1000.0), (100.0, 1.0));
    assert_eq!(advance(100.0, 0.0, 20.0, 0.5, 1000.0), (110.0, 1.0));
}

/// Three metres past the end is three metres short of it heading back; three metres past
/// the start, running back, is three metres in heading out. Arriving exactly at an end is
/// standing there turned round.
#[test]
fn a_train_reflects_at_either_end_with_the_overshoot_folded_back() {
    let (distance, direction) = advance(995.0, 1.0, 20.0, 0.4, 1000.0);
    assert!((distance - 997.0).abs() < 1e-9, "{distance}");
    assert_eq!(direction, -1.0);

    let (distance, direction) = advance(5.0, -1.0, 20.0, 0.4, 1000.0);
    assert!((distance - 3.0).abs() < 1e-9, "{distance}");
    assert_eq!(direction, 1.0);

    assert_eq!(advance(990.0, 1.0, 20.0, 0.5, 1000.0), (1000.0, -1.0));
    assert_eq!(advance(10.0, -1.0, 20.0, 0.5, 1000.0), (0.0, 1.0));
}

/// Walked a frame at a time at every rate from a crawl to a step longer than the line, the
/// train is never off the line, and over a whole lap out and back it comes home.
#[test]
fn a_train_never_leaves_its_line_however_long_the_step() {
    let length = 1000.0;
    for step in [0.1, 7.3, 333.0, 999.0, 1000.0, 1001.0, 2499.0, 7777.0] {
        let (mut distance, mut direction) = (0.0, 1.0);
        for tick in 0..500 {
            (distance, direction) = advance(distance, direction, step, 1.0, length);
            assert!(
                (0.0..=length).contains(&distance),
                "step {step}, tick {tick}: {distance}"
            );
            assert!(
                direction == 1.0 || direction == -1.0,
                "step {step}: {direction}"
            );
        }
    }

    // A lap is twice the length; after exactly one, at any step that divides it, the train
    // is back at the start heading out.
    let (mut distance, mut direction) = (0.0, 1.0);
    for _ in 0..40 {
        (distance, direction) = advance(distance, direction, 50.0, 1.0, length);
    }
    assert!(distance.abs() < 1e-9, "{distance}");
    assert_eq!(direction, 1.0);
}

/// A line shortened by an edit may leave the train past its new end; the step brings it
/// onto the line first, and a line of no length has nowhere but its start.
#[test]
fn a_train_off_a_shortened_line_is_brought_back_onto_it() {
    let (distance, direction) = advance(1500.0, 1.0, 20.0, 0.1, 1000.0);
    assert!((distance - 998.0).abs() < 1e-9, "{distance}");
    assert_eq!(direction, -1.0);

    let (distance, direction) = advance(1500.0, -1.0, 20.0, 0.1, 1000.0);
    assert!((distance - 998.0).abs() < 1e-9, "{distance}");
    assert_eq!(direction, -1.0);

    assert_eq!(advance(-7.0, -1.0, 20.0, 0.1, 1000.0), (2.0, 1.0));
    assert_eq!(advance(500.0, 1.0, 20.0, 0.1, 0.0), (0.0, 1.0));
    assert_eq!(advance(500.0, -1.0, 20.0, 0.1, f64::NAN), (0.0, -1.0));
    assert_eq!(advance(f64::NAN, 1.0, 20.0, 0.1, 1000.0), (2.0, 1.0));
}

#[test]
fn the_model_faces_along_the_frame_and_stands_on_the_rail_at_a_cars_length() {
    let transform = model_transform();

    // The model's long axis, +X, runs along the frame's forward, -Z.
    assert!(
        (transform.rotation * Vec3::X).distance(Vec3::NEG_Z) < 1e-6,
        "{:?}",
        transform.rotation * Vec3::X
    );
    assert!((transform.rotation * Vec3::Y).distance(Vec3::Y) < 1e-6);

    // Its lowest point is at the rail, wherever along or across it that point is.
    for (x, z) in [
        (MODEL_MIN.x, MODEL_MIN.z),
        (MODEL_MAX.x, MODEL_MAX.z),
        (0.0, 0.0),
    ] {
        let lowest = transform.transform_point(Vec3::new(x, MODEL_MIN.y, z));
        assert!(lowest.y.abs() < 1e-5, "{lowest}");
    }
    let highest = transform.transform_point(Vec3::new(0.0, MODEL_MAX.y, 0.0));
    assert!((highest.y - CARRIAGE_HEIGHT).abs() < 1e-5, "{highest}");

    // And it is a car's length from end to end, along the frame.
    let nose = transform.transform_point(Vec3::new(MODEL_MAX.x, 0.0, 0.0));
    let tail = transform.transform_point(Vec3::new(MODEL_MIN.x, 0.0, 0.0));
    assert!(((nose - tail).length() - CARRIAGE_LENGTH).abs() < 1e-4);
    assert!((nose - tail).z < 0.0, "the nose is not forward");
    assert!(nose.x.abs() < 1e-5 && tail.x.abs() < 1e-5);
}

/// At the same distance, the carriage heading out and the carriage heading back stand on
/// opposite sides of the centre line, a track's width apart, each to the left of its own
/// direction of travel, and the one heading back faces the other way.
#[test]
fn the_carriage_keeps_left_whichever_way_the_train_runs() {
    let line = quarter_circle(800.0, 50.0);
    let spline = TrackSpline::through(&line).unwrap();

    for distance in [0.0, 137.5, 600.0, spline.length()] {
        let centre = spline.frame_at(distance);
        let out = carriage_frame(&spline, &train(distance, 1.0));
        let back = carriage_frame(&spline, &train(distance, -1.0));
        let place = format!("{distance} m along");

        assert!(
            (out.position.distance(back.position) - TRACK_CENTRES).abs() < 1e-9,
            "{place}"
        );
        assert!(
            (out.position.distance(centre.position) - TRACK_CENTRES / 2.0).abs() < 1e-9,
            "{place}"
        );
        assert!(out.forward().distance(-back.forward()) < 1e-9, "{place}");
        assert!(out.up().distance(back.up()) < 1e-9, "{place}");
        assert!(out.right().distance(-back.right()) < 1e-9, "{place}");

        // Left of travel: the carriage is on the side the frame's right points away from.
        assert!(
            (out.position - centre.position).dot(out.right()) < 0.0,
            "{place}"
        );
        assert!(
            (back.position - centre.position).dot(back.right()) < 0.0,
            "{place}"
        );
        assert!(
            (back.position - centre.position).dot(centre.right()) > 0.0,
            "{place}"
        );
        assert_eq!(out.distance, distance);
        assert_eq!(back.distance, distance);
    }
}

/// On a climb the carriage heading back is descending, and the frame's grade says so.
#[test]
fn the_carriage_heading_back_descends_what_the_line_climbs() {
    let line = eastward(21, 50.0, |along| 20.0 + 0.02 * along);
    let spline = TrackSpline::through(&line).unwrap();

    let out = carriage_frame(&spline, &train(500.0, 1.0));
    let back = carriage_frame(&spline, &train(500.0, -1.0));
    assert!((out.grade - 0.02).abs() < 1e-3, "{}", out.grade);
    assert!((back.grade + 0.02).abs() < 1e-3, "{}", back.grade);
}

/// A stand-in's label is its line and its speed; a live train's has its unit between them.
/// Each piece after the first carries its own break, so the pieces concatenate into the
/// label, and the names from the file and the feed are folded to the font.
#[test]
fn the_label_is_the_line_the_unit_where_there_is_one_and_the_speed() {
    let stand_in = Train::stand_in(0);
    assert_eq!(label_spans("E-W", &stand_in), ["E-W", "", "\n72 km/h"]);
    assert_eq!(
        label_lines(&label_spans("E-W", &stand_in)),
        ["E-W", "72 km/h"]
    );

    let live = Train {
        id: "AMP\u{00a0}1142".to_string(),
        speed: 12.5,
        ..Train::stand_in(0)
    };
    assert_eq!(
        label_spans("Onehunga\u{2013}West", &live),
        ["Onehunga-West", "\nAMP 1142", "\n45 km/h"]
    );
    assert_eq!(
        label_lines(&label_spans("S-C", &live)),
        ["S-C", "AMP 1142", "45 km/h"]
    );
}

/// The footprint of a label of several lines is its widest line's width and a line taller
/// for each line after the first, and of one line it is the sheet grid's footprint.
#[test]
fn a_label_of_more_lines_is_as_wide_as_its_widest_and_a_line_taller_for_each() {
    let one = label_size_lines(&["AMP 1142"]);
    assert_eq!(one, label_size("AMP 1142"));

    let three = label_size_lines(&["S-C", "AMP 1142", "45 km/h"]);
    assert_eq!(three.x, label_size("AMP 1142").x);
    let two = label_size_lines(&["S-C", "45 km/h"]);
    assert!(two.y > one.y && three.y > two.y, "{one} {two} {three}");
    assert!((three.y - two.y - (two.y - one.y)).abs() < 1e-5);

    assert_eq!(label_size_lines(&[]), label_size(""));
}
