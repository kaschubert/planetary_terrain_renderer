//! The displacement and the start of a drag, checked with numbers: the displacement at
//! Auckland where the network is, and the drag on the editor's equator network. The frame
//! itself is tested beside its own module, in frame/tests.rs.

use super::*;
use crate::plugins::rail_editor::frame::tests::AUCKLAND;
use crate::plugins::rail_editor::tests::network;
use bevy_terrain::math::unit_position;

fn ground_point(mode: PointMode, height: Option<f64>, sampled: f64) -> RailPoint {
    RailPoint {
        longitude: AUCKLAND.0,
        latitude: AUCKLAND.1,
        mode,
        height,
        terrain: None,
        sampled: Some(sampled),
    }
}

/// A gizmo total along one local axis: x east, y up, z south.
fn east(metres: f64) -> DVec3 {
    DVec3::new(metres, 0.0, 0.0)
}

fn up(metres: f64) -> DVec3 {
    DVec3::new(0.0, metres, 0.0)
}

fn north(metres: f64) -> DVec3 {
    DVec3::new(0.0, 0.0, -metres)
}

#[test]
fn a_hundred_metres_east_at_auckland_moves_the_longitude_and_nothing_else() {
    let TerrainShape::Spheroid {
        major_axis,
        minor_axis,
    } = TerrainShape::WGS84
    else {
        unreachable!()
    };
    let point = ground_point(PointMode::Ground, Some(2.0), 30.0);
    let snapshot = PointSnapshot::new(0, 0, &point, 32.0);

    let moved = displace(&snapshot, east(100.0));

    // A chord of 100 m along the parallel, whose radius is the major axis times the cosine
    // of the latitude, since the unit sphere is scaled onto the spheroid.
    let expected = (100.0 / (major_axis * AUCKLAND.1.to_radians().cos())).to_degrees();
    let shift = moved.longitude - AUCKLAND.0;
    assert!(
        ((shift - expected) / expected).abs() < 1e-4,
        "shifted {shift} degrees, expected {expected}"
    );
    assert!(
        (moved.latitude - AUCKLAND.1).abs() < 1e-8,
        "{}",
        moved.latitude
    );

    assert_eq!(moved.mode, PointMode::Ground);
    assert_eq!(moved.height, Some(2.0));
    assert_eq!(moved.sampled, Some(30.0));

    // Half a kilometre north moves the latitude by the step over the radius in the
    // meridian, which lies between the two axes; the geometric mean of them is within a
    // part in a thousand of it at this latitude.
    let moved = displace(&snapshot, north(500.0));
    let expected = (500.0 / (major_axis * minor_axis).sqrt()).to_degrees();
    let shift = moved.latitude - AUCKLAND.1;
    assert!(
        ((shift - expected) / expected).abs() < 2e-3,
        "shifted {shift} degrees, expected {expected}"
    );
    assert!(
        (moved.longitude - AUCKLAND.0).abs() < 1e-8,
        "{}",
        moved.longitude
    );
}

#[test]
fn a_vertical_drag_changes_the_height_by_the_points_mode() {
    let lift = 3.0;

    // A ground point: the offset, not the terrain.
    let point = ground_point(PointMode::Ground, Some(2.0), 30.0);
    let snapshot = PointSnapshot::new(0, 0, &point, 32.0);
    let moved = displace(&snapshot, up(lift));
    assert_eq!(moved.mode, PointMode::Ground);
    assert!(
        (moved.height.unwrap() - 5.0).abs() < 1e-9,
        "{:?}",
        moved.height
    );
    assert!((moved.longitude - AUCKLAND.0).abs() < 1e-9);
    assert!((moved.latitude - AUCKLAND.1).abs() < 1e-9);

    // Down as well as up.
    let moved = displace(&snapshot, up(-lift));
    assert!(
        (moved.height.unwrap() + 1.0).abs() < 1e-9,
        "{:?}",
        moved.height
    );

    // A fixed point: its own height.
    let point = ground_point(PointMode::Fixed, Some(40.0), 30.0);
    let snapshot = PointSnapshot::new(0, 0, &point, 40.0);
    let moved = displace(&snapshot, up(lift));
    assert_eq!(moved.mode, PointMode::Fixed);
    assert!(
        (moved.height.unwrap() - 43.0).abs() < 1e-9,
        "{:?}",
        moved.height
    );

    // A between point becomes fixed where it was resolved, plus the lift.
    let point = ground_point(PointMode::Between, None, 30.0);
    let snapshot = PointSnapshot::new(0, 0, &point, 25.0);
    let moved = displace(&snapshot, up(lift));
    assert_eq!(moved.mode, PointMode::Fixed);
    assert!(
        (moved.height.unwrap() - 28.0).abs() < 1e-9,
        "{:?}",
        moved.height
    );
}

#[test]
fn a_rounding_error_of_lift_leaves_a_between_point_between() {
    let point = ground_point(PointMode::Between, None, 30.0);
    let snapshot = PointSnapshot::new(0, 0, &point, 25.0);

    let moved = displace(&snapshot, east(20.0) + up(1e-9));
    assert_eq!(moved.mode, PointMode::Between);
    assert_eq!(moved.height, None);
    assert!(moved.longitude > AUCKLAND.0);

    // A centimetre is meant.
    let moved = displace(&snapshot, up(0.01));
    assert_eq!(moved.mode, PointMode::Fixed);
    assert!(
        (moved.height.unwrap() - 25.01).abs() < 1e-9,
        "{:?}",
        moved.height
    );
}

#[test]
fn a_drag_along_the_ground_lifts_no_point_however_far_from_the_anchor() {
    // A between point three kilometres east of an anchor at Britomart, the far end of a run
    // dragged two hundred metres east along the green square.
    let mut far = ground_point(PointMode::Between, None, 30.0);
    far.longitude += 0.034;
    let snapshot = PointSnapshot::new(0, 7, &far, 25.0);
    let along_the_ground = east(200.0);

    // Read in the anchor's frame the drag has a lift here: level at the anchor is a slope
    // three kilometres along, by the curve of the planet, and nine centimetres is well over
    // what turns a between point fixed.
    let anchor = Frame::at_unit(unit_position(AUCKLAND.0, AUCKLAND.1));
    let in_anchors_frame = anchor.displacement(along_the_ground).dot(snapshot.frame.up);
    assert!(in_anchors_frame.abs() > 0.05, "{in_anchors_frame}");

    // Read in its own, as it is, there is none.
    let moved = displace(&snapshot, along_the_ground);
    assert_eq!(moved.mode, PointMode::Between);
    assert_eq!(moved.height, None);
    assert!(moved.longitude > far.longitude);

    // Nor does a ground point's offset drift, twenty kilometres from the anchor.
    let mut ground = ground_point(PointMode::Ground, Some(2.0), 30.0);
    ground.latitude += 0.18;
    let snapshot = PointSnapshot::new(0, 8, &ground, 32.0);
    let moved = displace(&snapshot, north(16.0));
    assert_eq!(moved.height, Some(2.0));
}

#[test]
fn a_drag_begins_with_every_selected_point_at_its_resolved_height() {
    // The equator network with a tunnel through "a": portals fixed at 10 m and 40 m and the
    // two points inside between, so they resolve onto the chord at 20 m and 30 m.
    let mut network = network();
    {
        let points = &mut network.lines[0].points;
        points[1].mode = PointMode::Fixed;
        points[1].height = Some(10.0);
        points[4].mode = PointMode::Fixed;
        points[4].height = Some(40.0);
        for point in &mut points[2..4] {
            point.mode = PointMode::Between;
            point.height = None;
        }
    }
    network.resolve();

    let mut editor = RailEditor::default();
    editor.select((0, 1), false);
    editor.select((0, 4), true);
    editor.select((1, 0), true);
    // A stale index, from a selection that outlived its point, is left out without a word.
    editor.selection.push((0, 10));
    let cell = CellCoord::new(3, -2, 7);

    let drag = Drag::begin(&editor, &network, cell).unwrap();
    assert_eq!(drag.cell, cell);
    assert!(!drag.ended);
    assert_eq!(drag.applied, None);

    let taken: Vec<(usize, usize)> = drag
        .points
        .iter()
        .map(|snapshot| (snapshot.line, snapshot.index))
        .collect();
    assert_eq!(taken, [(0, 1), (0, 2), (0, 3), (0, 4), (1, 0)]);

    for (snapshot, expected) in drag.points.iter().zip([10.0, 20.0, 30.0, 40.0, 0.0]) {
        let place = format!("{} {}", snapshot.line, snapshot.index);
        assert!(
            (snapshot.resolved - expected).abs() < 1e-6,
            "{place}: {}",
            snapshot.resolved
        );
        // Where the line draws it, and the point as it is.
        let line = &network.lines[snapshot.line];
        assert!(
            snapshot.position.distance(line.positions[snapshot.index]) < 1e-9,
            "{place}"
        );
        assert_eq!(snapshot.point, line.points[snapshot.index], "{place}");
    }

    // With the anchor not among the points there is nothing to drag from.
    editor.anchor = Some((0, 5));
    assert!(Drag::begin(&editor, &network, cell).is_none());
    editor.clear_selection();
    assert!(Drag::begin(&editor, &network, cell).is_none());
}

#[test]
fn a_press_without_a_move_is_not_a_drag_until_the_mouse_moves() {
    let network = network();
    let mut editor = RailEditor::default();
    editor.select((0, 1), false);
    let mut drag = Drag::begin(&editor, &network, CellCoord::default()).unwrap();

    // The nothing the gizmo reports on the frame of the press is not news; the first move
    // is, and a frame that repeats it is not.
    assert!(!drag.is_new(DVec3::ZERO));
    assert!(drag.is_new(east(1.0)));
    drag.applied = Some(east(1.0));
    assert!(!drag.is_new(east(1.0)));
    assert!(drag.is_new(east(2.0)));

    // Back where it started is a move too, once there has been one.
    assert!(drag.is_new(DVec3::ZERO));
}
