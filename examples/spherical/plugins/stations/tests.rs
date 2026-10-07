//! The compiled-in file read, and a station anchored to the track beside it, on the
//! synthetic line the track frames are tested on.

use super::*;
use crate::plugins::track_frames::TrackSpline;
use crate::plugins::track_frames::tests::eastward;

#[test]
fn the_compiled_in_file_has_aucklands_stations_each_with_its_platforms() {
    let stations = Stations::parse(STATIONS_CSV).unwrap();
    assert_eq!(stations.stations().count(), 45);
    assert!(stations.stops.len() > 100, "{}", stations.stops.len());

    for stop in &stations.stops {
        if let Some(parent) = &stop.parent {
            let station = stations
                .get(parent)
                .unwrap_or_else(|| panic!("{}", stop.name));
            assert!(station.is_station(), "{}", stop.name);
            assert_eq!(stations.name_of(&stop.id), Some(station.short_name()));
        }
        assert!(stop.name.is_ascii(), "{}", stop.name);
        assert!((-38.0..-36.0).contains(&stop.latitude), "{}", stop.name);
        assert!((174.0..176.0).contains(&stop.longitude), "{}", stop.name);
    }
    assert_eq!(stations.name_of("9002-ffdd37b7"), Some("Waitemata"));
}

/// A station 10 m beside the 300 m mark of a level line is anchored over that mark,
/// LABEL_LIFT up; one 10 km off has no anchor.
#[test]
fn a_station_is_anchored_over_the_nearest_line_and_none_when_far_off() {
    let line = eastward(21, 50.0, |_| 20.0);
    let spline = TrackSpline::through(&line).unwrap();
    let splines = TrackSplines {
        splines: vec![None, Some(spline.clone())],
    };

    let on_track = spline.frame_at(300.0).position;
    let ground = Frame::at_unit(unit_under(on_track));
    let beside =
        TerrainShape::WGS84.position_unit_to_local(unit_under(on_track + ground.north * 10.0), 0.0);

    let anchor = anchor_for(beside, &splines).unwrap();
    let expected = on_track + ground.up * LABEL_LIFT;
    assert!(
        anchor.distance(expected) < 0.1,
        "{}",
        anchor.distance(expected)
    );

    let far = TerrainShape::WGS84
        .position_unit_to_local(unit_under(on_track + ground.north * 10_000.0), 0.0);
    assert_eq!(anchor_for(far, &splines), None);
    assert_eq!(anchor_for(beside, &TrackSplines::default()), None);
}
