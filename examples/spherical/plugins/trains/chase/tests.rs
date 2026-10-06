//! The chase pose and the orbit the mouse moves it on, checked with numbers on the synthetic
//! lines the track frames are tested on, and the rules that let go of the camera, each
//! trigger once.

use super::*;
use crate::plugins::rail_editor::frame::tests::degrees_between;
use crate::plugins::rail_editor::frame::{Frame, unit_under};
use crate::plugins::track_frames::tests::{eastward, quarter_circle};
use crate::plugins::track_frames::{TrackSpline, reversed};
use crate::plugins::trains::TRAIN_SPEED;

/// On a level line due east the eye sits 60 m behind and 25 m above the frame, square to
/// it, looking at the point 20 m ahead with no roll: the camera's right is level with the
/// frame's up, and its up leans back from the frame's up by the pitch down to that point,
/// which from 80 m back and 25 m up is seventeen degrees.
#[test]
fn the_eye_sits_behind_and_above_the_frame_and_looks_a_little_ahead() {
    let line = eastward(21, 50.0, |_| 20.0);
    let spline = TrackSpline::through(&line).unwrap();
    let frame = spline.frame_at(500.0);
    let ground = Frame::at_unit(unit_under(frame.position));
    assert!(degrees_between(frame.forward(), ground.east) < 0.1);

    let (eye, rotation) = chase_pose(
        frame.position,
        chase_heading(&frame, 0.0, None),
        &Orbit::default(),
    );
    let offset = eye - frame.position;
    assert!(
        (offset.dot(frame.forward()) + CHASE_BACK).abs() < 1e-6,
        "{offset}"
    );
    assert!((offset.dot(frame.up()) - CHASE_UP).abs() < 1e-6, "{offset}");
    assert!(offset.dot(frame.right()).abs() < 1e-6, "{offset}");
    assert!((rotation.length() - 1.0).abs() < 1e-12);

    let ahead = frame.position + frame.forward() * CHASE_AHEAD;
    let forward = rotation * DVec3::NEG_Z;
    assert!(
        degrees_between(forward, (ahead - eye).normalize()) < 0.1,
        "{}",
        degrees_between(forward, (ahead - eye).normalize())
    );

    let right = rotation * DVec3::X;
    let up = rotation * DVec3::Y;
    assert!(
        right.dot(frame.up()).abs() < 1.0_f64.to_radians().sin(),
        "{}",
        right.dot(frame.up())
    );
    assert!(up.dot(frame.up()) > 0.0);
    let pitch = (CHASE_UP / (CHASE_BACK + CHASE_AHEAD)).atan().to_degrees();
    assert!(
        (degrees_between(up, frame.up()) - pitch).abs() < 0.1,
        "{}",
        degrees_between(up, frame.up())
    );
    assert!(degrees_between(right, -ground.north) < 1.0);
}

/// Without a previous heading the frame's is the heading; with one, no time passed is the
/// previous heading, and one time constant closes all but 1/e of the turn between them.
#[test]
fn the_heading_snaps_first_and_then_lags_by_the_time_constant() {
    let line = quarter_circle(800.0, 50.0);
    let spline = TrackSpline::through(&line).unwrap();
    let from = spline.frame_at(100.0);
    let to = spline.frame_at(400.0);

    let previous = chase_heading(&from, 0.0, None);
    assert!(previous.abs_diff_eq(from.rotation, 1e-12), "{previous}");
    let turn = previous.angle_between(to.rotation);
    assert!(turn > 0.3, "{turn}");

    // Component by component: the angle between two rotations a rounding error apart is the
    // arc cosine of a number next to one, which rounding moves by far more than 1e-9.
    let held = chase_heading(&to, 0.0, Some(previous));
    assert!(held.abs_diff_eq(previous, 1e-9), "{held}");

    let share = 1.0 - 1.0 / std::f64::consts::E;
    let heading = chase_heading(&to, CHASE_SMOOTHING, Some(previous));
    assert!(
        (heading.angle_between(previous) - share * turn).abs() < 1e-6,
        "{} of {turn}",
        heading.angle_between(previous)
    );
    assert!((heading.length() - 1.0).abs() < 1e-9);

    // A pause long against the time constant is a snap.
    let heading = chase_heading(&to, 100.0 * CHASE_SMOOTHING, Some(previous));
    assert!(heading.angle_between(to.rotation) < 1e-6);
}

/// A train running along a straight line keeps the eye at CHASE_BACK and CHASE_UP frame
/// after frame: the heading has next to nothing to catch up and the position is not
/// smoothed, so the lag never shows as a distance. An eye smoothed the same way would trail
/// by its speed times the time constant, 6 m at line speed, which is a tenth of CHASE_BACK.
#[test]
fn a_running_train_keeps_the_eye_at_its_distance() {
    let line = eastward(41, 50.0, |_| 20.0);
    let spline = TrackSpline::through(&line).unwrap();
    let dt = 1.0 / 60.0;

    let mut heading = None;
    for step in 0..120 {
        let frame = spline.frame_at(500.0 + step as f64 * TRAIN_SPEED * dt);
        let now = chase_heading(&frame, dt, heading);
        let (eye, _) = chase_pose(frame.position, now, &Orbit::default());
        let offset = eye - frame.position;
        assert!(
            (offset.dot(frame.forward()) + CHASE_BACK).abs() < 1e-2,
            "step {step}: {offset}"
        );
        assert!(
            (offset.dot(frame.up()) - CHASE_UP).abs() < 1e-2,
            "step {step}: {offset}"
        );
        heading = Some(now);
    }
}

/// Behind a train running back is the other side of the carriage: the two eyes stand along
/// the line either side of it, as high as each other, each looking its train's way. Halfway
/// through the swing from the one to the other the eye is out to the carriage's side, as far
/// from it and as high, with the carriage in the view: the swing goes round it, not over it.
#[test]
fn a_reversed_frame_puts_the_eye_on_the_other_side_and_the_swing_goes_round() {
    let line = eastward(21, 50.0, |_| 20.0);
    let spline = TrackSpline::through(&line).unwrap();
    let out = spline.frame_at(500.0);
    let back = reversed(&out);

    let (eye_out, rotation_out) = chase_pose(out.position, out.rotation, &Orbit::default());
    let (eye_back, rotation_back) = chase_pose(back.position, back.rotation, &Orbit::default());
    assert!((eye_out - out.position).dot(out.forward()) < 0.0);
    assert!((eye_back - out.position).dot(out.forward()) > 0.0);
    assert!(
        (eye_out.distance(eye_back) - 2.0 * CHASE_BACK).abs() < 1e-6,
        "{}",
        eye_out.distance(eye_back)
    );
    assert!(((eye_back - out.position).dot(out.up()) - CHASE_UP).abs() < 1e-6);

    let forward_out = rotation_out * DVec3::NEG_Z;
    let forward_back = rotation_back * DVec3::NEG_Z;
    assert!(forward_out.dot(out.forward()) > 0.9);
    assert!(forward_back.dot(out.forward()) < -0.9);

    // Half the turn is what the lag closes in ln 2 time constants.
    let halfway = chase_heading(&back, CHASE_SMOOTHING * 2.0_f64.ln(), Some(out.rotation));
    let (eye, rotation) = chase_pose(out.position, halfway, &Orbit::default());
    let offset = eye - out.position;
    assert!((offset.dot(out.up()) - CHASE_UP).abs() < 1e-6, "{offset}");
    assert!(offset.dot(out.forward()).abs() < 1e-6, "{offset}");
    assert!(
        (offset.dot(out.right()).abs() - CHASE_BACK).abs() < 1e-6,
        "{offset}"
    );
    let forward = rotation * DVec3::NEG_Z;
    let to_carriage = (out.position - eye).normalize();
    assert!(
        degrees_between(forward, to_carriage) < 10.0,
        "{}",
        degrees_between(forward, to_carriage)
    );
}

/// Nothing touched keeps the camera; any one touch lets go, each for a reason of its own.
#[test]
fn any_one_touch_lets_go_for_its_own_reason_and_none_keeps_following() {
    let none = Touch::default();
    assert_eq!(reason_to_let_go(none), None);

    let each = [
        Touch {
            fly_toggled: true,
            ..none
        },
        Touch {
            fly_key: true,
            ..none
        },
        Touch {
            orbital_toggled: true,
            ..none
        },
        Touch {
            escape: true,
            ..none
        },
        Touch {
            trains_hidden: true,
            ..none
        },
        Touch {
            train_gone: true,
            ..none
        },
        Touch {
            moved_elsewhere: true,
            ..none
        },
    ];
    let reasons: Vec<&str> = each
        .iter()
        .map(|touch| reason_to_let_go(*touch).unwrap_or_else(|| panic!("{touch:?}")))
        .collect();
    for (index, reason) in reasons.iter().enumerate() {
        assert!(!reasons[..index].contains(reason), "{reason} twice");
    }
}

#[test]
fn the_icon_follows_its_train_and_lets_go_of_the_train_already_followed() {
    let mut world = World::new();
    let (one, two) = (world.spawn_empty().id(), world.spawn_empty().id());

    assert_eq!(follow_or_let_go(None, two), Some(two));
    assert_eq!(follow_or_let_go(Some(one), two), Some(two));
    assert_eq!(follow_or_let_go(Some(two), two), None);
}

/// The default orbit is CHASE_BACK behind and CHASE_UP above. A quarter turn to the right
/// puts the eye out to the carriage's right at the same distance and height, square to the
/// line, with the carriage still in the view; a quarter turn the other way, to its left.
#[test]
fn turning_the_orbit_takes_the_eye_round_the_carriage() {
    let line = eastward(21, 50.0, |_| 20.0);
    let spline = TrackSpline::through(&line).unwrap();
    let frame = spline.frame_at(500.0);
    let orbit = Orbit::default();
    assert!((orbit.distance * orbit.pitch.cos() - CHASE_BACK).abs() < 1e-9);
    assert!((orbit.distance * orbit.pitch.sin() - CHASE_UP).abs() < 1e-9);

    for (yaw, side) in [(90.0_f64, 1.0), (-90.0, -1.0)] {
        let orbit = Orbit {
            yaw: yaw.to_radians(),
            ..Orbit::default()
        };
        let (eye, rotation) = chase_pose(frame.position, frame.rotation, &orbit);
        let offset = eye - frame.position;
        assert!(
            (offset.dot(frame.right()) - side * CHASE_BACK).abs() < 1e-6,
            "{yaw}: {offset}"
        );
        assert!(offset.dot(frame.forward()).abs() < 1e-6, "{yaw}: {offset}");
        assert!(
            (offset.dot(frame.up()) - CHASE_UP).abs() < 1e-6,
            "{yaw}: {offset}"
        );

        let forward = rotation * DVec3::NEG_Z;
        let to_carriage = (frame.position - eye).normalize();
        assert!(
            degrees_between(forward, to_carriage) < 20.0,
            "{yaw}: {}",
            degrees_between(forward, to_carriage)
        );
        let right = rotation * DVec3::X;
        assert!(right.dot(frame.up()).abs() < 1.0_f64.to_radians().sin());
    }
}

/// A drag right turns the orbit right by its pixels, a drag up tilts it up and down down,
/// and no drag tilts it past the limits.
#[test]
fn a_drag_turns_and_tilts_the_orbit_within_its_limits() {
    let mut orbit = Orbit::default();
    orbit.turn(Vec2::new(100.0, 0.0));
    assert!(
        (orbit.yaw - 100.0 * TURN_PER_PIXEL).abs() < 1e-9,
        "{}",
        orbit.yaw
    );
    assert_eq!(orbit.pitch, Orbit::default().pitch);

    let level = orbit.pitch;
    orbit.turn(Vec2::new(0.0, -10.0));
    assert!(orbit.pitch > level);
    orbit.turn(Vec2::new(0.0, 20.0));
    assert!(orbit.pitch < level);

    orbit.turn(Vec2::new(0.0, -1.0e6));
    assert!((orbit.pitch - PITCH_DEGREES.1.to_radians()).abs() < 1e-9);
    orbit.turn(Vec2::new(0.0, 1.0e6));
    assert!((orbit.pitch - PITCH_DEGREES.0.to_radians()).abs() < 1e-9);
    assert_eq!(orbit.distance, Orbit::default().distance);
}

/// Zooming scales the distance, so the wheel feels the same near and far, and stops at the
/// limits: five notches double it, as 140 px of drag do.
#[test]
fn zooming_scales_the_distance_within_its_limits() {
    let mut orbit = Orbit::default();
    let start = orbit.distance;
    orbit.zoom(2.0_f64.ln());
    assert!((orbit.distance - 2.0 * start).abs() < 1e-9);
    orbit.zoom(-2.0_f64.ln());
    assert!((orbit.distance - start).abs() < 1e-9);

    orbit.zoom(5.0 * ZOOM_PER_NOTCH);
    assert!(
        (orbit.distance / start - 2.0).abs() < 0.03,
        "{}",
        orbit.distance / start
    );
    orbit = Orbit::default();
    orbit.zoom(140.0 * ZOOM_PER_PIXEL);
    assert!(
        (orbit.distance / start - 2.0).abs() < 0.03,
        "{}",
        orbit.distance / start
    );

    orbit.zoom(100.0);
    assert_eq!(orbit.distance, DISTANCE_METRES.1);
    orbit.zoom(-100.0);
    assert_eq!(orbit.distance, DISTANCE_METRES.0);
}
