//! Auckland Transport's vehicle positions feed: fetched with the subscription key, parsed,
//! and reduced to the few fields the trains want. Nothing of Bevy's is in here, so the
//! parsing is tested from a string.
//!
//! The feed is GTFS-realtime as JSON, one entity per vehicle of every mode, wrapped in an
//! envelope of AT's own: {"status":"OK","response":{"header":…,"entity":[…]}}. Each
//! entity's vehicle has a position, a timestamp, a descriptor with the vehicle's id and
//! label, and a trip when the vehicle is on one, whose route_id is how a train is told from
//! a bus: a train's is its line's short name with a version on the end, "E-W-201". Vehicles
//! off a trip carry no route and cannot be told apart, so they are kept here and left to the
//! caller to drop. Two things the real feed does that the specification does not say: the
//! bearing arrives as a number for some vehicles and as a string for others, and a label is
//! padded with runs of spaces, "AMP        1142". A bearing of exactly nought is taken as
//! none, since a unit that has not sent one reads that way, and a train heading due north
//! is told by the fixes either side of it instead.

use serde::{Deserialize, Deserializer};
use std::time::Duration;

/// The endpoint, see the AT developer portal. The key goes in the header below.
pub const URL: &str = "https://api.at.govt.nz/realtime/legacy/vehiclelocations";

/// The header the subscription key is sent in, as the portal has it.
pub const KEY_HEADER: &str = "Ocp-Apim-Subscription-Key";

/// How long a fetch may take before it is given up, in seconds: the feed is a third of a
/// megabyte and arrives in a second or two, and a fetch that hangs must not hold the next
/// poll back for long.
const TIMEOUT_SECONDS: u64 = 20;

/// One fetch of the feed, reduced to what the trains want.
#[derive(Debug, Clone, PartialEq)]
pub struct Feed {
    /// The feed's own timestamp for the fetch, seconds since the epoch by its clock, which
    /// every vehicle's timestamp is in too.
    pub header_at: f64,
    /// Every vehicle with a position and an id, of every mode, deleted ones left out.
    pub vehicles: Vec<Vehicle>,
}

/// One vehicle's position, as the trains want it.
#[derive(Debug, Clone, PartialEq)]
pub struct Vehicle {
    /// The unit, see unit_name.
    pub id: String,
    /// The route the vehicle's trip is on, None off a trip; see line_name.
    pub route_id: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    /// Compass degrees the vehicle is heading, when the unit sent one.
    pub bearing: Option<f64>,
    /// Metres per second, nought when the feed gives none.
    pub speed: f64,
    /// The fix's timestamp, seconds since the epoch by the feed's clock.
    pub at: f64,
}

/// Fetches the feed with the key and parses it. The error is the message the caption shows:
/// ureq's for a status outside 2xx, "http status: 401" for a key the portal does not know,
/// or for a connection that failed, and parse's for a body that is not the feed.
pub fn fetch(key: &str) -> Result<Feed, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(TIMEOUT_SECONDS)))
        .build()
        .into();
    let mut response = agent
        .get(URL)
        .header(KEY_HEADER, key)
        .call()
        .map_err(|error| error.to_string())?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("the body did not arrive whole: {error}"))?;

    parse(&body)
}

/// The feed from its JSON, with or without AT's envelope round it.
pub fn parse(json: &str) -> Result<Feed, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|error| format!("the feed is not JSON: {error}"))?;
    let feed = match value {
        serde_json::Value::Object(mut envelope) if envelope.contains_key("response") => {
            envelope.remove("response").unwrap_or_default()
        }
        value => value,
    };
    let raw: RawFeed = serde_json::from_value(feed)
        .map_err(|error| format!("the feed is not the shape expected: {error}"))?;

    let vehicles = raw
        .entity
        .into_iter()
        .filter(|entity| !entity.is_deleted)
        .filter_map(|entity| {
            let vehicle = entity.vehicle?;
            let position = vehicle.position?;
            let descriptor = vehicle.vehicle?;
            let id = descriptor.id?;

            Some(Vehicle {
                id: unit_name(descriptor.label.as_deref(), &id),
                route_id: vehicle.trip.and_then(|trip| trip.route_id),
                latitude: position.latitude,
                longitude: position.longitude,
                bearing: position.bearing.filter(|&bearing| bearing != 0.0),
                speed: position.speed.unwrap_or(0.0),
                at: vehicle.timestamp.unwrap_or(raw.header.timestamp),
            })
        })
        .collect();

    Ok(Feed {
        header_at: raw.header.timestamp,
        vehicles,
    })
}

/// The line a route is on: the route id up to its last dash, so "E-W-201" is "E-W", the
/// short name the rail file names its lines by. A route id with no dash is its own name.
pub fn line_name(route_id: &str) -> &str {
    route_id.rsplit_once('-').map_or(route_id, |(name, _)| name)
}

/// What a train is called: its label with the padding taken out, "AMP 1142", or its
/// vehicle id where the label is blank, as a bus's is.
pub fn unit_name(label: Option<&str>, id: &str) -> String {
    let label = label
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if label.is_empty() {
        id.to_string()
    } else {
        label
    }
}

/// The feed as it arrives, the fields read and nothing else; serde leaves the rest.
#[derive(Deserialize)]
struct RawFeed {
    header: RawHeader,
    #[serde(default)]
    entity: Vec<RawEntity>,
}

#[derive(Deserialize)]
struct RawHeader {
    #[serde(deserialize_with = "number_or_string")]
    timestamp: f64,
}

#[derive(Deserialize)]
struct RawEntity {
    #[serde(default)]
    is_deleted: bool,
    vehicle: Option<RawVehiclePosition>,
}

#[derive(Deserialize)]
struct RawVehiclePosition {
    trip: Option<RawTrip>,
    position: Option<RawPosition>,
    #[serde(default, deserialize_with = "optional_number_or_string")]
    timestamp: Option<f64>,
    vehicle: Option<RawDescriptor>,
}

#[derive(Deserialize)]
struct RawTrip {
    route_id: Option<String>,
}

#[derive(Deserialize)]
struct RawPosition {
    latitude: f64,
    longitude: f64,
    #[serde(default, deserialize_with = "optional_number_or_string")]
    bearing: Option<f64>,
    #[serde(default, deserialize_with = "optional_number_or_string")]
    speed: Option<f64>,
}

#[derive(Deserialize)]
struct RawDescriptor {
    id: Option<String>,
    label: Option<String>,
}

/// A number the feed may write as a number or as a string, see the module doc.
#[derive(Deserialize)]
#[serde(untagged)]
enum NumberOrString {
    Number(f64),
    Text(String),
}

impl NumberOrString {
    fn value(self) -> Option<f64> {
        match self {
            Self::Number(number) => Some(number),
            Self::Text(text) => text.trim().parse().ok(),
        }
    }
}

fn number_or_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    NumberOrString::deserialize(deserializer)?
        .value()
        .ok_or_else(|| serde::de::Error::custom("a number written as text that is not one"))
}

fn optional_number_or_string<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    Ok(Option::<NumberOrString>::deserialize(deserializer)?.and_then(NumberOrString::value))
}
