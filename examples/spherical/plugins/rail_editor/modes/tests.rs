//! The mode operations on lines along the equator, where the points are 111 m apart and the
//! chords between them equal, so a between point's height is arithmetic on its index.

use super::*;
use crate::plugins::rail_editor::tests::network;
use crate::plugins::shared::rail_network::{MAX_GRADE, RailPoint};

/// One line of ground points 111 m apart along the equator, the terrain under them sampled
/// at the heights given, resolved.
fn draped(sampled: &[f64]) -> RailNetwork {
    let points = sampled
        .iter()
        .enumerate()
        .map(|(index, &height)| {
            let mut point = RailPoint::ground(index as f64 * 0.001, 0.0);
            point.sampled = Some(height);
            point
        })
        .collect();

    let mut network = RailNetwork {
        comments: Vec::new(),
        lines: vec![RailLine {
            name: "a".to_string(),
            points,
            positions: Vec::new(),
        }],
    };
    network.resolve();
    network
}

/// The editor's line "a" with the ground sampled under it and one of each mode: a fixed
/// point at 20 m where the ground is 12, a between point after it, and a ground point with
/// an offset of 2 m.
fn mixed() -> RailNetwork {
    let mut network = network();
    let points = &mut network.lines[0].points;
    for (point, sampled) in points.iter_mut().zip([10.0, 12.0, 30.0, 28.0, 15.0, 11.0]) {
        point.sampled = Some(sampled);
    }
    points[1].mode = PointMode::Fixed;
    points[1].height = Some(20.0);
    points[2].mode = PointMode::Between;
    points[2].height = None;
    points[3].height = Some(2.0);

    network.resolve();
    network
}

fn modes(network: &RailNetwork, line: usize) -> Vec<PointMode> {
    network.lines[line]
        .points
        .iter()
        .map(|point| point.mode)
        .collect()
}

fn select_run(editor: &mut RailEditor, line: usize, first: usize, last: usize) {
    editor.select((line, first), false);
    editor.select((line, last), true);
}

fn assert_close(actual: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (actual - expected).abs() < tolerance,
        "{what}: {actual} vs {expected}"
    );
}

#[test]
fn a_change_to_ground_or_fixed_keeps_every_resolved_height() {
    // Every point of the mixed line at once, so each of ground, fixed and between is taken
    // to each of ground and fixed.
    for mode in [PointMode::Fixed, PointMode::Ground] {
        let mut network = mixed();
        let mut editor = RailEditor::default();
        select_run(&mut editor, 0, 0, 5);
        let before = network.lines[0].resolved_heights();

        assert!(editor.set_mode(&mut network, mode));

        let line = &network.lines[0];
        let after = line.resolved_heights();
        for (index, (point, (&was, &is))) in line
            .points
            .iter()
            .zip(before.iter().zip(&after))
            .enumerate()
        {
            let what = format!("{mode:?} point {index}");
            assert_close(is, was, 1e-3, &what);
            assert_eq!(point.mode, mode, "{what}");
            let height = point
                .height
                .unwrap_or_else(|| panic!("{what} has no height"));
            match mode {
                PointMode::Fixed => assert_close(height, was, 1e-3, &what),
                _ => assert_close(height, was - point.sampled.unwrap(), 1e-3, &what),
            }
        }

        // One snapshot, the selection as it was, and undo brings the modes back.
        assert_eq!(editor.undo.len(), 1);
        assert_eq!(
            editor.selection,
            [(0, 0), (0, 1), (0, 2), (0, 3), (0, 4), (0, 5)]
        );
        assert_eq!(editor.anchor, Some((0, 5)));
        assert!(editor.undo(&mut network, None));
        assert_eq!(modes(&network, 0), modes(&mixed(), 0));
        for (is, was) in network.lines[0].resolved_heights().iter().zip(&before) {
            assert_close(*is, *was, 1e-9, "undone");
        }
    }
}

#[test]
fn a_change_to_between_moves_the_point_onto_the_chord() {
    let mut network = mixed();
    let mut editor = RailEditor::default();

    // The ground point at 30 m, between the fixed point at 20 m two segments before it and
    // the ground point at 15 m one after, lands two thirds of the way from 20 to 15.
    editor.select((0, 3), false);
    assert!(editor.set_mode(&mut network, PointMode::Between));

    let point = &network.lines[0].points[3];
    assert_eq!(point.mode, PointMode::Between);
    assert_eq!(point.height, None);
    assert_close(
        network.lines[0].resolved_heights()[3],
        20.0 - 5.0 * 2.0 / 3.0,
        1e-3,
        "on the chord",
    );

    assert_eq!(editor.undo.len(), 1);
    assert_eq!(editor.selection, [(0, 3)]);
    assert!(editor.undo(&mut network, None));
    assert_eq!(network.lines[0].points[3].mode, PointMode::Ground);
    assert_eq!(network.lines[0].points[3].height, Some(2.0));
}

#[test]
fn nothing_selected_changes_nothing_and_records_nothing() {
    let mut network = mixed();
    let mut editor = RailEditor::default();

    assert!(!editor.set_mode(&mut network, PointMode::Fixed));
    assert!(!editor.drop_to_ground(&mut network));
    assert!(!editor.span_selection(&mut network));
    assert_eq!(editor.select_steep_run(&network, MAX_GRADE), 0);

    // Nor does a selection that names no point.
    editor.selection = vec![(0, 10), (5, 0)];
    assert!(!editor.set_mode(&mut network, PointMode::Fixed));
    assert!(!editor.drop_to_ground(&mut network));

    assert!(editor.undo.is_empty());
    assert_eq!(modes(&network, 0), modes(&mixed(), 0));
}

#[test]
fn a_mode_the_points_have_already_is_no_edit() {
    let mut network = mixed();
    let mut editor = RailEditor::default();

    // Ground points made ground, the fixed point made fixed, the between point made between,
    // and ground points with no offset dropped to the ground: nothing changes, so nothing is
    // recorded and undo has nothing to offer.
    select_run(&mut editor, 0, 3, 4);
    assert!(!editor.set_mode(&mut network, PointMode::Ground));
    editor.select((0, 1), false);
    assert!(!editor.set_mode(&mut network, PointMode::Fixed));
    editor.select((0, 2), false);
    assert!(!editor.set_mode(&mut network, PointMode::Between));
    select_run(&mut editor, 0, 4, 5);
    assert!(!editor.drop_to_ground(&mut network));

    assert!(editor.undo.is_empty());
    assert_eq!(modes(&network, 0), modes(&mixed(), 0));
    assert_eq!(network.lines[0].points[3].height, Some(2.0));

    // An offset taken back through the resolved height comes back a rounding error off, which
    // is still the same height, and the offset stored is left exactly as it was.
    network.lines[0].points[5].height = Some(0.1);
    network.lines[0].points[5].sampled = Some(38.9);
    editor.select((0, 5), false);
    assert!(!editor.set_mode(&mut network, PointMode::Ground));
    assert_eq!(network.lines[0].points[5].height, Some(0.1));

    // One point to change among points that have the mode already is one edit.
    select_run(&mut editor, 0, 3, 4);
    assert!(editor.drop_to_ground(&mut network));
    assert_eq!(editor.undo.len(), 1);
    assert_eq!(network.lines[0].points[3].height, Some(0.0));
    assert_eq!(network.lines[0].points[4].height, Some(0.0));
    assert_eq!(network.lines[0].points[5].height, Some(0.1));
}

#[test]
fn drop_to_ground_forgets_the_offsets_and_the_anchors() {
    let mut network = mixed();
    let mut editor = RailEditor::default();
    select_run(&mut editor, 0, 1, 3);

    assert!(editor.drop_to_ground(&mut network));

    let line = &network.lines[0];
    for index in 1..=3 {
        assert_eq!(line.points[index].mode, PointMode::Ground, "{index}");
        assert_eq!(line.points[index].height, Some(0.0), "{index}");
    }
    // The ground under each, and nothing else.
    let heights = line.resolved_heights();
    for (index, expected) in [(1, 12.0), (2, 30.0), (3, 28.0)] {
        assert_close(heights[index], expected, 1e-9, "dropped");
    }
    // The points either side are as they were.
    assert_eq!(line.points[0].height, Some(0.0));
    assert_eq!(line.points[4].mode, PointMode::Ground);

    assert_eq!(editor.undo.len(), 1);
    assert_eq!(editor.selection, [(0, 1), (0, 2), (0, 3)]);
    assert_eq!(editor.anchor, Some((0, 3)));
    assert!(editor.undo(&mut network, None));
    assert_eq!(modes(&network, 0), modes(&mixed(), 0));
    assert_eq!(network.lines[0].points[3].height, Some(2.0));
}

#[test]
fn a_span_fixes_the_ends_where_they_are_and_puts_the_inside_between() {
    // Seven points over a hill, with a portal selected on each flank.
    let profile = [10.0, 12.0, 30.0, 45.0, 31.0, 14.0, 11.0];
    let mut network = draped(&profile);
    let mut editor = RailEditor::default();
    select_run(&mut editor, 0, 1, 5);

    assert!(editor.span_selection(&mut network));

    let line = &network.lines[0];
    assert_eq!(
        modes(&network, 0),
        [
            PointMode::Ground,
            PointMode::Fixed,
            PointMode::Between,
            PointMode::Between,
            PointMode::Between,
            PointMode::Fixed,
            PointMode::Ground,
        ]
    );
    assert_eq!(line.points[1].height, Some(12.0));
    assert_eq!(line.points[5].height, Some(14.0));
    for index in 2..=4 {
        assert_eq!(line.points[index].height, None, "{index}");
    }
    assert_eq!(line.points[0].height, Some(0.0));
    assert_eq!(line.points[6].height, Some(0.0));

    // The inside lies on the straight grade from 12 m to 14 m, 50 cm a point.
    let heights = line.resolved_heights();
    for (index, expected) in [(1, 12.0), (2, 12.5), (3, 13.0), (4, 13.5), (5, 14.0)] {
        assert_close(heights[index], expected, 1e-6, "on the chord");
    }
    assert_close(heights[0], 10.0, 1e-9, "outside");
    assert_close(heights[6], 11.0, 1e-9, "outside");

    assert_eq!(editor.undo.len(), 1);
    assert_eq!(editor.selection, [(0, 1), (0, 2), (0, 3), (0, 4), (0, 5)]);
    assert!(editor.undo(&mut network, None));
    assert_eq!(modes(&network, 0), vec![PointMode::Ground; 7]);
    for (height, expected) in network.lines[0].resolved_heights().iter().zip(profile) {
        assert_close(*height, expected, 1e-9, "undone");
    }
}

#[test]
fn a_span_needs_three_points_on_a_line() {
    let mut hill = draped(&[10.0, 12.0, 30.0, 45.0, 31.0, 14.0, 11.0]);
    let mut editor = RailEditor::default();

    // Two points, and nothing between them to span; the panel's button knows it too.
    select_run(&mut editor, 0, 2, 3);
    assert!(!editor.can_span(&hill));
    assert!(!editor.span_selection(&mut hill));
    assert!(editor.undo.is_empty());
    assert_eq!(modes(&hill, 0), vec![PointMode::Ground; 7]);

    // Two lines, three selected on one and two on the other: the one spans, the other is
    // left alone, and the two are one edit.
    let mut network = network();
    let mut editor = RailEditor::default();
    editor.selection = vec![(0, 1), (0, 2), (0, 3), (1, 0), (1, 1)];
    editor.anchor = Some((1, 1));

    assert!(editor.can_span(&network));
    assert!(editor.span_selection(&mut network));
    assert_eq!(
        modes(&network, 0),
        [
            PointMode::Ground,
            PointMode::Fixed,
            PointMode::Between,
            PointMode::Fixed,
            PointMode::Ground,
            PointMode::Ground,
        ]
    );
    assert_eq!(modes(&network, 1), vec![PointMode::Ground; 3]);
    assert_eq!(editor.undo.len(), 1);
}

#[test]
fn the_steep_run_grows_the_selection_over_one_climb() {
    // Level, then 10 m a segment up, then level: two steep segments between points 2 and 4.
    let network = draped(&[0.0, 0.0, 0.0, 10.0, 20.0, 20.0, 20.0]);
    let grades = network.lines[0].segment_grades();
    assert!(grades[2] > MAX_GRADE && grades[3] > MAX_GRADE);
    assert!(grades[1] < MAX_GRADE && grades[4] < MAX_GRADE);

    // From the middle of the climb.
    let mut editor = RailEditor::default();
    editor.select((0, 3), false);
    assert_eq!(editor.select_steep_run(&network, MAX_GRADE), 2);
    assert_eq!(editor.selection, [(0, 3), (0, 2), (0, 4)]);
    assert_eq!(editor.anchor, Some((0, 3)));
    assert!(editor.undo.is_empty());

    // From its foot, whose segment after is steep, and again from the top: the same run,
    // and a second time adds nothing.
    let mut editor = RailEditor::default();
    editor.select((0, 2), false);
    assert_eq!(editor.select_steep_run(&network, MAX_GRADE), 2);
    assert_eq!(editor.selection, [(0, 2), (0, 3), (0, 4)]);
    assert_eq!(editor.select_steep_run(&network, MAX_GRADE), 0);

    // From level ground, nothing.
    let mut editor = RailEditor::default();
    editor.select((0, 0), false);
    assert_eq!(editor.select_steep_run(&network, MAX_GRADE), 0);
    assert_eq!(editor.selection, [(0, 0)]);

    // A higher limit, and the climb is not steep.
    let mut editor = RailEditor::default();
    editor.select((0, 3), false);
    assert_eq!(editor.select_steep_run(&network, 0.1), 0);
}

#[test]
fn the_steep_run_stops_at_a_flat_top() {
    // Up, along the crown, and up again: from the first flank the crown is not taken, which
    // is the case the two-click span is for.
    let network = draped(&[0.0, 0.0, 10.0, 20.0, 20.0, 20.0, 30.0, 40.0, 40.0]);
    let mut editor = RailEditor::default();
    editor.select((0, 2), false);

    assert_eq!(editor.select_steep_run(&network, MAX_GRADE), 2);
    assert_eq!(editor.selection, [(0, 2), (0, 1), (0, 3)]);

    // A point on the second flank added to the selection, the crown between the two flanks
    // is still not taken; it has to be spanned over.
    editor.selection.push((0, 7));
    editor.anchor = Some((0, 7));
    assert_eq!(editor.select_steep_run(&network, MAX_GRADE), 2);
    assert_eq!(
        editor.selection,
        [(0, 2), (0, 1), (0, 3), (0, 7), (0, 5), (0, 6)]
    );
}

#[test]
fn the_steep_run_does_not_go_through_a_span() {
    // A climb of four steep segments, the first two of them spanned from point 1 to point 3:
    // the chord of the span is as steep as the ground was.
    let mut network = draped(&[0.0, 0.0, 10.0, 20.0, 30.0, 40.0, 40.0]);
    let mut editor = RailEditor::default();
    select_run(&mut editor, 0, 1, 3);
    assert!(editor.span_selection(&mut network));
    assert!(network.lines[0].segment_grades()[1] > MAX_GRADE);

    // From the far portal the run is the steep ground beyond it, and not the span; from the
    // near portal there is no steep ground, and nothing is added. Otherwise a second span
    // would take in the far portal and unfix it.
    editor.select((0, 3), false);
    assert_eq!(editor.select_steep_run(&network, MAX_GRADE), 2);
    assert_eq!(editor.selection, [(0, 3), (0, 4), (0, 5)]);

    editor.select((0, 1), false);
    assert_eq!(editor.select_steep_run(&network, MAX_GRADE), 0);
    assert_eq!(editor.selection, [(0, 1)]);
}

#[test]
fn the_steep_run_is_found_from_the_grades_and_stops_at_a_between_point() {
    let ground = [PointMode::Ground; 6];
    let grades = [0.0, 0.1, 0.1, 0.0, 0.1];

    assert_eq!(steep_run(&ground, &grades, 0, 0.05), (0, 0));
    assert_eq!(steep_run(&ground, &grades, 1, 0.05), (1, 3));
    assert_eq!(steep_run(&ground, &grades, 2, 0.05), (1, 3));
    assert_eq!(steep_run(&ground, &grades, 3, 0.05), (1, 3));
    assert_eq!(steep_run(&ground, &grades, 4, 0.05), (4, 5));
    assert_eq!(steep_run(&ground, &grades, 5, 0.05), (4, 5));

    // At the limit is not over it.
    assert_eq!(steep_run(&ground, &grades, 2, 0.1), (2, 2));
    // A point alone has no segments.
    assert_eq!(steep_run(&[PointMode::Ground], &[], 0, 0.05), (0, 0));

    // A tunnel whose chord is steep, with steep ground beyond its far portal: a segment with
    // a between point at either end is a span and not ground, as the strokes have it, so
    // from the near portal the run is the portal alone, and from the far one it is the
    // ground beyond and never the tunnel.
    let tunnel = [
        PointMode::Ground,
        PointMode::Fixed,
        PointMode::Between,
        PointMode::Between,
        PointMode::Fixed,
        PointMode::Ground,
    ];
    let grades = [0.01, 0.05, 0.05, 0.05, 0.05];
    assert_eq!(steep_run(&tunnel, &grades, 1, MAX_GRADE), (1, 1));
    assert_eq!(steep_run(&tunnel, &grades, 2, MAX_GRADE), (2, 2));
    assert_eq!(steep_run(&tunnel, &grades, 4, MAX_GRADE), (4, 5));
    assert_eq!(steep_run(&tunnel, &grades, 5, MAX_GRADE), (4, 5));
}
