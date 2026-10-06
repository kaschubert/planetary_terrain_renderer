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
fn a_cell_is_its_column_and_the_name_is_folded_to_the_font() {
    assert_eq!(cell_text(Column::Line, "S-C", 500.0, 1.0, 20.0), "S-C");
    assert_eq!(
        cell_text(Column::Line, "Onehunga\u{2013}West", 500.0, 1.0, 20.0),
        "Onehunga-West"
    );
    assert_eq!(cell_text(Column::Km, "S-C", 500.0, 1.0, 20.0), "0.5");
    assert_eq!(cell_text(Column::Dir, "S-C", 500.0, -1.0, 20.0), "<");
    assert_eq!(cell_text(Column::Speed, "S-C", 500.0, 1.0, 20.0), "72");
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
