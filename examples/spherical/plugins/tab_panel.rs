//! The F11 panel: three tabs over one body, built out of Bevy's own feathers widgets.
//!
//! Every other panel here is hand-drawn: a `Node` tree, `BackgroundColor` in the example's
//! colours and a system that paints hover and press. That is fine for a readout and a row of
//! buttons, and it is what the F2, F5 and F10 panels are. A panel that grows tabs wants more
//! than that — a checkbox, a slider, a theme the three tabs share — so this one is built on
//! `bevy_feathers`, Bevy's widget set, which the example now enables. The widgets carry their
//! own hover, press, focus and cursor behaviour, so there is no painting system below.
//!
//! Feathers has no tab control, and a tab strip is a row of buttons where one is held down:
//! the strip sits in the pane header as [`FeathersToolButton`]s, the active one in
//! `ButtonVariant::Primary` and the rest in `Plain`, and the bodies are siblings in the pane
//! body of which exactly one is laid out. Which one is a resource rather than a component on
//! the strip, so a tab can be switched from anywhere — a key, a command, another panel —
//! without finding the buttons first.
//!
//! Hiding is `Display::None` on the inactive bodies and `Visibility` on the panel as a whole.
//! The difference matters: a body that is `None` takes no space, so the panel is as tall as
//! the tab showing rather than as its tallest tab, while a hidden panel keeps its layout but
//! takes no clicks, which is what the F1 list relies on and what keeps this one out of the
//! way of the camera while it is down.
//!
//! Perf drives the FPS overlay's own config, which is the switch for every readout in the top
//! left corner — the frame rate, the frame time graph beneath it and the vram line beside them
//! — and Camera reuses the F1 list's two buttons, so one place still decides where home is and
//! what saving a view means. Both are small enough to live here.
//!
//! Marker, Terrain and AT trains are not filled here. Those controls are their own plugins'
//! business, so this one spawns the bodies, marks them [`MarkerTabBody`], [`TerrainTabBody`]
//! or [`SubTabBody`] and leaves them; `photo_markers::panel`, `provenance`, `trains::table`
//! and `trains::panel` fill them when they appear — the arrangement `camera_pose` already has
//! with the row the F1 list leaves empty for it. A plugin wanting its tab in front triggers
//! [`ShowTab`] rather than reaching for the panel's own components.
//!
//! A tab that holds one tool is one body. AT trains holds two, the stand-ins and AT's feed, so
//! it is divided again, into [`SubTab`]s: a second strip in a feathers subpane, which is the
//! same arrangement one level down and reads as nested rather than as a second panel of its
//! own. A tab that holds one tool declares no subtabs and is built as before.

use bevy::dev_tools::fps_overlay::FpsOverlayConfig;
use bevy::feathers::constants::size;
use bevy::feathers::containers::{
    pane, pane_body, pane_header, subpane, subpane_body, subpane_header,
};
use bevy::feathers::controls::{
    ButtonVariant, FeathersButton, FeathersCheckbox, FeathersSlider, FeathersToolButton,
};
use bevy::feathers::display::label_dim;
use bevy::feathers::font_styles::InheritableFont;
use bevy::feathers::theme::{ThemeBackgroundColor, ThemedText};
use bevy::feathers::tokens;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::text::FontSize;
use bevy::ui::Checked;
use bevy::ui::Overflow;
use bevy::ui_widgets::{Activate, ScrollArea, SliderPrecision, SliderValue, ValueChange};
use bevy::window::PrimaryWindow;
use bevy_terrain::prelude::OrbitalCameraController;
use big_space::grid::Grid;
use big_space::prelude::{CellCoord, Grids};

use crate::plugins::camera_pose::{home_pose, save_pose};
use crate::plugins::track_frames::move_to;
use crate::plugins::vram_usage::CopyStats;

/// Which tab is showing. One resource rather than a component on the strip, so that a tab
/// can be brought up from anywhere without the buttons having to be found first.
#[derive(Resource, Debug)]
struct ActiveTab(Tab);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Tab {
    Perf,
    Camera,
    Marker,
    Terrain,
    Trains,
    Help,
}

/// The one size every line of text in the panel is written at.
///
/// Fourteen, feathers' own `MEDIUM_FONT` and the size its widgets caption themselves at. The
/// tables in the panel were drawn at twelve and the whole panel was tried there, but twelve
/// reads as squeezed across a line: the face is the same width at either size and only the
/// letters shrink, so the words crowd rather than the column narrowing.
const PANEL_FONT: FontSize = size::MEDIUM_FONT;

/// The gap the panel keeps from the window's left and bottom edges, in pixels, as the other
/// panels keep. It is also what the panel's room is measured against: a panel free to be as
/// wide as the window would stand this much past its right edge.
const MARGIN: f32 = 12.0;

/// The gap between what sits in a column, in pixels, and half the gap between the columns
/// themselves.
const COLUMN_GAP: f32 = 6.0;

/// The least the panel is ever drawn at, in pixels, before anything has been measured: wide
/// enough for the marker readout's sentence and the six tabs in the strip.
const MIN_WIDTH: f32 = 420.0;

/// The panel's own furniture above and around a body, in pixels: the tab strip, the pane's
/// padding and the body's. Taken off the window before a body is told how much room it has, so
/// that it is the panel that keeps inside the screen and not the body alone.
const CHROME: f32 = 56.0;

/// Where the panel sits in the UI stack.
///
/// Every label this example draws over the terrain — the station names, the train names, the
/// markers' own readouts — is a root node of its own at the implicit zero, as the panel is, and
/// between roots of equal index the order is whatever the stack happened to collect first. That
/// is how a station name came to sit on top of a control. One index above them settles it, and
/// stays below the frame rate overlay, which Bevy puts near the top of the range on purpose.
/// The markers' cards are at -1 for the same reason in the other direction.
const PANEL_LAYER: i32 = 1;

impl Tab {
    /// The subtabs this tab is divided into, in the order they sit in its strip, and empty for
    /// a tab that is one tool.
    fn subtabs(self) -> &'static [SubTab] {
        match self {
            Self::Trains => &[SubTab::Live, SubTab::StandIns, SubTab::Tracks],
            Self::Help => &[
                SubTab::HelpCamera,
                SubTab::HelpVisu,
                SubTab::HelpMarkers,
                SubTab::HelpTrain,
            ],
            _ => &[],
        }
    }
}

/// The tabs in the order they sit in the header, which is also the order the bodies are
/// spawned in. The first is the one up when the panel is first shown.
const TABS: [Tab; 6] = [
    Tab::Perf,
    Tab::Camera,
    Tab::Marker,
    Tab::Terrain,
    Tab::Trains,
    Tab::Help,
];

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Self::Perf => "Perf",
            Self::Camera => "Camera",
            Self::Marker => "Marker",
            Self::Terrain => "Terrain",
            Self::Trains => "AT trains",
            Self::Help => "Help",
        }
    }

    /// The key that brings this tab forward, and puts the panel away when its tab is already
    /// in front: F1 at the left of the strip and on rightwards.
    ///
    /// The keys used to belong to the things themselves — F3 the sheet grid, F7 the trains, F10
    /// the markers — one key per feature, in the order the features were written, and a reader
    /// of the list had to learn ten of them. Every one of those switches is a box or a button
    /// on the tab it belongs to now, so the keys are the tabs': six of them, in the order they
    /// are read.
    fn key(self) -> KeyCode {
        match self {
            Self::Perf => KeyCode::F1,
            Self::Camera => KeyCode::F2,
            Self::Marker => KeyCode::F3,
            Self::Terrain => KeyCode::F4,
            Self::Trains => KeyCode::F5,
            Self::Help => KeyCode::F6,
        }
    }

    /// The key as the strip writes it, after the tab's own name.
    fn key_name(self) -> &'static str {
        match self {
            Self::Perf => "F1",
            Self::Camera => "F2",
            Self::Marker => "F3",
            Self::Terrain => "F4",
            Self::Trains => "F5",
            Self::Help => "F6",
        }
    }

    /// What the button in the strip says: the tab's name and the key that brings it up.
    fn caption(self) -> String {
        format!("{} ({})", self.title(), self.key_name())
    }
}

/// The panel as a whole: what F11 shows and hides, and the rectangle anything laying itself
/// out around the panel measures — the photo markers' cards do.
#[derive(Component)]
pub(crate) struct TabPanel;

/// One tab in the header's strip.
#[derive(Component)]
struct TabButton(Tab);

/// One tab's body in the pane body. Exactly one is laid out at a time.
#[derive(Component)]
struct TabBody(Tab);

/// The Marker tab's body, which `photo_markers::panel` fills. Marked rather than filled here
/// because the controls in it are that plugin's: it owns what a marker is and what the
/// buttons do, and this one owns where the body sits and when it is in front.
#[derive(Component)]
pub(crate) struct MarkerTabBody;

/// The body `provenance` fills: the sheet grid's height controls and the table of where each
/// terrain's pixels came from.
#[derive(Component)]
pub(crate) struct TerrainTabBody;

/// One page of a tab that holds several tools.
///
/// The AT trains tab's first two are the two drivers: each page switches its own driver and
/// lists the trains that driver is running, so the page you are on is the trains you are
/// looking at. The third is the track they run on rather than the trains.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SubTab {
    /// AT's feed: the trains it reports, where it says they are.
    Live,
    /// The stand-ins: one per line, driven at a speed of their own.
    StandIns,
    /// The track under them: the lines, their frames, and the editor that moves their points.
    Tracks,
    /// The two cameras' controls, as the README writes them.
    HelpCamera,
    /// What can be drawn over the terrain, and what it costs.
    HelpVisu,
    /// The photographs pinned over the terrain.
    HelpMarkers,
    /// The rail network's controls: the lines, the editor and the trains on them.
    HelpTrain,
}

impl SubTab {
    fn caption(self) -> &'static str {
        match self {
            Self::Live => "Live",
            Self::StandIns => "Stand-ins",
            Self::Tracks => "Tracks",
            Self::HelpCamera => "Camera",
            Self::HelpVisu => "Visu",
            Self::HelpMarkers => "Markers",
            Self::HelpTrain => "Train",
        }
    }

    /// The tab this page belongs to. Every page belongs to exactly one, which is what lets the
    /// pages of every tab share one enum and one resource.
    pub(crate) fn parent(self) -> Tab {
        match self {
            Self::Live | Self::StandIns | Self::Tracks => Tab::Trains,
            Self::HelpCamera | Self::HelpVisu | Self::HelpMarkers | Self::HelpTrain => Tab::Help,
        }
    }
}

/// The body a subtab's owner fills.
#[derive(Component)]
pub(crate) struct SubTabBody(pub(crate) SubTab);

/// One subtab in a tab's own strip.
#[derive(Component)]
struct SubTabButton(SubTab);

/// Which page is showing, for each tab that has pages.
///
/// Keyed by parent, because a tab keeps its page while another tab is looked at: one page for
/// the whole panel would blank every body of the tab that was not the one last clicked in. A
/// tab with no entry yet is on the first page it declares.
#[derive(Resource, Default, Debug)]
struct ActiveSubTab(HashMap<Tab, SubTab>);

impl ActiveSubTab {
    /// The page showing in a tab: what was last asked for in it, or its first.
    fn of(&self, parent: Tab) -> Option<SubTab> {
        self.0
            .get(&parent)
            .copied()
            .or_else(|| parent.subtabs().first().copied())
    }

    /// Whether this is the page showing in its own tab, which is the one laid out.
    fn showing(&self, subtab: SubTab) -> bool {
        self.of(subtab.parent()) == Some(subtab)
    }
}

/// Asks for a tab, from a plugin whose own key brings its tab forward.
#[derive(Event)]
pub(crate) enum ShowTab {
    /// Bring this tab forward, and the panel up with it.
    Up(Tab),
    /// Put the panel away, but only while this tab is the one in front, so a plugin standing
    /// its own tab down cannot take another's with it.
    Down(Tab),
    /// Away if this tab is already in front, forward if it is not: what a key owning one tab
    /// wants, whether the panel was last put away by that key or by F11.
    Toggle(Tab),
}

/// Says which page has been asked for, for the same reason as [`TabInFront`]: the AT trains
/// tab's Stand-ins page puts the stand-ins back that way.
///
/// Sent when a page's own button is pressed, and so never for the page that is merely up when
/// the panel is built, which has come to the front of nothing: a page that turned its own
/// driver on at startup would show a network of carriages the example deliberately opens
/// without.
#[derive(Event)]
pub(crate) struct SubTabInFront(pub(crate) SubTab);

/// Says which tab has come to the front, for a plugin that wants to turn its own thing on when
/// its tab is looked at: the AT trains tab shows the trains that way.
///
/// Sent as the tab changes, however it changed — a click on the strip, or a [`ShowTab`] from
/// elsewhere — and not when the panel is merely brought back up on a tab that was already in
/// front, which is F11's business rather than a tab's.
#[derive(Event)]
pub(crate) struct TabInFront(pub(crate) Tab);

/// Builds the panel: the pane, the strip of tabs in its header, and a body per tab.
///
/// The bodies are spawned with the active tab's display already set rather than left to
/// [`show_active_tab`], so that the panel is never laid out with all three at once, which
/// would size it to the three stacked for the frame before the first switch.
fn spawn_panel(
    mut commands: Commands,
    active: Res<ActiveTab>,
    active_sub: Res<ActiveSubTab>,
    config: Res<FpsOverlayConfig>,
) {
    let panel = commands
        .spawn_scene(pane())
        .insert((
            TabPanel,
            // The whole `Node`, not just the corner it sits in: an insert replaces the one
            // `pane` spawned with, so the column it lays its header and body out in has to
            // be said again here or the two would sit side by side.
            Node {
                position_type: PositionType::Absolute,
                left: px(MARGIN),
                bottom: px(MARGIN),
                // What the old hand-drawn marker panel was, as a floor rather than a width:
                // wide enough for the marker readout's sentence, and free to grow for the
                // provenance table, whose eight columns are wider than any sentence.
                //
                // A floor and no ceiling. A ceiling does not make the table narrower, since
                // its cells are deliberately NoWrap so that each column sizes to its whole
                // string: it only lets the rows run out past the panel's edge and over the
                // terrain. The panel owns the pointer over every pixel it covers, so the
                // Terrain tab does take a good part of the view — which is what that tab is,
                // and what the F2 panel it replaced was, the difference being that this one
                // grows rightwards from the corner rather than leftwards into it.
                // 420 rather than the old marker panel's 360: five tabs in the header is
                // what sets the floor now, not the widest sentence in a body.
                min_width: px(MIN_WIDTH),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                ..default()
            },
            GlobalZIndex(PANEL_LAYER),
            // Down until F11, as the F5 and F10 panels are until their keys.
            Visibility::Hidden,
        ))
        .id();

    let header = commands
        .spawn_scene(pane_header())
        .insert(ChildOf(panel))
        .id();
    // The buttons go in a row of their own inside the header, which lays its children out
    // with space between them: with the buttons as those children a strip of two would sit
    // one at either end of the panel. One child is one group, and a group starts where the
    // header starts, however wide the panel grows for the tab in front.
    let strip = commands.spawn((strip(), ChildOf(header))).id();

    for tab in TABS {
        commands
            .spawn_scene(bsn! {
                @FeathersToolButton {
                    @caption: bsn! { Text({tab.caption()}) ThemedText },
                    @variant: {variant_for(tab, active.0)},
                }
            })
            .insert((TabButton(tab), ChildOf(strip)))
            .observe(move |_: On<Activate>, mut active: ResMut<ActiveTab>| {
                active.0 = tab;
            });
    }

    let body = commands
        .spawn_scene(pane_body())
        .insert(ChildOf(panel))
        .id();

    for tab in TABS {
        let tab_body = commands
            .spawn((
                TabBody(tab),
                // What `fit_to_window` bounds, and a scroll so that a tab holding more than
                // the screen can is reachable rather than drawn past the edge: the provenance
                // table is wider than the window at the font it is read at, and the marker
                // controls are taller than a short window.
                ScrollArea,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(6),
                    overflow: Overflow::scroll(),
                    display: display_for(tab, active.0),
                    ..default()
                },
                ChildOf(body),
            ))
            .id();

        match tab {
            Tab::Perf => spawn_perf_tab(&mut commands, tab_body, &config),
            Tab::Camera => spawn_camera_tab(&mut commands, tab_body),
            // Spawned empty and filled by the markers' own plugin, see MarkerTabBody.
            Tab::Marker => {
                commands.entity(tab_body).insert(MarkerTabBody);
            }
            Tab::Terrain => {
                commands.entity(tab_body).insert(TerrainTabBody);
            }
            // Three pages of the README's controls, filled by the example itself.
            Tab::Help => spawn_subtabs(&mut commands, tab_body, tab.subtabs(), &active_sub),
            // Two drivers rather than one tool, so this body holds a strip of its own.
            Tab::Trains => spawn_subtabs(&mut commands, tab_body, tab.subtabs(), &active_sub),
        }
    }
}

/// A tab's own strip and a body per subtab, in a feathers subpane: the header carries the
/// buttons, and the bodies are plain nodes inside the one subpane body, as the tabs' own are
/// inside the pane body. Plain, because a subpane body arrives with a padding and a radius of
/// its own and an inserted `Node` would replace the lot.
fn spawn_subtabs(
    commands: &mut Commands,
    body: Entity,
    subtabs: &'static [SubTab],
    active: &ActiveSubTab,
) {
    let pane = commands.spawn_scene(subpane()).insert(ChildOf(body)).id();
    let header = commands
        .spawn_scene(subpane_header())
        .insert(ChildOf(pane))
        .id();
    let strip = commands.spawn((strip(), ChildOf(header))).id();

    for &subtab in subtabs {
        commands
            .spawn_scene(bsn! {
                @FeathersToolButton {
                    @caption: bsn! { Text({subtab.caption().to_string()}) ThemedText },
                    @variant: {variant_for_sub(subtab, active)},
                }
            })
            .insert((SubTabButton(subtab), ChildOf(strip)))
            .observe(
                move |_: On<Activate>, mut commands: Commands, mut active: ResMut<ActiveSubTab>| {
                    active.0.insert(subtab.parent(), subtab);
                    commands.trigger(SubTabInFront(subtab));
                },
            );
    }

    let pane_body = commands
        .spawn_scene(subpane_body())
        .insert(ChildOf(pane))
        .id();

    for &subtab in subtabs {
        commands.spawn((
            SubTabBody(subtab),
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                display: display_for_sub(subtab, active),
                ..default()
            },
            ChildOf(pane_body),
        ));
    }
}

/// Two columns side by side in a tab's body, with a rule between them, and the entities to
/// put things in.
///
/// A tab is one column by default, which is right for two or three widgets. A tab with more
/// than that is a panel's height and a half, and the panel stands in a corner of the very view
/// it is a tool for — so the long ones are read in two columns, with a line between them the
/// eye takes as two groups rather than as one list that happens to wrap. The rule is in the
/// colour the panes divide their headers with, and stretches to whichever column is taller.
///
/// Nothing is inserted on the body itself: it arrives with the `display` its tab is laid out
/// by, and an inserted `Node` would replace the lot.
pub(crate) fn two_columns(commands: &mut Commands, body: Entity) -> [Entity; 2] {
    let columns = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexStart,
                column_gap: px(2.0 * COLUMN_GAP),
                ..default()
            },
            ChildOf(body),
        ))
        .id();

    let left = spawn_column(commands, columns);
    commands.spawn((
        Node {
            width: px(1),
            align_self: AlignSelf::Stretch,
            ..default()
        },
        ThemeBackgroundColor(tokens::PANE_HEADER_DIVIDER),
        ChildOf(columns),
    ));
    let right = spawn_column(commands, columns);

    [left, right]
}

/// One of a body's columns, sizing itself to what goes in it.
fn spawn_column(commands: &mut Commands, columns: Entity) -> Entity {
    commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(COLUMN_GAP),
                ..default()
            },
            ChildOf(columns),
        ))
        .id()
}

/// The corner readouts' switch, their text size and the copy of the vram line, read out of the
/// overlay's config so the widgets open on what it is actually doing rather than on what it was
/// built with.
fn spawn_perf_tab(commands: &mut Commands, body: Entity, config: &FpsOverlayConfig) {
    // What the overlays show on the left, what is done with what they say on the right.
    let [left, right] = two_columns(commands, body);

    commands
        .spawn_scene(bsn! {
            @FeathersCheckbox {
                @caption: bsn! { Text("Overlays") ThemedText },
            }
        })
        .insert(ChildOf(left))
        .insert_if(Checked, || config.enabled)
        .observe(
            |change: On<ValueChange<bool>>,
             mut config: ResMut<FpsOverlayConfig>,
             mut commands: Commands| {
                // One box for the three readouts in that corner. The frame rate and the graph
                // are two switches in the overlay's own config rather than one, and the vram
                // line is another plugin's, which follows `enabled`: see
                // `vram_usage::follow_the_overlay`. They go up and down together because they
                // are one glance — a frame rate with no frame time beside it is half a story,
                // and a vram line on its own is a number with nothing to read it against.
                config.enabled = change.value;
                config.frame_time_graph_config.enabled = change.value;
                // The checkbox reports and changes nothing about itself, so the mark it
                // draws from is ours to keep, as bevy_ui_widgets' checkbox_self_update
                // would do were the value not also going somewhere.
                let mut checkbox = commands.entity(change.source);
                if change.value {
                    checkbox.insert(Checked);
                } else {
                    checkbox.remove::<Checked>();
                }
            },
        );

    commands
        .spawn_scene(label_dim("Overlay text size"))
        .insert(ChildOf(left));

    commands
        .spawn_scene(bsn! {
            @FeathersSlider {
                @min: 12.0,
                @max: 48.0,
                @value: {font_size_px(config)},
            }
        })
        .insert(ChildOf(left))
        // Whole pixels, and a slider with no precision is a slider that never redraws: the
        // widget's own update reads `SliderPrecision` as a plain query term rather than an
        // option, so without one neither the number on the slider nor the bar behind it is
        // ever written, and it sits at the 10.0 its scene is built with however far it is
        // dragged. The marker tab's sliders carry one for the same reason.
        .insert(SliderPrecision(0))
        .observe(
            |change: On<ValueChange<f32>>,
             mut config: ResMut<FpsOverlayConfig>,
             mut commands: Commands| {
                config.text_config.font_size = FontSize::Px(change.value);
                commands
                    .entity(change.source)
                    .insert(SliderValue(change.value));
            },
        );

    // The copy lives with the line it copies, in `vram_usage`; this is only where it is asked
    // for, so the corner holds readings and nothing to click.
    commands
        .spawn_scene(bsn! {
            @FeathersButton {
                @caption: bsn! { Text({COPY_CAPTION}) ThemedText CopyCaption },
            }
        })
        .insert(ChildOf(right))
        .observe(|copy: On<Activate>, mut commands: Commands| {
            commands.trigger(CopyStats);
            // The confirmation is the button saying so, rather than a note appearing in the
            // corner the copy came from: the press happened here, so the answer belongs here.
            // Inserted rather than reset, so a second press starts the two seconds again.
            commands
                .entity(copy.entity)
                .insert(Copied(Timer::from_seconds(2.0, TimerMode::Once)));
        });
}

/// What the copy button says, and what it says instead while a copy is being confirmed.
const COPY_CAPTION: &str = "Copy vram stats";
const COPIED_CAPTION: &str = "Copied!";

/// The copy button's caption, which [`flash_copied`] writes.
#[derive(Component, Default, Clone)]
struct CopyCaption;

/// On the copy button while it is confirming: the clock that puts its caption back.
#[derive(Component)]
struct Copied(Timer);

/// Says "Copied!" on the button for two seconds, then gives it its own caption back.
fn flash_copied(
    mut commands: Commands,
    time: Res<Time>,
    mut buttons: Query<(Entity, &mut Copied, &Children)>,
    mut captions: Query<&mut Text, With<CopyCaption>>,
) {
    for (entity, mut copied, children) in &mut buttons {
        let done = copied.0.tick(time.delta()).is_finished();

        for child in children.iter() {
            if let Ok(mut caption) = captions.get_mut(child) {
                caption.set_if_neq(Text::new(match done {
                    true => COPY_CAPTION,
                    false => COPIED_CAPTION,
                }));
            }
        }

        if done {
            commands.entity(entity).remove::<Copied>();
        }
    }
}

/// The F1 list's two camera buttons, on feathers ones. Both go through the same pair of
/// functions the list's do, [`home_pose`] and [`save_pose`], so the file is read afresh here
/// too and one place still decides both where home is and what writing it down means.
fn spawn_camera_tab(commands: &mut Commands, body: Entity) {
    // What the two buttons do on the left, what they mean on the right.
    let [left, right] = two_columns(commands, body);

    commands
        .spawn_scene(bsn! {
            @FeathersButton {
                @caption: bsn! { Text("Reset camera") ThemedText },
            }
        })
        .insert(ChildOf(left))
        .observe(|_: On<Activate>, grids: Grids, mut camera: CameraInGrid| {
            let Some((grid, mut transform, mut cell)) = camera_in_grid(&grids, &mut camera) else {
                return;
            };

            let (position, rotation) = home_pose();
            move_to(grid, &mut cell, &mut transform, position, rotation);
        });

    commands
        .spawn_scene(bsn! {
            @FeathersButton {
                @caption: bsn! { Text("Save this view") ThemedText },
            }
        })
        .insert(ChildOf(left))
        .observe(|_: On<Activate>, grids: Grids, mut camera: CameraInGrid| {
            let Some((grid, transform, cell)) = camera_in_grid(&grids, &mut camera) else {
                return;
            };

            // Out of the two components rather than the controller, which keeps only the
            // anchors a drag began from, as the F1 list's save does.
            save_pose(
                grid.grid_position_double(&cell, &transform),
                transform.rotation,
            );
        });

    commands
        .spawn_scene(label_dim(
            "Home is the saved view, or the start over Auckland",
        ))
        .insert(ChildOf(right));
}

/// The camera as the two buttons above need it: the entity to find its grid by, and the pair
/// of components a pose is read out of and written back through.
type CameraInGrid<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static mut Transform, &'static mut CellCoord),
    With<OrbitalCameraController>,
>;

/// The camera's grid and its pose, or nothing while there is no camera or it sits outside a
/// grid. Both buttons start here, so neither has to say what to do about a world that has no
/// camera in it yet.
fn camera_in_grid<'a>(
    grids: &'a Grids,
    camera: &'a mut CameraInGrid,
) -> Option<(&'a Grid, Mut<'a, Transform>, Mut<'a, CellCoord>)> {
    let (entity, transform, cell) = camera.single_mut().ok()?;

    Some((grids.parent_grid(entity)?, transform, cell))
}

/// Each tab's own key brings it forward, or puts the panel away when it is already in front.
///
/// One key per tab and none for the panel itself: the panel is only ever wanted for one of its
/// tabs, so the key that asks for the tab is the key that opens it, and pressing it again is
/// how it goes away. Hidden rather than despawned, as [`show_tab`] has it: a hidden node is not
/// rendered, so Bevy's UI picking skips it and the tabs neither take clicks nor keep them from
/// the camera while the panel is down.
fn tab_keys(keys: Res<ButtonInput<KeyCode>>, mut commands: Commands) {
    for tab in TABS {
        if keys.just_pressed(tab.key()) {
            commands.trigger(ShowTab::Toggle(tab));
        }
    }
}

/// The least the panel is drawn at, in logical pixels: the widest it has had to be, and as
/// tall as the AT trains tab makes it. None until that tab has been in front once.
#[derive(Resource, Default, Debug)]
struct PanelFloor {
    width: f32,
    height: Option<f32>,
    /// What the Terrain tab measured, which is what the Help tab is held to: see
    /// [`fit_to_window`]. None until that tab has been in front once.
    terrain_width: Option<f32>,
}

/// Holds the panel at a size that does not move under the pointer that is using it.
///
/// Two different rules, because the two edges that move are not alike. The panel is anchored in
/// the bottom left corner, so its width moves its right edge and its height moves its top one.
///
/// Width is the widest it has had to be: the Terrain tab's table is far wider than anything
/// else in the panel, and a panel that shrank back to a row of buttons between glances would
/// take its right edge across half the view every time the strip was clicked.
///
/// Height is the AT trains tab's, whichever tab is in front. That tab is the one left open
/// while flying — a table of trains is something to watch rather than something to set — so it
/// is the panel's resting height; the tabs with more in them grow past it and the ones with
/// less are held at it. Taking the tallest instead would mean a Perf tab of two widgets in a
/// panel the height of the marker controls.
///
/// Both are bounded by the room there is, so a table wider than the window cannot pin the
/// panel wider than the screen, and a window made smaller brings the floor down with it. The
/// measurement is the panel's own: `ComputedNode` is in physical pixels, hence the scale
/// factor, as the markers' card layout has it.
fn hold_panel_size(
    mut floor: ResMut<PanelFloor>,
    active: Res<ActiveTab>,
    window: Single<&Window, With<PrimaryWindow>>,
    panel: Single<(&mut Node, &ComputedNode), With<TabPanel>>,
) {
    let (mut node, computed) = panel.into_inner();
    let size = computed.size() * computed.inverse_scale_factor();
    let room = Vec2::new(window.width(), window.height()) - Vec2::splat(MARGIN);

    floor.width = floor.width.max(size.x).max(MIN_WIDTH).min(room.x);
    if active.0 == Tab::Trains {
        floor.height = Some(size.y.min(room.y));
    }
    if active.0 == Tab::Terrain {
        floor.terrain_width = Some(size.x.min(room.x));
    }

    let width = px(floor.width);
    if node.min_width != width {
        node.min_width = width;
    }

    let height = floor.height.map_or(Val::Auto, px);
    if node.min_height != height {
        node.min_height = height;
    }
}

/// Writes the panel's one size over every piece of text in it.
///
/// A size cannot simply be set where each line is spawned. A feathers widget carries an
/// `InheritableFont` of its own, at the 14 px its captions are meant for, which its caption
/// takes in preference to anything an ancestor says; a feathers label carries a `TextFont` and
/// a `PropagateOver` that keeps propagation off it; and a bare `Text` beside them carries
/// neither and is left at Bevy's 20 px default. Three ways to be a different size, so the
/// panel says it once, here, over all three.
///
/// Walked from the panel's own root rather than over every font in the world, so that the
/// labels over the terrain, the vram line and the frame rate overlay keep their own sizes. The
/// write is skipped where the size is already right, which is every frame but the one a widget
/// appears on.
fn one_font_size(
    panel: Single<Entity, With<TabPanel>>,
    children: Query<&Children>,
    mut inheritable: Query<&mut InheritableFont>,
    mut fonts: Query<&mut TextFont>,
) {
    for node in children.iter_descendants(*panel) {
        if let Ok(mut font) = inheritable.get_mut(node)
            && font.font_size != PANEL_FONT
        {
            font.font_size = PANEL_FONT;
        }
        if let Ok(mut font) = fonts.get_mut(node)
            && font.font_size != PANEL_FONT
        {
            font.font_size = PANEL_FONT;
        }
    }
}

/// Gives every tab body the room there is: the window, less the gap the panel keeps from the
/// edge it stands at and the panel's own furniture.
///
/// In pixels from the window rather than as a percentage, because a percentage knows about
/// neither: a body at 100% would put the panel a margin past the opposite edge, and the strip
/// above it a strip's height past the top.
fn fit_to_window(
    floor: Res<PanelFloor>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut bodies: Query<(&TabBody, &mut Node)>,
) {
    let room = Vec2::new(window.width(), window.height()) - Vec2::splat(MARGIN + CHROME);

    for (tab, mut body) in &mut bodies {
        // The Help tab is lines of prose rather than widgets, and prose given the whole window
        // runs to a width nothing is read at. It is held to what the Terrain tab measured —
        // the widest the panel is for a reason — so the panel's edge does not move when the
        // help is opened, and the lines wrap at a width the rest of the panel already has.
        let room = match tab.0 {
            Tab::Help => Vec2::new(
                floor.terrain_width.unwrap_or(floor.width).min(room.x),
                room.y,
            ),
            _ => room,
        };

        let (width, height) = (px(room.x.max(0.0)), px(room.y.max(0.0)));

        if body.max_width != width {
            body.max_width = width;
        }
        if body.max_height != height {
            body.max_height = height;
        }
    }
}

/// Lays out the active tab's body and holds its button down.
///
/// Runs when the resource changes, which is the only thing that moves a tab, and writes
/// through `set_if_neq` so feathers' own styling pass sees a changed variant only on the two
/// buttons that did change.
fn show_active_tab(
    active: Res<ActiveTab>,
    mut commands: Commands,
    mut bodies: Query<(&TabBody, &mut Node)>,
    mut buttons: Query<(&TabButton, &mut ButtonVariant)>,
) {
    commands.trigger(TabInFront(active.0));

    for (body, mut node) in &mut bodies {
        let display = display_for(body.0, active.0);
        if node.display != display {
            node.display = display;
        }
    }

    for (button, mut variant) in &mut buttons {
        variant.set_if_neq(variant_for(button.0, active.0));
    }
}

/// Brings a tab forward, or puts the panel away with that tab in front, for [`ShowTab`].
fn show_tab(
    request: On<ShowTab>,
    mut active: ResMut<ActiveTab>,
    mut panel: Single<(&mut Visibility, &InheritedVisibility), With<TabPanel>>,
) {
    let (tab, up) = match *request {
        ShowTab::Up(tab) => (tab, true),
        ShowTab::Down(tab) if active.0 != tab => return,
        ShowTab::Down(tab) => (tab, false),
        // Judged by what is on screen rather than by what was asked for last: F11 may have put
        // the panel away since, and the key should then bring its tab back rather than toggle
        // it off.
        ShowTab::Toggle(tab) => (tab, !(panel.1.get() && active.0 == tab)),
    };

    if up {
        active.0 = tab;
        panel.0.set_if_neq(Visibility::Visible);
    } else {
        panel.0.set_if_neq(Visibility::Hidden);
    }
}

/// Lays out the active subtab's body and holds its button down, as [`show_active_tab`] does a
/// level up.
fn show_active_subtab(
    active: Res<ActiveSubTab>,
    mut bodies: Query<(&SubTabBody, &mut Node)>,
    mut buttons: Query<(&SubTabButton, &mut ButtonVariant)>,
) {
    for (body, mut node) in &mut bodies {
        let display = display_for_sub(body.0, &active);
        if node.display != display {
            node.display = display;
        }
    }

    for (button, mut variant) in &mut buttons {
        variant.set_if_neq(variant_for_sub(button.0, &active));
    }
}

/// The row the buttons of a strip sit in, side by side from the left.
fn strip() -> Node {
    Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: px(4),
        ..default()
    }
}

/// `Primary` for the page showing in its tab, `Plain` for the others.
fn variant_for_sub(subtab: SubTab, active: &ActiveSubTab) -> ButtonVariant {
    if active.showing(subtab) {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Plain
    }
}

/// `None` rather than hidden for the pages not showing, so a tab is as tall as its page that
/// is.
fn display_for_sub(subtab: SubTab, active: &ActiveSubTab) -> Display {
    if active.showing(subtab) {
        Display::Flex
    } else {
        Display::None
    }
}

/// `Primary` for the tab showing, `Plain` for the others: the strip's one highlight.
fn variant_for(tab: Tab, active: Tab) -> ButtonVariant {
    if tab == active {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Plain
    }
}

/// `None` rather than hidden for the tabs not showing, so the panel is as tall as the one
/// that is.
fn display_for(tab: Tab, active: Tab) -> Display {
    if tab == active {
        Display::Flex
    } else {
        Display::None
    }
}

/// The overlay's text size as the slider's number. `TextFont` holds a [`FontSize`], which is
/// pixels or a share of the viewport, and only the first has a number the slider can open on.
fn font_size_px(config: &FpsOverlayConfig) -> f32 {
    match config.text_config.font_size {
        FontSize::Px(px) => px,
        _ => 32.0,
    }
}

/// The panel: a tab per tool, each on its own key, F1 at the left of the strip and on
/// rightwards.
pub struct TabPanelPlugin;

impl Plugin for TabPanelPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ActiveTab(TABS[0]))
            .init_resource::<PanelFloor>()
            .init_resource::<ActiveSubTab>()
            .add_systems(Startup, spawn_panel)
            .add_observer(show_tab)
            .add_systems(
                Update,
                (
                    tab_keys,
                    fit_to_window,
                    hold_panel_size,
                    one_font_size,
                    show_active_tab.run_if(resource_changed::<ActiveTab>),
                    show_active_subtab.run_if(resource_changed::<ActiveSubTab>),
                    flash_copied,
                ),
            );
    }
}
