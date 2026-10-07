//! The frames checked with numbers on lines built from points with their terrain heights
//! given, so that a line resolves without a file or a terrain: a straight line along the
//! parallel through Britomart, level and then climbing, evenly spaced and then spaced as
//! the network's worst stretch is, a quarter circle on the ground there, and a short chord
//! into a sharp turn.

use super::*;
use crate::plugins::rail_editor::frame::tests::{AUCKLAND, degrees_between};
use crate::plugins::shared::rail_network::{PointMode, RailPoint, lon_lat_from_unit};
use bevy_terrain::math::unit_position;

/// A ground point with no offset and the terrain under it given.
fn point(longitude: f64, latitude: f64, terrain: f64) -> RailPoint {
    RailPoint {
        longitude,
        latitude,
        mode: PointMode::Ground,
        height: Some(0.0),
        terrain: None,
        sampled: Some(terrain),
    }
}

fn line(points: Vec<RailPoint>) -> RailLine {
    let mut line = RailLine {
        name: "test".to_string(),
        points,
        positions: Vec::new(),
    };
    line.resolve();
    line
}

/// So many points due east from Britomart along the parallel, the metres apart given, with
/// the terrain under each a function of its distance along.
pub(crate) fn eastward(count: usize, spacing: f64, terrain: impl Fn(f64) -> f64) -> RailLine {
    eastward_at(
        &(0..count)
            .map(|index| index as f64 * spacing)
            .collect::<Vec<_>>(),
        terrain,
    )
}

/// Points due east from Britomart along the parallel at the metres along given, with the
/// terrain under each a function of its distance along. The parallel's radius is the major
/// axis times the cosine of the latitude, since the unit sphere is scaled onto the spheroid,
/// so a chord along it is in proportion to the longitude.
fn eastward_at(along: &[f64], terrain: impl Fn(f64) -> f64) -> RailLine {
    let TerrainShape::Spheroid { major_axis, .. } = TerrainShape::WGS84 else {
        unreachable!()
    };
    let degrees_per_metre = (1.0 / (major_axis * AUCKLAND.1.to_radians().cos())).to_degrees();

    line(
        along
            .iter()
            .map(|&along| {
                point(
                    AUCKLAND.0 + along * degrees_per_metre,
                    AUCKLAND.1,
                    terrain(along),
                )
            })
            .collect(),
    )
}

/// Points on the ground at Britomart, each so many metres east and north of it. Each is laid
/// out in the plane tangent there and then dropped onto the ellipsoid under it, which over a
/// kilometre is eight centimetres and no change of place.
fn on_ground(offsets: impl IntoIterator<Item = (f64, f64)>) -> RailLine {
    let unit = unit_position(AUCKLAND.0, AUCKLAND.1);
    let ground = Frame::at_unit(unit);
    let centre = TerrainShape::WGS84.position_unit_to_local(unit, 0.0);

    line(
        offsets
            .into_iter()
            .map(|(east, north)| {
                let position = centre + ground.east * east + ground.north * north;
                let (longitude, latitude) = lon_lat_from_unit(unit_under(position));
                point(longitude, latitude, 0.0)
            })
            .collect(),
    )
}

/// A quarter circle of the radius given on the ground at Britomart, as points so many metres
/// apart along it, from due east of the centre round towards due north, so it turns left.
pub(crate) fn quarter_circle(radius: f64, spacing: f64) -> RailLine {
    let count = (std::f64::consts::FRAC_PI_2 * radius / spacing) as usize;

    on_ground((0..=count).map(|index| {
        let angle = index as f64 * spacing / radius;
        (angle.cos() * radius, angle.sin() * radius)
    }))
}

/// The frames are a step apart by their distances and by the chords between them, which
/// within the tolerance given are the same thing, and the first is at the first point.
fn assert_evenly_spaced(frames: &[TrackFrame], line: &RailLine, step: f64, tolerance: f64) {
    assert!(frames.len() > 2, "{} frames", frames.len());
    assert!(frames[0].position.distance(line.positions[0]) < 1e-6);
    assert_eq!(frames[0].distance, 0.0);

    for (index, pair) in frames.windows(2).enumerate() {
        let chord = pair[0].position.distance(pair[1].position);
        assert!(
            (chord - step).abs() < step * tolerance,
            "frames {index} and {}: {chord} m apart",
            index + 1
        );
        assert!(
            (pair[1].distance - pair[0].distance - step).abs() < step * tolerance,
            "frames {index} and {}: {} m along",
            index + 1,
            pair[1].distance - pair[0].distance
        );
    }
}

#[test]
fn a_level_line_due_east_faces_east_and_stands_up_from_the_ground() {
    let line = eastward(41, 50.0, |_| 50.0);
    let frames = track_frames(&line, FRAME_STEP);

    // Two kilometres at 25 m: a frame at every multiple, the last one at the last point
    // give or take the 1.6 cm that fifty metres of height stretches two kilometres by.
    assert_eq!(frames.len(), 81);
    assert_evenly_spaced(&frames, &line, FRAME_STEP, 0.01);
    assert!(frames[80].position.distance(line.positions[40]) < 0.05);

    for (index, frame) in frames.iter().enumerate() {
        let ground = Frame::at_unit(unit_under(frame.position));
        let place = format!("frame {index}");

        assert!(
            degrees_between(frame.forward(), ground.east) < 0.1,
            "{place}"
        );
        assert!(degrees_between(frame.up(), ground.up) < 0.1, "{place}");
        assert!(
            degrees_between(frame.right(), -ground.north) < 0.1,
            "{place}"
        );
        assert!(frame.grade.abs() < 1e-4, "{place}: grade {}", frame.grade);

        // The rotation is a rotation: the three axes it gives are unit and perpendicular.
        assert!((frame.rotation.length() - 1.0).abs() < 1e-12, "{place}");
        assert!(frame.forward().dot(frame.up()).abs() < 1e-12, "{place}");
        assert!(frame.forward().dot(frame.right()).abs() < 1e-12, "{place}");
        assert!(
            frame.right().cross(frame.up()).distance(-frame.forward()) < 1e-12,
            "{place}"
        );
    }
}

#[test]
fn a_climbing_line_pitches_up_by_its_grade_and_stays_level_across() {
    let grade = 0.035;
    let line = eastward(41, 50.0, |along| 50.0 + grade * along);
    let frames = track_frames(&line, FRAME_STEP);
    assert_evenly_spaced(&frames, &line, FRAME_STEP, 0.01);

    let pitch = grade.atan().to_degrees();
    for (index, frame) in frames.iter().enumerate() {
        let ground = Frame::at_unit(unit_under(frame.position));
        let place = format!("frame {index}");

        let forward = frame.forward();
        let level = (forward - ground.up * forward.dot(ground.up)).normalize();
        assert!(forward.dot(ground.up) > 0.0, "{place}: not climbing");
        assert!(
            (degrees_between(forward, level) - pitch).abs() < 0.1,
            "{place}: pitched {} degrees",
            degrees_between(forward, level)
        );
        assert!(
            frame.right().dot(ground.up).abs() < 1e-3,
            "{place}: right leans {}",
            frame.right().dot(ground.up)
        );
        assert!(
            (frame.grade - grade).abs() < 1e-3,
            "{place}: grade {}",
            frame.grade
        );
    }

    // Run the other way the same line descends.
    let mut reversed = line.points.clone();
    reversed.reverse();
    let reversed = self::line(reversed);
    for (index, frame) in track_frames(&reversed, FRAME_STEP).iter().enumerate() {
        assert!(
            (frame.grade + grade).abs() < 1e-3,
            "frame {index}: grade {}",
            frame.grade
        );
    }
}

#[test]
fn a_quarter_circle_is_resampled_evenly_and_turns_at_a_steady_rate() {
    let radius = 1000.0;
    let line = quarter_circle(radius, 50.0);
    let frames = track_frames(&line, FRAME_STEP);
    assert_evenly_spaced(&frames, &line, FRAME_STEP, 0.02);

    // The circle starts due east of its centre heading north, and the end tangent, being
    // that of the circle through the first three points, is the circle's own. The heading
    // is compared level, since forward lies in the ground, which leans the
    // geocentric-geodetic difference off the frame's level in the meridian.
    let start = Frame::at_unit(unit_under(frames[0].position));
    let forward = frames[0].forward();
    let heading = (forward - start.up * forward.dot(start.up)).normalize();
    assert!(
        degrees_between(heading, start.north) < 0.05,
        "set off {} degrees off north",
        degrees_between(heading, start.north)
    );

    // Consecutive forwards turn by the step over the radius, every pair, and all the same
    // way: anticlockwise seen from above, which is east round to north.
    let turn = FRAME_STEP / radius;
    for (index, pair) in frames.windows(2).enumerate() {
        let (a, b) = (pair[0].forward(), pair[1].forward());
        let angle = a.dot(b).clamp(-1.0, 1.0).acos();
        assert!(
            ((angle - turn) / turn).abs() < 0.1,
            "frames {index} and {}: turned {angle} rad, expected {turn}",
            index + 1
        );
        let ground = Frame::at_unit(unit_under(pair[0].position));
        assert!(
            a.cross(b).dot(ground.up) > 0.0,
            "frames {index} and {}: turned right",
            index + 1
        );
    }

    // On the ground, level: the grade is nothing whichever way the line runs, and up stays
    // within the geocentric-geodetic difference of the ground's, which it leans towards the
    // true vertical by on the stretches running north.
    for (index, frame) in frames.iter().enumerate() {
        let ground = Frame::at_unit(unit_under(frame.position));
        assert!(
            frame.grade.abs() < 1e-3,
            "frame {index}: grade {}",
            frame.grade
        );
        assert!(
            degrees_between(frame.up(), ground.up) < 0.2,
            "frame {index}"
        );
    }
}

/// The network's spacing at its worst: on the E-W line a chord of 3 m sits beside one of
/// 1191 m, and then 89 m beside 1867 m. The spline's speed along the long segment grows
/// several-fold from the short end, and a frame placed by interpolating the parameter alone
/// landed up to four metres off its distance. Along the parallel the spline is as good as
/// straight, so the chord between frames is their distance apart, and both are held to a
/// millimetre of the step, which is where Newton's method brings them.
#[test]
fn frames_stay_a_step_apart_where_a_long_segment_meets_a_short_one() {
    let worst = eastward_at(&[0.0, 3.0, 1194.0, 1283.0, 3150.0], |_| 20.0);
    let frames = track_frames(&worst, FRAME_STEP);
    assert_eq!(frames.len(), 127);
    assert_evenly_spaced(&frames, &worst, FRAME_STEP, 0.001 / FRAME_STEP);

    let lopsided = eastward_at(&[0.0, 30.0, 2030.0, 2060.0], |_| 20.0);
    let frames = track_frames(&lopsided, FRAME_STEP);
    assert_eq!(frames.len(), 83);
    assert_evenly_spaced(&frames, &lopsided, FRAME_STEP, 0.001 / FRAME_STEP);
}

/// The E-W line opens as this does: a chord of a score of metres, then one of hundreds
/// turning thirty degrees. The first frame faces along the first chord, give or take the
/// couple of degrees the circle through the first three points turns over so short a chord,
/// and from the other end the last frame faces back along it; the parabola through the three
/// faced both thirty-six degrees across the track.
#[test]
fn a_short_first_chord_into_a_turn_sets_off_along_that_chord() {
    let (sin, cos) = 30f64.to_radians().sin_cos();
    let offsets = [
        (0.0, 0.0),
        (20.0, 0.0),
        (20.0 + 300.0 * cos, 300.0 * sin),
        (20.0 + 600.0 * cos, 600.0 * sin),
    ];
    let line = on_ground(offsets);
    let frames = track_frames(&line, FRAME_STEP);
    let start = Frame::at_unit(unit_under(frames[0].position));
    assert!(
        degrees_between(frames[0].forward(), start.east) < 3.0,
        "set off {} degrees off the first chord",
        degrees_between(frames[0].forward(), start.east)
    );

    let reversed = on_ground(offsets.iter().rev().copied());
    let frames = track_frames(&reversed, FRAME_STEP);
    let last = frames.last().unwrap();
    let end = Frame::at_unit(unit_under(last.position));
    assert!(
        degrees_between(last.forward(), -end.east) < 3.0,
        "arrived {} degrees off the last chord",
        degrees_between(last.forward(), -end.east)
    );
}

/// Eighty metres at a step of 25 leaves five after the frame at 75, under half a step, so no
/// frame is added at the end; ninety leaves fifteen, over half, so one is.
#[test]
fn a_frame_stands_at_the_end_of_a_line_when_more_than_half_a_step_remains() {
    let short_of_a_frame = eastward(3, 40.0, |_| 0.0);
    let frames = track_frames(&short_of_a_frame, FRAME_STEP);
    assert_eq!(frames.len(), 4);
    assert_eq!(frames[3].distance, 75.0);

    let long_enough = eastward(3, 45.0, |_| 0.0);
    let frames = track_frames(&long_enough, FRAME_STEP);
    assert_eq!(frames.len(), 5);
    assert_eq!(frames[3].distance, 75.0);
    assert!((frames[4].distance - 90.0).abs() < 1e-3);
    assert!(frames[4].position.distance(long_enough.positions[2]) < 1e-6);
    assert!(frames[4].forward().distance(frames[3].forward()) < 1e-4);
}

#[test]
fn a_repeated_point_changes_nothing() {
    let line = eastward(11, 50.0, |_| 20.0);
    let mut repeated = line.points.clone();
    repeated.insert(5, repeated[5].clone());
    let repeated = self::line(repeated);

    let frames = track_frames(&line, FRAME_STEP);
    let with_repeat = track_frames(&repeated, FRAME_STEP);
    assert_eq!(frames.len(), with_repeat.len());
    for (a, b) in frames.iter().zip(&with_repeat) {
        assert!(a.position.distance(b.position) < 1e-6);
        assert!(a.forward().distance(b.forward()) < 1e-9);
    }
}

/// The spline gives a frame at any distance, not only at the resampler's steps: a metre on
/// is a metre away, give or take the curve, the ends are the line's ends, and at the
/// resampler's distances it is the resampler's frame.
#[test]
fn a_spline_gives_the_frame_at_any_distance_and_agrees_with_the_resampler() {
    // The quarter circle is cut to whole chords: 31 of 50 m, so 1550 m of arc.
    let line = quarter_circle(1000.0, 50.0);
    let spline = TrackSpline::through(&line).unwrap();
    let length = spline.length();
    assert!((length - 1550.0).abs() < 1.0, "{length}");

    // Continuous: a metre along is within a metre and a bit, every metre of the line.
    let mut previous = spline.frame_at(0.0);
    let mut distance = 1.0;
    while distance < length {
        let frame = spline.frame_at(distance);
        let chord = frame.position.distance(previous.position);
        assert!((0.95..1.05).contains(&chord), "{distance} m: {chord} m on");
        assert!(
            frame.forward().distance(previous.forward()) < 0.01,
            "{distance} m"
        );
        assert_eq!(frame.distance, distance);
        previous = frame;
        distance += 1.0;
    }

    // The ends are the line's first and last points, and beyond them is them.
    let first = line.positions[0];
    let last = *line.positions.last().unwrap();
    assert!(spline.frame_at(0.0).position.distance(first) < 1e-6);
    assert!(spline.frame_at(length).position.distance(last) < 1e-6);
    assert!(spline.frame_at(-50.0).position.distance(first) < 1e-6);
    assert!(spline.frame_at(length + 50.0).position.distance(last) < 1e-6);
    assert_eq!(spline.frame_at(length + 50.0).distance, length);
    assert_eq!(spline.frame_at(f64::NAN).distance, 0.0);

    // The resampler's frames, asked for one at a time, are the same frames.
    for (index, resampled) in spline.resample(FRAME_STEP).iter().enumerate() {
        let frame = spline.frame_at(resampled.distance);
        assert!(
            frame.position.distance(resampled.position) < 1e-3,
            "frame {index}: {} m apart",
            frame.position.distance(resampled.position)
        );
        assert!(
            frame.forward().distance(resampled.forward()) < 1e-6,
            "frame {index}"
        );
    }

    // On the network's worst spacing too, where the cursor sought by bisection has to land
    // where the walk did for Newton's method to agree.
    let worst = eastward_at(&[0.0, 3.0, 1194.0, 1283.0, 3150.0], |_| 20.0);
    let spline = TrackSpline::through(&worst).unwrap();
    for (index, resampled) in track_frames(&worst, FRAME_STEP).iter().enumerate() {
        let frame = spline.frame_at(resampled.distance);
        assert!(
            frame.position.distance(resampled.position) < 1e-3,
            "frame {index}: {} m apart",
            frame.position.distance(resampled.position)
        );
    }
}

/// Reversed, a frame faces back along the line with the same up, its right is what was left,
/// its grade changes sign and its place does not change.
#[test]
fn a_reversed_frame_faces_back_along_the_line_and_stands_as_it_did() {
    let line = eastward(11, 50.0, |along| 20.0 + 0.01 * along);
    let frame = track_frames(&line, FRAME_STEP)[4];
    let back = reversed(&frame);

    assert_eq!(back.position, frame.position);
    assert_eq!(back.distance, frame.distance);
    assert!(back.forward().distance(-frame.forward()) < 1e-12);
    assert!(back.up().distance(frame.up()) < 1e-12);
    assert!(back.right().distance(-frame.right()) < 1e-12);
    assert!((back.grade + frame.grade).abs() < 1e-12);
    assert!((back.rotation.length() - 1.0).abs() < 1e-12);

    // Reversed again it is the frame it was, as a rotation: two half turns are a whole one,
    // which as a quaternion is the negative of the one it started as.
    let again = reversed(&back);
    assert!(again.forward().distance(frame.forward()) < 1e-12);
    assert!(again.up().distance(frame.up()) < 1e-12);
    assert!(again.right().distance(frame.right()) < 1e-12);
}

#[test]
fn an_offset_moves_a_frame_along_its_right_axis_and_nothing_else() {
    let line = eastward(11, 50.0, |along| 20.0 + 0.01 * along);
    let frame = track_frames(&line, FRAME_STEP)[7];

    let shifted = offset(&frame, 3.0);
    assert!((shifted.position - frame.position).distance(frame.right() * 3.0) < 1e-9);
    assert_eq!(shifted.rotation, frame.rotation);
    assert_eq!(shifted.distance, frame.distance);
    assert_eq!(shifted.grade, frame.grade);

    // To the left for a negative offset, and the two running lines a track's width apart.
    let left = offset(&frame, -TRACK_CENTRES / 2.0);
    let right = offset(&frame, TRACK_CENTRES / 2.0);
    assert!((left.position - frame.position).dot(frame.right()) < 0.0);
    assert!((left.position.distance(right.position) - TRACK_CENTRES).abs() < 1e-9);
    assert!((left.position.distance(frame.position) - TRACK_CENTRES / 2.0).abs() < 1e-9);
}

#[test]
fn in_grid_round_trips_to_the_millimetre_six_thousand_kilometres_out() {
    let grid = Grid::default();
    let line = eastward(11, 50.0, |along| 20.0 + 0.02 * along);

    for (index, frame) in track_frames(&line, FRAME_STEP).iter().enumerate() {
        assert!(frame.position.length() > 6.3e6);
        let (cell, transform) = in_grid(frame, &grid);
        let place = format!("frame {index}");

        // Split, not merely narrowed: the cell carries the millions and the translation
        // stays within a cell.
        assert_ne!(cell, CellCoord::default(), "{place}");
        assert!(
            transform.translation.abs().max_element() <= grid.cell_edge_length(),
            "{place}"
        );

        let back = grid.grid_position_double(&cell, &transform);
        assert!(
            back.distance(frame.position) < 1e-3,
            "{place}: {} m off",
            back.distance(frame.position)
        );
        assert!(
            transform
                .rotation
                .abs_diff_eq(frame.rotation.as_quat(), 1e-6),
            "{place}"
        );
        assert_eq!(transform.scale, Vec3::ONE, "{place}");
    }
}

#[test]
fn too_few_points_or_unresolved_positions_give_no_frames() {
    assert!(track_frames(&line(Vec::new()), FRAME_STEP).is_empty());
    assert!(track_frames(&eastward(1, 50.0, |_| 0.0), FRAME_STEP).is_empty());

    // Two points at the same place are one.
    let same = point(AUCKLAND.0, AUCKLAND.1, 0.0);
    assert!(track_frames(&line(vec![same.clone(), same]), FRAME_STEP).is_empty());

    // Never resolved, and resolved a point ago, as on the frame after an insert.
    let mut unresolved = eastward(5, 50.0, |_| 0.0);
    unresolved.positions.clear();
    assert!(track_frames(&unresolved, FRAME_STEP).is_empty());
    let mut behind = eastward(5, 50.0, |_| 0.0);
    behind
        .points
        .push(point(AUCKLAND.0 + 0.01, AUCKLAND.1, 0.0));
    assert!(track_frames(&behind, FRAME_STEP).is_empty());

    // Two points are a line, and a step that is not a distance is refused.
    let two = eastward(2, 50.0, |_| 0.0);
    assert_eq!(track_frames(&two, FRAME_STEP).len(), 3);
    assert!(track_frames(&two, 0.0).is_empty());
    assert!(track_frames(&two, f64::NAN).is_empty());
}
