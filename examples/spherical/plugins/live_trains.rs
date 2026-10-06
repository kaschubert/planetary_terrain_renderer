//! The carriages where Auckland Transport's realtime feed says the trains are, in place of
//! the stand-ins, on F8: one per train on a trip, fetched every POLL_SECONDS and run on
//! between fetches at the speed each reported, as the live maps on the web do in 2D.
//!
//! The feed gives every vehicle of every mode a latitude and longitude. A train's is told
//! by its trip's route, whose id is its line's name with a version on the end, and the
//! position is dropped onto that line's spline to get the one thing the placer in trains.rs
//! reads: a distance along the line, see TrackSpline::nearest_between. From there a train is
//! what a stand-in is, and the carriage, the label, the table and the chase camera need no
//! word of where it came from. This driver takes the trains over through Trains::live, so
//! the stand-in driver stands aside, and keeps the roster as the feed has it: a train the
//! feed starts reporting is added, one it stops reporting is removed, and one that has
//! moved to another line, a unit finishing a run and starting the next, is removed and
//! added so its label is its new line's.
//!
//! A fix is some seconds old by the time it is drawn, the feed giving each train a new one
//! about every ten seconds and the fetch running as often, so between fetches each train is
//! reckoned on from its fix at the speed the fix reported, in the direction it is going,
//! and the drawn distance eases towards that reckoning rather than jumping to it, so a fix
//! that lands a few metres ahead of where the reckoning had the train is a nudge and not a
//! pop. A fix that lands behind the carriage, as one does when the train braked for a
//! station after its last fix and the reckoning ran it on past the platform, holds the
//! carriage where it is until the reckoning catches up, rather than backing it up: trains
//! do not reverse into platforms, and a carriage standing a little past one through the
//! dwell is the lesser wrong, see approach. A fix far from the carriage either way, a train
//! that was wrongly placed, is a jump, since easing across hundreds of metres would look
//! like a train at speed where there is none. The reckoning stops after REACH_SECONDS, so a
//! unit that has gone quiet stands rather than runs off the end of its line. The direction
//! comes from the fixes themselves, see direction_of: a train that has moved on along the
//! line since its last fix is heading that way, which no bearing can argue with.
//!
//! The fetch runs on the IO pool, since it blocks on the network for a second or two, and
//! lands through a Task polled each frame as the rail sampler's does. Nothing here runs
//! without the key, which comes from AT_API_KEY in the environment, a free subscription on
//! AT's developer portal: without it the stand-ins drive and the table's caption says so.
//! The headless test adds the plugin offline, so it never touches the network.

use super::auckland_rail::AucklandRail;
use super::rail_editor::frame::{Frame, unit_under};
use super::track_frames::{TrackSpline, TrackSplines, refresh_track_splines};
use super::trains::{Train, Trains, drive_trains};
use bevy::{
    math::DVec3,
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, poll_once},
};
use bevy_terrain::{math::unit_position, prelude::TerrainShape};
use feed::Feed;
use std::collections::HashMap;

mod feed;

/// The environment variable the subscription key is read from.
pub const KEY_VARIABLE: &str = "AT_API_KEY";

/// Seconds between fetches. The feed gives each train a new fix about every ten seconds,
/// measured on 7 October 2026 over a morning peak, and a fix is some nine seconds old by
/// the time it is fetched, so fetching slower only ages what is drawn. The key allows 600
/// calls a minute and 35,000 a week: one every 10 s is 6 a minute, and 35,000 of them is
/// some 97 hours of running a week, which an evening's run is nowhere near; left running
/// round the clock the week's calls would run out on the fifth day.
pub const POLL_SECONDS: f32 = 10.0;

/// How far a reported position may stand from its line, level, and still be a train on it,
/// in metres. A GPS fix is metres out, tens under the city's buildings; a unit at the depot
/// still attached to a trip, or a position that is garbage, is hundreds and is left out.
pub const OFF_LINE_METRES: f64 = 500.0;

/// How old a fix may be, by the feed's clock, before its unit is taken to have gone quiet
/// and is left out, in seconds. The feed keeps a unit's last fix for a while after it stops
/// reporting, and a train drawn where it was ten minutes ago is not where it is.
pub const STALE_SECONDS: f64 = 600.0;

/// How long a train is reckoned on from its fix at the fix's speed, in seconds: three
/// fetches' worth, so a fetch or two that fails leaves the trains moving, and no more, so a
/// unit that has gone quiet stands where it was last seen rather than running on at a
/// speed it may no longer have.
pub const REACH_SECONDS: f64 = 30.0;

/// A reckoning further than this from the drawn distance is a jump and not an ease, in
/// metres: a train wrongly placed, or placed for the first time, which is nowhere yet.
pub const SNAP_METRES: f64 = 300.0;

/// The time constant of the ease towards the reckoning, in seconds: a fix that moves the
/// reckoning by a few metres is closed in a few seconds, which the eye reads as the train
/// running on rather than as a correction.
pub const EASE_SECONDS: f64 = 2.0;

/// Below this reported speed a bearing is not trusted, in metres per second: a unit standing
/// at a platform reports whatever its last heading was, or nought.
pub const MOVING_MPS: f64 = 1.0;

/// A fix this much further along the line than the last, or this much back, says which way
/// the train is going, in metres: more than a GPS fix wanders between two fixes of a train
/// standing still.
pub const MOVED_METRES: f64 = 10.0;

/// How far either side of a train's last fix its next is looked for first, in metres: a
/// train runs a few hundred metres between fetches, and three kilometres leaves room for a
/// run of them missed, while keeping the S-C's two passes through Newmarket apart, see snap.
pub const SEARCH_METRES: f64 = 3_000.0;

/// The live trains, added by spherical.rs with the key from the environment, and by the
/// headless test without one.
pub struct LiveTrainsPlugin {
    key: Option<String>,
}

impl LiveTrainsPlugin {
    /// With the key from AT_API_KEY, trimmed; an unset or blank variable is no key.
    pub fn from_env() -> Self {
        let key = std::env::var(KEY_VARIABLE)
            .ok()
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty());

        Self { key }
    }

    /// Without a key, so that nothing is ever fetched: for the test.
    #[cfg(test)]
    pub fn offline() -> Self {
        Self { key: None }
    }
}

impl Plugin for LiveTrainsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(LiveTrains::new(self.key.clone()))
            // After start_trains has made the stand-ins, and before the first Update spawns
            // their carriages, so with a key no stand-in is ever drawn.
            .add_systems(PostStartup, start_live)
            .add_systems(
                Update,
                (toggle_live, poll_feed, steer_trains, write_caption)
                    .chain()
                    // After the splines are through this frame's line, as the stand-in
                    // driver is, and before it, so the table, which reads after it, reads
                    // this frame's steering.
                    .after(refresh_track_splines)
                    .before(drive_trains),
            );
    }
}

/// The feed and the switch for it.
#[derive(Resource)]
pub struct LiveTrains {
    /// Toggled by F8. On, the feed drives the trains; off, the stand-ins do. Nothing without
    /// a key.
    pub on: bool,
    key: Option<String>,
    /// Ticks while on; every time it finishes a fetch is due.
    poll: Timer,
    /// A fetch is wanted as soon as none is running: set when the poll finishes and when
    /// the feed is switched on, so the first fetch is at once.
    due: bool,
    /// The fetch in flight, until it lands.
    task: Option<Task<Result<Feed, String>>>,
    /// The latest fix of every train the last fetch reported, by the train's id.
    fixes: HashMap<String, Fix>,
    status: FeedStatus,
}

/// How the feed is doing, for the caption.
#[derive(Debug, Clone, PartialEq)]
pub enum FeedStatus {
    /// No key, so no feed: the stand-ins drive.
    NoKey,
    /// Switched off by F8: the stand-ins drive.
    Off,
    /// On, and the first fetch has not landed.
    Fetching,
    /// The last fetch landed: when, by Time::elapsed, and with how many trains.
    Live { received: f64, trains: usize },
    /// The last fetch failed, with ureq's or parse's word on why; the trains stand where
    /// the fetch before put them, and the poll goes on.
    Failed(String),
}

/// Where the feed last put a train, as a distance along its line, and what else the
/// reckoning between fetches wants.
#[derive(Debug, Clone, PartialEq)]
pub struct Fix {
    /// Index into the network's lines.
    pub line: usize,
    /// Metres along the line, see TrackSpline::nearest_between.
    pub distance: f64,
    /// +1 along point order, -1 back, see direction_of.
    pub direction: f64,
    /// Metres per second, as reported.
    pub speed: f64,
    /// The fix's timestamp, seconds since the epoch by the feed's clock.
    pub at: f64,
    /// The feed's timestamp for the fetch the fix came in, by the same clock.
    pub header_at: f64,
    /// Time::elapsed when the fetch landed.
    pub received: f64,
}

impl LiveTrains {
    fn new(key: Option<String>) -> Self {
        Self {
            on: false,
            key,
            poll: Timer::from_seconds(POLL_SECONDS, TimerMode::Repeating),
            due: false,
            task: None,
            fixes: HashMap::new(),
            status: FeedStatus::NoKey,
        }
    }

    /// Takes the trains over: the roster is cleared for the feed to fill, and a fetch is
    /// due at once.
    fn switch_on(&mut self, trains: &mut Trains, commands: &mut Commands) {
        self.on = true;
        trains.live = true;
        trains.clear(commands);
        self.fixes.clear();
        self.poll.reset();
        self.due = true;
        self.status = FeedStatus::Fetching;
    }

    /// Hands the trains back: the stand-ins return, and a fetch in flight is dropped, which
    /// cancels it.
    fn switch_off(&mut self, rail: &AucklandRail, trains: &mut Trains, commands: &mut Commands) {
        self.on = false;
        trains.live = false;
        trains.reset_stand_ins(rail, commands);
        self.fixes.clear();
        self.task = None;
        self.due = false;
        self.status = FeedStatus::Off;
    }

    /// Takes a fetch that landed: every train on a line, placed on it, and the roster made
    /// to match, see the module doc.
    fn apply(
        &mut self,
        feed: &Feed,
        now: f64,
        rail: &AucklandRail,
        splines: &TrackSplines,
        trains: &mut Trains,
        commands: &mut Commands,
    ) {
        let mut fixes: HashMap<String, Fix> = HashMap::with_capacity(self.fixes.len());
        let mut off_line = 0;

        for vehicle in &feed.vehicles {
            let Some(route_id) = vehicle.route_id.as_deref() else {
                continue;
            };
            let name = feed::line_name(route_id);
            let Some(line) = rail.network.lines.iter().position(|line| line.name == name) else {
                continue;
            };
            if feed.header_at - vehicle.at > STALE_SECONDS {
                continue;
            }
            let Some(spline) = splines.splines.get(line).and_then(Option::as_ref) else {
                continue;
            };

            let position = TerrainShape::WGS84
                .position_unit_to_local(unit_position(vehicle.longitude, vehicle.latitude), 0.0);
            let previous = self.fixes.get(&vehicle.id).filter(|fix| fix.line == line);
            let (distance, off) = snap(spline, position, previous.map(|fix| fix.distance));
            if off > OFF_LINE_METRES {
                off_line += 1;
                debug!(
                    "live trains: {} is {off:.0} m off the {name} line, left out",
                    vehicle.id
                );
                continue;
            }

            let direction = direction_of(
                previous,
                distance,
                vehicle.bearing,
                vehicle.speed,
                heading_along(spline, distance),
            );
            fixes.insert(
                vehicle.id.clone(),
                Fix {
                    line,
                    distance,
                    direction,
                    speed: vehicle.speed,
                    at: vehicle.at,
                    header_at: feed.header_at,
                    received: now,
                },
            );
        }

        // The roster as the feed has it: out with the trains the feed no longer reports or
        // reports on another line, in with the ones it has started to, in a settled order.
        for index in (0..trains.trains.len()).rev() {
            let train = &trains.trains[index];
            if fixes
                .get(&train.id)
                .is_none_or(|fix| fix.line != train.line)
            {
                trains.remove(index, commands);
            }
        }
        let mut arrivals: Vec<(&String, &Fix)> = fixes
            .iter()
            .filter(|(id, _)| !trains.trains.iter().any(|train| &train.id == *id))
            .collect();
        arrivals.sort_by(|(a, fix_a), (b, fix_b)| (fix_a.line, a).cmp(&(fix_b.line, b)));
        for (id, fix) in arrivals {
            trains.add(Train {
                id: id.clone(),
                line: fix.line,
                distance: fix.distance,
                direction: fix.direction,
                speed: fix.speed,
                entity: None,
                label: None,
            });
        }

        let before = match self.status {
            FeedStatus::Live { trains, .. } => Some(trains),
            _ => None,
        };
        if before != Some(fixes.len()) {
            info!(
                "live trains: {} trains on the lines, {off_line} off them, of {} vehicles",
                fixes.len(),
                feed.vehicles.len()
            );
        }
        self.status = FeedStatus::Live {
            received: now,
            trains: fixes.len(),
        };
        self.fixes = fixes;
    }
}

/// Where on its line a reported position falls, and how far off the line it is: nearest
/// to the last fix first, within SEARCH_METRES either side, since a line that passes a
/// place twice, as the S-C does at Newmarket, has two answers there and the one near where
/// the train was is the one; and over the whole line when there was no last fix, or the
/// near answer is off the line, as it is when the search window missed.
pub fn snap(spline: &TrackSpline, position: DVec3, previous: Option<f64>) -> (f64, f64) {
    if let Some(around) = previous
        && let Some(near) =
            spline.nearest_between(position, around - SEARCH_METRES, around + SEARCH_METRES)
        && near.1 <= OFF_LINE_METRES
    {
        return near;
    }

    spline.nearest(position)
}

/// The line's heading at a distance along it, in point order, as a compass bearing in
/// degrees: the frame's forward against east and north on the ground there.
pub fn heading_along(spline: &TrackSpline, distance: f64) -> f64 {
    let frame = spline.frame_at(distance);
    let ground = Frame::at_unit(unit_under(frame.position));
    let forward = frame.forward();

    forward
        .dot(ground.east)
        .atan2(forward.dot(ground.north))
        .to_degrees()
        .rem_euclid(360.0)
}

/// Which way a train is going from its fixes: along point order when this fix is further
/// along the line than the last by more than a GPS fix wanders, back when it is nearer the
/// start. A train that has not moved that far, or has no last fix, is going the way its
/// bearing says against the line's heading there, within a right angle either way, when it
/// is moving fast enough for the bearing to mean anything; failing that, the way it was
/// going; failing that, along point order, which the next fix corrects.
pub fn direction_of(
    previous: Option<&Fix>,
    distance: f64,
    bearing: Option<f64>,
    speed: f64,
    heading: f64,
) -> f64 {
    if let Some(previous) = previous {
        let moved = distance - previous.distance;
        if moved.abs() > MOVED_METRES {
            return moved.signum();
        }
    }
    if let Some(bearing) = bearing.filter(|_| speed >= MOVING_MPS) {
        let turn = (bearing - heading).rem_euclid(360.0);
        return if !(90.0..270.0).contains(&turn) {
            1.0
        } else {
            -1.0
        };
    }

    previous.map_or(1.0, |previous| previous.direction)
}

/// How old a fix is now, in seconds: the feed's own clock for how old it was when it was
/// fetched, so a wrong clock on this machine cannot run the trains off, and this machine's
/// for the time since.
pub fn age(fix: &Fix, elapsed: f64) -> f64 {
    (fix.header_at - fix.at) + (elapsed - fix.received)
}

/// Where a fix puts the train now: its distance run on at the fix's speed, in its direction,
/// for its age up to REACH_SECONDS, and kept on the line. A fix from the future, which a
/// clock's skew could give, is where it says.
pub fn reckoned(fix: &Fix, age: f64, length: f64) -> f64 {
    let run = fix.speed * age.clamp(0.0, REACH_SECONDS);

    (fix.distance + fix.direction * run).clamp(0.0, length.max(0.0))
}

/// The drawn distance moved towards the reckoning: the whole way when the gap is more than
/// SNAP_METRES, or the distance is nothing yet; nowhere when the reckoning is behind the
/// carriage in its direction of travel, which holds a carriage that was run on past a
/// station until the train leaves it and the reckoning comes past, see the module doc; and
/// otherwise the share an exponential lag of EASE_SECONDS closes in the time given. A train
/// that has turned round has its direction turned round first, see direction_of, so the
/// reckoning back along the line is ahead of it and it follows.
pub fn approach(distance: f64, target: f64, direction: f64, dt: f64) -> f64 {
    let gap = target - distance;
    if !gap.is_finite() || gap.abs() > SNAP_METRES {
        return target;
    }
    if gap * direction < 0.0 {
        return distance;
    }

    distance + gap * (1.0 - (-dt / EASE_SECONDS).exp())
}

/// The caption for the table, ASCII throughout, the font being what it is.
pub fn caption(status: &FeedStatus, elapsed: f64) -> String {
    match status {
        FeedStatus::NoKey => format!("stand-ins; no {KEY_VARIABLE}"),
        FeedStatus::Off => "stand-ins; F8 for AT live".to_string(),
        FeedStatus::Fetching => "AT live: fetching".to_string(),
        FeedStatus::Live { received, trains } => {
            let plural = if *trains == 1 { "" } else { "s" };
            format!(
                "AT live: {trains} train{plural}, {:.0} s ago",
                (elapsed - received).max(0.0)
            )
        }
        FeedStatus::Failed(error) => format!("AT live: failed, {}", first_line(error, 60)),
    }
}

/// The first line of an error, cut to so many characters, for a caption that is one line.
fn first_line(error: &str, characters: usize) -> String {
    let line = error.lines().next().unwrap_or("").trim();
    if line.chars().count() <= characters {
        line.to_string()
    } else {
        let mut cut: String = line.chars().take(characters.saturating_sub(3)).collect();
        cut.push_str("...");
        cut
    }
}

fn toggle_live(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    rail: Res<AucklandRail>,
    mut live: ResMut<LiveTrains>,
    mut trains: ResMut<Trains>,
) {
    if !keys.just_pressed(KeyCode::F8) {
        return;
    }
    if live.key.is_none() {
        info!("live trains: {KEY_VARIABLE} is not set, so there is no feed to switch to");
        return;
    }
    if live.on {
        live.switch_off(&rail, &mut trains, &mut commands);
        info!("live trains: off, the stand-ins drive");
    } else {
        live.switch_on(&mut trains, &mut commands);
        info!("live trains: on, fetching AT's vehicle positions every {POLL_SECONDS} s");
    }
}

/// With a key, takes the trains over before any stand-in is drawn; without, says so once.
fn start_live(mut commands: Commands, mut live: ResMut<LiveTrains>, mut trains: ResMut<Trains>) {
    if live.key.is_some() {
        live.switch_on(&mut trains, &mut commands);
        info!("live trains: on, fetching AT's vehicle positions every {POLL_SECONDS} s");
    } else {
        live.status = FeedStatus::NoKey;
        info!(
            "live trains: {KEY_VARIABLE} is not set, so the stand-ins drive; a free key is a \
             subscription on dev-portal.at.govt.nz"
        );
    }
}

/// Starts a fetch when one is due and none is running, and takes the one running when it
/// lands, see LiveTrains::apply. A fetch that fails is logged once per message and leaves
/// the trains where the last good one put them.
fn poll_feed(
    time: Res<Time>,
    mut commands: Commands,
    rail: Res<AucklandRail>,
    splines: Res<TrackSplines>,
    mut live: ResMut<LiveTrains>,
    mut trains: ResMut<Trains>,
) {
    if !live.on {
        return;
    }
    let Some(key) = live.key.clone() else {
        return;
    };
    live.poll.tick(time.delta());
    if live.poll.just_finished() {
        live.due = true;
    }

    if let Some(task) = live.task.as_mut() {
        let Some(result) = block_on(poll_once(task)) else {
            return;
        };
        live.task = None;
        match result {
            Ok(feed) => live.apply(
                &feed,
                time.elapsed_secs_f64(),
                &rail,
                &splines,
                &mut trains,
                &mut commands,
            ),
            Err(error) => {
                if live.status != FeedStatus::Failed(error.clone()) {
                    error!("live trains: the fetch failed: {error}");
                }
                live.status = FeedStatus::Failed(error);
            }
        }
    }

    if live.task.is_none() && live.due {
        live.due = false;
        live.task = Some(IoTaskPool::get().spawn(async move { feed::fetch(&key) }));
    }
}

/// Moves every train with a fix towards where its fix puts it now, see reckoned and
/// approach, and gives it the fix's direction and speed. A train on a line with no spline
/// this frame stays put, as a stand-in does.
fn steer_trains(
    time: Res<Time>,
    splines: Res<TrackSplines>,
    live: Res<LiveTrains>,
    mut trains: ResMut<Trains>,
) {
    if !trains.live {
        return;
    }
    let elapsed = time.elapsed_secs_f64();
    let dt = time.delta_secs_f64();

    for train in &mut trains.trains {
        let Some(fix) = live.fixes.get(&train.id) else {
            continue;
        };
        let Some(spline) = splines.splines.get(fix.line).and_then(Option::as_ref) else {
            continue;
        };
        let target = reckoned(fix, age(fix, elapsed), spline.length());
        train.distance = approach(train.distance, target, fix.direction, dt);
        train.direction = fix.direction;
        train.speed = fix.speed;
    }
}

/// The caption as the feed stands this frame: the age ticks up between fetches.
fn write_caption(time: Res<Time>, live: Res<LiveTrains>, mut trains: ResMut<Trains>) {
    let wanted = caption(&live.status, time.elapsed_secs_f64());
    if trains.caption != wanted {
        trains.caption = wanted;
    }
}

#[cfg(test)]
mod tests;
