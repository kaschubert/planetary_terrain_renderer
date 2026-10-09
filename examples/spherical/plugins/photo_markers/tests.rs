//! The pure parts of the photo markers: the card's shape, the colour's round trip and the
//! readout's sentences. Nothing here needs a window, a renderer or an asset.

use super::card::{CARD_PIXELS, EMPTY_ASPECT, MAX_RADIUS, card_size, corner_radius};
use super::file::{MarkerFile, load_markers, to_ron};
use super::panel::{hex_of, readout_text, view_text};
use super::photo::readable;
use super::*;
use crate::plugins::rail_editor::frame::tests::AUCKLAND;
use bevy_terrain::math::unit_position;
use std::path::{Path, PathBuf};

/// The place the editor's frame tests use, as a direction on the unit sphere: central Auckland.
fn auckland() -> DVec3 {
    unit_position(AUCKLAND.0, AUCKLAND.1)
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
        aspect: EMPTY_ASPECT,
        card: None,
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
        aspect: EMPTY_ASPECT,
        card: None,
    };

    assert!((marker.longitude() - 174.76334).abs() < 1e-9);
    assert!((marker.latitude() - (-36.84851)).abs() < 1e-9);
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

    let marker = markers.selected_mut().expect("still selected");
    marker.aspect = 1.5;
    super::photo::clear_photo(&mut marker.photo, &mut marker.missing, &mut marker.aspect);

    let marker = markers.selected().expect("still selected");
    assert_eq!(marker.photo, None);
    assert!(!marker.missing);
    // And the card goes back to the shape it had before a photo gave it one.
    assert_eq!(marker.aspect, EMPTY_ASPECT);
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

#[test]
fn a_landscape_photo_gets_a_landscape_card_and_a_portrait_one_a_portrait_card() {
    let wide = card_size(CARD_PIXELS, 16.0 / 9.0);
    let tall = card_size(CARD_PIXELS, 9.0 / 16.0);

    assert!(wide.x > wide.y, "a wide photo makes a wide card: {wide:?}");
    assert!(tall.y > tall.x, "a tall photo makes a tall card: {tall:?}");
    // And neither is stretched: the card is the photo's own shape, which is the whole point of
    // there being no frame to pad onto.
    assert!((wide.x / wide.y - 16.0 / 9.0).abs() < 1e-4);
    assert!((tall.x / tall.y - 9.0 / 16.0).abs() < 1e-4);
}

#[test]
fn the_long_edge_is_always_what_the_slider_says() {
    for aspect in [0.2, 0.5, 1.0, 1.5, 4.0] {
        let size = card_size(200.0, aspect);

        assert!(
            (size.max_element() - 200.0).abs() < 1e-4,
            "{aspect} gave {size:?}, whose long edge is not the slider's 200"
        );
    }
}

#[test]
fn a_card_with_no_photo_is_four_by_three() {
    let size = card_size(CARD_PIXELS, EMPTY_ASPECT);

    assert!((size.x / size.y - 4.0 / 3.0).abs() < 1e-4, "{size:?}");
    // A photo arriving changes the card's size, not the kind of thing it is.
    assert!((size.x - CARD_PIXELS).abs() < 1e-4);
}

#[test]
fn the_corner_radius_is_held_under_half_the_short_edge() {
    // A long thin card at the top of the slider: a radius of MAX_RADIUS would leave the sides
    // bulging through each other, and the renderer would clamp it anyway without the panel's
    // number ever saying so.
    let thin = card_size(160.0, 8.0);
    assert!((corner_radius(MAX_RADIUS, thin) - thin.y / 2.0).abs() < 1e-4);

    // A card with room for it keeps the number the slider is on.
    let square = card_size(160.0, 1.0);
    assert_eq!(corner_radius(12.0, square), 12.0);

    // And zero is square corners rather than anything negative.
    assert_eq!(corner_radius(0.0, square), 0.0);
}

#[test]
fn the_view_readout_says_how_many_of_the_collection_got_a_card() {
    assert_eq!(view_text(0, 0), "no markers yet: Ctrl+click the ground");
    assert_eq!(view_text(0, 12), "none of 12 shown");
    assert_eq!(view_text(3, 12), "3 of 12 shown");
}

#[test]
fn a_card_finds_its_marker_by_id_and_not_by_index() {
    let mut markers = PhotoMarkers::default();
    let first = markers.place(auckland(), 10.0);
    let second = markers.place(auckland(), 20.0);

    assert_eq!(markers.index_of(first), Some(0));
    assert_eq!(markers.index_of(second), Some(1));

    // Removing the first moves the second's index, which is why a card holds an id: looked up by
    // index it would be showing the wrong marker's photo from here on.
    markers.selected = Some(0);
    markers.remove_selected();

    assert_eq!(markers.index_of(first), None);
    assert_eq!(markers.index_of(second), Some(0));
}

// The layout engine. A viewport 1200 by 800, which is the shape of the window the example opens
// at, and cards of one size unless a test wants otherwise.

use super::layout::{CARD_GAP, Candidate, Column, EDGE_MARGIN, Side, ease, lay_out};

const VIEWPORT: Vec2 = Vec2::new(1200.0, 800.0);
/// A landscape card and an upright one, which is what decides a card's column.
const CARD: Vec2 = Vec2::new(160.0, 120.0);
const UPRIGHT: Vec2 = Vec2::new(120.0, 160.0);

/// No order from a frame before, which makes the engine settle one from the anchors.
const NOTHING_HELD: [Vec<MarkerId>; 2] = [Vec::new(), Vec::new()];

fn columns() -> [Column; 2] {
    [
        Column {
            side: Side::Left,
            top: EDGE_MARGIN,
            bottom: VIEWPORT.y - EDGE_MARGIN,
            outer: EDGE_MARGIN,
        },
        Column {
            side: Side::Right,
            top: EDGE_MARGIN,
            bottom: VIEWPORT.y - EDGE_MARGIN,
            outer: VIEWPORT.x - EDGE_MARGIN,
        },
    ]
}

fn at(x: f32, y: f32) -> Candidate {
    // The id matters only to the order-holding, which the tests that care about it set.
    Candidate {
        id: MarkerId(0),
        anchor: Vec2::new(x, y),
        size: CARD,
        // Every card the same distance from the point of interest unless a test says otherwise,
        // so that only the thing under test decides who keeps a card.
        nearness: 1000.0,
        selected: false,
        at: None,
    }
}

/// The same with an upright photograph on it, which sends it to the other column.
fn upright(x: f32, y: f32) -> Candidate {
    Candidate {
        size: UPRIGHT,
        ..at(x, y)
    }
}

/// The layout with nothing in the way: no cursor, and the circle never consulted.
fn settled(candidates: &[Candidate]) -> Vec<super::layout::Placement> {
    // No order held, so every call settles one afresh. A sixtieth of a second, as a frame is;
    // with every card already where it belongs, which `at: None` says, the easing has nothing to
    // do and the answer is the resting one.
    lay_out(candidates, &columns(), &NOTHING_HELD, 1.0 / 60.0).0
}

/// Numbered, so that a test can tell one card from another across frames.
fn numbered(index: u32, x: f32, y: f32) -> Candidate {
    Candidate {
        id: MarkerId(index),
        ..at(x, y)
    }
}

#[test]
fn a_card_goes_to_the_side_its_photograph_is_shaped_for_and_hugs_that_edge() {
    // Two marks in the same place: only the shape of the photograph decides the column.
    let placements = settled(&[at(600.0, 300.0), upright(600.0, 300.0)]);

    let left = placements.iter().find(|p| p.index == 0).expect("placed");
    let right = placements.iter().find(|p| p.index == 1).expect("placed");

    assert_eq!(left.side, Side::Left, "a landscape photograph goes left");
    assert_eq!(right.side, Side::Right, "an upright one goes right");
    assert!((left.rect.min.x - EDGE_MARGIN).abs() < 1e-3, "{left:?}");
    assert!(
        (right.rect.max.x - (VIEWPORT.x - EDGE_MARGIN)).abs() < 1e-3,
        "{right:?}"
    );
}

#[test]
fn the_cards_on_a_side_keep_the_order_of_their_anchors() {
    // Deliberately out of order, and crowded enough that the packing has to move them.
    let candidates = [
        at(200.0, 420.0),
        at(300.0, 100.0),
        at(100.0, 260.0),
        at(250.0, 180.0),
    ];
    let mut placements = settled(&candidates);
    placements.sort_by(|a, b| a.rect.min.y.total_cmp(&b.rect.min.y));

    let anchors: Vec<f32> = placements
        .iter()
        .map(|placement| candidates[placement.index].anchor.y)
        .collect();
    let mut sorted = anchors.clone();
    sorted.sort_by(f32::total_cmp);

    // Top to bottom on screen is top to bottom in the world, which is the whole no-crossing
    // argument: any inversion here would be two tethers crossing.
    assert_eq!(anchors, sorted, "the stack is out of anchor order");
}

#[test]
fn no_two_cards_in_a_column_overlap() {
    // Four anchors within forty pixels of each other: every card has to be moved.
    let candidates = [
        at(200.0, 300.0),
        at(210.0, 310.0),
        at(220.0, 320.0),
        at(230.0, 330.0),
    ];
    let mut placements = settled(&candidates);
    placements.sort_by(|a, b| a.rect.min.y.total_cmp(&b.rect.min.y));

    assert_eq!(placements.len(), 4, "all four fit in an empty column");
    for pair in placements.windows(2) {
        let clear = pair[1].rect.min.y - pair[0].rect.max.y;
        assert!(
            clear >= CARD_GAP - 1e-3,
            "only {clear} px between them, wanted {CARD_GAP}"
        );
    }
}

#[test]
fn every_card_stays_inside_its_column() {
    let candidates: Vec<Candidate> = (0..5).map(|i| at(200.0, 300.0 + i as f32)).collect();

    for placement in settled(&candidates) {
        assert!(
            placement.rect.min.y >= EDGE_MARGIN - 1e-3
                && placement.rect.max.y <= VIEWPORT.y - EDGE_MARGIN + 1e-3,
            "{placement:?} is outside the margins"
        );
    }
}

#[test]
fn the_markers_nearest_the_point_of_interest_are_the_ones_that_keep_a_card() {
    // Six cards of 120 plus the gaps want 770 px; the column has 768, so one must go.
    let near_the_end = |i: usize| Candidate {
        nearness: (5 - i) as f64 * 100.0,
        ..at(200.0, 100.0 + i as f32 * 120.0)
    };
    let candidates: Vec<Candidate> = (0..6).map(near_the_end).collect();

    // The first is the furthest from what was asked for, so it is the one left out.
    let placements = settled(&candidates);
    assert_eq!(placements.len(), 5, "one more than fits was placed");
    assert!(
        !placements.iter().any(|placement| placement.index == 0),
        "the marker furthest from the point of interest kept its card"
    );

    // Ask somewhere else and the answer turns over: now the last one goes.
    let candidates: Vec<Candidate> = (0..6)
        .map(|i| Candidate {
            nearness: i as f64 * 100.0,
            ..near_the_end(i)
        })
        .collect();
    let placements = settled(&candidates);
    assert_eq!(placements.len(), 5);
    assert!(
        !placements.iter().any(|placement| placement.index == 5),
        "the point of interest moved and the same marker was dropped"
    );
}

#[test]
fn the_selected_card_is_never_the_one_left_out() {
    let candidates: Vec<Candidate> = (0..6)
        .map(|i| Candidate {
            // The furthest from what was asked for, which the budget would drop first.
            nearness: (5 - i) as f64 * 100.0,
            selected: i == 0,
            ..at(200.0, 100.0 + i as f32 * 120.0)
        })
        .collect();

    let placements = settled(&candidates);
    assert_eq!(placements.len(), 5);
    assert!(
        placements.iter().any(|placement| placement.index == 0),
        "the selected marker lost its card to a nearer one"
    );
}

#[test]
fn the_ease_closes_on_its_target_and_then_snaps_to_it() {
    let (target, mut at) = (400.0f32, 0.0f32);

    let first = ease(at, target, 1.0 / 60.0);
    assert!(first > at && first < target, "{first}");

    // A second of sixtieths gets there, and the last step is exact rather than asymptotic.
    for _ in 0..60 {
        at = ease(at, target, 1.0 / 60.0);
    }
    assert_eq!(at, target);
}

// The tether: where it leaves the card, and the rule that keeps its end cap out of sight.

use super::card::corner_radius as card_radius;
use super::tether::{attach, cap, curve, distance_to, start};

/// Every direction round a full turn, as a card's anchor might lie in any of them.
fn around() -> impl Iterator<Item = Vec2> {
    (0..72).map(|step| {
        let angle = step as f32 * std::f32::consts::TAU / 72.0;
        Vec2::new(angle.cos(), angle.sin())
    })
}

/// The sizes and radii worth sweeping: the ends of both sliders and something ordinary, plus a
/// long thin card, which is the shape that puts the guard under pressure.
fn shapes() -> impl Iterator<Item = (Vec2, f32)> {
    let sizes = [
        Vec2::new(60.0, 45.0),
        Vec2::new(160.0, 120.0),
        Vec2::new(480.0, 120.0),
        Vec2::new(120.0, 480.0),
    ];

    sizes
        .into_iter()
        .flat_map(|size| [0.0f32, 6.0, 12.0, 40.0].map(move |radius| (size, radius)))
}

#[test]
fn the_tether_leaves_the_card_on_its_border() {
    let centre = Vec2::new(400.0, 300.0);

    for (size, radius) in shapes() {
        let half = size / 2.0;
        let radius = card_radius(radius, size);

        for direction in around() {
            let on = attach(centre, half, radius, direction);
            let distance = distance_to(on, centre, half, radius);

            assert!(
                distance.abs() < 1e-2,
                "{size:?} r{radius}: the attach point is {distance} from the border"
            );
        }
    }
}

#[test]
fn the_end_cap_is_always_hidden_under_the_card() {
    let centre = Vec2::new(400.0, 300.0);

    for (size, radius) in shapes() {
        let half = size / 2.0;
        let radius = card_radius(radius, size);
        let card = Rect::from_center_size(centre, size);

        for direction in around() {
            let anchor = centre + direction * 500.0;
            let (from, normal) = start(card, radius, anchor);

            for corner in cap(from, normal, half) {
                let distance = distance_to(corner, centre, half, radius);

                assert!(
                    distance <= 0.0,
                    "{size:?} r{radius} towards {direction:?}: a cap corner is {distance} px \
                     outside the card, so the flare would show a notch"
                );
            }
        }
    }
}

#[test]
fn the_tether_leaves_an_edge_square_to_it_and_a_corner_along_its_arc() {
    let centre = Vec2::new(400.0, 300.0);
    let size = Vec2::new(200.0, 120.0);
    let card = Rect::from_center_size(centre, size);

    // Straight out to the right, which can only be the right edge.
    let (_, normal) = start(card, 12.0, centre + Vec2::X * 400.0);
    assert!((normal - Vec2::X).length() < 1e-3, "{normal:?}");

    // Down and to the right at 45 degrees, which on a card wider than it is tall is still the
    // bottom edge and not the corner. Worth pinning: it is the easy thing to get wrong by eye.
    let (_, normal) = start(card, 12.0, centre + Vec2::splat(100.0));
    assert!((normal - Vec2::Y).length() < 1e-3, "{normal:?}");

    // Straight at the bottom right corner. Anything shallower leaves the right edge and
    // anything steeper the bottom one, so this is the only direction that crosses the arc: for
    // this card the corner lies about 31 degrees below the horizontal.
    let (_, normal) = start(card, 12.0, centre + Vec2::new(100.0, 60.0) * 3.0);
    assert!(
        normal.x > 0.1 && normal.y > 0.1,
        "a corner's normal points out of both: {normal:?}"
    );
    assert!((normal.length() - 1.0).abs() < 1e-3);
}

#[test]
fn the_curve_runs_from_the_card_to_the_dot_and_leaves_the_way_the_border_faces() {
    let centre = Vec2::new(400.0, 300.0);
    let card = Rect::from_center_size(centre, Vec2::new(160.0, 120.0));
    let anchor = Vec2::new(900.0, 700.0);

    let (from, normal) = start(card, 12.0, anchor);
    let points = curve(from, normal, anchor);

    assert_eq!(points.first().copied(), Some(from));
    assert_eq!(points.last().copied(), Some(anchor));

    // The first step goes the way the border faces, not straight at the dot.
    let first = (points[1] - points[0]).normalize();
    assert!(
        first.dot(normal) > 0.9,
        "the tether sets off at {first:?} rather than along {normal:?}"
    );

    // And it arrives from above, which is what makes it rise out of the mark on the ground.
    let last = (points[points.len() - 1] - points[points.len() - 2]).normalize();
    assert!(last.y > 0.5, "the tether comes into the dot at {last:?}");
}

// The scatter button: the folder it reads and the disc it lays markers in.

use super::scatter::{Scatter, photos_in, placed_unit, spread_at};

#[test]
fn the_panel_says_what_the_scatter_would_draw_on() {
    let mut scatter = Scatter::default();
    assert_eq!(scatter.caption(), "no folder chosen");
    assert!(!scatter.ready(), "nothing to scatter from yet");

    scatter.folder = Some(PathBuf::from("/home/kai/Pictures/piha"));
    assert_eq!(scatter.caption(), "piha: no photos this build can read");
    assert!(!scatter.ready(), "a folder with no photos is not ready");

    scatter.photos = vec![PathBuf::from("a.jpg")];
    assert_eq!(scatter.caption(), "piha: 1 photo");
    assert!(scatter.ready());

    scatter.photos.push(PathBuf::from("b.png"));
    assert_eq!(scatter.caption(), "piha: 2 photos");

    // And the one case that is not about the folder at all.
    let scatter = Scatter {
        no_chooser: true,
        ..default()
    };
    assert_eq!(
        scatter.caption(),
        "no zenity or kdialog to choose a folder with"
    );
}

#[test]
fn only_the_readable_photos_in_a_folder_are_taken_and_in_name_order() {
    let folder = std::env::temp_dir().join("photo-markers-test-scatter");
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("the temporary folder is writable");
    for name in ["b.jpg", "a.png", "notes.txt", "clip.webp"] {
        std::fs::write(folder.join(name), b"not really an image").expect("writable");
    }
    std::fs::create_dir_all(folder.join("subfolder")).expect("writable");

    let photos = photos_in(&folder);
    let names: Vec<String> = photos
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    // The two this build decodes, in name order, and neither the text file, the format it has
    // no codec for, nor the folder.
    assert_eq!(names, vec!["a.png", "b.jpg"]);

    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_scatter_lands_inside_its_own_spread_and_spreads_with_the_altitude() {
    let unit = auckland();
    let frame = Frame::at_unit(unit);
    let ground = TerrainShape::WGS84.position_unit_to_local(unit, 0.0);

    // Dead centre is the point under the camera itself.
    let middle = placed_unit(ground, &frame, 0.0, 0.0);
    assert!(middle.distance(unit) < 1e-12, "{middle:?}");

    let spread = spread_at(10_000.0);
    for step in 0..16 {
        let angle = step as f64 * std::f64::consts::TAU / 16.0;
        let placed = placed_unit(ground, &frame, spread, angle);
        let away = TerrainShape::WGS84
            .position_unit_to_local(placed, 0.0)
            .distance(ground);

        // On the ellipsoid rather than the tangent plane, so a little short of the radius laid
        // out, and never past it.
        assert!(
            away <= spread + 1.0 && away > spread * 0.99,
            "{away} m away, wanted about {spread}"
        );
    }
}

#[test]
fn the_scatter_is_as_wide_as_the_view_and_never_a_single_point() {
    // Four tenths of the altitude, so a press from a kilometre up fills a kilometre of ground.
    assert!((spread_at(10_000.0) - 4_000.0).abs() < 1e-6);
    // And from the deck it is still wide enough that the markers are not all on one spot.
    assert!(spread_at(0.0) >= 200.0);
}

#[test]
fn f10_opens_for_work_then_stands_back_then_puts_away() {
    let showing = Showing::default();
    assert_eq!(showing, Showing::Nothing);
    assert!(!showing.cards(), "nothing is drawn to begin with");
    assert!(!showing.editing());

    // A press from cold goes straight to the working state, as the toggle used to.
    let showing = showing.next();
    assert_eq!(showing, Showing::Editing);
    assert!(showing.cards() && showing.editing());

    // The middle state is the point of the cycle: the photographs stay, and nothing claims the
    // keys that destroy something or the corner the rail editor wants.
    let showing = showing.next();
    assert_eq!(showing, Showing::Photos);
    assert!(showing.cards(), "the photographs stay");
    assert!(!showing.editing(), "but it is no longer editing");

    assert_eq!(showing.next(), Showing::Nothing);
}

#[test]
fn a_card_in_flight_never_laps_the_one_it_is_moving_past() {
    // Four cards whose anchors have just jumped to the other end of the column, which is what a
    // turn of the camera does. Every one of them is a long way from where it now belongs.
    let anchors = [700.0, 730.0, 760.0, 790.0];
    let mut places = [60.0f32, 200.0, 340.0, 480.0];

    // Half a second of frames, which is several times the easing's own constant.
    for _ in 0..30 {
        let candidates: Vec<Candidate> = anchors
            .iter()
            .zip(places)
            .map(|(&y, place)| Candidate {
                at: Some(place),
                ..at(200.0, y)
            })
            .collect();

        let mut drawn = lay_out(&candidates, &columns(), &NOTHING_HELD, 1.0 / 60.0).0;
        assert_eq!(drawn.len(), 4);
        drawn.sort_by(|a, b| a.rect.min.y.total_cmp(&b.rect.min.y));

        // The gaps hold on every frame of the move, not only once it has finished. This is the
        // whole reason the easing lives inside the engine.
        for pair in drawn.windows(2) {
            let clear = pair[1].rect.min.y - pair[0].rect.max.y;
            assert!(clear >= CARD_GAP - 1e-3, "only {clear} px apart mid-move");
        }

        for placement in &drawn {
            // And a card is always flush to its column, so nothing flies across the viewport.
            assert!(
                (placement.rect.min.x - EDGE_MARGIN).abs() < 1e-3,
                "{:?} has left its column",
                placement.rect
            );
            places[placement.index] = placement.rect.center().y;
        }
    }

    // And they do arrive. Four cards of 120 with 10 between them is a stack 510 tall, so they
    // cannot all sit on their anchors; what they can do is get there, which from 60 they have.
    assert!(places[0] > 300.0, "nothing moved: {places:?}");
    for pair in places.windows(2) {
        assert!(
            (pair[1] - pair[0] - (CARD + Vec2::splat(CARD_GAP)).y).abs() < 1.0,
            "the stack has not closed up: {places:?}"
        );
    }
}

#[test]
fn the_right_column_stops_above_the_panel_and_only_when_it_is_really_there() {
    let widest = CARD.x;
    let full = VIEWPORT.y - EDGE_MARGIN;

    // No panel at all, which is every state but the editing one: the columns run the full drop.
    let [left, right] = super::card::columns(VIEWPORT, widest, None);
    assert_eq!((left.bottom, right.bottom), (full, full));

    // A panel in the bottom right corner, given in the same logical pixels the columns are in.
    let panel = Rect::from_corners(Vec2::new(820.0, 500.0), Vec2::new(1184.0, 784.0));
    let [left, right] = super::card::columns(VIEWPORT, widest, Some(panel));
    assert_eq!(
        right.bottom,
        500.0 - CARD_GAP,
        "the right column runs under the panel"
    );
    assert_eq!(
        left.bottom, full,
        "and the left one is none of its business"
    );

    // One that does not reach the column leaves it alone, so a narrow panel costs nothing.
    let clear = Rect::from_corners(Vec2::new(200.0, 500.0), Vec2::new(600.0, 784.0));
    let [_, right] = super::card::columns(VIEWPORT, widest, Some(clear));
    assert_eq!(right.bottom, full);
}

#[test]
fn a_click_on_a_card_picks_that_marker_and_a_click_past_them_picks_none() {
    use super::card::card_under;

    // Two cards down the left margin, as the layout puts them.
    let drawn = [
        (
            3,
            Rect::from_corners(Vec2::new(16.0, 16.0), Vec2::new(176.0, 136.0)),
        ),
        (
            7,
            Rect::from_corners(Vec2::new(16.0, 146.0), Vec2::new(176.0, 266.0)),
        ),
    ];

    assert_eq!(card_under(&drawn, Vec2::new(90.0, 70.0)), Some(3));
    assert_eq!(card_under(&drawn, Vec2::new(90.0, 200.0)), Some(7));

    // The gap between them belongs to neither, and so does the terrain beside them.
    assert_eq!(card_under(&drawn, Vec2::new(90.0, 141.0)), None);
    assert_eq!(card_under(&drawn, Vec2::new(400.0, 70.0)), None);

    // The marker is named by its index in the list, not by where it sits in the column: the
    // second card down is marker 7, and a click on it must not come back as marker 1.
    assert_eq!(card_under(&drawn, Vec2::new(20.0, 150.0)), Some(7));
}

#[test]
fn the_slider_sets_the_selected_marker_alone_and_always_above_the_rest() {
    use super::tether::{MAX_WIDTH, MIN_WIDTH, SELECTED_WIDTH, TETHER_WIDTH};

    // The slider starts where the constant does, so the panel reads as what is drawn.
    assert!(MIN_WIDTH <= SELECTED_WIDTH && SELECTED_WIDTH <= MAX_WIDTH);

    // It can be taken under the width everything else is drawn at. That is allowed on purpose:
    // the honest way to find out whether the selected marker wants to be heavier at all is to
    // be able to make it lighter. The default stands well clear of it.
    assert!(
        MIN_WIDTH < TETHER_WIDTH,
        "the range cannot reach below the rest"
    );
    assert!(
        SELECTED_WIDTH > TETHER_WIDTH,
        "the selected marker does not start out bolder than the others"
    );
}

#[test]
fn the_two_columns_hold_the_two_shapes_and_nothing_mixes() {
    // Marks scattered anywhere, shapes alternating: where a mark is must not matter any more.
    let candidates: Vec<Candidate> = (0..6)
        .map(|i| {
            let (x, y) = (100.0 + i as f32 * 180.0, 100.0 + i as f32 * 90.0);
            match i % 2 {
                0 => at(x, y),
                _ => upright(x, y),
            }
        })
        .collect();

    for placement in settled(&candidates) {
        let size = candidates[placement.index].size;
        let wanted = match size.x >= size.y {
            true => Side::Left,
            false => Side::Right,
        };
        assert_eq!(
            placement.side, wanted,
            "a {size:?} card went to the {:?} column",
            placement.side
        );
    }
}

#[test]
fn the_order_holds_while_the_cards_do_and_settles_again_when_they_change() {
    let frame = |candidates: &[Candidate], held: &[Vec<MarkerId>; 2]| {
        let (mut placements, order) = lay_out(candidates, &columns(), held, 1.0 / 60.0);
        placements.sort_by(|a, b| a.rect.min.y.total_cmp(&b.rect.min.y));

        (
            placements
                .iter()
                .map(|placement| candidates[placement.index].id)
                .collect::<Vec<_>>(),
            order,
        )
    };

    // Three cards, settled from their anchors: top to bottom they are 0, 1, 2.
    let first = [
        numbered(0, 200.0, 100.0),
        numbered(1, 200.0, 300.0),
        numbered(2, 200.0, 500.0),
    ];
    let (down_the_column, held) = frame(&first, &NOTHING_HELD);
    assert_eq!(down_the_column, vec![MarkerId(0), MarkerId(1), MarkerId(2)]);

    // The camera turns and the marks swap over: 2 is now the highest and 0 the lowest. Nothing
    // about which cards are shown has changed, so neither does the order they are shown in.
    let turned = [
        numbered(0, 200.0, 500.0),
        numbered(1, 200.0, 300.0),
        numbered(2, 200.0, 100.0),
    ];
    let (after, held) = frame(&turned, &held);
    assert_eq!(
        after,
        vec![MarkerId(0), MarkerId(1), MarkerId(2)],
        "the cards reordered themselves for a camera move"
    );

    // A fourth card arrives, which is a change of cast, and the order is settled afresh from
    // where the marks now are.
    let joined = [turned[0], turned[1], turned[2], numbered(3, 200.0, 650.0)];
    let (resettled, _) = frame(&joined, &held);
    assert_eq!(
        resettled,
        vec![MarkerId(2), MarkerId(1), MarkerId(0), MarkerId(3)],
        "a new card should have settled the order from the anchors"
    );
}

// The seasons: which one a photograph was taken in, and the colour that gives it.

use super::season::{Season, Seasons, season_of, taken_in};

/// Auckland is south of the equator and Hamburg north of it, which is the whole of the
/// hemisphere question.
const AUCKLAND_LATITUDE: f64 = -36.85;
const HAMBURG_LATITUDE: f64 = 53.55;

#[test]
fn a_december_photograph_is_summer_in_auckland_and_winter_in_hamburg() {
    assert_eq!(season_of(12, AUCKLAND_LATITUDE), Season::Summer);
    assert_eq!(season_of(12, HAMBURG_LATITUDE), Season::Winter);

    // And the other three, turned half a year round against each other.
    for (month, north) in [
        (4, Season::Spring),
        (7, Season::Summer),
        (10, Season::Autumn),
        (1, Season::Winter),
    ] {
        assert_eq!(season_of(month, HAMBURG_LATITUDE), north);
        let south = season_of(month, AUCKLAND_LATITUDE);
        assert_ne!(south, north, "month {month} came out the same either side");
    }
}

#[test]
fn every_month_lands_in_a_season_on_both_sides_of_the_equator() {
    for month in 1..=12 {
        // Nothing to assert beyond its not panicking and the two differing: the point is that
        // the arithmetic that turns the year round cannot fall off the end of it.
        let south = season_of(month, AUCKLAND_LATITUDE);
        let north = season_of(month, HAMBURG_LATITUDE);
        assert_ne!(south, north, "month {month}");
    }
}

#[test]
fn the_month_comes_out_of_the_name_a_phone_gives_a_photograph() {
    assert_eq!(taken_in(Path::new("/photos/20251212_173914.jpg")), Some(12));
    assert_eq!(taken_in(Path::new("20260330_115408.jpg")), Some(3));

    // Anything that is not a phone's date leaves the marker its own colour rather than a guess.
    assert_eq!(taken_in(Path::new("piha.jpg")), None);
    assert_eq!(taken_in(Path::new("1234.jpg")), None, "too few digits");
    assert_eq!(
        taken_in(Path::new("20251312_0000.jpg")),
        None,
        "no month 13"
    );
    assert_eq!(taken_in(Path::new("99999999.jpg")), None, "not a year");
}

#[test]
fn the_palettes_in_the_repository_parse_and_hold_four_seasons_of_colours() {
    let seasons = Seasons::load();
    let mut marker = PhotoMarker {
        id: MarkerId(0),
        unit: auckland(),
        ground: 0.0,
        colour: DEFAULT_COLOUR,
        photo: Some(PathBuf::from("/photos/20251212_173914.jpg")),
        missing: false,
        riding: None,
        at: DVec3::ZERO,
        aspect: EMPTY_ASPECT,
        card: None,
    };

    // A December photograph of Auckland is a summer one, so it is not the default grey.
    let summer = seasons.colour_of(&marker);
    assert_ne!(summer, Color::from(DEFAULT_COLOUR), "no palette was found");

    // The same photograph asked about twice is the same colour: the choice comes from the name,
    // so it survives a save, a reload and a rebuild.
    assert_eq!(seasons.colour_of(&marker), summer);

    // The same file taken half a year away is a different season, so a different colour.
    marker.photo = Some(PathBuf::from("/photos/20250612_173914.jpg"));
    assert_ne!(seasons.colour_of(&marker), summer);

    // And a photograph with no date in its name keeps the colour the sliders hold.
    marker.photo = Some(PathBuf::from("/photos/piha.jpg"));
    assert_eq!(seasons.colour_of(&marker), Color::from(DEFAULT_COLOUR));
}

#[test]
fn the_whole_year_maps_the_way_the_two_hemispheres_do() {
    use Season::{Autumn, Spring, Summer, Winter};

    // The mapping in full, so that it is read off the test rather than worked out from the
    // arithmetic. January to December, north and then south.
    let wanted = [
        (Winter, Summer),
        (Winter, Summer),
        (Spring, Autumn),
        (Spring, Autumn),
        (Spring, Autumn),
        (Summer, Winter),
        (Summer, Winter),
        (Summer, Winter),
        (Autumn, Spring),
        (Autumn, Spring),
        (Autumn, Spring),
        (Winter, Summer),
    ];

    for (month, (north, south)) in wanted.into_iter().enumerate() {
        let month = month as u32 + 1;
        assert_eq!(
            season_of(month, HAMBURG_LATITUDE),
            north,
            "month {month} north"
        );
        assert_eq!(
            season_of(month, AUCKLAND_LATITUDE),
            south,
            "month {month} south"
        );
    }
}

#[test]
fn no_season_is_drawn_in_a_colour_another_season_could_be_mistaken_for() {
    // The file's palettes overlap: whole, the nearest pair across two seasons is summer's
    // Driftwood and autumn's Faded Brick, eight parts in two hundred and fifty-five apart. What
    // is drawn from is cut down to the swatches that keep their distance.
    const APART: f32 = 0.15; // of the 0..1 sRGB range, which is about 38 of 255

    let seasons = Seasons::load();
    let mut closest = f32::INFINITY;
    let mut pair = String::new();

    for one in Season::ALL {
        for other in Season::ALL {
            if one == other {
                continue;
            }
            for a in seasons.swatches(one) {
                for b in seasons.swatches(other) {
                    let (dr, dg, db) = (
                        a.srgb.0 - b.srgb.0,
                        a.srgb.1 - b.srgb.1,
                        a.srgb.2 - b.srgb.2,
                    );
                    let distance = (dr * dr + dg * dg + db * db).sqrt();
                    if distance < closest {
                        closest = distance;
                        pair = format!("{} ({one:?}) and {} ({other:?})", a.name, b.name);
                    }
                }
            }
        }
    }

    assert!(
        closest >= APART,
        "{pair} are only {closest:.3} apart, which reads as the same season"
    );

    // And each season still has something to choose from, or the colouring says nothing.
    for season in Season::ALL {
        assert!(
            seasons.swatches(season).len() >= 3,
            "{season:?} was cut to {} swatches",
            seasons.swatches(season).len()
        );
    }
}
