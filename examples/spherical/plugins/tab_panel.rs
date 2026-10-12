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
//! The three tabs are what a panel like this is for rather than an end in themselves: View
//! drives the FPS overlay's own config, so the checkbox and slider move something real;
//! Camera reuses the F1 list's reset, so there is one place that decides where home is; and
//! Frame reads the diagnostics the overlay already collects, which is the tab that proves a
//! body goes on updating while it is the one showing.

use bevy::dev_tools::fps_overlay::FpsOverlayConfig;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::feathers::containers::{pane, pane_body, pane_header};
use bevy::feathers::controls::{
    ButtonVariant, FeathersButton, FeathersCheckbox, FeathersSlider, FeathersToolButton,
};
use bevy::feathers::display::label_dim;
use bevy::feathers::theme::ThemedText;
use bevy::prelude::*;
use bevy::text::FontSize;
use bevy::ui::Checked;
use bevy::ui_widgets::{Activate, SliderValue, ValueChange};
use bevy_terrain::prelude::OrbitalCameraController;
use big_space::prelude::{CellCoord, Grids};

use crate::plugins::camera_pose::home_pose;
use crate::plugins::track_frames::move_to;

/// Which tab is showing. One resource rather than a component on the strip, so that a tab
/// can be brought up from anywhere without the buttons having to be found first.
#[derive(Resource, Debug)]
struct ActiveTab(Tab);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    View,
    Camera,
    Frame,
}

/// The tabs in the order they sit in the header, which is also the order the bodies are
/// spawned in. The first is the one up when the panel is first shown.
const TABS: [Tab; 3] = [Tab::View, Tab::Camera, Tab::Frame];

impl Tab {
    fn caption(self) -> &'static str {
        match self {
            Self::View => "View",
            Self::Camera => "Camera",
            Self::Frame => "Frame",
        }
    }
}

/// The panel as a whole: what F11 shows and hides.
#[derive(Component)]
struct TabPanel;

/// One tab in the header's strip.
#[derive(Component)]
struct TabButton(Tab);

/// One tab's body in the pane body. Exactly one is laid out at a time.
#[derive(Component)]
struct TabBody(Tab);

/// The Frame tab's one line, rewritten from the diagnostics each frame.
#[derive(Component)]
struct FrameReadout;

/// Builds the panel: the pane, the strip of tabs in its header, and a body per tab.
///
/// The bodies are spawned with the active tab's display already set rather than left to
/// [`show_active_tab`], so that the panel is never laid out with all three at once, which
/// would size it to the three stacked for the frame before the first switch.
fn spawn_panel(mut commands: Commands, active: Res<ActiveTab>, config: Res<FpsOverlayConfig>) {
    let panel = commands
        .spawn_scene(pane())
        .insert((
            TabPanel,
            // The whole `Node`, not just the corner it sits in: an insert replaces the one
            // `pane` spawned with, so the column it lays its header and body out in has to
            // be said again here or the two would sit side by side.
            Node {
                position_type: PositionType::Absolute,
                left: px(12),
                bottom: px(12),
                width: px(260),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                ..default()
            },
            // Down until F11, as the F5 and F10 panels are until their keys.
            Visibility::Hidden,
        ))
        .id();

    let header = commands
        .spawn_scene(pane_header())
        .insert(ChildOf(panel))
        .id();

    for tab in TABS {
        commands
            .spawn_scene(bsn! {
                @FeathersToolButton {
                    @caption: bsn! { Text({tab.caption().to_string()}) ThemedText },
                    @variant: {variant_for(tab, active.0)},
                }
            })
            .insert((TabButton(tab), ChildOf(header)))
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
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(6),
                    display: display_for(tab, active.0),
                    ..default()
                },
                ChildOf(body),
            ))
            .id();

        match tab {
            Tab::View => spawn_view_tab(&mut commands, tab_body, &config),
            Tab::Camera => spawn_camera_tab(&mut commands, tab_body),
            Tab::Frame => spawn_frame_tab(&mut commands, tab_body),
        }
    }
}

/// The FPS overlay's own two settings, read out of its config so the widgets open on what
/// the overlay is actually doing rather than on what it was built with.
fn spawn_view_tab(commands: &mut Commands, body: Entity, config: &FpsOverlayConfig) {
    commands
        .spawn_scene(bsn! {
            @FeathersCheckbox {
                @caption: bsn! { Text("FPS overlay") ThemedText },
            }
        })
        .insert(ChildOf(body))
        .insert_if(Checked, || config.enabled)
        .observe(
            |change: On<ValueChange<bool>>,
             mut config: ResMut<FpsOverlayConfig>,
             mut commands: Commands| {
                config.enabled = change.value;
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
        .insert(ChildOf(body));

    commands
        .spawn_scene(bsn! {
            @FeathersSlider {
                @min: 12.0,
                @max: 48.0,
                @value: {font_size_px(config)},
            }
        })
        .insert(ChildOf(body))
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
}

/// The F1 list's reset, on a feathers button. Both go through [`home_pose`], so the saved
/// pose is read afresh here too and one place still decides where home is.
fn spawn_camera_tab(commands: &mut Commands, body: Entity) {
    commands
        .spawn_scene(bsn! {
            @FeathersButton {
                @caption: bsn! { Text("Reset camera") ThemedText },
            }
        })
        .insert(ChildOf(body))
        .observe(
            |_: On<Activate>,
             grids: Grids,
             mut camera: Query<
                (Entity, &mut Transform, &mut CellCoord),
                With<OrbitalCameraController>,
            >| {
                let Ok((entity, mut transform, mut cell)) = camera.single_mut() else {
                    return;
                };
                let Some(grid) = grids.parent_grid(entity) else {
                    return;
                };

                let (position, rotation) = home_pose();
                move_to(grid, &mut cell, &mut transform, position, rotation);
            },
        );

    commands
        .spawn_scene(label_dim("The saved pose, or the start over Auckland"))
        .insert(ChildOf(body));
}

/// One line, rewritten by [`update_frame_readout`].
fn spawn_frame_tab(commands: &mut Commands, body: Entity) {
    commands.spawn((
        FrameReadout,
        Text::new("no measurement yet"),
        ThemedText,
        ChildOf(body),
    ));
}

/// F11 shows and hides the panel. Hidden rather than despawned: a hidden node is not
/// rendered, so Bevy's UI picking skips it and the tabs neither take clicks nor keep them
/// from the camera while the panel is down.
fn toggle_panel(
    keys: Res<ButtonInput<KeyCode>>,
    mut panel: Single<&mut Visibility, With<TabPanel>>,
) {
    if !keys.just_pressed(KeyCode::F11) {
        return;
    }

    let visibility = match **panel {
        Visibility::Hidden => Visibility::Visible,
        _ => Visibility::Hidden,
    };
    panel.set_if_neq(visibility);
}

/// Lays out the active tab's body and holds its button down.
///
/// Runs when the resource changes, which is the only thing that moves a tab, and writes
/// through `set_if_neq` so feathers' own styling pass sees a changed variant only on the two
/// buttons that did change.
fn show_active_tab(
    active: Res<ActiveTab>,
    mut bodies: Query<(&TabBody, &mut Node)>,
    mut buttons: Query<(&TabButton, &mut ButtonVariant)>,
) {
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

/// The Frame tab's line: what the overlay graphs, as numbers.
///
/// Written every frame while the panel exists rather than only while Frame is up: the two
/// diagnostics are already collected and the comparison below is cheaper than a run
/// condition that would have to find the panel's visibility first.
fn update_frame_readout(
    diagnostics: Res<DiagnosticsStore>,
    mut readout: Single<&mut Text, With<FrameReadout>>,
) {
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|fps| fps.smoothed());
    let frame_time = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|frame_time| frame_time.smoothed());

    let line = match (fps, frame_time) {
        (Some(fps), Some(frame_time)) => format!("{fps:.0} fps, {frame_time:.2} ms"),
        _ => "no measurement yet".to_string(),
    };

    if readout.0 != line {
        readout.0 = line;
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

/// The F11 panel: tabs over the FPS overlay's settings, the camera's reset and the frame
/// diagnostics.
pub struct TabPanelPlugin;

impl Plugin for TabPanelPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ActiveTab(TABS[0]))
            .add_systems(Startup, spawn_panel)
            .add_systems(
                Update,
                (
                    toggle_panel,
                    show_active_tab.run_if(resource_changed::<ActiveTab>),
                    update_frame_readout,
                ),
            );
    }
}
