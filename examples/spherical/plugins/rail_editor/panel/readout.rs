//! The readout's words: what the anchor is, where it stands, what its height means and how
//! steep the line is either side of it, and the sentence about the ground having moved. Pure,
//! so the tests can check the wording with numbers.

use super::RailEditor;
use crate::plugins::auckland_rail::MOVED_LIMIT;
use crate::plugins::provenance::ascii;
use crate::plugins::shared::rail_network::{
    MAX_GRADE, PointMode, RailLine, RailNetwork, RailPoint,
};

/// The height a point has, with what the number means for its mode: a ground point's is an
/// offset from the terrain, a fixed point's is its own, and a between point has none, so what
/// is shown is where the chord puts it.
pub(super) fn height_text(point: &RailPoint, resolved: f64) -> String {
    match point.mode {
        PointMode::Ground => format!("{:+.1} m above ground", point.height.unwrap_or(0.0)),
        PointMode::Fixed => format!("{:.1} m", point.height.unwrap_or(0.0)),
        PointMode::Between => format!("on the chord, {resolved:.1} m"),
    }
}

/// The terrain under a point: this run's sample, else what the file cached, else unknown,
/// which is only ever a point added before the startup sampling landed.
pub(super) fn terrain_text(point: &RailPoint) -> String {
    point
        .sampled
        .or(point.terrain)
        .map_or_else(|| "?".to_string(), |height| format!("{height:.1} m"))
}

/// The grades of the two segments a point joins, the one before it and the one after, as
/// percentages, with the ones steeper than rail climbs marked. A missing side is the end of
/// the line. Two points at the same place make a segment with no run, whose grade is
/// infinite when they differ in height; that reads as vertical.
pub(super) fn grade_text(before: Option<f64>, after: Option<f64>) -> String {
    let side = |grade: Option<f64>| match grade {
        None => "-".to_string(),
        Some(grade) => {
            let text = if grade.is_infinite() {
                "vertical".to_string()
            } else {
                format!("{:.1} %", grade * 100.0)
            };
            if grade > MAX_GRADE {
                format!("{text} (steep)")
            } else {
                text
            }
        }
    };

    format!("grades {} / {}", side(before), side(after))
}

/// How many points the startup sampling found the ground moved under, against the terrain
/// the file had cached, and which of them the camera is at when it has been flown to one.
/// Worded as what startup found and not as what moved since the last save: a save writes
/// this run's samples into the file, and the list stays for the review under way, so after
/// one it is no longer what has moved since the file was saved.
pub(super) fn moved_ground_text(count: usize, cursor: Option<usize>) -> String {
    let points = if count == 1 { "point" } else { "points" };
    let at = cursor.map_or_else(String::new, |at| format!(", at {} of {count}", at + 1));

    format!("startup found the ground moved > {MOVED_LIMIT} m under {count} {points}{at}")
}

/// The anchor and its line, when the anchor names a point that exists, which it may not for
/// the frame after an undo restores a selection the lines have moved on from.
fn anchor_point<'a>(
    editor: &RailEditor,
    network: &'a RailNetwork,
) -> Option<(usize, &'a RailLine)> {
    let (line, index) = editor.anchor?;
    let line = network.lines.get(line)?;

    (index < line.points.len()).then_some((index, line))
}

/// The readout: the anchor's line and index, where it is, the terrain under it, its height
/// by its mode and the grades either side of it, one line each; or, with no anchor, that
/// there is none, and how many points are selected all the same.
pub(super) fn readout_text(editor: &RailEditor, network: &RailNetwork) -> String {
    let Some((index, line)) = anchor_point(editor, network) else {
        return match editor.selection.len() {
            0 => "no point selected".to_string(),
            count => format!("no point selected; {count} selected"),
        };
    };

    let point = &line.points[index];
    let resolved = line.resolved_heights()[index];
    let grades = line.segment_grades();
    let before = index.checked_sub(1).map(|segment| grades[segment]);
    let after = grades.get(index).copied();
    let selected = match editor.selection.len() {
        0 | 1 => String::new(),
        count => format!(", {count} selected"),
    };

    // The name is the one piece of the file in the text; the rest is written here.
    format!(
        "{} {index}{selected}\nlat {:.5}  lon {:.5}\nterrain {}\n{}: {}\n{}",
        ascii(&line.name),
        point.latitude,
        point.longitude,
        terrain_text(point),
        point.mode.name(),
        height_text(point, resolved),
        grade_text(before, after),
    )
}

/// The mode of the anchor, whose mode button is drawn as the current one.
pub(super) fn anchor_mode(editor: &RailEditor, network: &RailNetwork) -> Option<PointMode> {
    anchor_point(editor, network).map(|(index, line)| line.points[index].mode)
}
