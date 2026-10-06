//! How each stretch of a line is drawn: solid where the track is on the ground, dashed where
//! it is between two anchors, in a tunnel or on a bridge, and in a warning colour while
//! editing where a draped stretch climbs steeper than rail can, which is where a tunnel or a
//! bridge is still to be made. The splitting is pure, so it is tested with numbers; the
//! drawing in auckland_rail.rs puts the runs into the two gizmo groups.

use crate::plugins::shared::rail_network::{MAX_GRADE, PointMode};
use bevy::color::Color;

/// The colour of a stretch too steep for rail, shown while editing. A red leaning to orange
/// and brighter than the S-C line's crimson, D52923, so that on that line the steep stretches
/// still stand apart, while on the green E-W and the blue O-W it is the only red there is.
/// Not the amber of a fixed point's disc, which a stretch of it would be mistaken for. The
/// editor's panel puts its error toast in the same red, through auckland_rail's re-export,
/// so that a retune here reaches both.
pub const WARNING_COLOUR: Color = Color::srgb(1.0, 0.25, 0.1);

/// How one stretch of a line is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stroke {
    /// Solid, in the line's colour: track on the ground.
    Track,
    /// Dashed, in the line's colour: between two anchors, a tunnel or a bridge.
    Span,
    /// Solid, in the warning colour: track on the ground steeper than rail climbs.
    Steep,
}

impl Stroke {
    pub(super) fn dashed(self) -> bool {
        matches!(self, Self::Span)
    }

    /// The colour the stroke draws in, given the line's own.
    pub(super) fn colour(self, line: Color) -> Color {
        match self {
            Self::Track | Self::Span => line,
            Self::Steep => WARNING_COLOUR,
        }
    }
}

/// Consecutive segments drawn the same way, as the indices of the first and the last point
/// they cover, inclusive. Neighbouring runs share their boundary point, so each draws as one
/// linestrip and the line has no gap at the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Run {
    pub stroke: Stroke,
    pub first: usize,
    pub last: usize,
}

/// Splits a line into runs by the modes of its points and the grades of its segments, one
/// grade per segment and so one fewer than the modes. A segment with a between point at
/// either end is a span: the chord into a tunnel starts at the portal, which is fixed, so the
/// dashes have to reach it. The rest is track, and while editing, track steeper than
/// MAX_GRADE is steep; off, the lines look as the file has them, since the warning is for the
/// person about to act on it. Fewer than two points make no segment and no run.
pub(super) fn split_runs(modes: &[PointMode], grades: &[f64], editing: bool) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();

    for (index, pair) in modes.windows(2).enumerate() {
        let steep = editing && grades.get(index).is_some_and(|&grade| grade > MAX_GRADE);
        let stroke = if pair.contains(&PointMode::Between) {
            Stroke::Span
        } else if steep {
            Stroke::Steep
        } else {
            Stroke::Track
        };

        match runs.last_mut() {
            Some(run) if run.stroke == stroke => run.last = index + 1,
            _ => runs.push(Run {
                stroke,
                first: index,
                last: index + 1,
            }),
        }
    }

    runs
}

#[cfg(test)]
mod tests;
