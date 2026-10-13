//! The AT trains tab's two pages: the stand-ins and AT's feed.
//!
//! One page per driver, because the two drivers are exclusive: the feed takes the trains over
//! when it is switched on and gives them back when it is switched off, so the trains on screen
//! are always one page's or the other's. Each page carries that driver's switch, its settings
//! and a grid of its trains with the camera icon that rides behind one, see `table::spawn_grid`
//! — the page you are on is the trains you are looking at.
//!
//! Neither switch had an interface before this: F7 and F8 were the whole of them, and the speed
//! a stand-in runs at was a constant a reader of the source could find and nobody else. A panel
//! is where a number like that belongs.
//!
//! The boxes say what their own page is about. The Stand-ins box ticked means the stand-ins are
//! what runs and what is seen, so it stands the feed down if the feed was driving; unticked, it
//! hides the carriages, stand-ins and all. The Live box is the other way round: switching the
//! feed on takes the trains over, and makes sure there is something to see if they had been
//! hidden. So the Stand-ins box is ticked only while the stand-ins are both driving and drawn,
//! which is what `sync_boxes` holds it to.
//!
//! Each page reads and writes the resource that owns the setting rather than keeping a copy:
//! `Trains` for the driver and its speed, `LiveTrains` for the feed. So a key and a widget can
//! never disagree — F7 and the box both write `Trains::shown`, and the box follows it back in
//! `sync_boxes`, as the sheet grid's checkboxes follow theirs. The feed is the one that cannot
//! be written directly: switching it on and off takes the rail, the trains and a `Commands` of
//! its own, so the box asks for it with [`ToggleFeed`] and live_trains does the work, as F8
//! does.

use bevy::feathers::controls::{FeathersCheckbox, FeathersSlider};
use bevy::feathers::display::label_dim;
use bevy::feathers::theme::ThemedText;
use bevy::prelude::*;
use bevy::ui::Checked;
use bevy::ui_widgets::{SliderPrecision, SliderValue, ValueChange};

use super::Trains;
use super::table::{Driver, KM_H, spawn_caption, spawn_grid};
use crate::plugins::live_trains::{LiveTrains, ToggleFeed};
use crate::plugins::tab_panel::{SubTab, SubTabBody, SubTabInFront, Tab, TabInFront};

/// The two pages' systems, added by TrainsPlugin.
pub(super) struct TrainsPanelPlugin;

impl Plugin for TrainsPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(show_on_tab)
            .add_observer(drive_on_page)
            .add_systems(Update, (fill_pages, sync_boxes, sync_speed_slider).chain());
    }
}

/// How fast a stand-in may be driven, in kilometres per hour: the unit the table reads the
/// speed in and the README quotes it in, rather than the metres per second the resource keeps.
/// Up to 600, which no train anywhere does, so that a stand-in can be sent along a line far
/// faster than one runs when it is the line that is being looked at rather than the train.
const SPEED_RANGE: (f32, f32) = (0.0, 600.0);

/// Which switch a checkbox drives. The resource is the truth for both, so a box and its key
/// can never disagree: both write the field, and the box is brought back to it every frame.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Switch {
    /// `Trains::shown`, with the feed stood down: the stand-ins drive and are drawn.
    StandIns,
    /// `LiveTrains::on`: AT's feed drives instead.
    Feed,
}

/// The slider over `Trains::speed`.
#[derive(Component)]
struct SpeedSlider;

/// Shows the trains as the AT trains tab comes forward.
///
/// A tab of train tools with no trains running is two pages about nothing, and the box on the
/// page says plainly what has just happened and puts them away again. Only on the way in: F7,
/// F4 and that box all still hide them while the tab is open, and none of them is overruled
/// from here.
fn show_on_tab(tab: On<TabInFront>, mut trains: ResMut<Trains>) {
    if tab.0 == Tab::Trains && !trains.shown {
        trains.shown = true;
        info!("trains: shown, the AT trains tab was opened");
    }
}

/// Puts the stand-ins back as their page comes forward, as ticking their box would.
///
/// A page of stand-in tools is about the stand-ins, so coming to it says which trains are
/// wanted: they are shown, and the feed stands down if it had the trains, which is what brings
/// them back. The Live page does not switch the feed on in return — a page clicked through
/// should not start fetching from AT — so coming to it is a look at what the feed is doing, and
/// its box is what asks for it.
fn drive_on_page(
    page: On<SubTabInFront>,
    mut commands: Commands,
    mut trains: ResMut<Trains>,
    live: Res<LiveTrains>,
) {
    if page.0 != SubTab::StandIns {
        return;
    }

    trains.shown = true;
    if live.on {
        commands.trigger(ToggleFeed);
        info!("trains: the feed stood down, the Stand-ins page was opened");
    }
}

/// Fills each page the moment its body appears: the switch, what that driver has to set, and
/// its grid of trains under them.
///
/// Keyed off the bodies rather than ordered against the tab panel's startup system, so neither
/// plugin has to know when the other runs. The widgets open on what the resources say, which is
/// what the keys may have changed before the tab was ever looked at.
fn fill_pages(
    mut commands: Commands,
    bodies: Query<(Entity, &SubTabBody), Added<SubTabBody>>,
    trains: Res<Trains>,
    live: Res<LiveTrains>,
) {
    for (body, page) in &bodies {
        match page.0 {
            SubTab::StandIns => {
                spawn_box(
                    &mut commands,
                    body,
                    Switch::StandIns,
                    "show stand-ins",
                    stand_ins_running(&trains, &live),
                );
                commands
                    .spawn_scene(label_dim("how fast a stand-in runs, in km/h"))
                    .insert(ChildOf(body));
                spawn_speed_slider(&mut commands, body, kilometres_per_hour(&trains));
                spawn_grid(&mut commands, body, Driver::StandIns);
            }
            SubTab::Live => {
                spawn_box(&mut commands, body, Switch::Feed, "live trains", live.on);
                // How the fetching is going, which is the feed's own account of itself.
                spawn_caption(&mut commands, body);
                spawn_grid(&mut commands, body, Driver::Live);
            }
            // The other tabs' pages are their own plugins' to fill.
            _ => {}
        }
    }
}

/// A checkbox over one of the two switches. The press writes the resource, or asks for the
/// feed to be switched, and `sync_boxes` brings the mark back to whatever the resource says.
fn spawn_box(commands: &mut Commands, body: Entity, switch: Switch, caption: &str, on: bool) {
    commands
        .spawn_scene(bsn! {
            @FeathersCheckbox {
                @caption: bsn! { Text({caption.to_string()}) ThemedText },
            }
        })
        .insert((switch, ChildOf(body)))
        .insert_if(Checked, || on)
        .observe(
            move |change: On<ValueChange<bool>>,
                  mut commands: Commands,
                  mut trains: ResMut<Trains>,
                  live: Res<LiveTrains>| {
                match switch {
                    Switch::StandIns => {
                        trains.shown = change.value;
                        // Ticked, this page's own trains are what should be running: the feed
                        // stands down if it was driving, which brings the stand-ins back.
                        // Unticked, nothing is drawn, the stand-ins included.
                        if change.value && live.on {
                            commands.trigger(ToggleFeed);
                        }
                    }
                    Switch::Feed => {
                        // Something to see when the feed comes on, in case the carriages were
                        // hidden before it was asked for.
                        if !live.on {
                            trains.shown = true;
                        }
                        // The feed's own plugin owns switching it: see the module doc.
                        commands.trigger(ToggleFeed);
                    }
                }
            },
        );
}

/// The speed slider, in whole kilometres per hour: a stand-in is not driven any finer, and a
/// slider with no precision never redraws itself at all.
fn spawn_speed_slider(commands: &mut Commands, body: Entity, speed: f32) {
    let (min, max) = SPEED_RANGE;

    commands
        .spawn_scene(bsn! {
            @FeathersSlider {
                @min: {min},
                @max: {max},
                @value: {speed},
            }
        })
        .insert((SpeedSlider, SliderPrecision(0), ChildOf(body)))
        .observe(
            |change: On<ValueChange<f32>>, mut commands: Commands, mut trains: ResMut<Trains>| {
                trains.speed = change.value as f64 / KM_H;
                commands
                    .entity(change.source)
                    .insert(SliderValue(change.value));
            },
        );
}

/// Brings each box back to its switch, for a key pressed since it was drawn.
fn sync_boxes(
    mut commands: Commands,
    trains: Res<Trains>,
    live: Res<LiveTrains>,
    boxes: Query<(Entity, &Switch, Has<Checked>)>,
) {
    for (entity, switch, checked) in &boxes {
        let on = match switch {
            Switch::StandIns => stand_ins_running(&trains, &live),
            Switch::Feed => live.on,
        };

        match (on, checked) {
            (true, false) => {
                commands.entity(entity).insert(Checked);
            }
            (false, true) => {
                commands.entity(entity).remove::<Checked>();
            }
            _ => {}
        }
    }
}

/// Whether the stand-ins are both driving and drawn, which is what their box means: with the
/// feed on it is the feed's trains that are seen, whatever the carriages' own switch says.
fn stand_ins_running(trains: &Trains, live: &LiveTrains) -> bool {
    trains.shown && !live.on
}

/// The stand-ins' speed as the slider has it: the resource keeps metres per second, which is
/// what the driver moves them by, and the panel shows the unit a timetable would.
fn kilometres_per_hour(trains: &Trains) -> f32 {
    (trains.speed * KM_H) as f32
}

/// Brings the speed slider back to the resource, for a speed set from anywhere else.
fn sync_speed_slider(
    mut commands: Commands,
    trains: Res<Trains>,
    sliders: Query<(Entity, &SliderValue), With<SpeedSlider>>,
) {
    for (entity, value) in &sliders {
        let wanted = kilometres_per_hour(&trains);
        if (value.0 - wanted).abs() > 0.5 {
            commands.entity(entity).insert(SliderValue(wanted));
        }
    }
}
