//! The feed's parsing on a cut of the real thing, and the reckoning between fetches checked
//! with numbers: which way a train is going, where its fix puts it now, how the drawn
//! distance follows, and the words in the caption.

use super::*;
use crate::plugins::rail_editor::frame::{Frame, unit_under};
use crate::plugins::track_frames::tests::eastward;

/// Three entities as the feed wrote them on 6 October 2026, trimmed: a bus off a trip with
/// its bearing as a string, a train standing at a platform with its bearing a nought and its
/// label padded, and an entity marked deleted.
const FEED: &str = r#"{"status":"OK","response":{"header":{"timestamp":1791275740.763,"gtfs_realtime_version":"1.0","incrementality":0},"entity":[
{"id":"512000041","vehicle":{"position":{"latitude":-34.90613833333333,"longitude":172.92260166666668,"bearing":"315.9","speed":1.6976651999999999},"timestamp":1791275720,"vehicle":{"id":"512000041","label":"","license_plate":""}},"is_deleted":false},
{"id":"59142","vehicle":{"trip":{"trip_id":"259-860001-76020-2-H136401-0d11f95d","start_time":"21:07:00","start_date":"20261006","schedule_relationship":0,"route_id":"O-W-201","direction_id":0},"position":{"latitude":-36.9091527777778,"longitude":174.684855555556,"bearing":0,"odometer":72101364,"speed":0},"timestamp":1791275706,"vehicle":{"id":"59142","label":"AMP        1142"},"occupancy_status":1},"is_deleted":false},
{"id":"gone","vehicle":{"position":{"latitude":-36.9,"longitude":174.7},"timestamp":1791275700,"vehicle":{"id":"gone"}},"is_deleted":true}
]}}"#;

fn fix(distance: f64, direction: f64, speed: f64) -> Fix {
    Fix {
        line: 0,
        distance,
        direction,
        speed,
        at: 1000.0,
        header_at: 1010.0,
        received: 5.0,
    }
}

#[test]
fn the_feed_parses_out_of_its_envelope_with_the_deleted_left_out() {
    let feed = feed::parse(FEED).unwrap();
    assert_eq!(feed.header_at, 1791275740.763);
    assert_eq!(feed.vehicles.len(), 2);

    let bus = &feed.vehicles[0];
    assert_eq!(bus.id, "512000041");
    assert_eq!(bus.route_id, None);
    assert_eq!(bus.bearing, Some(315.9));
    assert!((bus.speed - 1.6976652).abs() < 1e-9);
    assert_eq!(bus.at, 1791275720.0);

    let train = &feed.vehicles[1];
    assert_eq!(train.id, "AMP 1142");
    assert_eq!(train.route_id.as_deref(), Some("O-W-201"));
    assert_eq!(train.latitude, -36.9091527777778);
    assert_eq!(train.longitude, 174.684855555556);
    // A bearing of nought is no bearing.
    assert_eq!(train.bearing, None);
    assert_eq!(train.speed, 0.0);
    assert_eq!(train.at, 1791275706.0);

    // The same feed without AT's envelope round it: the envelope's opening up to the feed,
    // and its one closing brace.
    let bare = FEED
        .strip_prefix(r#"{"status":"OK","response":"#)
        .and_then(|rest| rest.strip_suffix('}'))
        .unwrap();
    assert_eq!(feed::parse(bare).unwrap(), feed);

    assert!(feed::parse("not json").unwrap_err().contains("not JSON"));
    assert!(
        feed::parse(r#"{"status":"OK"}"#)
            .unwrap_err()
            .contains("shape")
    );
}

#[test]
fn a_route_names_its_line_and_a_unit_its_label_or_its_id() {
    assert_eq!(feed::line_name("E-W-201"), "E-W");
    assert_eq!(feed::line_name("S-C-201"), "S-C");
    assert_eq!(feed::line_name("O-W-201"), "O-W");
    assert_eq!(feed::line_name("HUIA-404"), "HUIA");
    assert_eq!(feed::line_name("393-203"), "393");
    assert_eq!(feed::line_name("nodash"), "nodash");

    assert_eq!(
        feed::unit_name(Some("AMP        1142"), "59142"),
        "AMP 1142"
    );
    assert_eq!(feed::unit_name(Some("   "), "59142"), "59142");
    assert_eq!(feed::unit_name(None, "59142"), "59142");
}

/// Along a level line due east, a position 10 m north of the 300 m mark on the ellipsoid
/// under the track is at 300 m and 10 m off, the track's 20 m of height not counting; a
/// position past the end is at the end, as far off as it is past.
#[test]
fn a_position_snaps_to_its_distance_along_the_line_and_its_offset_is_level() {
    let line = eastward(21, 50.0, |_| 20.0);
    let spline = TrackSpline::through(&line).unwrap();

    let on_track = spline.frame_at(300.0).position;
    let ground = Frame::at_unit(unit_under(on_track));
    let beside = on_track + ground.north * 10.0;
    let on_ellipsoid = TerrainShape::WGS84.position_unit_to_local(unit_under(beside), 0.0);

    let (distance, off) = snap(&spline, on_ellipsoid, None);
    assert!((distance - 300.0).abs() < 0.05, "{distance}");
    assert!((off - 10.0).abs() < 0.05, "{off}");

    // The same with the last fix near by, and with it far away, which falls back to the
    // whole line.
    assert_eq!(snap(&spline, on_ellipsoid, Some(250.0)), (distance, off));
    assert_eq!(snap(&spline, on_ellipsoid, Some(50_000.0)), (distance, off));

    let end = spline.frame_at(spline.length());
    let past = end.position + end.forward() * 100.0;
    let (distance, off) = snap(&spline, past, None);
    assert!((distance - spline.length()).abs() < 1e-6, "{distance}");
    assert!((off - 100.0).abs() < 0.1, "{off}");

    // A stretch wholly off the line has no answer.
    assert_eq!(spline.nearest_between(past, 5_000.0, 6_000.0), None);
    assert_eq!(spline.nearest_between(past, -600.0, -500.0), None);
}

#[test]
fn a_line_due_east_heads_ninety_degrees() {
    let line = eastward(21, 50.0, |_| 20.0);
    let spline = TrackSpline::through(&line).unwrap();
    for distance in [0.0, 333.0, 1000.0] {
        let heading = heading_along(&spline, distance);
        assert!((heading - 90.0).abs() < 0.5, "{distance} m: {heading}");
    }
}

/// Movement along the line decides; without it the bearing, when moving; without that the
/// last direction; without anything, forward.
#[test]
fn the_direction_comes_from_the_fixes_then_the_bearing_then_the_past() {
    let last = fix(1000.0, -1.0, 15.0);
    assert_eq!(
        direction_of(Some(&last), 1300.0, Some(270.0), 20.0, 90.0),
        1.0
    );
    assert_eq!(
        direction_of(Some(&last), 700.0, Some(90.0), 20.0, 90.0),
        -1.0
    );

    // Within the wander of a fix, the bearing against the heading, a right angle either way.
    assert_eq!(
        direction_of(Some(&last), 1005.0, Some(95.0), 20.0, 90.0),
        1.0
    );
    assert_eq!(
        direction_of(Some(&last), 1005.0, Some(275.0), 20.0, 90.0),
        -1.0
    );
    assert_eq!(direction_of(None, 1005.0, Some(350.0), 20.0, 10.0), 1.0);
    assert_eq!(direction_of(None, 1005.0, Some(170.0), 20.0, 10.0), -1.0);

    // Standing, the bearing means nothing: the past decides, or forward.
    assert_eq!(
        direction_of(Some(&last), 1005.0, Some(95.0), 0.0, 90.0),
        -1.0
    );
    assert_eq!(direction_of(Some(&last), 1005.0, None, 20.0, 90.0), -1.0);
    assert_eq!(direction_of(None, 1005.0, Some(95.0), 0.5, 90.0), 1.0);
    assert_eq!(direction_of(None, 1005.0, None, 20.0, 90.0), 1.0);
}

#[test]
fn a_fix_is_as_old_as_the_feed_said_plus_the_time_since_it_landed() {
    let fix = fix(1000.0, 1.0, 20.0);
    assert_eq!(age(&fix, 5.0), 10.0);
    assert_eq!(age(&fix, 35.0), 40.0);
}

#[test]
fn the_reckoning_runs_the_fix_on_at_its_speed_within_reach_and_on_the_line() {
    let out = fix(1000.0, 1.0, 20.0);
    assert_eq!(reckoned(&out, 10.0, 5000.0), 1200.0);
    assert_eq!(reckoned(&out, 0.0, 5000.0), 1000.0);
    assert_eq!(reckoned(&out, -30.0, 5000.0), 1000.0);
    assert_eq!(
        reckoned(&out, 1000.0, 5000.0),
        1000.0 + 20.0 * REACH_SECONDS
    );
    // Kept on the line: a reach that would run past the end stands at the end.
    assert_eq!(reckoned(&out, 1000.0, 1300.0), 1300.0);

    let back = fix(1000.0, -1.0, 20.0);
    assert_eq!(reckoned(&back, 10.0, 5000.0), 800.0);
    assert_eq!(reckoned(&fix(300.0, -1.0, 20.0), 1000.0, 5000.0), 0.0);

    let standing = fix(1000.0, 1.0, 0.0);
    assert_eq!(reckoned(&standing, 60.0, 5000.0), 1000.0);
}

/// Ahead of the carriage the reckoning is eased to, whichever way the train runs; behind
/// it the carriage holds; far from it either way the carriage jumps.
#[test]
fn the_drawn_distance_eases_ahead_holds_against_a_reckoning_behind_and_jumps_when_far() {
    assert_eq!(approach(1000.0, 1000.0, 1.0, 0.1), 1000.0);
    assert_eq!(approach(1000.0, 1100.0, 1.0, 0.0), 1000.0);

    let share = 1.0 - 1.0 / std::f64::consts::E;
    let eased = approach(1000.0, 1100.0, 1.0, EASE_SECONDS);
    assert!((eased - (1000.0 + 100.0 * share)).abs() < 1e-9, "{eased}");
    let eased = approach(1000.0, 900.0, -1.0, EASE_SECONDS);
    assert!((eased - (1000.0 - 100.0 * share)).abs() < 1e-9, "{eased}");

    // A long pause closes the gap.
    assert!((approach(1000.0, 1100.0, 1.0, 1000.0) - 1100.0).abs() < 1e-9);

    // Behind: the train braked for a station after its fix, and the carriage was run on
    // past it. It holds, however long, rather than backing up.
    assert_eq!(approach(1000.0, 950.0, 1.0, EASE_SECONDS), 1000.0);
    assert_eq!(approach(1000.0, 950.0, 1.0, 1000.0), 1000.0);
    assert_eq!(approach(1000.0, 1050.0, -1.0, EASE_SECONDS), 1000.0);

    // A jump is a jump whichever side it is on.
    let far = SNAP_METRES + 1.0;
    assert_eq!(approach(1000.0, 1000.0 + far, 1.0, 0.1), 1000.0 + far);
    assert_eq!(approach(1000.0, 1000.0 - far, 1.0, 0.1), 1000.0 - far);
    assert_eq!(approach(f64::NAN, 1100.0, 1.0, 0.1), 1100.0);
}

#[test]
fn the_caption_says_how_the_feed_stands_in_ascii() {
    assert_eq!(caption(&FeedStatus::NoKey, 0.0), "stand-ins; no AT_API_KEY");
    assert_eq!(caption(&FeedStatus::Off, 0.0), "stand-ins; F8 for AT live");
    assert_eq!(caption(&FeedStatus::Fetching, 0.0), "AT live: fetching");
    let live = FeedStatus::Live {
        received: 100.0,
        trains: 16,
    };
    assert_eq!(caption(&live, 112.4), "AT live: 16 trains, 12 s ago");
    assert_eq!(caption(&live, 90.0), "AT live: 16 trains, 0 s ago");
    let one = FeedStatus::Live {
        received: 100.0,
        trains: 1,
    };
    assert_eq!(caption(&one, 100.0), "AT live: 1 train, 0 s ago");
    assert_eq!(
        caption(&FeedStatus::Failed("http status: 401".to_string()), 0.0),
        "AT live: failed, http status: 401"
    );

    let long = "x".repeat(100) + "\nsecond line";
    let cut = caption(&FeedStatus::Failed(long), 0.0);
    assert!(cut.ends_with("..."), "{cut}");
    assert!(!cut.contains('\n'));
    assert!(cut.chars().count() <= "AT live: failed, ".len() + 60);
    assert!(cut.is_ascii());
}
