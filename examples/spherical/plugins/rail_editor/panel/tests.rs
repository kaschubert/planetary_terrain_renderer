//! The words the panel puts things into, the pose a fly-to builds and the rules that dim a
//! button, each checked with numbers; the panel's own entities need a window and are not.

use super::fly_to::{fly_to_pose, step_moved};
use super::readout::{grade_text, height_text, readout_text, terrain_text};
use super::*;
use crate::plugins::auckland_rail::MovedGround;
use crate::plugins::rail_editor::frame::tests::{AUCKLAND, degrees_between};
use crate::plugins::rail_editor::tests::network;
use crate::plugins::shared::rail_network::{MAX_GRADE, RailPoint};
use bevy_terrain::math::unit_position;

fn point(mode: PointMode, height: Option<f64>) -> RailPoint {
    RailPoint {
        longitude: AUCKLAND.0,
        latitude: AUCKLAND.1,
        mode,
        height,
        terrain: Some(38.4),
        sampled: Some(38.9),
    }
}

#[test]
fn the_height_reads_by_its_mode() {
    assert_eq!(
        height_text(&point(PointMode::Ground, Some(3.2)), 42.1),
        "+3.2 m above ground"
    );
    assert_eq!(
        height_text(&point(PointMode::Ground, Some(0.0)), 38.9),
        "+0.0 m above ground"
    );
    assert_eq!(
        height_text(&point(PointMode::Ground, Some(-1.5)), 37.4),
        "-1.5 m above ground"
    );
    assert_eq!(
        height_text(&point(PointMode::Fixed, Some(41.0)), 41.0),
        "41.0 m"
    );
    // A between point stores nothing; the chord's height is what is shown.
    assert_eq!(
        height_text(&point(PointMode::Between, None), 38.44),
        "on the chord, 38.4 m"
    );
}

#[test]
fn the_terrain_is_the_sample_else_the_cache_else_unknown() {
    let mut point = point(PointMode::Ground, Some(0.0));
    assert_eq!(terrain_text(&point), "38.9 m");

    point.sampled = None;
    assert_eq!(terrain_text(&point), "38.4 m");

    point.terrain = None;
    assert_eq!(terrain_text(&point), "?");
}

#[test]
fn the_grades_are_percentages_with_the_steep_side_marked() {
    assert_eq!(
        grade_text(Some(0.012), Some(0.041)),
        "grades 1.2 % / 4.1 % (steep)"
    );
    // The ends of a line have one segment only.
    assert_eq!(grade_text(None, Some(0.012)), "grades - / 1.2 %");
    assert_eq!(grade_text(Some(0.05), None), "grades 5.0 % (steep) / -");
    // At the limit is not over it.
    assert_eq!(
        grade_text(Some(MAX_GRADE), Some(MAX_GRADE + 1e-6)),
        "grades 3.5 % / 3.5 % (steep)"
    );
    // Two points at the same place, one above the other.
    assert_eq!(
        grade_text(Some(f64::INFINITY), Some(0.0)),
        "grades vertical (steep) / 0.0 %"
    );
}

#[test]
fn the_moved_ground_sentence_counts_the_points_and_the_cursor() {
    assert_eq!(
        moved_ground_text(37, None),
        "startup found the ground moved > 1 m under 37 points"
    );
    assert_eq!(
        moved_ground_text(1, None),
        "startup found the ground moved > 1 m under 1 point"
    );
    // The cursor counts from one for the reader.
    assert_eq!(
        moved_ground_text(37, Some(11)),
        "startup found the ground moved > 1 m under 37 points, at 12 of 37"
    );
}

#[test]
fn the_readout_without_an_anchor_says_so() {
    let network = network();
    let mut editor = RailEditor::default();
    assert_eq!(readout_text(&editor, &network), "no point selected");

    // A selection that has lost its anchor, as after an undo can leave it.
    editor.selection = vec![(0, 1), (0, 2)];
    assert_eq!(
        readout_text(&editor, &network),
        "no point selected; 2 selected"
    );

    // An anchor that names no point counts as none.
    editor.anchor = Some((0, 10));
    assert_eq!(
        readout_text(&editor, &network),
        "no point selected; 2 selected"
    );
}

#[test]
fn the_readout_describes_the_anchor_line_by_line() {
    let mut network = network();
    {
        let points = &mut network.lines[0].points;
        for (point, sampled) in points.iter_mut().zip([10.0, 12.0, 30.0, 28.0, 15.0, 11.0]) {
            point.sampled = Some(sampled);
        }
        points[2].height = Some(2.0);
    }
    network.resolve();

    let mut editor = RailEditor::default();
    editor.select((0, 1), false);
    editor.select((0, 2), true);

    let text = readout_text(&editor, &network);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 5, "{text}");
    assert_eq!(lines[0], "a 2, 2 selected");
    assert_eq!(lines[1], "lat 0.00000  lon 0.00200");
    assert_eq!(lines[2], "terrain 30.0 m");
    assert_eq!(lines[3], "ground: +2.0 m above ground");
    // From 12 m to 32 m over 111 m is 18 per cent, and 32 to 28 is 3.6: both too steep.
    assert_eq!(lines[4], "grades 18.0 % (steep) / 3.6 % (steep)");

    // The first point of a line has no segment before it.
    editor.select((1, 0), false);
    let text = readout_text(&editor, &network);
    assert!(text.starts_with("b 0\n"), "{text}");
    assert!(text.ends_with("grades - / 0.0 %"), "{text}");
}

#[test]
fn a_fly_to_looks_at_the_point_from_the_south_and_above() {
    let unit = unit_position(AUCKLAND.0, AUCKLAND.1);
    let target = TerrainShape::WGS84.position_unit_to_local(unit, 30.0);
    let (position, rotation) = fly_to_pose(target);

    // Up as the start pose has it, and north along the meridian at the target's height. The
    // chord north is tangent to the spheroid where up is the parametric direction, a tenth
    // of a degree off the normal here, so the two components leak into each other by under
    // a metre over 400.
    let up = unit;
    let north = (TerrainShape::WGS84
        .position_unit_to_local(unit_position(AUCKLAND.0, AUCKLAND.1 + 0.001), 30.0)
        - target)
        .normalize();
    let east = north.cross(up);

    let offset = position - target;
    assert!((offset.dot(up) - 400.0).abs() < 1.0, "{}", offset.dot(up));
    assert!(
        (offset.dot(north) + 400.0).abs() < 1.0,
        "{}",
        offset.dot(north)
    );
    assert!(offset.dot(east).abs() < 1.0, "{}", offset.dot(east));

    // Looking at the point, which is 45 degrees down and to the north.
    let forward = (rotation * -Vec3::Z).as_dvec3();
    let to_target = (target - position).normalize();
    assert!(
        degrees_between(forward, to_target) < 1.0,
        "{}",
        degrees_between(forward, to_target)
    );
    assert!(
        (degrees_between(forward, -up) - 45.0).abs() < 1.0,
        "{}",
        degrees_between(forward, -up)
    );

    // No roll: the camera's right is level, and its up leans towards the sky and not away
    // from it. A camera tilted 45 degrees down cannot have its up along the normal, but
    // its up and the normal lie in one vertical plane.
    let right = (rotation * Vec3::X).as_dvec3();
    let camera_up = (rotation * Vec3::Y).as_dvec3();
    assert!(
        right.dot(up).abs() < 1.0_f64.to_radians().sin(),
        "{}",
        right.dot(up)
    );
    assert!(camera_up.dot(up) > 0.0);
    assert!(
        (degrees_between(camera_up, up) - 45.0).abs() < 1.0,
        "{}",
        degrees_between(camera_up, up)
    );
}

#[test]
fn the_moved_points_are_stepped_round_both_ways_and_selected() {
    let mut network = network();
    let mut editor = RailEditor::default();
    // Named by place, as the sampling's landing names them.
    let at = |line: usize, index: usize, delta: f64| {
        let point = &network.lines[line].points[index];
        MovedGround {
            line,
            longitude: point.longitude,
            latitude: point.latitude,
            delta,
        }
    };
    let moved = vec![at(0, 1, 2.0), at(0, 4, -3.5), at(1, 0, 1.2)];
    let mut cursor = None;

    // Forward from nothing is the first, and the point is where the line has it.
    let target = step_moved(true, &mut cursor, &moved, &network, &mut editor).unwrap();
    assert_eq!(cursor, Some(0));
    assert_eq!(editor.selection, [(0, 1)]);
    assert_eq!(editor.anchor, Some((0, 1)));
    assert!(target.distance(network.lines[0].positions[1]) < 1e-6);

    assert!(step_moved(true, &mut cursor, &moved, &network, &mut editor).is_some());
    assert_eq!(editor.selection, [(0, 4)]);
    assert!(step_moved(true, &mut cursor, &moved, &network, &mut editor).is_some());
    assert_eq!(editor.selection, [(1, 0)]);

    // Round the end, and back again the other way.
    assert!(step_moved(true, &mut cursor, &moved, &network, &mut editor).is_some());
    assert_eq!(cursor, Some(0));
    assert!(step_moved(false, &mut cursor, &moved, &network, &mut editor).is_some());
    assert_eq!(cursor, Some(2));
    assert_eq!(editor.selection, [(1, 0)]);

    // Backward from nothing is the last.
    let mut cursor = None;
    assert!(step_moved(false, &mut cursor, &moved, &network, &mut editor).is_some());
    assert_eq!(cursor, Some(2));

    // A point added before the first entry's point on its line shifts the index, and the
    // entry still finds its point.
    network.lines[0]
        .points
        .insert(0, RailPoint::ground(-0.001, 0.0));
    let mut cursor = None;
    let target = step_moved(true, &mut cursor, &moved, &network, &mut editor).unwrap();
    assert_eq!(editor.selection, [(0, 2)]);
    assert!(target.distance(network.lines[0].positions[1]) < 1e-6);

    // An entry whose point has been moved or removed steps the cursor but selects nothing.
    let stale = vec![MovedGround {
        line: 0,
        longitude: 0.5,
        latitude: 0.5,
        delta: 1.0,
    }];
    editor.clear_selection();
    let mut cursor = None;
    assert!(step_moved(true, &mut cursor, &stale, &network, &mut editor).is_none());
    assert_eq!(cursor, Some(0));
    assert!(editor.selection.is_empty());

    // And no moved points is nothing to step through.
    assert!(step_moved(true, &mut cursor, &[], &network, &mut editor).is_none());
}

#[test]
fn a_button_is_disabled_when_it_cannot_apply() {
    let all = Readiness {
        selection_empty: false,
        can_span: true,
        undo_empty: false,
        redo_empty: false,
        can_save: true,
        lines_hidden: false,
    };
    let none = Readiness {
        selection_empty: true,
        can_span: false,
        undo_empty: true,
        redo_empty: true,
        can_save: false,
        lines_hidden: false,
    };
    let edits = [
        Action::Mode(PointMode::Ground),
        Action::Mode(PointMode::Fixed),
        Action::Mode(PointMode::Between),
        Action::DropToGround,
        Action::SelectSteepRun,
    ];

    for action in edits {
        assert!(action.enabled(all), "{action:?}");
        assert!(!action.enabled(none), "{action:?}");
        // The selection alone decides.
        assert!(
            action.enabled(Readiness {
                selection_empty: false,
                ..none
            }),
            "{action:?}"
        );
    }

    // A span needs more than a selection: three points of it on one line.
    assert!(Action::SpanSelection.enabled(all));
    assert!(!Action::SpanSelection.enabled(Readiness {
        can_span: false,
        ..all
    }));
    assert!(!Action::SpanSelection.enabled(Readiness {
        selection_empty: false,
        ..none
    }));

    assert!(Action::Undo.enabled(all));
    assert!(!Action::Undo.enabled(Readiness {
        undo_empty: true,
        ..all
    }));
    assert!(Action::Redo.enabled(all));
    assert!(!Action::Redo.enabled(Readiness {
        redo_empty: true,
        ..all
    }));
    assert!(Action::Save.enabled(all));
    assert!(!Action::Save.enabled(Readiness {
        can_save: false,
        ..all
    }));

    // The moved-ground buttons are governed by their row being shown, not by these.
    assert!(Action::PreviousMoved.enabled(none));
    assert!(Action::NextMoved.enabled(none));

    // With the lines hidden by F4 nothing but save applies, whatever else is ready.
    let hidden = Readiness {
        lines_hidden: true,
        ..all
    };
    for action in edits {
        assert!(!action.enabled(hidden), "{action:?}");
    }
    for action in [
        Action::SpanSelection,
        Action::Undo,
        Action::Redo,
        Action::PreviousMoved,
        Action::NextMoved,
    ] {
        assert!(!action.enabled(hidden), "{action:?}");
    }
    assert!(Action::Save.enabled(hidden));
}

#[test]
fn a_disabled_button_takes_no_colour_from_the_pointer() {
    assert_eq!(
        button_colour(Interaction::Hovered, false, false),
        DISABLED_COLOUR
    );
    assert_eq!(
        button_colour(Interaction::Pressed, false, false),
        DISABLED_COLOUR
    );
    assert_eq!(button_colour(Interaction::None, true, false), BUTTON_COLOUR);
    assert_eq!(button_colour(Interaction::None, true, true), CURRENT_COLOUR);
    // Hover and press show over the current mode too, so the button still answers.
    assert_eq!(
        button_colour(Interaction::Hovered, true, true),
        HOVER_COLOUR
    );
    assert_eq!(
        button_colour(Interaction::Pressed, true, true),
        PRESSED_COLOUR
    );
}
