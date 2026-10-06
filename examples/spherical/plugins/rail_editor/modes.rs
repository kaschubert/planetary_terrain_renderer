//! The mode operations: the selected points made ground, fixed or between, dropped onto the
//! ground, spanned as a tunnel or a bridge, and the steep run found for them.
//!
//! A change of mode is a change of where a point's height comes from, not of the height, so
//! nothing jumps: a point made fixed holds the height it had, and a point made ground takes
//! as its offset the difference between that height and the terrain under it. Only a point
//! made between moves, onto the chord between its anchors, which is the point of it. The
//! heights are read through RailLine::resolved_heights rather than the positions, so an
//! operation on a dirty frame, with the positions a mutation behind the points, still reads
//! the points as they are. Each operation that changes a point records one undo snapshot
//! first and leaves the selection as it was; the caller marks the network dirty, as for
//! every edit. One that would change nothing, a mode given to points that have it, records
//! nothing and says so, or undo would offer an edit that puts back what is there. The F5
//! panel is what calls them.

use super::{RailEditor, selected_by_line};
use crate::plugins::shared::rail_network::{PointMode, RailLine, RailNetwork, RailPoint};

impl RailEditor {
    /// Gives every selected point the mode, keeping its resolved height: a point made fixed
    /// holds the height it resolves to now, a point made ground takes that height less the
    /// terrain under it as its offset, and a point made between drops its height and moves
    /// onto the chord. The heights are read before any point changes, so the points keep
    /// where they were and not where their neighbours' new modes would put them. False when
    /// nothing is selected, or when every selected point has the mode already, in which case
    /// nothing is recorded either.
    pub fn set_mode(&mut self, network: &mut RailNetwork, mode: PointMode) -> bool {
        let changes = plan(&self.selection, network, |point, resolved| {
            let height = match mode {
                PointMode::Ground => Some(resolved - point.terrain_height()),
                PointMode::Fixed => Some(resolved),
                PointMode::Between => None,
            };
            (mode, height)
        });

        self.apply(network, &changes)
    }

    /// Puts every selected point on the ground with no offset, whatever it was: back to the
    /// state of an import, for a point whose edits are to be forgotten. False when nothing
    /// is selected, or when every selected point is there already, in which case nothing is
    /// recorded either.
    pub fn drop_to_ground(&mut self, network: &mut RailNetwork) -> bool {
        let changes = plan(&self.selection, network, |_, _| {
            (PointMode::Ground, Some(0.0))
        });

        self.apply(network, &changes)
    }

    /// Whether span_selection has anything to span: a line with three or more of the selected
    /// points on it. The panel dims its button by this.
    pub fn can_span(&self, network: &RailNetwork) -> bool {
        spannable(&selected_by_line(&self.selection, network))
    }

    /// Makes the selection on each line a span: the lowest and the highest selected points
    /// become fixed at the heights they resolve to now, and every point between them,
    /// selected or not, becomes between and drops onto the chord joining the two. This is
    /// the two-click tunnel: click one portal, Shift-click the other, span. A line with fewer
    /// than three points selected has no inside to span and is left alone; false when no line
    /// has three, in which case nothing is recorded either.
    pub fn span_selection(&mut self, network: &mut RailNetwork) -> bool {
        let selected = selected_by_line(&self.selection, network);
        if !spannable(&selected) {
            return false;
        }
        self.record(network);

        for (line, indices) in network.lines.iter_mut().zip(&selected) {
            if let (true, Some(&first), Some(&last)) =
                (indices.len() >= 3, indices.first(), indices.last())
            {
                span(line, first, last);
            }
        }

        true
    }

    /// Grows the selection along each selected point's line over the steep ground either
    /// side of it: every consecutive segment whose grade exceeds max_grade, starting with
    /// the ones the point itself is an end of, and then every point of that run. What is
    /// selected is the run from the last point on gentle ground before the climb to the first
    /// after it, which is what span_selection wants. A stretch already spanned is not ground,
    /// however steep its chord, and stops the run, as it stops the red the lines draw; see
    /// steep_run. Returns how many points were added. The selection is all that changes, so
    /// there is nothing to undo and nothing is recorded, as with a click.
    ///
    /// A run is one climb or one descent, and a hill with a flat top is two: a steep flank up,
    /// a gentle crown, and a steep flank down. From a point on one flank this selects that
    /// flank alone, and spanning it would lay the chord up the hillside. For such a hill,
    /// click the portal on one side, Shift-click the portal on the other, and span that.
    pub fn select_steep_run(&mut self, network: &RailNetwork, max_grade: f64) -> usize {
        let selected = selected_by_line(&self.selection, network);
        let mut added = 0;

        for (line_index, (line, indices)) in network.lines.iter().zip(&selected).enumerate() {
            if indices.is_empty() {
                continue;
            }
            let modes: Vec<PointMode> = line.points.iter().map(|point| point.mode).collect();
            let grades = line.segment_grades();

            for &index in indices {
                let (first, last) = steep_run(&modes, &grades, index, max_grade);
                for point in first..=last {
                    if !self.selection.contains(&(line_index, point)) {
                        self.selection.push((line_index, point));
                        added += 1;
                    }
                }
            }
        }

        added
    }

    /// Records one snapshot and makes the changes. False, and nothing recorded, when there
    /// are none: an undo that restores what is already there is not one to offer.
    fn apply(&mut self, network: &mut RailNetwork, changes: &[Change]) -> bool {
        if changes.is_empty() {
            return false;
        }
        self.record(network);

        for &(line, index, mode, height) in changes {
            let point = &mut network.lines[line].points[index];
            point.mode = mode;
            point.height = height;
        }

        true
    }
}

/// Heights closer than this are the same height, in metres. A ground point made ground again
/// takes its offset back from its resolved height, a sum and then a difference in floating
/// point, which lands a rounding error off the offset it had and not on it; a micrometre is
/// far above that error at any height on land, and a ten-thousandth of the centimetre the
/// file keeps.
const SAME_HEIGHT: f64 = 1e-6;

/// What an operation would make of one point: its line, its index, and the mode and height
/// it is to have.
type Change = (usize, usize, PointMode, Option<f64>);

/// What an operation would do to the selected points, as the changes that change anything:
/// the mode and height each is to have, from the point and the height it resolves to now,
/// less the points the operation would leave as they are. The heights are read per line
/// before any point changes.
fn plan(
    selection: &[(usize, usize)],
    network: &RailNetwork,
    new: impl Fn(&RailPoint, f64) -> (PointMode, Option<f64>),
) -> Vec<Change> {
    let mut changes = Vec::new();

    for (line_index, (line, indices)) in network
        .lines
        .iter()
        .zip(selected_by_line(selection, network))
        .enumerate()
    {
        if indices.is_empty() {
            continue;
        }
        let heights = line.resolved_heights();

        for index in indices {
            let point = &line.points[index];
            let (mode, height) = new(point, heights[index]);
            if !unchanged(point, mode, height) {
                changes.push((line_index, index, mode, height));
            }
        }
    }

    changes
}

/// Whether the point has the mode and the height already, give or take SAME_HEIGHT.
fn unchanged(point: &RailPoint, mode: PointMode, height: Option<f64>) -> bool {
    point.mode == mode
        && match (point.height, height) {
            (None, None) => true,
            (Some(has), Some(wants)) => (has - wants).abs() < SAME_HEIGHT,
            _ => false,
        }
}

/// Whether any line has the three selected points a span needs: two ends and an inside.
fn spannable(selected: &[Vec<usize>]) -> bool {
    selected.iter().any(|indices| indices.len() >= 3)
}

/// The span itself: the two ends fixed at the heights they have, the inside between.
fn span(line: &mut RailLine, first: usize, last: usize) {
    let heights = line.resolved_heights();

    for end in [first, last] {
        let point = &mut line.points[end];
        point.mode = PointMode::Fixed;
        point.height = Some(heights[end]);
    }
    for point in &mut line.points[first + 1..last] {
        point.mode = PointMode::Between;
        point.height = None;
    }
}

/// The run of consecutive segments of steep ground that the point stands in, as the indices
/// of the first and the last point it covers; the point alone when neither segment beside it
/// is one. Steep is a grade over max_grade, and ground is a segment with no between point at
/// either end, the rule the strokes draw by, so the run stops where the red does. A span's
/// chord is often steep, a tunnel through a hill being what it is, and a run that went on
/// through it would reach the far portal and hand it to span_selection to unfix. Segment i
/// joins points i and i + 1, so the segment before a point is i - 1 and the one after it is
/// i.
fn steep_run(modes: &[PointMode], grades: &[f64], index: usize, max_grade: f64) -> (usize, usize) {
    let steep_ground = |segment: usize| {
        grades[segment] > max_grade && !modes[segment..=segment + 1].contains(&PointMode::Between)
    };

    let mut first = index;
    while first > 0 && steep_ground(first - 1) {
        first -= 1;
    }

    let mut last = index;
    while last < grades.len() && steep_ground(last) {
        last += 1;
    }

    (first, last)
}

#[cfg(test)]
mod tests;
