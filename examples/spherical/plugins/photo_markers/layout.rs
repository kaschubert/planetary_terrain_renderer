//! Where the cards go: docked down the sides of the viewport, never overlapping, never under the
//! cursor.
//!
//! This is boundary labelling, which has one useful result in it and the whole design leans on
//! that one: **put the cards on a side in the same top-to-bottom order as their anchors and no
//! two leaders cross**. If two crossed, swapping the two cards would uncross them and shorten the
//! total leader length, so a crossing-free arrangement is always available and sorting by anchor
//! `y` is the one that finds it. Nothing iterative, nothing to tune: one sort does it.
//!
//! The pass, in order. Which side, with a dead band at the middle so a card near it does not flip
//! sides every frame. Who fits, nearest first, the rest left as dots. In what order, by anchor
//! `y`, which is the rule above. Then the column is settled with one sweep down and one back up,
//! which is order-preserving by construction, so the sort's work is not undone.
//!
//! The cursor is one more obstacle rather than a special case. A circle round the pointer forbids
//! an interval of `y` in each column — the chord of the circle across that column's width — and
//! the interval simply cuts the column into two shorter ones, each packed the same way.
//!
//! Everything here is a function of its arguments. No `World`, no queries, nothing that needs a
//! window: the hard part of this is the part that can be tested without running anything.

use bevy::prelude::*;

/// Clear space between two stacked cards, in pixels.
pub(super) const CARD_GAP: f32 = 10.0;

/// How far a column stands off the edge of the viewport, in pixels.
pub(super) const EDGE_MARGIN: f32 = 16.0;

/// How wide the dead band at the middle is, in pixels, within which a card keeps the side it had.
/// Without it a marker sitting on the centre line swaps columns every frame the camera breathes.
pub(super) const SIDE_BAND: f32 = 60.0;

/// The radius of the circle held clear around the cursor, in pixels, and how much wider that
/// circle is for a card already out of the way. The second is hysteresis: without it a card at
/// the boundary flickers between two placements as the pointer jitters by a pixel.
pub(super) const KEEP_OUT: f32 = 140.0;
pub(super) const KEEP_OUT_LEAVE: f32 = 30.0;

/// How quickly a card moves to a new place: the time constant of an exponential ease, in seconds.
pub(super) const SMOOTH_TAU: f32 = 0.12;

/// How near its target a card has to be before it simply snaps, in pixels. Without this the ease
/// never quite arrives and every card writes its node every frame for ever.
const SNAP: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    Left,
    Right,
}

/// One side's stack: where it is and how much room it has.
///
/// The two are given rather than worked out here because what limits them is not symmetric: the
/// F10 panel sits in the bottom right, so the right column stops above it while the left runs the
/// full height.
#[derive(Debug, Clone, Copy)]
pub(super) struct Column {
    pub side: Side,
    pub top: f32,
    pub bottom: f32,
    /// The edge cards are flush to: a column's left for the left side, its right for the right.
    pub outer: f32,
}

impl Column {
    /// Where a card of this width sits horizontally: against the outer edge either way.
    fn x(&self, width: f32) -> (f32, f32) {
        match self.side {
            Side::Left => (self.outer, self.outer + width),
            Side::Right => (self.outer - width, self.outer),
        }
    }
}

/// One marker as the layout sees it. Nothing of the marker itself: where it falls on screen, how
/// big its card is, how far away it is, and the two things that make the layout steady from one
/// frame to the next.
#[derive(Debug, Clone, Copy)]
pub(super) struct Candidate {
    /// Where the marker falls in the viewport.
    pub anchor: Vec2,
    /// The card's size in pixels, which varies with the photo's shape.
    pub size: Vec2,
    /// Metres from the camera, which decides who keeps a card when there is not room for all.
    pub distance: f64,
    /// The selected marker is never the one dropped for want of room.
    pub selected: bool,
    /// Which side it was on last frame, for the dead band. None for a card that is new.
    pub was: Option<Side>,
}

/// A stretch of a column a card may stand in, and which of the column's cards belong to it.
///
/// A column with no band in it is one stretch that takes all of them; a band cuts it into two,
/// each taking the cards whose anchors are on its side of the band's middle.
#[derive(Debug, Clone, Copy)]
struct Segment {
    top: f32,
    bottom: f32,
    /// The range of anchor `y` this stretch takes, or all of them.
    wants: Option<(f32, f32)>,
}

/// Where one card goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Placement {
    /// Which candidate this is, as an index into the slice given.
    pub index: usize,
    pub rect: Rect,
    pub side: Side,
}

/// The whole engine. Candidates in, placements out; anything with no placement is left as a dot.
/// The radius is given per column rather than once, because the circle is widened for a column
/// it has already pushed something out of and the two sides engage separately.
pub(super) fn lay_out(
    candidates: &[Candidate],
    columns: &[Column; 2],
    middle: f32,
    cursor: Option<Vec2>,
    keep_out: &[f32; 2],
) -> Vec<Placement> {
    let mut placements = Vec::new();

    for (column, &keep_out) in columns.iter().zip(keep_out) {
        let mut mine: Vec<usize> = (0..candidates.len())
            .filter(|&index| side_of(&candidates[index], middle) == column.side)
            .collect();
        if mine.is_empty() {
            continue;
        }

        // The widest card decides the column's span, which is what the cursor's circle is
        // measured against: every card in it is flush to the outer edge, so they share a span.
        let widest = mine.iter().fold(0.0f32, |widest, &index| {
            widest.max(candidates[index].size.x)
        });
        let span = column.x(widest);
        let band = cursor.and_then(|cursor| forbidden_band(cursor, keep_out, span));

        // Nearest first for the budget, then back into anchor order for the packing. The two
        // orders are what the engine is: who gets a card, and where it goes.
        mine.sort_by(|&a, &b| candidates[a].distance.total_cmp(&candidates[b].distance));

        for segment in segments(column, band) {
            let mut seats = fit(
                candidates,
                &mine,
                segment.bottom - segment.top,
                segment.wants,
            );
            seats.sort_by(|&a, &b| candidates[a].anchor.y.total_cmp(&candidates[b].anchor.y));

            let centres = settle(candidates, &seats, segment.top, segment.bottom);
            for (index, centre) in seats.iter().copied().zip(centres) {
                let size = candidates[index].size;
                let (left, right) = column.x(size.x);

                placements.push(Placement {
                    index,
                    rect: Rect {
                        min: Vec2::new(left, centre - size.y / 2.0),
                        max: Vec2::new(right, centre + size.y / 2.0),
                    },
                    side: column.side,
                });
            }
        }
    }

    placements
}

/// Which side a card belongs to: its half of the viewport, except within the dead band either way
/// of the middle, where it keeps the side it already had.
fn side_of(candidate: &Candidate, middle: f32) -> Side {
    if let Some(was) = candidate.was
        && (candidate.anchor.x - middle).abs() < SIDE_BAND / 2.0
    {
        return was;
    }

    match candidate.anchor.x < middle {
        true => Side::Left,
        false => Side::Right,
    }
}

/// The interval of `y` the cursor's circle forbids in a column of this horizontal span, if it
/// reaches the column at all.
///
/// The circle meets the column in a chord, and what matters is how tall that chord is: at a
/// horizontal distance `dx` from the centre, a circle of radius `R` reaches `sqrt(R² - dx²)`
/// either way. Further off than `R` it misses the column and there is no band.
pub(super) fn forbidden_band(cursor: Vec2, radius: f32, span: (f32, f32)) -> Option<(f32, f32)> {
    let dx = (span.0 - cursor.x).max(cursor.x - span.1).max(0.0);
    if dx >= radius {
        return None;
    }

    let half = (radius * radius - dx * dx).sqrt();

    Some((cursor.y - half, cursor.y + half))
}

/// The stretches of a column left once the cursor's band is taken out of it.
///
/// The split is by a single threshold, the band's middle, so it is monotonic in anchor `y`: every
/// card above it goes to the upper stretch and every card below to the lower. That is what keeps
/// the order, and with it the promise that no two tethers cross.
fn segments(column: &Column, band: Option<(f32, f32)>) -> Vec<Segment> {
    let Some((from, to)) = band else {
        return vec![Segment {
            top: column.top,
            bottom: column.bottom,
            wants: None,
        }];
    };

    let middle = (from + to) / 2.0;
    let mut out = Vec::new();

    if from > column.top {
        out.push(Segment {
            top: column.top,
            bottom: from.min(column.bottom),
            wants: Some((f32::MIN, middle)),
        });
    }
    if to < column.bottom {
        out.push(Segment {
            top: to.max(column.top),
            bottom: column.bottom,
            wants: Some((middle, f32::MAX)),
        });
    }

    out
}

/// Who gets a card in a stretch this tall: the ones whose anchors want it, nearest first, taken
/// while the heights and the gaps between them still fit. The selected marker is never the one
/// left out.
fn fit(
    candidates: &[Candidate],
    by_distance: &[usize],
    height: f32,
    wants: Option<(f32, f32)>,
) -> Vec<usize> {
    let mut taken: Vec<usize> = Vec::new();
    let mut used = 0.0;

    // The selected card is offered the room first, so that running out never takes it.
    let order = by_distance
        .iter()
        .copied()
        .filter(|&index| candidates[index].selected)
        .chain(
            by_distance
                .iter()
                .copied()
                .filter(|&index| !candidates[index].selected),
        );

    for index in order {
        if let Some((from, to)) = wants
            && !(from..to).contains(&candidates[index].anchor.y)
        {
            continue;
        }

        let card = candidates[index].size.y;
        let wanted = match taken.is_empty() {
            true => card,
            false => used + CARD_GAP + card,
        };
        if wanted > height {
            continue;
        }

        used = wanted;
        taken.push(index);
    }

    taken
}

/// Where each card's centre ends up, given in anchor order and returned in the same order.
///
/// Each wants its centre at its anchor's height. One pass down pushes each card clear of the one
/// above it, one pass back up pushes it clear of the one below, and both are clamped to the
/// stretch. Two passes, order preserved, done — which is the classic one-dimensional label
/// stacking, and the reason the no-crossing property survives the packing.
fn settle(candidates: &[Candidate], seats: &[usize], top: f32, bottom: f32) -> Vec<f32> {
    let height = |index: usize| candidates[index].size.y;
    let mut centres: Vec<f32> = Vec::with_capacity(seats.len());

    for (place, &index) in seats.iter().enumerate() {
        let wanted = candidates[index].anchor.y;
        let floor = match place {
            0 => top + height(index) / 2.0,
            place => {
                let above = seats[place - 1];
                centres[place - 1] + height(above) / 2.0 + CARD_GAP + height(index) / 2.0
            }
        };
        centres.push(wanted.max(floor));
    }

    for place in (0..seats.len()).rev() {
        let index = seats[place];
        let ceiling = match place + 1 == seats.len() {
            true => bottom - height(index) / 2.0,
            false => {
                let below = seats[place + 1];
                centres[place + 1] - height(below) / 2.0 - CARD_GAP - height(index) / 2.0
            }
        };
        centres[place] = centres[place].min(ceiling);
    }

    centres
}

/// A card's new position on its way to the one the layout gave it: an exponential ease, which is
/// the same however many frames a second the window is running at, and a snap once it is within a
/// pixel so that nothing writes its node for ever.
pub(super) fn ease(current: Vec2, target: Vec2, delta: f32) -> Vec2 {
    if current.distance_squared(target) <= SNAP * SNAP {
        return target;
    }

    current.lerp(target, 1.0 - (-delta / SMOOTH_TAU).exp())
}
