//! A Catmull-Rom spline through points in f64, with its length measured so that it can be
//! resampled by distance.
//!
//! Bevy has CubicCardinalSpline, but it builds curves only over vector spaces with f32
//! scalars, and the positions here are absolute metres on the spheroid, six million and
//! more, where f32 keeps about half a metre. So the little that is needed is written here
//! in f64: a tangent at every point, a cubic between each pair, a position and a tangent at
//! a parameter, and a table of distance against parameter to look the frames up in.
//!
//! The spline is the centripetal variant: the parameter runs between two points by the
//! square root of the distance between them, not by one as the uniform variant has it. The
//! lines' points are anywhere from a metre to two kilometres apart, and neighbouring
//! segments differ by a factor of four hundred at the worst, where a uniform Catmull-Rom
//! throws a loop out of the short segment to match the tangent the long one asks for. The
//! centripetal one has no loops or cusps whatever the spacing, and keeps close to the
//! polyline, which is what the track wants: the points are where the line is, and the
//! spline is only there to round the corners.

use bevy::math::DVec3;

/// Two points closer than this, in metres, are the same place twice and the second is left
/// out: a segment of no length has no tangent to speak of, and the parameter would not
/// advance across it.
const COINCIDENT: f64 = 0.001;

/// Newton steps taken to find the parameter at a distance within a piece of the table, see
/// ArcLength::parameter. The speed across a piece changes most where a segment of over a
/// kilometre leaves a point whose other chord is a few metres: there it quadruples within
/// the first sixteenth of the segment, and the interpolated start is metres out. Three
/// steps bring it from there to under a millimetre, which the test on the network's worst
/// spacing holds it to.
const NEWTON_STEPS: usize = 3;

/// One cubic between two neighbouring points, as the coefficients of a + bt + ct² + dt³ for
/// t from 0 to 1.
#[derive(Debug, Clone, Copy)]
struct Segment {
    coefficients: [DVec3; 4],
}

impl Segment {
    /// The cubic from one point to the next with the tangents given at each end, as a Hermite
    /// segment. The tangents are in the segment's own parameter, so a velocity at a point
    /// in the spline's parameter is scaled by the segment's span first.
    fn hermite(start: DVec3, end: DVec3, start_tangent: DVec3, end_tangent: DVec3) -> Self {
        let chord = end - start;

        Self {
            coefficients: [
                start,
                start_tangent,
                chord * 3.0 - start_tangent * 2.0 - end_tangent,
                chord * -2.0 + start_tangent + end_tangent,
            ],
        }
    }

    fn position(&self, t: f64) -> DVec3 {
        let [a, b, c, d] = self.coefficients;
        a + (b + (c + d * t) * t) * t
    }

    fn tangent(&self, t: f64) -> DVec3 {
        let [_, b, c, d] = self.coefficients;
        b + (c * 2.0 + d * 3.0 * t) * t
    }
}

/// A spline through a line's points, parameterised from 0 at the first point to the number
/// of segments at the last, one unit of parameter per segment.
#[derive(Debug, Clone)]
pub(super) struct CatmullRom {
    segments: Vec<Segment>,
}

impl CatmullRom {
    /// The spline through the points in order, leaving out a point that repeats the one
    /// before it. None when fewer than two distinct points remain, since a spline through one
    /// point is nowhere.
    pub(super) fn through(points: &[DVec3]) -> Option<Self> {
        let mut distinct: Vec<DVec3> = Vec::with_capacity(points.len());
        for &point in points {
            if distinct
                .last()
                .is_none_or(|&last| last.distance(point) >= COINCIDENT)
            {
                distinct.push(point);
            }
        }
        if distinct.len() < 2 {
            return None;
        }

        // The parameter span of each segment: the square root of its chord, which is the
        // centripetal spacing.
        let spans: Vec<f64> = distinct
            .windows(2)
            .map(|pair| pair[0].distance(pair[1]).sqrt())
            .collect();
        let velocities = velocities(&distinct, &spans);

        let segments = distinct
            .windows(2)
            .zip(&spans)
            .enumerate()
            .map(|(index, (pair, &span))| {
                Segment::hermite(
                    pair[0],
                    pair[1],
                    velocities[index] * span,
                    velocities[index + 1] * span,
                )
            })
            .collect();

        Some(Self { segments })
    }

    /// The parameter at the last point.
    pub(super) fn end(&self) -> f64 {
        self.segments.len() as f64
    }

    pub(super) fn position(&self, t: f64) -> DVec3 {
        let (segment, t) = self.segment(t);
        segment.position(t)
    }

    /// The direction the spline runs in at the parameter, not normalised: its length is in
    /// metres per unit of parameter, which is nothing in particular.
    pub(super) fn tangent(&self, t: f64) -> DVec3 {
        let (segment, t) = self.segment(t);
        segment.tangent(t)
    }

    /// The length of the spline from its start to every subdivision of every segment, so a
    /// distance along it can be turned back into a parameter. The curve between two
    /// subdivisions is taken as its chord, so the table is short of the true length by a
    /// little, and the more subdivisions the less.
    pub(super) fn measure(&self, subdivisions: usize) -> ArcLength {
        let mut entries = Vec::with_capacity(self.segments.len() * subdivisions + 1);
        let mut distance = 0.0;
        let mut previous = self.position(0.0);
        entries.push((0.0, 0.0));

        for (index, segment) in self.segments.iter().enumerate() {
            for piece in 1..=subdivisions {
                let within = piece as f64 / subdivisions as f64;
                let position = segment.position(within);
                distance += previous.distance(position);
                previous = position;
                entries.push((index as f64 + within, distance));
            }
        }

        ArcLength { entries }
    }

    /// The segment a parameter falls in and the parameter within it. The end of the spline
    /// is the end of its last segment rather than the start of one past it, and anything
    /// beyond either end is clamped.
    fn segment(&self, t: f64) -> (&Segment, f64) {
        let last = self.segments.len() - 1;
        let index = (t.floor().max(0.0) as usize).min(last);
        (&self.segments[index], (t - index as f64).clamp(0.0, 1.0))
    }
}

/// The spline's velocity at every point, in position per unit of parameter: at a point
/// between two others, the slope at the middle point of the parabola through the three,
/// which is what the Catmull-Rom tangent is; at an end, along the circle through the first
/// or last three, see end_velocity. With two points there is one chord and the velocity is
/// along it at both ends.
fn velocities(points: &[DVec3], spans: &[f64]) -> Vec<DVec3> {
    let count = points.len();
    if count == 2 {
        let along = (points[1] - points[0]) / spans[0];
        return vec![along, along];
    }

    let mut velocities = Vec::with_capacity(count);
    velocities.push(end_velocity(points[0], points[1], points[2], spans[0]));
    for index in 1..count - 1 {
        let (before, span_before) = (points[index - 1], spans[index - 1]);
        let (after, span_after) = (points[index + 1], spans[index]);
        let point = points[index];

        velocities.push(
            (point - before) / span_before - (after - before) / (span_before + span_after)
                + (after - point) / span_after,
        );
    }
    velocities.push(-end_velocity(
        points[count - 1],
        points[count - 2],
        points[count - 3],
        spans[count - 2],
    ));

    velocities
}

/// The velocity at the first of three points: along the tangent there of the circle through
/// the three, at the speed of the first chord over its span, which is the speed a straight
/// run has everywhere. The circle is the arc the three would be samples of, whatever their
/// spacing. In line, it is the first chord. With the two chords alike its tangent leaves the
/// first chord by half the turn at the second point, towards the outside, as the slope of
/// the parabola through the three would too. With the first chord much the shorter it stays
/// close to that chord, since a short chord on an arc turns little; the parabola's slope
/// instead swings the other way, past the whole turn, as a parabola must to come back round
/// to the third point, and on the E-W line, which opens with a chord of 22 m then one of
/// 260 m turning thirty degrees, it faced the first piece thirty-six degrees across the
/// track. Mirroring the second point past the end, the other common closure, is the first
/// chord's direction whatever the spacing, and sets a curve of like chords off half a
/// segment's turn from where it is going.
///
/// The tangent is found by inverting the points in the first: a circle through the centre of
/// inversion becomes a line parallel to the circle's tangent there, and that line runs
/// through the inverted second and third points. Written in differences, so that the six
/// million metres the points share cancels before anything is squared. Where the three fold
/// back on themselves so that the circle's tangent faces backwards, as no rail does, and
/// where the third point is the first again, the first chord's direction stands in.
fn end_velocity(first: DVec3, second: DVec3, third: DVec3, span: f64) -> DVec3 {
    let chord = second - first;
    let reach = third - first;
    let along = chord / span;

    let tangent = chord / chord.length_squared() - reach / reach.length_squared();
    match tangent.try_normalize() {
        Some(direction) if direction.dot(chord) > 0.0 => direction * (chord.length() / span),
        _ => along,
    }
}

/// Distance along the spline against parameter, at every subdivision made in measuring it.
#[derive(Debug, Clone)]
pub(super) struct ArcLength {
    /// (parameter, distance) pairs, both increasing, from (0, 0) to the end of the spline.
    entries: Vec<(f64, f64)>,
}

impl ArcLength {
    /// The length of the whole spline.
    pub(super) fn total(&self) -> f64 {
        self.entries.last().map_or(0.0, |&(_, distance)| distance)
    }

    /// The cursor to hand [`Self::parameter`] for a distance looked up on its own, rather
    /// than as the next of a walk: the subdivision the distance falls after, found by
    /// bisection, where the walk from a cursor of zero would step through the table from
    /// the start. A train asks for one distance a frame anywhere on its line, and the
    /// longest line's table is some thousands of entries.
    pub(super) fn seek(&self, distance: f64) -> usize {
        let last = self.entries.len() - 1;
        self.entries
            .partition_point(|&(_, along)| along <= distance)
            .saturating_sub(1)
            .min(last.saturating_sub(1))
    }

    /// The parameter at a distance along the spline it was measured from. The table gives
    /// the two subdivisions either side of the distance; between them the parameter is
    /// interpolated by distance and then refined by Newton's method, since interpolation
    /// alone takes the speed along the curve, metres per unit of parameter, as steady
    /// across the piece, and it is not where a long segment meets a short one: their shared
    /// tangent is sized for both, and the long segment's speed grows several-fold from that
    /// end, so the frames on it would fall anywhere from 21 to 29 m apart at a step of 25.
    /// Each step corrects the parameter by the distance still to go, measured as the chord
    /// from the piece's start, over the speed there; the chord stands for the arc within a
    /// piece as it does in the table, and the parameter stays within the piece. The cursor
    /// is the subdivision the last distance fell after, kept by the caller, so that a walk
    /// along the spline in increasing distance looks each distance up where the last one was
    /// found rather than from the start. A distance past the end gives the end.
    pub(super) fn parameter(&self, spline: &CatmullRom, distance: f64, cursor: &mut usize) -> f64 {
        let last = self.entries.len() - 1;
        while *cursor + 1 < last && self.entries[*cursor + 1].1 <= distance {
            *cursor += 1;
        }

        let (t0, d0) = self.entries[*cursor];
        let (t1, d1) = self.entries[(*cursor + 1).min(last)];
        if d1 <= d0 {
            return t1;
        }
        let fraction = ((distance - d0) / (d1 - d0)).clamp(0.0, 1.0);
        let mut t = t0 + (t1 - t0) * fraction;

        let start = spline.position(t0);
        for _ in 0..NEWTON_STEPS {
            let speed = spline.tangent(t).length();
            if speed <= 0.0 {
                break;
            }
            let along = d0 + start.distance(spline.position(t));
            t = (t - (along - distance) / speed).clamp(t0, t1);
        }

        t
    }
}
