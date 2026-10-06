//! The run splitting on a short line that has a bit of everything, with the editor on and
//! off.

use super::*;
use crate::plugins::shared::rail_network::PointMode::{Between, Fixed, Ground};

fn run(stroke: Stroke, first: usize, last: usize) -> Run {
    Run {
        stroke,
        first,
        last,
    }
}

#[test]
fn spans_dash_and_steep_track_warns_only_while_editing() {
    // Eight points: level ground, a steep climb, level ground to a portal, a tunnel of two
    // between points to the other portal, and level ground again. The grade inside the
    // tunnel is steep too, which a span does not care about.
    let modes = [
        Ground, Ground, Ground, Fixed, Between, Between, Fixed, Ground,
    ];
    let grades = [0.01, 0.05, 0.02, 0.1, 0.0, 0.1, 0.01];

    assert_eq!(
        split_runs(&modes, &grades, true),
        [
            run(Stroke::Track, 0, 1),
            run(Stroke::Steep, 1, 2),
            run(Stroke::Track, 2, 3),
            run(Stroke::Span, 3, 6),
            run(Stroke::Track, 6, 7),
        ]
    );

    // Off, the climb is track like the rest, and the runs either side of it join up.
    assert_eq!(
        split_runs(&modes, &grades, false),
        [
            run(Stroke::Track, 0, 3),
            run(Stroke::Span, 3, 6),
            run(Stroke::Track, 6, 7),
        ]
    );
}

#[test]
fn a_grade_at_the_limit_is_track_and_one_over_it_is_steep() {
    let modes = [Ground; 4];
    let grades = [MAX_GRADE, MAX_GRADE + 1e-9, f64::INFINITY];

    assert_eq!(
        split_runs(&modes, &grades, true),
        [run(Stroke::Track, 0, 1), run(Stroke::Steep, 1, 3)]
    );
}

#[test]
fn a_between_point_at_either_end_makes_a_span() {
    // A lone between point spans the segment into it and the one out of it.
    let modes = [Ground, Between, Ground, Ground];
    assert_eq!(
        split_runs(&modes, &[0.0, 0.0, 0.0], false),
        [run(Stroke::Span, 0, 2), run(Stroke::Track, 2, 3)]
    );

    // A line that is all between is one span.
    assert_eq!(
        split_runs(&[Between; 3], &[0.0, 0.0], true),
        [run(Stroke::Span, 0, 2)]
    );
}

#[test]
fn fewer_than_two_points_make_no_run() {
    assert!(split_runs(&[], &[], true).is_empty());
    assert!(split_runs(&[Ground], &[], true).is_empty());
}

#[test]
fn a_stroke_knows_its_style_and_its_colour() {
    let line = Color::srgb(0.0, 1.0, 0.0);

    assert!(!Stroke::Track.dashed());
    assert!(Stroke::Span.dashed());
    assert!(!Stroke::Steep.dashed());

    assert_eq!(Stroke::Track.colour(line), line);
    assert_eq!(Stroke::Span.colour(line), line);
    assert_eq!(Stroke::Steep.colour(line), WARNING_COLOUR);
}
