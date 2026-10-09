//! The colour a marker is drawn in, taken from the season its photograph was taken in.
//!
//! A hundred markers in a hundred arbitrary colours says nothing; the same hundred coloured by
//! when the picture was taken says something at a glance, and a column of cards reads as a time
//! of year rather than as a paint chart. The palettes are assets/palettes/seasons.ron, ten
//! swatches each for spring, summer, autumn and winter.
//!
//! Compiled in rather than loaded, as the rail network's stations are: four lines of colours that
//! nothing edits while the example runs, and loading them as an asset would leave the first few
//! frames with no palette to draw from.
//!
//! The season is the photograph's month turned round for the hemisphere its marker stands in. A
//! December picture of Auckland is a summer picture, and the same date in Hamburg is a winter
//! one; taking the month at face value would have painted half this collection backwards, since
//! the terrain it is pinned to is New Zealand's.
//!
//! The date comes from the file's name, which is where a phone puts it: `20251212_173914.jpg` and
//! the like. Not from the file's EXIF, which would mean a parser and a dependency for a string of
//! eight digits that is already in hand; and not from the file's own timestamp, which says when
//! it was copied off the phone rather than when it was taken.

use bevy::prelude::*;
use serde::Deserialize;
use std::path::Path;

use super::PhotoMarker;

/// The palettes, as the file holds them.
const PALETTES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/palettes/seasons.ron"
));

/// How many of each season's ten swatches are drawn from.
///
/// The file's four palettes overlap badly: summer's Driftwood and autumn's Faded Brick are eight
/// units apart in sRGB, which no eye separates, and seven of summer's ten sit nearer an autumn
/// colour than they do to anything of their own. A photograph was being painted a perfectly
/// correct summer that read as autumn.
///
/// So each palette is cut to the swatches furthest from every other season's. At four a season
/// the worst pair across two seasons goes from eight apart to forty-nine, which does separate,
/// and four colours is still enough that a column of cards is not monotonous. The cut is worked
/// out from the file rather than written down here, so editing the palettes re-chooses it.
const DISTINCT: usize = 4;

/// One swatch. The name and the hex are the file's own words for it, kept so that the file reads
/// as the palette it is rather than as a list of numbers; only the sRGB triple is drawn with.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct Swatch {
    #[allow(dead_code)]
    pub name: String,
    #[allow(dead_code)]
    pub hex: String,
    pub srgb: (f32, f32, f32),
}

/// The four palettes, parsed once at startup.
#[derive(Resource, Debug, Clone, Deserialize)]
pub(super) struct Seasons {
    spring: Vec<Swatch>,
    summer: Vec<Swatch>,
    autumn: Vec<Swatch>,
    winter: Vec<Swatch>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Season {
    pub(super) const ALL: [Self; 4] = [Self::Spring, Self::Summer, Self::Autumn, Self::Winter];
}

impl Seasons {
    /// The compiled-in palettes. A file that will not parse is a mistake in the repository rather
    /// than something a run can recover from, so this says what is wrong and stands the colouring
    /// down rather than drawing from half a palette.
    pub(super) fn load() -> Self {
        match ron::from_str::<Self>(PALETTES) {
            Ok(mut seasons) => {
                seasons.sharpen();
                seasons
            }
            Err(error) => {
                error!("photo markers: assets/palettes/seasons.ron: {error}");
                Self {
                    spring: Vec::new(),
                    summer: Vec::new(),
                    autumn: Vec::new(),
                    winter: Vec::new(),
                }
            }
        }
    }

    fn palette(&self, season: Season) -> &[Swatch] {
        match season {
            Season::Spring => &self.spring,
            Season::Summer => &self.summer,
            Season::Autumn => &self.autumn,
            Season::Winter => &self.winter,
        }
    }

    fn palette_mut(&mut self, season: Season) -> &mut Vec<Swatch> {
        match season {
            Season::Spring => &mut self.spring,
            Season::Summer => &mut self.summer,
            Season::Autumn => &mut self.autumn,
            Season::Winter => &mut self.winter,
        }
    }

    /// Cuts each palette to the DISTINCT swatches that could least be mistaken for another
    /// season's, measured against the file's palettes whole rather than against what is left of
    /// them, so that the four are chosen independently and the order they are cut in cannot
    /// change the answer.
    fn sharpen(&mut self) {
        let whole = self.clone();

        for season in Season::ALL {
            let mut kept = whole.palette(season).to_vec();
            kept.sort_by(|a, b| whole.apart(b, season).total_cmp(&whole.apart(a, season)));
            kept.truncate(DISTINCT);
            *self.palette_mut(season) = kept;
        }
    }

    /// How far a swatch of this season is from the nearest swatch of any other, as a plain
    /// distance between sRGB triples. Crude against how an eye actually judges colour, and
    /// enough to tell a brown from a teal, which is all that is asked of it.
    fn apart(&self, swatch: &Swatch, season: Season) -> f32 {
        Season::ALL
            .into_iter()
            .filter(|other| *other != season)
            .flat_map(|other| self.palette(other))
            .map(|other| {
                let (dr, dg, db) = (
                    swatch.srgb.0 - other.srgb.0,
                    swatch.srgb.1 - other.srgb.1,
                    swatch.srgb.2 - other.srgb.2,
                );

                (dr * dr + dg * dg + db * db).sqrt()
            })
            .fold(f32::INFINITY, f32::min)
    }

    /// The swatches a season is drawn from, for the test that checks they stay apart.
    #[cfg(test)]
    pub(super) fn swatches(&self, season: Season) -> &[Swatch] {
        self.palette(season)
    }

    /// The colour a marker is drawn in: its season's, when the photograph on it says when it was
    /// taken, and the colour its own sliders hold when it does not.
    ///
    /// Which of the ten is settled by the file's name rather than by where the marker sits in the
    /// list, so a photograph keeps its colour when a marker before it is removed, across a save
    /// and a reload, and between one run and the next.
    pub(super) fn colour_of(&self, marker: &PhotoMarker) -> Color {
        let Some(season) = marker
            .photo
            .as_deref()
            .and_then(taken_in)
            .map(|month| season_of(month, marker.latitude()))
        else {
            return Color::from(marker.colour);
        };

        let palette = self.palette(season);
        let Some(swatch) = palette.get(scramble(marker.photo.as_deref()) as usize % palette.len())
        else {
            return Color::from(marker.colour);
        };

        Color::srgb(swatch.srgb.0, swatch.srgb.1, swatch.srgb.2)
    }
}

/// The month a photograph was taken in, from the eight digits a phone puts at the front of the
/// name: `20251212_173914.jpg` is December. None when the name does not start that way, or when
/// the digits are not a date, which leaves the marker its own colour rather than guessing.
pub(super) fn taken_in(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() < 8 {
        return None;
    }

    let month: u32 = digits.get(4..6)?.parse().ok()?;
    let year: u32 = digits.get(0..4)?.parse().ok()?;

    (1..=12).contains(&month).then_some(month).filter(|_| {
        // A sanity check on the year, so that a file named 99999999.jpg is not read as a date.
        (1900..=2200).contains(&year)
    })
}

/// The season a month falls in where a marker stands.
///
/// Meteorological seasons, which begin on the first of the month rather than at a solstice: the
/// photographs are months, not days. South of the equator the year is turned half round, so
/// December is summer in Auckland and winter in Hamburg.
pub(super) fn season_of(month: u32, latitude: f64) -> Season {
    let month = match latitude < 0.0 {
        true => (month + 5) % 12 + 1,
        false => month,
    };

    match month {
        3..=5 => Season::Spring,
        6..=8 => Season::Summer,
        9..=11 => Season::Autumn,
        _ => Season::Winter,
    }
}

/// A number from a photograph's name, the same every time the same name is asked about.
///
/// FNV-1a, eight lines of it, rather than the standard library's hasher, which promises nothing
/// about giving the same answer in another run or another version of Rust. A photograph that
/// changed colour when the example was rebuilt would be worse than no colouring at all.
fn scramble(path: Option<&Path>) -> u64 {
    let Some(name) = path
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
    else {
        return 0;
    };

    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }

    hash
}
