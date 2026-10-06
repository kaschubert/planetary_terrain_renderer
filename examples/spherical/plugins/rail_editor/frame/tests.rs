//! The frame against the geometry of the spheroid at a spread of places. The places and
//! the angle helper are shared with the gizmo's, the panel's and the track frames' tests,
//! which import them from here.

use super::*;
use bevy_terrain::math::unit_position;

/// Britomart, where the S-C line starts.
pub(crate) const AUCKLAND: (f64, f64) = (174.7682, -36.8441);

/// Places in all four quadrants, both hemispheres and across the date line, none at a pole.
const PLACES: [(f64, f64); 8] = [
    (0.0, 0.0),
    AUCKLAND,
    (-70.6, -33.4),
    (139.7, 35.7),
    (-0.1, 51.5),
    (179.9, -16.5),
    (-179.9, 66.0),
    (30.0, -75.0),
];

pub(crate) fn degrees_between(a: DVec3, b: DVec3) -> f64 {
    a.dot(b).clamp(-1.0, 1.0).acos().to_degrees()
}

#[test]
fn the_direction_under_a_position_undoes_the_height_exactly() {
    for (longitude, latitude) in PLACES {
        let unit = unit_position(longitude, latitude);
        for height in [-50.0, 0.0, 30.0, 1_000.0] {
            let position = TerrainShape::WGS84.position_unit_to_local(unit, height);
            assert!(
                unit_under(position).distance(unit) < 1e-12,
                "at {longitude}, {latitude}, {height} m"
            );
        }
    }
}

#[test]
fn the_frame_is_orthonormal_and_right_handed_everywhere() {
    for (longitude, latitude) in PLACES {
        let unit = unit_position(longitude, latitude);
        let frame = Frame::at_unit(unit);
        let place = format!("at {longitude}, {latitude}");

        assert!((frame.east.length() - 1.0).abs() < 1e-12, "{place}");
        assert!((frame.north.length() - 1.0).abs() < 1e-12, "{place}");
        assert!((frame.up.length() - 1.0).abs() < 1e-12, "{place}");
        assert!(frame.east.dot(frame.north).abs() < 1e-12, "{place}");
        assert!(frame.east.dot(frame.up).abs() < 1e-12, "{place}");
        assert!(frame.north.dot(frame.up).abs() < 1e-12, "{place}");

        // Right-handed as the gizmo holds it: x east, y up, z south.
        let south = frame.east.cross(frame.up);
        assert!(south.distance(-frame.north) < 1e-12, "{place}");

        // Up is where a height goes, within the geocentric-geodetic difference, which
        // peaks at a fifth of a degree in the mid latitudes.
        let normal = geodetic_normal(unit);
        assert!(degrees_between(frame.up, normal) < 0.2, "{place}");
        assert!(frame.up.dot(unit) > 0.999, "{place}");

        // North towards increasing latitude and east towards increasing longitude. A chord
        // of a thousandth of a degree leans half that off the tangent; north leans the
        // geocentric-geodetic difference off the step besides, east nothing more.
        let northward = unit_position(longitude, latitude + 0.001) - unit;
        let eastward = unit_position(longitude + 0.001, latitude) - unit;
        assert!(
            degrees_between(frame.north, northward.normalize()) < 0.2,
            "{place}"
        );
        assert!(
            degrees_between(frame.east, eastward.normalize()) < 0.001,
            "{place}"
        );
    }
}

#[test]
fn the_rotation_maps_the_local_axes_to_the_frame() {
    for (longitude, latitude) in PLACES {
        let frame = Frame::at_unit(unit_position(longitude, latitude));
        let rotation = frame.rotation();

        assert!((rotation * DVec3::X).distance(frame.east) < 1e-12);
        assert!((rotation * DVec3::Y).distance(frame.up) < 1e-12);
        assert!((rotation * DVec3::Z).distance(-frame.north) < 1e-12);

        // And the displacement is the rotation applied, which is how a gizmo total in local
        // axes becomes metres in space.
        let local = DVec3::new(3.0, -2.0, 7.0);
        assert!(frame.displacement(local).distance(rotation * local) < 1e-9);
    }
}
