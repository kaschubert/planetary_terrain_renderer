//! Where the cards go: docked down the sides of the viewport, never overlapping, never under the
//! cursor.
//!
//! This is boundary labelling, which has one useful result in it and the whole design leans on
//! that one: **put the cards on a side in the same top-to-bottom order as their anchors and no
//! two leaders cross**. If two crossed, swapping the two cards would uncross them and shorten the
//! total leader length, so a crossing-free arrangement is always available and sorting by anchor
//! `y` is the one that finds it. Nothing iterative, nothing to tune: one sort does it.
//!
//! The pass, in order. Which side, by the shape of the photograph: the landscape ones down one
//! edge and the upright ones down the other. Who fits, nearest the pointer first, the rest left
//! as dots. In what order, by anchor `y`, which is the rule above. Then the column is settled
//! with one sweep down and one back up, which is order-preserving by construction, so the sort's
//! work is not undone.
//!
//! A point of interest decides who gets a card, not where the cards go. When there is not room
//! for every marker on a side, the ones lying nearest that point keep their cards and the rest
//! are left as dots. The nearness is given per candidate and measured in the world, so the
//! engine never learns what the point is — only which markers are close to it.
//!
//! Everything here is a function of its arguments. No `World`, no queries, nothing that needs a
//! window: the hard part of this is the part that can be tested without running anything.

use bevy::prelude::*;

use super::MarkerId;

/// Clear space between two stacked cards, in pixels.
pub(super) const CARD_GAP: f32 = 10.0;

/// How far a column stands off the edge of the viewport, in pixels.
pub(super) const EDGE_MARGIN: f32 = 16.0;

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

/// Which edge each shape of photograph docks to: the landscape ones one side, the upright ones
/// the other. A choice rather than a law — swapping the two turns the layout round.
const LANDSCAPE_SIDE: Side = Side::Left;
const UPRIGHT_SIDE: Side = Side::Right;

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
    /// Which marker this is, which is how an order outlives a frame.
    pub id: MarkerId,
    /// Where the marker falls in the viewport.
    pub anchor: Vec2,
    /// The card's size in pixels, which varies with the photo's shape.
    pub size: Vec2,
    /// How far this marker lies from the point of interest, in metres on the ground. The nearest
    /// keep their cards when a column cannot hold every marker of its shape.
    pub nearness: f64,
    /// The selected marker is never the one dropped for want of room.
    pub selected: bool,
    /// Where the card's middle is on screen at this moment, which is what it eases away from.
    /// None for a card that is new, which simply appears where it belongs.
    pub at: Option<f32>,
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
///
/// What comes back is where to draw a card **this** frame, easing included, and not where it is
/// eventually headed. That is deliberate: a layout that only promises the destinations promises
/// nothing about the journey, and a card in flight will happily cross one that has already
/// arrived. So the move is made here, between the two sweeps, and the second sweep holds the
/// gaps open on every frame rather than only on the last one.
pub(super) fn lay_out(
    candidates: &[Candidate],
    columns: &[Column; 2],
    held: &[Vec<MarkerId>; 2],
    delta: f32,
) -> (Vec<Placement>, [Vec<MarkerId>; 2]) {
    let mut placements = Vec::new();
    let mut order = [Vec::new(), Vec::new()];

    for (slot, column) in columns.iter().enumerate() {
        let mut mine: Vec<usize> = (0..candidates.len())
            .filter(|&index| side_of(&candidates[index]) == column.side)
            .collect();
        if mine.is_empty() {
            continue;
        }

        // Nearest the point of interest first, which is the whole of the choosing: a column
        // that cannot hold every card of its shape holds the ones nearest what you asked for.
        // Then back into anchor order for the packing, which is the no-crossing rule and has
        // nothing to do with the choosing.
        mine.sort_by(|&a, &b| candidates[a].nearness.total_cmp(&candidates[b].nearness));

        let mut seats = fit(candidates, &mine, column.bottom - column.top);

        // The order is settled when the cast changes and held until it changes again.
        //
        // Sorting by the anchors afresh every frame was the obvious thing and the wrong one:
        // the anchors move whenever the camera does, so two cards whose marks drifted past each
        // other swapped places in the column and slid through one another, for a reason nobody
        // watching could see. Holding the order costs the no-crossing promise between the frame
        // it was settled on and the next change — two tethers in a column may cross once their
        // marks have swapped over — and that is the cheaper of the two prices.
        match same_cast(&held[slot], &seats, candidates) {
            true => seats = in_the_order_held(&held[slot], &seats, candidates),
            false => {
                seats.sort_by(|&a, &b| candidates[a].anchor.y.total_cmp(&candidates[b].anchor.y))
            }
        }
        order[slot] = seats.iter().map(|&seat| candidates[seat].id).collect();

        let heights: Vec<f32> = seats.iter().map(|&seat| candidates[seat].size.y).collect();
        let anchors: Vec<f32> = seats
            .iter()
            .map(|&seat| candidates[seat].anchor.y)
            .collect();

        // Where each card is headed, where it has got to, and then the gaps again.
        let targets = settle(&heights, &anchors, column.top, column.bottom);
        let moved: Vec<f32> = seats
            .iter()
            .zip(&targets)
            .map(|(&seat, &target)| match candidates[seat].at {
                Some(at) => ease(at, target, delta),
                // A card that is new appears where it belongs rather than flying in from
                // wherever the last one to hold that place happened to be.
                None => target,
            })
            .collect();
        let centres = settle(&heights, &moved, column.top, column.bottom);

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

    (placements, order)
}

/// Whether these are the same cards as last frame, in any order. Ten at the most, so the pair of
/// loops costs nothing worth saving.
fn same_cast(held: &[MarkerId], seats: &[usize], candidates: &[Candidate]) -> bool {
    held.len() == seats.len()
        && held
            .iter()
            .all(|id| seats.iter().any(|&seat| candidates[seat].id == *id))
}

/// The same cards, put back into the order they were in. Only called when `same_cast` says every
/// one of them is still there, so nothing is lost to a lookup that fails.
fn in_the_order_held(held: &[MarkerId], seats: &[usize], candidates: &[Candidate]) -> Vec<usize> {
    held.iter()
        .filter_map(|id| {
            seats
                .iter()
                .copied()
                .find(|&seat| candidates[seat].id == *id)
        })
        .collect()
}

/// Which side a card belongs to: the shape of the photograph on it, and nothing else.
///
/// Not the half of the viewport its mark happens to fall in, which is what this was. Sorting by
/// shape gives each column one kind of card to stack, and a landscape photograph never sits in a
/// column sized for upright ones. It needs no hysteresis either: a photograph's proportions do
/// not change as the camera turns, so there is nothing to flicker between.
///
/// What it costs is the tethers. A landscape photograph of something away to the right now docks
/// on the left, and its line crosses the view to get there; lines to opposite columns cross each
/// other freely. The no-crossing rule still holds inside a column, which is where it was ever a
/// promise, but the picture is a busier one than sorting by place gave.
fn side_of(candidate: &Candidate) -> Side {
    match candidate.size.x >= candidate.size.y {
        true => LANDSCAPE_SIDE,
        false => UPRIGHT_SIDE,
    }
}

/// Who gets a card in a column this tall: taken in the order given, which is nearest the pointer
/// first, while the heights and the gaps between them still fit. The selected marker is never
/// the one left out.
fn fit(candidates: &[Candidate], by_nearness: &[usize], height: f32) -> Vec<usize> {
    let mut taken: Vec<usize> = Vec::new();
    let mut used = 0.0;

    // The selected card is offered the room first, so that running out never takes it.
    let order = by_nearness
        .iter()
        .copied()
        .filter(|&index| candidates[index].selected)
        .chain(
            by_nearness
                .iter()
                .copied()
                .filter(|&index| !candidates[index].selected),
        );

    for index in order {
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
/// Each wants to be where `wanted` says. One pass down pushes each card clear of the one above
/// it, one pass back up pushes it clear of the one below, and both are clamped to the stretch.
/// Two passes, order preserved, done — which is the classic one-dimensional label stacking, and
/// the reason the no-crossing property survives the packing.
///
/// It is run twice a frame on the same cards: once on where their anchors are, to find where
/// they are headed, and once on where the easing has actually put them, so that what is drawn
/// obeys the gaps too.
fn settle(heights: &[f32], wanted: &[f32], top: f32, bottom: f32) -> Vec<f32> {
    let mut centres: Vec<f32> = Vec::with_capacity(heights.len());

    for (place, &height) in heights.iter().enumerate() {
        let floor = match place {
            0 => top + height / 2.0,
            place => centres[place - 1] + heights[place - 1] / 2.0 + CARD_GAP + height / 2.0,
        };
        centres.push(wanted[place].max(floor));
    }

    for place in (0..heights.len()).rev() {
        let ceiling = match place + 1 == heights.len() {
            true => bottom - heights[place] / 2.0,
            false => {
                centres[place + 1] - heights[place + 1] / 2.0 - CARD_GAP - heights[place] / 2.0
            }
        };
        centres[place] = centres[place].min(ceiling);
    }

    centres
}

/// A card's height on its way to the one the layout gave it: an exponential ease, the same
/// however many frames a second the window is running at, and a snap once it is within a pixel so
/// that nothing writes its node for ever.
///
/// Only the height. A card is always flush to its column's outer edge, so there is nothing to
/// ease sideways; a card that changes column arrives at the new one's edge at once and walks the
/// rest of the way up or down. Easing the width as well was what sent one flying diagonally
/// across the viewport, over everything in its path.
pub(super) fn ease(current: f32, target: f32, delta: f32) -> f32 {
    if (target - current).abs() <= SNAP {
        return target;
    }

    current + (target - current) * (1.0 - (-delta / SMOOTH_TAU).exp())
}
