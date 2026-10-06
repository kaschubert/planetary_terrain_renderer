//! The rail network as data: points with modes, the file they live in, and the heights that
//! follow from them.
//!
//! A point is one of three kinds, and the kind says where its height comes from. Most
//! points follow the ground: the terrain under them plus an offset. A few hold a height of
//! their own, a portal or an abutment that does not move when the dataset does. Everything
//! between two of those lies on the straight line joining them, which is what a tunnel or a
//! bridge is, so a tunnel is two edits and not forty. The file keeps the kind and the
//! number, and the resolution here turns them into positions on the spheroid.
//!
//! The file is a CSV behind a comment header. Its first version had three columns,
//! line,latitude,longitude, and every such row reads as a ground point with no offset. The
//! second adds mode,height,terrain, where blank means not applicable rather than zero. Both
//! read, so the committed file needs no rewrite; only the second is written.

use bevy::math::DVec3;
use bevy_terrain::{math::unit_position, prelude::TerrainShape};
use std::{
    fmt::Write,
    time::{SystemTime, UNIX_EPOCH},
};

/// The column header of the first format, as the download script first wrote it.
const HEADER_V1: &str = "line,latitude,longitude";

/// The column header of the second format, the one this writes.
const HEADER_V2: &str = "line,latitude,longitude,mode,height,terrain";

/// How the comment that marks a hand-edited file begins. The parser drops it so that a save
/// writes it afresh with the new time, and the download script looks for it to know not to
/// overwrite the edits.
pub const EDITED_BY: &str = "# Edited by hand in the spherical example";

/// The steepest grade rail climbs, as rise over run. An adhesion railway does not climb much
/// steeper than about 1 in 30, and the City Rail Link, the steepest stretch on this network,
/// is built to 3.5 per cent. A segment of the draped line steeper than this is therefore not
/// track on the ground but somewhere the track leaves it: a tunnel under a hill, or a bridge
/// over a dip, still to be made.
pub const MAX_GRADE: f64 = 0.035;

/// Where a point's height comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointMode {
    /// The terrain under the point plus an offset. Every imported point starts here.
    Ground,
    /// A height of its own above the ellipsoid, in the terrain's datum, which the terrain
    /// does not move.
    Fixed,
    /// On the straight line between the nearest fixed points either side, by distance along
    /// the line. Stores no height of its own.
    Between,
}

impl PointMode {
    fn parse(field: &str) -> Result<Self, String> {
        match field {
            "ground" => Ok(Self::Ground),
            "fixed" => Ok(Self::Fixed),
            "between" => Ok(Self::Between),
            _ => Err(format!("{field:?} is not a mode; ground, fixed or between")),
        }
    }

    /// The mode as the file writes it and the panel's buttons read it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ground => "ground",
            Self::Fixed => "fixed",
            Self::Between => "between",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RailPoint {
    pub longitude: f64,
    pub latitude: f64,
    pub mode: PointMode,
    /// The offset of a ground point or the height of a fixed one, in metres. None for a
    /// between point, which stores nothing.
    pub height: Option<f64>,
    /// The terrain height the file recorded at its last save. It puts the line on the
    /// ground from the first frame, while the sampler is still reading tiles, and it is the
    /// witness a fresh sample is compared against to find where the ground has moved.
    pub terrain: Option<f64>,
    /// This run's sample, once the sampler has landed. None before that, and None after
    /// when no terrain on disk has data under the point.
    pub sampled: Option<f64>,
}

impl RailPoint {
    /// A point as the download script writes it: on the ground, nothing known about it.
    pub fn ground(longitude: f64, latitude: f64) -> Self {
        Self {
            longitude,
            latitude,
            mode: PointMode::Ground,
            height: Some(0.0),
            terrain: None,
            sampled: None,
        }
    }

    pub fn unit(&self) -> DVec3 {
        unit_position(self.longitude, self.latitude)
    }

    /// The place as the bits of its two coordinates, for matching points by place exactly:
    /// the samples the startup task read to the points they were read at, a snapshot's points
    /// to the network's on undo, and a point the ground moved under to where it is now. A
    /// point the editor has not touched still has its coordinates to the bit.
    pub fn place_bits(&self) -> (u64, u64) {
        (self.longitude.to_bits(), self.latitude.to_bits())
    }

    /// A ground point with no offset at a direction on the unit sphere, for a point placed
    /// with the mouse. The height is sampled afterwards; until then the point reads as a
    /// fresh import, on the ellipsoid.
    pub fn ground_at_unit(unit: DVec3) -> Self {
        let (longitude, latitude) = lon_lat_from_unit(unit);
        Self::ground(longitude, latitude)
    }

    /// The terrain height the point goes by: this run's sample, else the file's, else the
    /// ellipsoid, which is only ever the very first run before anything has been saved.
    pub fn terrain_height(&self) -> f64 {
        self.sampled.or(self.terrain).unwrap_or(0.0)
    }
}

/// The inverse of [`unit_position`]: a direction on the unit sphere back to longitude and
/// latitude in degrees. The latitude is the elevation of the direction and the longitude
/// its bearing in the equatorial plane, measured from the -x axis towards +z, which is what
/// the convention there puts a longitude of zero and ninety at. At a pole the bearing is
/// whatever the rounding left in the equatorial components, and nothing turns on it.
pub fn lon_lat_from_unit(unit: DVec3) -> (f64, f64) {
    let latitude = unit.y.clamp(-1.0, 1.0).asin();
    let longitude = unit.z.atan2(-unit.x);

    (longitude.to_degrees(), latitude.to_degrees())
}

#[derive(Debug)]
pub struct RailLine {
    pub name: String,
    pub points: Vec<RailPoint>,
    /// One per point, on the spheroid at the resolved height, in absolute metres. Filled by
    /// [`Self::resolve`] when the points or their samples change, so a frame only has to
    /// shift them into the camera's cell.
    pub positions: Vec<DVec3>,
}

impl RailLine {
    /// The height above the ellipsoid of every point, by its mode.
    ///
    /// A between point finds the nearest fixed point on each side, or failing that the
    /// nearest ground point, and sits on the chord between them by its distance along the
    /// line. With an anchor on one side only it holds that anchor's height, and with none at
    /// all it follows the ground with no offset, so a half-made edit never leaves a point
    /// without a height.
    pub fn resolved_heights(&self) -> Vec<f64> {
        let points = &self.points;
        let along = distances_along(points);

        let mut heights: Vec<f64> = points
            .iter()
            .map(|point| match point.mode {
                PointMode::Ground => point.terrain_height() + point.height.unwrap_or(0.0),
                PointMode::Fixed => point.height.unwrap_or(0.0),
                PointMode::Between => point.terrain_height(),
            })
            .collect();

        for index in 0..points.len() {
            if points[index].mode != PointMode::Between {
                continue;
            }

            let before = anchor(points, (0..index).rev());
            let after = anchor(points, index + 1..points.len());

            heights[index] = match (before, after) {
                (Some(a), Some(b)) => {
                    let span = along[b] - along[a];
                    let fraction = if span > 0.0 {
                        (along[index] - along[a]) / span
                    } else {
                        0.0
                    };
                    heights[a] + (heights[b] - heights[a]) * fraction
                }
                (Some(a), None) | (None, Some(a)) => heights[a],
                (None, None) => heights[index],
            };
        }

        heights
    }

    /// The grade of every segment, rise over run, one per segment and so one fewer than the
    /// points: the difference of the resolved heights of its two points over the horizontal
    /// distance between them. The run is the chord between the two points on the ellipsoid,
    /// the same chord [`distances_along`] sums. Between neighbours, see [`chord_lengths`] for
    /// how far apart they are, the chord, the arc and the distance at track height agree to
    /// better than a part in ten thousand, which no grade this is compared against can tell.
    /// Two points at
    /// the same place have no run: their grade is zero when they share a height and infinite
    /// when they do not, so a vertical step reads as steep rather than as nothing.
    pub fn segment_grades(&self) -> Vec<f64> {
        let heights = self.resolved_heights();

        chord_lengths(&self.points)
            .into_iter()
            .zip(heights.windows(2))
            .map(|(run, pair)| {
                let rise = (pair[1] - pair[0]).abs();
                if run > 0.0 {
                    rise / run
                } else if rise > 0.0 {
                    f64::INFINITY
                } else {
                    0.0
                }
            })
            .collect()
    }

    /// Recomputes the positions from the points and their heights.
    pub fn resolve(&mut self) {
        self.positions = self
            .points
            .iter()
            .zip(self.resolved_heights())
            .map(|(point, height)| TerrainShape::WGS84.position_unit_to_local(point.unit(), height))
            .collect();
    }
}

/// The nearest fixed point among the candidates, which run outward from the point being
/// resolved, else the nearest point of any kind that has a height of its own.
fn anchor(
    points: &[RailPoint],
    mut candidates: impl Iterator<Item = usize> + Clone,
) -> Option<usize> {
    candidates
        .clone()
        .find(|&index| points[index].mode == PointMode::Fixed)
        .or_else(|| candidates.find(|&index| points[index].mode != PointMode::Between))
}

/// The chord between each pair of neighbouring points on the ellipsoid, in metres, one per
/// segment. Neighbours on the network are from a metre to two kilometres apart, mostly a few
/// tens of metres, and even over two kilometres the chord and the geodesic agree to within a
/// hundredth of a millimetre.
fn chord_lengths(points: &[RailPoint]) -> Vec<f64> {
    let on_the_ground: Vec<DVec3> = points
        .iter()
        .map(|point| TerrainShape::WGS84.position_unit_to_local(point.unit(), 0.0))
        .collect();

    on_the_ground
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .collect()
}

/// How far along the line each point is, in metres, summing the chords.
fn distances_along(points: &[RailPoint]) -> Vec<f64> {
    if points.is_empty() {
        return Vec::new();
    }

    let mut total = 0.0;
    std::iter::once(0.0)
        .chain(chord_lengths(points).into_iter().map(|chord| {
            total += chord;
            total
        }))
        .collect()
}

#[derive(Debug)]
pub struct RailNetwork {
    /// The header's comment lines, kept so that a save loses none of the provenance the
    /// download script wrote. Without the edited-by line, which a save writes afresh.
    pub comments: Vec<String>,
    /// In file order, each line's points in order along it.
    pub lines: Vec<RailLine>,
}

impl RailNetwork {
    /// Reads either format. The error names the line of the file, not the path, since this
    /// does not know where the text came from.
    pub fn parse(csv: &str) -> Result<Self, String> {
        let mut comments = Vec::new();
        let mut columns = None;
        let mut lines: Vec<RailLine> = Vec::new();

        for (number, row) in csv.lines().enumerate() {
            let number = number + 1;
            let row = row.trim_end();

            if row.is_empty() {
                continue;
            }
            if row.starts_with('#') {
                if !row.starts_with(EDITED_BY) {
                    comments.push(row.to_string());
                }
                continue;
            }
            if row.starts_with("line,") {
                columns = Some(match row {
                    HEADER_V1 => 3,
                    HEADER_V2 => 6,
                    _ => return Err(format!("line {number}: unknown header {row:?}")),
                });
                continue;
            }

            let Some(columns) = columns else {
                return Err(format!("line {number}: a row before the header"));
            };
            let (name, point) =
                parse_row(row, columns).map_err(|error| format!("line {number}: {error}"))?;

            match lines.last_mut() {
                Some(line) if line.name == name => line.points.push(point),
                _ => lines.push(RailLine {
                    name: name.to_string(),
                    points: vec![point],
                    positions: Vec::new(),
                }),
            }
        }

        Ok(Self { comments, lines })
    }

    /// The file in the second format, with the edited-by line stamped with the time given.
    /// The terrain column records this run's sample where there is one, else what the file
    /// had, so a save never forgets a height it knew. The coordinates are written to seven
    /// decimals, a centimetre, so a point placed with the gizmo is saved where it was put and
    /// its terrain column was sampled; the download script's rows, at five, come back as they
    /// went in.
    pub fn to_csv(&self, saved_at: &str) -> String {
        let mut csv = String::new();

        for comment in &self.comments {
            csv.push_str(comment);
            csv.push('\n');
        }
        let _ = writeln!(csv, "{EDITED_BY}, last saved {saved_at}");
        csv.push_str(HEADER_V2);
        csv.push('\n');

        for line in &self.lines {
            for point in &line.points {
                let _ = writeln!(
                    csv,
                    "{},{:.7},{:.7},{},{},{}",
                    line.name,
                    point.latitude,
                    point.longitude,
                    point.mode.name(),
                    metres(point.height),
                    metres(point.sampled.or(point.terrain)),
                );
            }
        }

        csv
    }

    /// Recomputes every line's positions. Call after changing points or their samples.
    pub fn resolve(&mut self) {
        for line in &mut self.lines {
            line.resolve();
        }
    }

    /// The mean of every resolved position. It sits a little inside the spheroid, which
    /// the distance and horizon tests do not mind.
    pub fn centre(&self) -> DVec3 {
        let positions = self.lines.iter().flat_map(|line| &line.positions);
        let count = positions.clone().count();

        if count == 0 {
            DVec3::ZERO
        } else {
            positions.sum::<DVec3>() / count as f64
        }
    }

    pub fn point_count(&self) -> usize {
        self.lines.iter().map(|line| line.points.len()).sum()
    }

    /// Every point, line by line in file order.
    pub fn points(&self) -> impl Iterator<Item = &RailPoint> {
        self.lines.iter().flat_map(|line| &line.points)
    }
}

/// One data row in either format.
fn parse_row(row: &str, columns: usize) -> Result<(&str, RailPoint), String> {
    let fields: Vec<&str> = row.split(',').map(str::trim).collect();

    if fields.len() != columns {
        return Err(format!("expected {columns} fields, got {}", fields.len()));
    }

    let latitude = number(fields[1], "latitude")?;
    let longitude = number(fields[2], "longitude")?;

    if !(-90.0..=90.0).contains(&latitude) {
        return Err(format!("latitude {latitude} is off the planet"));
    }
    if !(-180.0..=180.0).contains(&longitude) {
        return Err(format!("longitude {longitude} is off the planet"));
    }

    let mut point = RailPoint::ground(longitude, latitude);

    if columns == 6 {
        point.mode = PointMode::parse(fields[3])?;
        point.height = match point.mode {
            // A between point stores nothing, so whatever the column holds is ignored.
            PointMode::Between => None,
            _ => Some(
                optional_number(fields[4], "height")?
                    .ok_or_else(|| format!("a {} point needs a height", point.mode.name()))?,
            ),
        };
        point.terrain = optional_number(fields[5], "terrain")?;
    }

    Ok((fields[0], point))
}

fn number(field: &str, column: &str) -> Result<f64, String> {
    field
        .parse::<f64>()
        .map_err(|_| format!("{column} {field:?} is not a number"))
}

/// Blank is None, not zero.
fn optional_number(field: &str, column: &str) -> Result<Option<f64>, String> {
    if field.is_empty() {
        Ok(None)
    } else {
        number(field, column).map(Some)
    }
}

/// A height with two decimals, a centimetre, or blank for not applicable.
fn metres(value: Option<f64>) -> String {
    value.map_or_else(String::new, |value| format!("{value:.2}"))
}

/// Now, as "YYYY-MM-DD HH:MM UTC" for the edited-by line.
pub fn utc_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());

    format_utc(seconds)
}

/// Seconds since the epoch as a date and time, to the minute, labelled UTC because that is
/// what it is. The standard library has no calendar and a dependency for one line would be a
/// lot, so the day count is turned into a date by hand.
fn format_utc(seconds: u64) -> String {
    let (year, month, day) = civil_from_days((seconds / 86_400) as i64);
    let seconds_of_day = seconds % 86_400;

    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds_of_day / 3600,
        seconds_of_day % 3600 / 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date. Howard Hinnant's civil_from_days:
/// the calendar is shifted to start on 1 March, so that the leap day falls at the end of the
/// year and the months from March run in a fixed pattern, and 400 year eras are counted off
/// first because the calendar repeats exactly over 146097 days.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests;
