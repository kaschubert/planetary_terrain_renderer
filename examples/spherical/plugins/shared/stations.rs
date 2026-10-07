//! Auckland's train stations and their platforms, read from the CSV the download script
//! writes beside the rail lines: what the labels on the map name, and what a stop id in
//! the realtime feed turns into.
//!
//! A station in the GTFS feed is a parent stop, "Avondale Train Station", with a platform
//! stop under it per platform, "Waitemata Train Station 2", and a trip update names the
//! platform. The file carries both, a platform with its station's id as its parent, so a
//! stop id of either kind comes back as the station's short name, "Waitemata": the name up
//! to " Train Station", which leaves the platform number behind with the words.

use bevy::math::DVec3;
use bevy_terrain::math::unit_position;

/// The header the file must carry, and the one the script writes. The name is last so that
/// it may hold a comma.
pub const HEADER: &str = "stop_id,parent,code,latitude,longitude,name";

/// A stop of the rail network: a station, or a platform of one.
#[derive(Debug, Clone, PartialEq)]
pub struct Station {
    /// The GTFS stop id, "105-474861ff", which the realtime feed names stops by.
    pub id: String,
    /// For a platform, the id of the station it is under; None for a station.
    pub parent: Option<String>,
    /// The stop code riders see, "105".
    pub code: String,
    /// The name as the feed has it, "Avondale Train Station".
    pub name: String,
    pub longitude: f64,
    pub latitude: f64,
}

impl Station {
    /// A station rather than a platform of one.
    pub fn is_station(&self) -> bool {
        self.parent.is_none()
    }

    pub fn unit(&self) -> DVec3 {
        unit_position(self.longitude, self.latitude)
    }

    /// The name for a label, see short_name.
    pub fn short_name(&self) -> &str {
        short_name(&self.name)
    }
}

/// The name up to " Train Station", which drops the words and any platform number after
/// them: "Waitemata Train Station 2" is "Waitemata". A name without the words is itself.
pub fn short_name(name: &str) -> &str {
    name.split(" Train Station").next().unwrap_or(name).trim()
}

/// Every stop of the network, in the file's order: each station followed by its platforms.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stations {
    pub stops: Vec<Station>,
}

impl Stations {
    /// Reads the file: comment lines beginning with #, the header, and a row per stop. A
    /// row that is not six fields or whose coordinates are not numbers is an error naming
    /// the row, as the rail file's parser reports its rows.
    pub fn parse(csv: &str) -> Result<Self, String> {
        let mut lines = csv
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'));
        let header = lines.next().ok_or("the stations file has no header")?;
        if header != HEADER {
            return Err(format!(
                "the stations file's header is {header:?}, not {HEADER:?}"
            ));
        }

        let mut stops = Vec::new();
        for (index, line) in lines.enumerate() {
            let row = index + 1;
            let fields: Vec<&str> = line.splitn(6, ',').collect();
            let [id, parent, code, latitude, longitude, name] = fields[..] else {
                return Err(format!(
                    "stations row {row}: {} fields, not 6: {line:?}",
                    fields.len()
                ));
            };
            let number = |text: &str, what: &str| {
                text.trim()
                    .parse::<f64>()
                    .map_err(|_| format!("stations row {row}: the {what} {text:?} is not a number"))
            };

            stops.push(Station {
                id: id.trim().to_string(),
                parent: Some(parent.trim())
                    .filter(|parent| !parent.is_empty())
                    .map(str::to_string),
                code: code.trim().to_string(),
                name: name.trim().to_string(),
                latitude: number(latitude, "latitude")?,
                longitude: number(longitude, "longitude")?,
            });
        }

        Ok(Self { stops })
    }

    /// The stations alone, without their platforms.
    pub fn stations(&self) -> impl Iterator<Item = &Station> {
        self.stops.iter().filter(|stop| stop.is_station())
    }

    pub fn get(&self, id: &str) -> Option<&Station> {
        self.stops.iter().find(|stop| stop.id == id)
    }

    /// The short name of the station a stop id names, through its parent for a platform,
    /// and of the stop itself where the parent is not in the file. None for an id the file
    /// does not have, which a caller shows as the id.
    pub fn name_of(&self, id: &str) -> Option<&str> {
        let stop = self.get(id)?;
        let station = stop
            .parent
            .as_deref()
            .and_then(|parent| self.get(parent))
            .unwrap_or(stop);

        Some(station.short_name())
    }
}

#[cfg(test)]
mod tests;
