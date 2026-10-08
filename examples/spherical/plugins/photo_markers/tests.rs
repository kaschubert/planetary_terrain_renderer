//! The pure parts of the photo markers: the model's geometry, the colour's round trip and the
//! readout's sentences. Nothing here needs a window, a renderer or an asset.

use super::file::{MarkerFile, load_markers, size_row, to_ron};
use super::model::{
    HOVER_HEIGHT, MARKER_HEIGHT, MIN_PIXELS, SCREEN_ASPECT, apparent_pixels, billboard_rotation,
    model_transform, pixel_floor_scale,
};
use super::panel::{hex_of, readout_text};
use super::photo::{canvas_size, readable};
use super::*;
use crate::plugins::rail_editor::frame::tests::AUCKLAND;
use bevy::math::DMat3;
use bevy_terrain::math::unit_position;
use std::path::{Path, PathBuf};

/// The place the editor's frame tests use, as a direction on the unit sphere: central Auckland.
fn auckland() -> DVec3 {
    unit_position(AUCKLAND.0, AUCKLAND.1)
}

/// The two corners of the marker mesh as the file holds them, before its node's transform. The
/// model transform is checked against them rather than against numbers typed in again.
const FILE_MIN: Vec3 = Vec3::new(-1.0, -0.4940650, -0.0560187);
const FILE_MAX: Vec3 = Vec3::new(1.0, 0.5049493, 0.1057776);
const NODE_SCALE: Vec3 = Vec3::new(0.3534846, 1.0, 1.0);

/// A point of the mesh as the holder entity sees it. The model transform is the only one the
/// mesh child carries, so it takes the file's vertex straight through, the node's placement
/// included; applying the node's transform here as well would count it twice.
fn placed(vertex: Vec3) -> Vec3 {
    model_transform().transform_point(vertex)
}

#[test]
fn the_model_stands_on_the_origin_at_its_drawn_height() {
    let base = placed(Vec3::new(
        (FILE_MIN.x + FILE_MAX.x) / 2.0,
        FILE_MIN.y,
        (FILE_MIN.z + FILE_MAX.z) / 2.0,
    ));
    let top = placed(Vec3::new(0.0, FILE_MAX.y, 0.0));

    // The base sits on the origin, so the hover height is measured to the bottom of the plate.
    assert!(base.length() < 1e-4, "base at {base:?}");
    // And the top stands MARKER_HEIGHT above it.
    assert!(
        (top.y - MARKER_HEIGHT).abs() < 1e-3,
        "top at {} m, wanted {MARKER_HEIGHT}",
        top.y
    );
}

#[test]
fn the_model_is_as_wide_as_the_file_says_and_not_stretched() {
    let left = placed(Vec3::new(FILE_MIN.x, 0.0, 0.0));
    let right = placed(Vec3::new(FILE_MAX.x, 0.0, 0.0));
    let width = right.x - left.x;

    // 0.707 of the file's units wide against 0.999 tall, at MARKER_HEIGHT tall.
    let expected =
        MARKER_HEIGHT * (FILE_MAX.x - FILE_MIN.x) * NODE_SCALE.x / (FILE_MAX.y - FILE_MIN.y);
    assert!(
        (width - expected).abs() < 1e-3,
        "{width} m wide, wanted {expected}"
    );
}

#[test]
fn the_screen_is_about_four_by_three() {
    // Measured from the file: 0.628 by 0.463 units once the node's x scale is in.
    assert!(
        (SCREEN_ASPECT - 1.3555).abs() < 1e-3,
        "aspect {SCREEN_ASPECT}"
    );
}

#[test]
fn a_marker_floats_the_hover_height_over_its_ground() {
    let unit = auckland();
    let marker = PhotoMarker {
        id: MarkerId(0),
        unit,
        ground: 41.0,
        colour: DEFAULT_COLOUR,
        photo: None,
        missing: false,
        riding: None,
        at: DVec3::ZERO,
        entity: None,
        body_material: None,
        screen_material: None,
    };

    let ground = TerrainShape::WGS84.position_unit_to_local(unit, 41.0);
    let floated = marker.anchor_position();

    // Exactly the hover height further out, along the direction heights run in.
    let up = Frame::at_unit(unit).up;
    assert!((floated.distance(ground) - HOVER_HEIGHT).abs() < 1e-6);
    assert!((floated - ground).normalize().dot(up) > 1.0 - 1e-12);
}

#[test]
fn a_markers_longitude_and_latitude_come_back_out_of_its_direction() {
    let marker = PhotoMarker {
        id: MarkerId(0),
        unit: unit_position(174.76334, -36.84851),
        ground: 0.0,
        colour: DEFAULT_COLOUR,
        photo: None,
        missing: false,
        riding: None,
        at: DVec3::ZERO,
        entity: None,
        body_material: None,
        screen_material: None,
    };

    assert!((marker.longitude() - 174.76334).abs() < 1e-9);
    assert!((marker.latitude() - (-36.84851)).abs() < 1e-9);
}

#[test]
fn the_screen_faces_the_camera_and_the_marker_stands_up() {
    let unit = auckland();
    let up = Frame::at_unit(unit).up;
    let position = TerrainShape::WGS84.position_unit_to_local(unit, 0.0);
    // A camera off to the east and above, as the example's opening view is.
    let east = Frame::at_unit(unit).east;
    let camera = position + east * 1000.0 + up * 400.0;

    let rotation = billboard_rotation(up, camera - position);

    // Local Y is the ground's up, so the marker stands rather than leans.
    assert!((rotation * DVec3::Y).dot(up) > 1.0 - 1e-12);
    // Local Z, the screen's normal, points at the camera once the climb is taken out of it.
    let facing = rotation * DVec3::Z;
    assert!(facing.dot(up).abs() < 1e-12, "the facing leans: {facing:?}");
    assert!(facing.dot(east) > 1.0 - 1e-9);
    // And it is a rotation, not a reflection.
    let matrix = DMat3::from_quat(rotation);
    assert!((matrix.determinant() - 1.0).abs() < 1e-12);
}

#[test]
fn a_marker_overhead_faces_north() {
    let unit = auckland();
    let frame = Frame::at_unit(unit);
    let position = TerrainShape::WGS84.position_unit_to_local(unit, 0.0);
    // Straight above, where there is nothing level in the direction to the camera.
    let rotation = billboard_rotation(frame.up, position + frame.up * 1000.0 - position);

    assert!((rotation * DVec3::Z).dot(frame.north) > 1.0 - 1e-9);
}

/// The example's opening view: a kilometre up, tilted halfway down, so about 1414 m of slant
/// range, at the default 45 degree vertical field of view over 1080 pixels.
fn opening_focal_pixels() -> f32 {
    1080.0 / (2.0 * (std::f32::consts::FRAC_PI_4 / 2.0).tan())
}

#[test]
fn a_far_marker_is_scaled_up_to_the_pixel_floor_and_a_near_one_is_left_alone() {
    let focal_pixels = opening_focal_pixels();
    let floor = 24.0;

    // Near enough that the marker is already taller than the floor.
    let near = pixel_floor_scale(MARKER_HEIGHT, 100.0, focal_pixels, floor);
    assert_eq!(near, 1.0, "a near marker should not be scaled");

    // Far enough that it is not, and the scale brings it to exactly the floor. Worked out from
    // the height rather than typed in, so that moving the default size does not quietly turn
    // this into a test of a marker that was already above the floor.
    let far = 4.0 * (MARKER_HEIGHT * focal_pixels / floor) as f64;
    let scale = pixel_floor_scale(MARKER_HEIGHT, far, focal_pixels, floor);
    let pixels = apparent_pixels(MARKER_HEIGHT * scale, far, focal_pixels);
    assert!(scale > 1.0, "a far marker should be scaled up, got {scale}");
    assert!((pixels - floor).abs() < 1e-3, "{pixels} px, wanted {floor}");
}

#[test]
fn a_floor_of_zero_never_scales_anything() {
    // Which is how it starts, so that the size slider is the only thing deciding how big a
    // marker is while that is being settled by eye.
    assert_eq!(MIN_PIXELS, 0.0);

    let focal_pixels = opening_focal_pixels();
    for distance in [10.0, 1_000.0, 100_000.0] {
        let scale = pixel_floor_scale(MARKER_HEIGHT, distance, focal_pixels, MIN_PIXELS);
        assert_eq!(scale, 1.0, "scaled at {distance} m with the floor off");
    }
}

#[test]
fn the_first_size_was_a_speck_and_the_floor_was_carrying_it() {
    // Why there is a slider at all. At the size this started on, 12 m, from about the distance
    // the example opens at, a marker is eleven pixels tall: a speck. What was on screen looked
    // bigger only because the floor was scaling it up to its 24, which is to say the size was
    // never really 12 m at that range and the slider had nothing to show for itself.
    let focal_pixels = opening_focal_pixels();
    let raw = apparent_pixels(12.0, 1414.0, focal_pixels);
    assert!((10.0..=12.0).contains(&raw), "{raw} px, wanted about 11");

    let floored = pixel_floor_scale(12.0, 1414.0, focal_pixels, 24.0);
    assert!(
        floored > 2.0,
        "the floor was more than doubling it, got {floored}"
    );
}

#[test]
fn a_size_row_reads_as_one_line_of_numbers() {
    let sample = SizeSample {
        distance: 1414.0,
        altitude: 1000.0,
        pixels: 22.1,
        viewport: 1080.0,
        fov: 45.0,
    };

    let row = size_row("2026-10-07 09:12 UTC", 48.0, 0.0, &sample);

    assert_eq!(
        row,
        "2026-10-07 09:12 UTC,48.0,0,1414,1000,22.1,1080,45.0\n"
    );
}

#[test]
fn the_default_colour_is_a_very_light_grey() {
    assert_eq!(hex_of(DEFAULT_COLOUR), "#E6E6E6");
}

#[test]
fn a_grey_keeps_its_hue_so_the_hue_slider_still_means_something() {
    // The point of holding hue, saturation and value rather than a colour: a grey has no hue
    // to read back, so a round trip through Color would lose a hue set before the saturation.
    let mut colour = DEFAULT_COLOUR;
    colour.hue = 205.0;

    assert_eq!(colour.hue, 205.0);
    // And raising the saturation from there gives the blue the hue promised, not a grey.
    colour.saturation = 0.72;
    let srgba = Srgba::from(Color::from(colour));
    assert!(
        srgba.blue > srgba.red && srgba.blue > srgba.green,
        "wanted a blue, got {srgba:?}"
    );
}

#[test]
fn the_readout_says_which_marker_of_how_many_or_what_to_do() {
    assert_eq!(
        readout_text(None, 0),
        "no markers: Ctrl+click the terrain to place one"
    );
    assert_eq!(readout_text(None, 3), "3 markers, none selected");
    assert_eq!(readout_text(Some(0), 3), "marker 1 of 3");
    assert_eq!(readout_text(Some(2), 3), "marker 3 of 3");
}

#[test]
fn placing_selects_the_new_marker_and_gives_it_the_current_colour() {
    let mut markers = PhotoMarkers {
        colour: Hsva::hsv(205.0, 0.72, 0.85),
        ..default()
    };

    let first = markers.place(auckland(), 41.0);
    assert_eq!(markers.selected, Some(0));
    assert_eq!(markers.markers[0].colour, markers.colour);
    assert!(markers.dirty);

    let second = markers.place(auckland(), 42.0);
    assert_ne!(first, second, "every marker gets an id of its own");
    assert_eq!(markers.selected, Some(1));
}

#[test]
fn removing_the_selection_leaves_nothing_selected() {
    let mut markers = PhotoMarkers::default();
    markers.place(auckland(), 41.0);
    markers.place(auckland(), 42.0);
    markers.selected = Some(0);

    markers.remove_selected();

    assert_eq!(markers.markers.len(), 1);
    assert_eq!(
        markers.selected, None,
        "the selection should not slide onto a neighbour"
    );
    // The one left is the second, not the first.
    assert_eq!(markers.markers[0].ground, 42.0);
}

#[test]
fn a_wide_photo_gets_bars_above_and_below_and_a_tall_one_to_either_side() {
    // Wider than the screen's 1.3555: the width is kept and the height grown.
    let (width, height) = canvas_size(UVec2::new(400, 200));
    assert_eq!(width, 400);
    assert_eq!(height, (400.0 / SCREEN_ASPECT).round() as u32);
    assert!(height > 200, "a wide photo wants bars above and below");

    // Taller: the height is kept and the width grown.
    let (width, height) = canvas_size(UVec2::new(200, 400));
    assert_eq!(height, 400);
    assert_eq!(width, (400.0 * SCREEN_ASPECT).round() as u32);
    assert!(width > 200, "a tall photo wants bars to either side");
}

#[test]
fn a_photo_already_the_screens_shape_gets_no_bars() {
    // 1356 by 1000 is the screen's shape to within a pixel.
    let (width, height) = canvas_size(UVec2::new(1356, 1000));

    assert_eq!(width, 1356);
    assert!(
        height.abs_diff(1000) <= 1,
        "wanted no bars worth seeing, got {height} against 1000"
    );
}

#[test]
fn the_canvas_never_loses_a_pixel_of_the_photo() {
    // Whatever the shape, the canvas holds the whole photo: it is only ever padded.
    for (width, height) in [(1, 1), (3, 2), (2, 3), (4000, 3000), (1080, 1920), (17, 5)] {
        let (canvas_width, canvas_height) = canvas_size(UVec2::new(width, height));
        assert!(
            canvas_width >= width && canvas_height >= height,
            "{width}x{height} would be cropped to {canvas_width}x{canvas_height}"
        );
    }
}

#[test]
fn only_the_formats_this_build_decodes_are_taken() {
    for name in ["holiday.png", "holiday.jpg", "holiday.jpeg", "HOLIDAY.JPG"] {
        assert!(
            readable(Path::new(name)).is_ok(),
            "{name} should be accepted"
        );
    }

    // A format Bevy is not compiled with says so by name, rather than failing deeper down.
    let refused = readable(Path::new("holiday.webp")).unwrap_err();
    assert!(refused.contains("webp"), "{refused}");
    assert!(refused.contains("png and jpeg"), "{refused}");

    // And a file with no extension at all is refused too, with the same advice.
    let refused = readable(Path::new("holiday")).unwrap_err();
    assert!(refused.contains("no extension"), "{refused}");
}

#[test]
fn two_markers_survive_a_round_trip_through_the_file() {
    let mut markers = PhotoMarkers::default();
    markers.colour = Hsva::hsv(205.0, 0.72, 0.85);
    markers.place(unit_position(174.76334, -36.84851), 41.2);
    markers.colour = Hsva::hsv(36.0, 0.8, 0.9);
    markers.place(unit_position(174.81, -36.9), -3.5);
    markers.markers[1].photo = Some(PathBuf::from("/home/kai/Pictures/piha.jpg"));

    let text = to_ron(&markers.markers, "2026-10-07 09:12 UTC").expect("the markers serialise");
    let file: MarkerFile = ron::from_str(&text).expect("and parse back");

    assert_eq!(file.saved_at, "2026-10-07 09:12 UTC");
    assert_eq!(file.markers.len(), 2);
    // The places come back to the degree they went in at.
    assert!((file.markers[0].longitude - 174.76334).abs() < 1e-9);
    assert!((file.markers[0].latitude - (-36.84851)).abs() < 1e-9);
    assert_eq!(file.markers[0].ground, 41.2);
    // And so do the colours, hue included, which is the point of storing the three channels.
    assert_eq!(file.markers[0].hue, 205.0);
    assert_eq!(file.markers[0].saturation, 0.72);
    assert_eq!(file.markers[1].hue, 36.0);
    // A photo's path is kept, and a marker without one stays without.
    assert_eq!(file.markers[0].photo, None);
    assert_eq!(
        file.markers[1].photo,
        Some(PathBuf::from("/home/kai/Pictures/piha.jpg"))
    );
}

#[test]
fn a_file_that_is_not_there_is_no_markers_and_no_complaint() {
    let (markers, unreadable) = load_markers(Path::new("/nonexistent/markers.ron"));

    assert!(markers.is_empty());
    assert!(!unreadable, "a missing file is not a broken one");
}

#[test]
fn a_file_that_will_not_parse_blocks_saving_over_it() {
    let path = std::env::temp_dir().join("photo-markers-test-garbage.ron");
    std::fs::write(&path, "this is not RON at all").expect("the temporary file is writable");

    let (markers, unreadable) = load_markers(&path);
    assert!(markers.is_empty());
    assert!(unreadable, "garbage should be reported, not ignored");

    // And a save with that flag set refuses rather than writing the emptiness over the file.
    let mut markers = PhotoMarkers {
        file_unreadable: true,
        dirty: true,
        ..default()
    };
    assert!(!markers.can_save());
    let refused = markers.save().expect_err("the save should refuse");
    assert!(refused.contains("fix or move the file"), "{refused}");

    std::fs::remove_file(&path).ok();
}

#[test]
fn saving_is_offered_only_when_there_is_something_new_to_write() {
    let mut markers = PhotoMarkers::default();
    assert!(!markers.can_save(), "nothing has changed yet");

    markers.place(auckland(), 41.0);
    assert!(markers.can_save(), "a placed marker is worth saving");
}

#[test]
fn selecting_a_marker_brings_its_colour_to_the_sliders() {
    let mut markers = PhotoMarkers::default();
    markers.colour = Hsva::hsv(205.0, 0.72, 0.85);
    markers.place(auckland(), 41.0);
    markers.colour = Hsva::hsv(36.0, 0.8, 0.9);
    markers.place(auckland(), 42.0);

    // The second marker is selected and the panel holds its amber.
    assert_eq!(markers.colour, Hsva::hsv(36.0, 0.8, 0.9));

    // Selecting the first brings its blue back, so a drag starts from what is on screen.
    markers.select(0);
    assert_eq!(markers.selected, Some(0));
    assert_eq!(markers.colour, Hsva::hsv(205.0, 0.72, 0.85));

    // And an index past the end changes nothing.
    markers.select(7);
    assert_eq!(markers.selected, Some(0));
}

#[test]
fn the_clickable_size_follows_the_size_slider() {
    // The bug this test is here for: the hit test used the model's own 12 m whatever the slider
    // was on, so a marker dragged up to 200 m kept a hit area the size of the 12 m one, down
    // near its foot, and could not be clicked at all.
    let mut markers = PhotoMarkers {
        height: 12.0,
        min_pixels: 0.0,
        ..default()
    };
    markers.place(auckland(), 0.0);
    let marker = &markers.markers[0];

    // A kilometre out, straight along the east so the marker is seen side on and not
    // foreshortened, which is where its height on screen is its full height.
    let camera_position = marker.anchor_position() + Frame::at_unit(marker.unit).east * 1000.0;

    let small = markers.drawn_height(marker, camera_position, Some(1303.6));
    markers.height = 200.0;
    let large = markers.drawn_height(&markers.markers[0], camera_position, Some(1303.6));

    assert_eq!(small, 12.0);
    assert_eq!(large, 200.0, "the drawn height should be the slider's");
}

#[test]
fn the_pixel_floor_counts_towards_the_clickable_size() {
    // A marker held at the floor is drawn larger than the slider says, and is clickable at the
    // size it is drawn, not the size it was asked for.
    let mut markers = PhotoMarkers {
        height: 12.0,
        min_pixels: 48.0,
        ..default()
    };
    markers.place(auckland(), 0.0);
    let marker = &markers.markers[0];
    let camera_position = marker.anchor_position() + Frame::at_unit(marker.unit).east * 10_000.0;

    let drawn = markers.drawn_height(marker, camera_position, Some(1303.6));

    assert!(
        drawn > 12.0,
        "the floor should have scaled it up, got {drawn} m"
    );
    // And with the floor off it is left at the slider's height however far away it is.
    markers.min_pixels = 0.0;
    let bare = markers.drawn_height(&markers.markers[0], camera_position, Some(1303.6));
    assert_eq!(bare, 12.0);
}

#[test]
fn a_marker_keeps_its_photos_path_through_the_file_so_it_can_be_read_back() {
    // What makes a photo come back after a restart: the path is written with the marker, read
    // back with it, and the decode is started again from it as the marker is spawned.
    let mut markers = PhotoMarkers::default();
    markers.place(auckland(), 41.0);
    markers.markers[0].photo = Some(PathBuf::from("/home/kai/Pictures/piha.jpg"));

    let text = to_ron(&markers.markers, "2026-10-08 09:12 UTC").expect("serialises");
    let file: MarkerFile = ron::from_str(&text).expect("parses back");

    assert_eq!(
        file.markers[0].photo,
        Some(PathBuf::from("/home/kai/Pictures/piha.jpg"))
    );

    // And a marker built from that row carries the path, with nothing yet said about whether
    // the file is still there: that is found out by trying to read it.
    let (loaded, unreadable) = {
        let path = std::env::temp_dir().join("photo-markers-test-roundtrip.ron");
        std::fs::write(&path, &text).expect("the temporary file is writable");
        let loaded = load_markers(&path);
        std::fs::remove_file(&path).ok();
        loaded
    };

    assert!(!unreadable);
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0].photo,
        Some(PathBuf::from("/home/kai/Pictures/piha.jpg"))
    );
    assert!(!loaded[0].missing, "nothing has tried to read it yet");
}

#[test]
fn clearing_a_photo_forgets_the_path_and_the_complaint_with_it() {
    let mut markers = PhotoMarkers::default();
    markers.place(auckland(), 41.0);
    let marker = markers.selected_mut().expect("just placed");
    marker.photo = Some(PathBuf::from("/gone/holiday.jpg"));
    marker.missing = true;

    let mut materials = Assets::<StandardMaterial>::default();
    let marker = markers.selected_mut().expect("still selected");
    super::photo::clear_photo(&mut marker.photo, &mut marker.missing, None, &mut materials);

    let marker = markers.selected().expect("still selected");
    assert_eq!(marker.photo, None);
    assert!(!marker.missing);
}

#[test]
fn a_marker_placed_on_a_train_rides_it_and_knows_where_to_fall_back_to() {
    let mut markers = PhotoMarkers::default();
    let carriage = Entity::from_raw_u32(7).expect("a plain entity for the test");

    markers.place_riding(auckland(), 12.0, Some(carriage));
    let marker = markers.selected().expect("just placed");

    assert_eq!(marker.riding, Some(carriage));
    // The ground it was placed over is kept, so losing the train leaves it somewhere sensible
    // rather than at the centre of the earth.
    assert_eq!(marker.ground, 12.0);
    assert_eq!(marker.at, marker.anchor_position());
}

#[test]
fn a_marker_placed_on_the_ground_rides_nothing() {
    let mut markers = PhotoMarkers::default();
    markers.place(auckland(), 12.0);

    assert_eq!(markers.selected().expect("just placed").riding, None);
}

#[test]
fn riding_a_train_is_not_written_to_the_file() {
    // An entity means nothing in the next run, and the trains a marker could ride are not there
    // when the file is read, so a riding marker is written down where it last rode and comes
    // back standing on the ground there.
    let mut markers = PhotoMarkers::default();
    let carriage = Entity::from_raw_u32(7).expect("a plain entity for the test");
    markers.place_riding(auckland(), 12.0, Some(carriage));

    let text = to_ron(&markers.markers, "2026-10-08 09:12 UTC").expect("serialises");
    let path = std::env::temp_dir().join("photo-markers-test-riding.ron");
    std::fs::write(&path, &text).expect("the temporary file is writable");
    let (loaded, unreadable) = load_markers(&path);
    std::fs::remove_file(&path).ok();

    assert!(!unreadable);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].riding, None, "it should come back on the ground");
    assert_eq!(loaded[0].ground, 12.0, "where it last rode");
}
