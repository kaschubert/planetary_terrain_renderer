//! The editing rules on a small network on the equator, where a thousandth of a degree of
//! longitude is 111 m and the arithmetic can be checked by hand.

use super::*;
use crate::plugins::shared::rail_network::PointMode;
use bevy_terrain::math::unit_position;
use discs::disc_style;
use std::path::Path;

/// Two lines, "a" with six points 111 m apart along the equator and "b" with three a little
/// south of it, resolved so the segments have positions. The gizmo's, the modes' and the
/// panel's tests borrow it.
pub(super) fn network() -> RailNetwork {
    let mut csv = String::from("line,latitude,longitude\n");
    for index in 0..6 {
        csv.push_str(&format!("a,0.0,{:.4}\n", index as f64 * 0.001));
    }
    for index in 0..3 {
        csv.push_str(&format!("b,-0.01,{:.4}\n", index as f64 * 0.001));
    }

    let mut network = RailNetwork::parse(&csv).unwrap();
    network.resolve();
    network
}

/// A position on the ellipsoid at a longitude and latitude, the way a terrain hit is.
fn on_the_ground(longitude: f64, latitude: f64) -> DVec3 {
    TerrainShape::WGS84.position_unit_to_local(unit_position(longitude, latitude), 0.0)
}

fn longitudes(network: &RailNetwork, line: usize) -> Vec<f64> {
    network.lines[line]
        .points
        .iter()
        .map(|point| point.longitude)
        .collect()
}

#[test]
fn a_plain_click_selects_one_point() {
    let mut editor = RailEditor::default();

    editor.select((0, 2), false);
    assert_eq!(editor.selection, [(0, 2)]);
    assert_eq!(editor.anchor, Some((0, 2)));

    editor.select((1, 0), false);
    assert_eq!(editor.selection, [(1, 0)]);
    assert_eq!(editor.anchor, Some((1, 0)));

    editor.clear_selection();
    assert!(editor.selection.is_empty());
    assert_eq!(editor.anchor, None);
}

#[test]
fn a_shift_click_on_the_same_line_selects_the_run_either_way() {
    let mut editor = RailEditor::default();

    // Forward along the line, from the anchor.
    editor.select((0, 1), false);
    editor.select((0, 4), true);
    assert_eq!(editor.selection, [(0, 1), (0, 2), (0, 3), (0, 4)]);
    assert_eq!(editor.anchor, Some((0, 4)));

    // And back, from the new anchor, replacing the run rather than extending it.
    editor.select((0, 2), true);
    assert_eq!(editor.selection, [(0, 4), (0, 3), (0, 2)]);
    assert_eq!(editor.anchor, Some((0, 2)));

    // The anchor alone is a run of one.
    editor.select((0, 2), true);
    assert_eq!(editor.selection, [(0, 2)]);
}

#[test]
fn a_shift_click_on_another_line_adds_the_point() {
    let mut editor = RailEditor::default();

    editor.select((0, 1), false);
    editor.select((1, 2), true);
    assert_eq!(editor.selection, [(0, 1), (1, 2)]);
    assert_eq!(editor.anchor, Some((1, 2)));

    // Again is not twice.
    editor.select((0, 1), true);
    editor.select((1, 2), true);
    assert_eq!(editor.selection, [(0, 1), (1, 2)]);

    // With nothing selected a shift-click is a plain one.
    editor.clear_selection();
    editor.select((1, 0), true);
    assert_eq!(editor.selection, [(1, 0)]);
    assert_eq!(editor.anchor, Some((1, 0)));
}

#[test]
fn a_short_still_press_is_a_click_and_the_rest_are_not() {
    let mut clicks = ClickDetector::default();
    let at = Vec2::new(100.0, 100.0);

    clicks.press(at, 1.0, false);
    assert_eq!(clicks.release(at + 2.0, 1.1), Some(Click::Single(at + 2.0)));

    // Moved too far: a pan.
    clicks.press(at, 2.0, false);
    assert_eq!(clicks.release(at + Vec2::new(5.0, 0.0), 2.1), None);

    // Held too long.
    clicks.press(at, 3.0, false);
    assert_eq!(clicks.release(at, 3.4), None);

    // Pressed over the UI.
    clicks.press(at, 4.0, true);
    assert_eq!(clicks.release(at, 4.1), None);

    // A release with no press, as when the press came while editing was off.
    assert_eq!(clicks.release(at, 5.0), None);
}

#[test]
fn two_clicks_close_in_time_and_place_are_a_double() {
    let mut clicks = ClickDetector::default();
    let at = Vec2::new(100.0, 100.0);

    clicks.press(at, 1.0, false);
    assert_eq!(clicks.release(at, 1.1), Some(Click::Single(at)));
    clicks.press(at + 1.0, 1.4, false);
    assert_eq!(clicks.release(at + 1.0, 1.5), Some(Click::Double(at + 1.0)));

    // A third click is the first of a new pair, not another double.
    clicks.press(at, 1.7, false);
    assert_eq!(clicks.release(at, 1.8), Some(Click::Single(at)));

    // Too long after the first.
    clicks.press(at, 2.3, false);
    assert_eq!(clicks.release(at, 2.4), Some(Click::Single(at)));

    // Too far from the first.
    clicks.press(at + Vec2::new(10.0, 0.0), 2.5, false);
    assert_eq!(
        clicks.release(at + Vec2::new(10.0, 0.0), 2.6),
        Some(Click::Single(at + Vec2::new(10.0, 0.0)))
    );

    // A pan between two clicks keeps them apart.
    clicks.press(at, 3.0, false);
    assert_eq!(clicks.release(at, 3.1), Some(Click::Single(at)));
    clicks.press(at, 3.2, false);
    assert_eq!(clicks.release(at + Vec2::new(20.0, 0.0), 3.25), None);
    clicks.press(at, 3.3, false);
    assert_eq!(clicks.release(at, 3.4), Some(Click::Single(at)));
}

#[test]
fn the_nearest_segment_is_found_by_distance_in_three_dimensions() {
    let network = network();

    // Between the second and third points of "a", 55 m south of the line.
    let hit = on_the_ground(0.0015, -0.0005);
    let (line, segment, distance) = nearest_segment(&network.lines, hit).unwrap();
    assert_eq!((line, segment), (0, 1));
    assert!((distance - 55.6).abs() < 1.0, "{distance}");

    // Past the end of "a", where the last segment is nearest and the distance is to its
    // end point rather than to the line through it.
    let hit = on_the_ground(0.006, 0.0);
    let (line, segment, distance) = nearest_segment(&network.lines, hit).unwrap();
    assert_eq!((line, segment), (0, 4));
    assert!((distance - 111.3).abs() < 1.0, "{distance}");

    // Nearer "b".
    let hit = on_the_ground(0.0012, -0.0095);
    let (line, segment, _) = nearest_segment(&network.lines, hit).unwrap();
    assert_eq!((line, segment), (1, 1));

    // A degree away is beyond the search.
    assert_eq!(
        nearest_segment(&network.lines, on_the_ground(1.0, 0.0)),
        None
    );
}

#[test]
fn a_point_is_inserted_after_the_segments_first_point_and_selected() {
    let mut network = network();
    let mut editor = RailEditor::default();
    editor.select((1, 0), false);

    let hit = on_the_ground(0.0025, 0.0003);
    assert_eq!(editor.insert_point(&mut network, hit), Some((0, 3)));

    let longitudes = longitudes(&network, 0);
    assert_eq!(longitudes.len(), 7);
    assert!((longitudes[3] - 0.0025).abs() < 1e-9, "{longitudes:?}");
    let point = &network.lines[0].points[3];
    assert!((point.latitude - 0.0003).abs() < 1e-9);
    assert_eq!(point.mode, PointMode::Ground);
    assert_eq!(point.height, Some(0.0));
    assert_eq!(point.sampled, None);

    assert_eq!(editor.selection, [(0, 3)]);
    assert_eq!(editor.anchor, Some((0, 3)));
    assert_eq!(editor.undo.len(), 1);

    // Undo takes the point out and gives the old selection back.
    assert!(editor.undo(&mut network, None));
    assert_eq!(network.lines[0].points.len(), 6);
    assert_eq!(editor.selection, [(1, 0)]);
}

#[test]
fn a_double_click_far_from_every_line_inserts_nothing() {
    let mut network = network();
    let mut editor = RailEditor::default();

    // 300 m south of "a", 700 m north of "b".
    let hit = on_the_ground(0.002, -0.0027);
    assert_eq!(editor.insert_point(&mut network, hit), None);
    assert_eq!(network.point_count(), 9);
    assert!(editor.undo.is_empty());
}

#[test]
fn delete_removes_the_selection_but_leaves_a_line_two_points() {
    let mut network = network();
    let mut editor = RailEditor::default();

    // Nothing selected, nothing happens, nothing recorded.
    assert!(!editor.delete_selection(&mut network));
    assert!(editor.undo.is_empty());

    editor.select((0, 1), false);
    editor.select((0, 3), true);
    assert!(editor.delete_selection(&mut network));
    assert_eq!(longitudes(&network, 0), [0.0, 0.004, 0.005]);
    assert!(editor.selection.is_empty());
    assert_eq!(editor.anchor, None);
    assert_eq!(editor.undo.len(), 1);

    // All of "b" selected: the first two stay, the one with the highest index goes.
    editor.select((1, 0), false);
    editor.select((1, 2), true);
    assert!(editor.delete_selection(&mut network));
    assert_eq!(longitudes(&network, 1), [0.0, 0.001]);

    // Selected again, nothing can go, so nothing is recorded and the selection stays.
    editor.select((1, 0), false);
    editor.select((1, 1), true);
    assert!(!editor.delete_selection(&mut network));
    assert_eq!(longitudes(&network, 1), [0.0, 0.001]);
    assert_eq!(editor.undo.len(), 2);
    assert_eq!(editor.selection, [(1, 0), (1, 1)]);

    // A stale index, from a selection that outlived its point, is dropped first.
    editor.selection = vec![(0, 10), (5, 0)];
    assert!(!editor.delete_selection(&mut network));
}

#[test]
fn undo_and_redo_walk_the_stack_and_a_new_edit_forgets_the_redo() {
    let mut network = network();
    let mut editor = RailEditor::default();

    editor.select((0, 2), false);
    assert!(editor.delete_selection(&mut network));
    assert_eq!(longitudes(&network, 0).len(), 5);

    editor.select((0, 0), false);
    assert!(editor.delete_selection(&mut network));
    assert_eq!(longitudes(&network, 0), [0.001, 0.003, 0.004, 0.005]);

    // Nothing to redo yet.
    assert!(!editor.redo(&mut network, None));

    assert!(editor.undo(&mut network, None));
    assert_eq!(longitudes(&network, 0), [0.0, 0.001, 0.003, 0.004, 0.005]);
    assert_eq!(editor.selection, [(0, 0)]);
    assert_eq!(editor.anchor, Some((0, 0)));

    assert!(editor.undo(&mut network, None));
    assert_eq!(longitudes(&network, 0).len(), 6);
    assert_eq!(editor.selection, [(0, 2)]);
    assert!(!editor.undo(&mut network, None));

    assert!(editor.redo(&mut network, None));
    assert_eq!(longitudes(&network, 0).len(), 5);
    // Back where the second undo started from, with the point that was about to be deleted
    // selected again.
    assert_eq!(editor.selection, [(0, 0)]);

    // A new edit from here drops the second delete from the redo stack.
    editor.select((0, 3), false);
    assert!(editor.delete_selection(&mut network));
    assert!(!editor.redo(&mut network, None));
    assert_eq!(longitudes(&network, 0), [0.0, 0.001, 0.003, 0.005]);

    // And the undo stack still walks all the way back.
    assert!(editor.undo(&mut network, None));
    assert!(editor.undo(&mut network, None));
    assert!(!editor.undo(&mut network, None));
    assert_eq!(longitudes(&network, 0).len(), 6);
}

#[test]
fn undo_keeps_the_runs_samples_and_resamples_the_points_it_moves() {
    let mut network = network();
    let mut editor = RailEditor::default();

    // An edit recorded before the startup sampling landed, so the snapshot has no samples:
    // the third point of "a" dragged a little east.
    editor.select((0, 2), false);
    editor.record(&network);
    network.lines[0].points[2].longitude += 0.0002;

    // The sampling lands: every point has a sample, and the dragged one no cached terrain.
    for (index, point) in network.lines[0].points.iter_mut().enumerate() {
        point.sampled = Some(10.0 + index as f64);
        point.terrain = Some(9.0 + index as f64);
    }
    network.lines[0].points[2].terrain = None;

    // Undone without a sampler, the points that stayed keep the run's samples, and the one
    // that comes back comes back as the snapshot had it, unsampled.
    assert!(editor.undo(&mut network, None));
    for (index, point) in network.lines[0].points.iter().enumerate() {
        if index == 2 {
            assert!((point.longitude - 0.002).abs() < 1e-9);
            assert_eq!(point.sampled, None);
            assert_eq!(point.terrain, None);
        } else {
            assert_eq!(point.sampled, Some(10.0 + index as f64), "{index}");
            assert_eq!(point.terrain, Some(9.0 + index as f64), "{index}");
        }
    }

    // Redone with a sampler, the point that moves is sampled where it goes. A sampler with
    // no terrain on it answers None and drops the cached terrain, as every resample does,
    // which tells a resampled point from one left as the snapshot had it, with the sample
    // the drag gave it.
    network.lines[0].points[2].sampled = Some(99.0);
    let mut sampler = RailSampler(TerrainHeightSampler::load(std::iter::empty::<&Path>()).unwrap());
    assert!(editor.redo(&mut network, Some(&mut sampler)));
    for (index, point) in network.lines[0].points.iter().enumerate() {
        if index == 2 {
            assert!((point.longitude - 0.0022).abs() < 1e-9);
            assert_eq!(point.sampled, None);
            assert_eq!(point.terrain, None);
        } else {
            assert_eq!(point.sampled, Some(10.0 + index as f64), "{index}");
        }
    }
}

#[test]
fn a_restored_selection_is_clamped_to_the_points_that_exist() {
    let mut network = network();
    let mut editor = RailEditor::default();

    // A snapshot that names points the network no longer has, as if the lines had been
    // changed under the stack.
    editor.select((0, 5), false);
    editor.record(&network);
    network.lines[0].points.truncate(3);

    assert!(editor.undo(&mut network, None));
    assert_eq!(network.lines[0].points.len(), 6);
    assert_eq!(editor.selection, [(0, 5)]);

    editor.selection = vec![(0, 1), (0, 5), (1, 2)];
    editor.anchor = Some((0, 5));
    editor.record(&network);
    network.lines[0].points.truncate(3);
    network.lines[1].points.truncate(2);
    // The snapshot holds six and three points; restoring puts them back, so clamping only
    // shows when the snapshot itself is short. Make one that is.
    editor.undo.last_mut().unwrap().points[0].truncate(3);
    assert!(editor.undo(&mut network, None));
    assert_eq!(editor.selection, [(0, 1), (1, 2)]);
    assert_eq!(editor.anchor, None);
}

#[test]
fn the_undo_stack_holds_two_hundred_edits() {
    let network = network();
    let mut editor = RailEditor::default();

    for _ in 0..UNDO_DEPTH + 50 {
        editor.record(&network);
    }
    assert_eq!(editor.undo.len(), UNDO_DEPTH);

    let mut network = network;
    let mut undone = 0;
    while editor.undo(&mut network, None) {
        undone += 1;
    }
    assert_eq!(undone, UNDO_DEPTH);
    assert_eq!(editor.redo.len(), UNDO_DEPTH);
}

#[test]
fn a_disc_is_sized_and_coloured_by_its_mode_and_white_when_selected() {
    let line = Color::srgb(0.0, 1.0, 0.0);

    let (ground, ground_colour) = disc_style(PointMode::Ground, false, line);
    let (fixed, fixed_colour) = disc_style(PointMode::Fixed, false, line);
    let (between, between_colour) = disc_style(PointMode::Between, false, line);

    assert_eq!(ground_colour, line);
    assert_ne!(fixed_colour, line);
    assert_ne!(between_colour, line);
    assert_ne!(fixed_colour, between_colour);
    assert!(
        between < ground && ground < fixed,
        "{between} {ground} {fixed}"
    );
    // A third larger, near enough.
    assert!((fixed / ground - 4.0 / 3.0).abs() < 0.1, "{fixed} {ground}");

    // Selected, every mode is white and larger, and the fixed point is still the largest.
    for (mode, unselected) in [
        (PointMode::Ground, ground),
        (PointMode::Fixed, fixed),
        (PointMode::Between, between),
    ] {
        let (pixels, colour) = disc_style(mode, true, line);
        assert_eq!(colour, Color::WHITE);
        assert!(pixels > unselected, "{mode:?}");
    }
    let (selected_fixed, _) = disc_style(PointMode::Fixed, true, line);
    let (selected_ground, _) = disc_style(PointMode::Ground, true, line);
    assert!(selected_fixed > selected_ground);
}

#[test]
fn the_selection_is_sorted_and_clamped_line_by_line() {
    let network = network();
    let selection = [(0, 4), (1, 2), (0, 1), (0, 4), (0, 9), (3, 0), (1, 0)];

    assert_eq!(
        selected_by_line(&selection, &network),
        [vec![1, 4], vec![0, 2]]
    );
}

#[test]
fn distance_to_a_segment_is_to_its_nearest_point() {
    let a = DVec3::ZERO;
    let b = DVec3::new(10.0, 0.0, 0.0);

    assert!((distance_to_segment(DVec3::new(5.0, 3.0, 0.0), a, b) - 3.0).abs() < 1e-12);
    assert!((distance_to_segment(DVec3::new(-4.0, 3.0, 0.0), a, b) - 5.0).abs() < 1e-12);
    assert!((distance_to_segment(DVec3::new(13.0, 4.0, 0.0), a, b) - 5.0).abs() < 1e-12);
    // A degenerate segment is a point.
    assert!((distance_to_segment(DVec3::new(0.0, 2.0, 0.0), a, a) - 2.0).abs() < 1e-12);
}
