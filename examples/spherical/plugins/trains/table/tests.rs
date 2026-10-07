//! The table's words and the icon's colours checked with numbers; its nodes need a window
//! and are not.

use super::*;

#[test]
fn the_distance_reads_in_kilometres_to_one_decimal() {
    assert_eq!(km_text(0.0), "0.0");
    assert_eq!(km_text(1234.56), "1.2");
    assert_eq!(km_text(1250.0), "1.2");
    assert_eq!(km_text(31_849.9), "31.8");
}

#[test]
fn the_direction_is_an_arrow_along_the_point_order() {
    assert_eq!(dir_text(1.0), ">");
    assert_eq!(dir_text(-1.0), "<");
    // A heading of nought is taken as forward, as advance takes it.
    assert_eq!(dir_text(0.0), ">");
}

#[test]
fn the_speed_reads_in_whole_kilometres_per_hour() {
    assert_eq!(speed_text(20.0), "72");
    assert_eq!(speed_text(0.0), "0");
    assert_eq!(speed_text(30.5556), "110");
}

#[test]
fn a_cell_is_its_column_and_the_names_are_folded_to_the_font() {
    let train = Train {
        id: "AMP\u{00a0}1142".to_string(),
        distance: 500.0,
        direction: 1.0,
        speed: 20.0,
        ..Train::stand_in(0)
    };
    assert_eq!(cell_text(Column::Line, "S-C", &train), "S-C");
    assert_eq!(
        cell_text(Column::Line, "Onehunga\u{2013}West", &train),
        "Onehunga-West"
    );
    assert_eq!(cell_text(Column::Unit, "S-C", &train), "AMP 1142");
    assert_eq!(cell_text(Column::Km, "S-C", &train), "0.5");
    assert_eq!(cell_text(Column::Speed, "S-C", &train), "72");
    assert_eq!(cell_text(Column::Next, "S-C", &train), "");
    assert_eq!(cell_text(Column::Late, "S-C", &train), "");

    let due = Train {
        next_stop: Some("Te Waihorotiu\u{2013}2".to_string()),
        delay: Some(130.0),
        ..train.clone()
    };
    assert_eq!(cell_text(Column::Next, "S-C", &due), "Te Waihorotiu-2");
    assert_eq!(cell_text(Column::Late, "S-C", &due), "+2 min");

    let back = Train {
        direction: -1.0,
        ..train
    };
    assert_eq!(cell_text(Column::Dir, "S-C", &back), "<");

    // A stand-in has no unit, and the column is blank for it.
    assert_eq!(cell_text(Column::Unit, "S-C", &Train::stand_in(0)), "");
    assert_eq!(Column::ALL.len() + 1, HEADINGS.len());
}

#[test]
fn how_late_reads_in_whole_minutes_and_within_a_minute_is_on_time() {
    assert_eq!(late_text(None), "");
    assert_eq!(late_text(Some(0.0)), "on time");
    assert_eq!(late_text(Some(59.0)), "on time");
    assert_eq!(late_text(Some(-59.0)), "on time");
    assert_eq!(late_text(Some(60.0)), "+1 min");
    assert_eq!(late_text(Some(130.0)), "+2 min");
    assert_eq!(late_text(Some(-106.0)), "-1 min");
}

#[test]
fn the_icon_is_grey_until_followed_and_answers_the_pointer_either_way() {
    assert_eq!(icon_colour(Interaction::None, None), IDLE_COLOUR);
    assert_eq!(icon_colour(Interaction::Hovered, None), HOVER_COLOUR);
    assert_eq!(icon_colour(Interaction::Pressed, None), PRESSED_COLOUR);

    let line = Color::srgb(0.2, 0.4, 0.8);
    assert_eq!(icon_colour(Interaction::None, Some(line)), line);
    for interaction in [Interaction::Hovered, Interaction::Pressed] {
        let lit = icon_colour(interaction, Some(line));
        assert_ne!(lit, line);
        assert!(lit.luminance() > line.luminance(), "{lit:?}");
    }
}
