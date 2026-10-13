//! The marker controls: which marker is selected, what colour it is, and how its card is drawn.
//!
//! These used to be a panel of their own in the bottom right corner. They are now the F11
//! panel's Marker tab: this plugin no longer spawns a panel, it fills the body that
//! `tab_panel` marks [`MarkerTabBody`] and leaves for it, the way `camera_pose` fills the row
//! the F1 list leaves empty. The frame, the corner, the background and the showing and hiding
//! are the tab panel's; what is in the body is still this plugin's. F10 keeps its three states
//! and brings the tab forward as the editing one comes on.
//!
//! F10 still stands the rail editor down as F5 stands these controls down, because Delete and
//! Escape have one owner at a time, even though the two no longer share a corner.
//!
//! The buttons, the size sliders and the readouts are feathers widgets, which carry their own
//! hover, press, focus and disabled states, so there is nothing here that paints a button.
//! What cannot apply is given `InteractionDisabled` and the widget does the rest.
//!
//! The colour is hue, saturation and value on three sliders, the headless slider of the sheet
//! grid's height control with a gradient under each track so a drag's effect can be seen before
//! it is made. Those three stay hand-drawn: feathers' own colour slider draws the same gradient
//! but only for RGB and HSL channels, and the marker holds hue, saturation and value. It holds
//! those three numbers rather than a colour because a very light grey, which is the default, has
//! no hue to read back: deriving the sliders from a colour would send the hue to zero the moment
//! the saturation reached it, and the hue slider would seem dead. Going through HSL to borrow the
//! widget would put that loss back in by another door.
//! The presets beside them are shortcuts, not the only colours: the sliders reach all of them.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::feathers::controls::{FeathersButton, FeathersSlider};
use bevy::feathers::display::label_dim;
use bevy::feathers::theme::ThemedText;
use bevy::ui::{Display, InteractionDisabled, UiSystems};
use bevy::ui_widgets::{
    Activate, Slider, SliderPrecision, SliderRange, SliderStep, SliderThumb, SliderValue,
    TrackClick, ValueChange, observe,
};
use bevy::{prelude::*, text::FontSize};

use crate::plugins::tab_panel::{MarkerTabBody, ShowTab, Tab, TabInFront, two_columns};

use super::card::MarkerCard;
use super::card::{MAX_CARD, MAX_RADIUS, MIN_CARD, MIN_RADIUS};
use super::scatter::{self, MAX_COUNT, MIN_COUNT, Scatter};
use super::tether::{MAX_WIDTH, MIN_WIDTH};
use super::{DEFAULT_COLOUR, MarkerView, PhotoMarkerSystems, PhotoMarkers, Showing, file, photo};
use crate::plugins::provenance::ascii;
use crate::plugins::rail_editor::RailEditor;

/// The panel and everything on it, added by PhotoMarkersPlugin.
pub(super) struct PhotoPanelPlugin;

impl Plugin for PhotoPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_pressed)
            .add_observer(show_on_tab)
            .add_systems(
                Update,
                (
                    open_marker_tab,
                    fill_marker_tab,
                    press_swatches,
                    enable_buttons,
                    apply_colour,
                    sync_sliders,
                    style_sliders,
                )
                    .chain(),
            );
        app.add_systems(
            PostUpdate,
            // After the mouse has had its say on the selection, so the readout is of the marker
            // clicked this frame rather than the one before it.
            // After the placing too, which is what measures the marker on screen.
            (update_readout, update_size_readout, update_scatter_readout).after(PhotoMarkerSystems),
        );
        app.add_systems(
            PostUpdate,
            // After every system that places a label or a panel, all of which run before the
            // nodes are laid out, and before that laying out, which is what reads this.
            quieten_everything_else
                .after(UiSystems::Prepare)
                .before(UiSystems::Layout),
        );
    }
}

/// The gap between the controls in a row, as the F5 panel and the F2 table keep.
const MARGIN: f32 = 6.0;

/// The colour sliders' track width and thumb size, in pixels, as the sheet grid's.
const SLIDER_WIDTH: f32 = 150.0;
const THUMB_SIZE: f32 = 12.0;

/// The swatch showing the current colour, square, in pixels.
const SWATCH_SIZE: f32 = 22.0;

/// The preset colours, very light grey first and as the default. Held as hue, saturation and
/// value, so that pressing one sets the sliders rather than a colour the sliders cannot read.
const PRESETS: [(&str, Hsva); 8] = [
    ("grey", DEFAULT_COLOUR),
    ("white", Hsva::hsv(0.0, 0.0, 1.0)),
    ("black", Hsva::hsv(0.0, 0.0, 0.08)),
    ("red", Hsva::hsv(2.0, 0.78, 0.85)),
    ("amber", Hsva::hsv(36.0, 0.8, 0.9)),
    ("green", Hsva::hsv(128.0, 0.6, 0.72)),
    ("blue", Hsva::hsv(205.0, 0.72, 0.85)),
    ("magenta", Hsva::hsv(320.0, 0.6, 0.85)),
];

/// The text describing the selection.
#[derive(Component)]
struct Readout;

/// The line under it, saying how the selected marker stands on screen.
#[derive(Component)]
struct SizeReadout;

/// The square showing the colour the sliders hold.
#[derive(Component)]
struct ColourSwatch;

/// The hex code beside it.
#[derive(Component)]
struct ColourHex;

/// Which channel a slider edits.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Channel {
    Hue,
    Saturation,
    Value,
}

impl Channel {
    fn name(self) -> &'static str {
        match self {
            Self::Hue => "hue",
            Self::Saturation => "saturation",
            Self::Value => "value",
        }
    }

    /// The slider's range: hue is degrees round the wheel, the other two a fraction.
    fn range(self) -> SliderRange {
        match self {
            Self::Hue => SliderRange::new(0.0, 360.0),
            Self::Saturation | Self::Value => SliderRange::new(0.0, 1.0),
        }
    }

    fn of(self, colour: Hsva) -> f32 {
        match self {
            Self::Hue => colour.hue,
            Self::Saturation => colour.saturation,
            Self::Value => colour.value,
        }
    }

    fn set(self, colour: &mut Hsva, value: f32) {
        match self {
            Self::Hue => colour.hue = value,
            Self::Saturation => colour.saturation = value,
            Self::Value => colour.value = value,
        }
    }

    /// How far along its track the channel's value stands, 0 to 1, which is where its thumb goes.
    fn fraction(self, colour: Hsva) -> f32 {
        match self {
            Self::Hue => self.of(colour) / 360.0,
            Self::Saturation | Self::Value => self.of(colour),
        }
    }

    /// The gradient under the track: what the colour would be at either end of the drag, with
    /// the other two channels as they are. Hue takes the six corners of the wheel, so the strip
    /// reads as a rainbow rather than as a fade from red to red.
    fn gradient(self, colour: Hsva) -> Vec<ColorStop> {
        match self {
            Self::Hue => (0..=6)
                .map(|step| {
                    let hue = step as f32 * 60.0;
                    ColorStop::auto(Color::from(Hsva::hsv(
                        hue,
                        colour.saturation.max(0.85),
                        colour.value.max(0.85),
                    )))
                })
                .collect(),
            Self::Saturation => vec![
                ColorStop::auto(Color::from(Hsva::hsv(colour.hue, 0.0, colour.value))),
                ColorStop::auto(Color::from(Hsva::hsv(colour.hue, 1.0, colour.value))),
            ],
            Self::Value => vec![
                ColorStop::auto(Color::from(Hsva::hsv(colour.hue, colour.saturation, 0.0))),
                ColorStop::auto(Color::from(Hsva::hsv(colour.hue, colour.saturation, 1.0))),
            ],
        }
    }
}

/// A slider's track, which carries the gradient, and its thumb, which is moved to match.
#[derive(Component)]
struct SliderTrack(Channel);

#[derive(Component)]
struct ColourThumb(Channel);

/// A preset swatch and the colour it sets.
#[derive(Component, Clone, Copy)]
struct Preset(Hsva);

/// A button on the panel and what pressing it does.
#[derive(Component, Clone, Copy)]
struct MarkerButton(Action);

/// A button's text, which a two-state button rewrites. Spawned as the feathers button's own
/// caption, so the widget still owns how it is laid out and coloured.
#[derive(Component, Default, Clone)]
struct Caption;

/// The line that says which folder a scatter would take its photographs from.
#[derive(Component)]
struct ScatterReadout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    /// Steps through what is drawn: nothing, the photographs on their own, the photographs
    /// with these controls. The key that cycled it has gone to the tabs, so the cycle is a
    /// button, captioned with what pressing it would show rather than with what is shown.
    Showing,
    /// Draws every tether or only the selected marker's: a two-state button, whose caption
    /// says what pressing it would do rather than what the state is, as a switch on a wall
    /// does not.
    Tethers,
    ClearPhoto,
    Remove,
    Save,
    /// The two testing ones: choose a folder of photographs, and fill the view from it.
    Folder,
    Scatter,
}

/// What the buttons can be judged against: whether there is a marker to work on, and whether it
/// has a photo to take off it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Readiness {
    selected: bool,
    has_photo: bool,
    can_save: bool,
    /// Whether a folder with something readable in it has been chosen, which is what a scatter
    /// needs and the only thing it needs.
    has_photos: bool,
}

impl Readiness {
    fn of(markers: &PhotoMarkers, scatter: &Scatter) -> Self {
        Self {
            selected: markers.selected.is_some(),
            has_photo: markers
                .selected()
                .is_some_and(|marker| marker.photo.is_some()),
            can_save: markers.can_save(),
            has_photos: scatter.ready(),
        }
    }
}

impl Action {
    pub(super) fn caption(self, markers: &PhotoMarkers) -> &'static str {
        match self {
            Self::Showing => markers.showing.next().name(),
            Self::Tethers if markers.all_tethers => "one tether",
            Self::Tethers => "all tethers",
            Self::ClearPhoto => "clear photo",
            Self::Remove => "remove",
            Self::Save => "save",
            Self::Folder => "folder...",
            Self::Scatter => "scatter",
        }
    }

    /// Whether the button can apply. The two edits want a marker, and clearing wants a photo on
    /// it; saving wants something unsaved and a file it may write over.
    fn enabled(self, readiness: Readiness) -> bool {
        match self {
            Self::Showing => true,
            Self::Tethers => true,
            Self::ClearPhoto => readiness.has_photo,
            Self::Remove => readiness.selected,
            Self::Save => readiness.can_save,
            Self::Folder => true,
            Self::Scatter => readiness.has_photos,
        }
    }
}

fn font() -> TextFont {
    TextFont {
        font_size: FontSize::Px(12.0),
        ..default()
    }
}

/// A row of controls, wrapping onto a second line when the panel is too narrow for them all.
fn row() -> Node {
    Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        align_items: AlignItems::Center,
        column_gap: Val::Px(MARGIN),
        row_gap: Val::Px(4.0),
        ..default()
    }
}

/// One channel's slider: the headless widget keeps the value and handles the pointer, while the
/// gradient track and the thumb are ours to draw. The thumb's parent is inset by the thumb's
/// width so its centre lands under the pointer at both ends, as the sheet grid's does.
fn spawn_slider(panel: &mut RelatedSpawnerCommands<ChildOf>, channel: Channel, colour: Hsva) {
    panel.spawn(row()).with_children(|line| {
        line.spawn((
            Text::new(channel.name()),
            font(),
            TextColor(Color::WHITE),
            Node {
                min_width: Val::Px(66.0),
                ..default()
            },
        ));

        line.spawn((
            Slider {
                track_click: TrackClick::Snap,
                ..default()
            },
            SliderValue(channel.of(colour)),
            channel.range(),
            Node {
                width: Val::Px(SLIDER_WIDTH),
                height: Val::Px(THUMB_SIZE),
                justify_content: JustifyContent::Center,
                flex_direction: FlexDirection::Column,
                ..default()
            },
            observe(
                move |change: On<ValueChange<f32>>,
                      mut markers: ResMut<PhotoMarkers>,
                      mut commands: Commands| {
                    let mut colour = markers.colour;
                    channel.set(&mut colour, change.value);
                    markers.colour = colour;
                    if let Some(marker) = markers.selected_mut() {
                        marker.colour = colour;
                    }
                    markers.dirty = true;
                    commands
                        .entity(change.source)
                        .insert(SliderValue(change.value));
                },
            ),
            children![
                (
                    SliderTrack(channel),
                    Node {
                        height: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundGradient(vec![
                        LinearGradient::to_right(channel.gradient(colour)).into()
                    ]),
                ),
                (
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        right: Val::Px(THUMB_SIZE),
                        top: Val::Px(0.0),
                        bottom: Val::Px(0.0),
                        ..default()
                    },
                    children![(
                        ColourThumb(channel),
                        SliderThumb,
                        Node {
                            position_type: PositionType::Absolute,
                            width: Val::Px(THUMB_SIZE),
                            height: Val::Px(THUMB_SIZE),
                            left: Val::Percent(0.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.85, 0.85, 0.85)),
                    )],
                ),
            ],
        ));
    });
}

/// Which of the two size sliders this is: how tall a marker is drawn, and the floor it is held to
/// when far away. They answer the same question from opposite ends, which is why they sit
/// together: a size that reads well overhead is a speck from orbit unless a floor catches it.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Size {
    Card,
    Corner,
    /// How heavy the selected marker is drawn, in every part of itself. The rest keep a fixed
    /// width, which is what makes this one mean anything.
    Selected,
    /// How many markers the scatter button makes. A testing affordance rather than a size, but
    /// it is a number on a track like the other two and there is no reason to build it twice.
    Count,
}

impl Size {
    fn name(self) -> &'static str {
        match self {
            Self::Card => "card",
            Self::Corner => "corner",
            Self::Selected => "selected",
            Self::Count => "how many",
        }
    }

    fn range(self) -> SliderRange {
        match self {
            Self::Card => SliderRange::new(MIN_CARD, MAX_CARD),
            Self::Corner => SliderRange::new(MIN_RADIUS, MAX_RADIUS),
            Self::Selected => SliderRange::new(MIN_WIDTH, MAX_WIDTH),
            Self::Count => SliderRange::new(MIN_COUNT, MAX_COUNT),
        }
    }

    fn of(self, markers: &PhotoMarkers) -> f32 {
        match self {
            Self::Card => markers.card_pixels,
            Self::Corner => markers.corner_radius,
            Self::Selected => markers.selected_width,
            Self::Count => markers.scatter_count,
        }
    }

    fn set(self, markers: &mut PhotoMarkers, value: f32) {
        match self {
            Self::Card => markers.card_pixels = value,
            Self::Corner => markers.corner_radius = value,
            Self::Selected => markers.selected_width = value,
            // Whole markers: the slider is continuous and a press makes a countable number.
            Self::Count => markers.scatter_count = value.round(),
        }
    }

    fn reading(self, markers: &PhotoMarkers) -> String {
        match self {
            Self::Card => format!("{:.0} px", markers.card_pixels),
            Self::Corner if markers.corner_radius < 0.5 => "square".to_string(),
            Self::Corner => format!("{:.0} px", markers.corner_radius),
            Self::Selected => format!("{:.1} px", markers.selected_width),
            Self::Count => format!("{:.0}", markers.scatter_count),
        }
    }
}

/// Which size a feathers slider sets, and the number beside it.
#[derive(Component)]
struct SizeSlider(Size);

#[derive(Component)]
struct SizeReading(Size);

/// One of the two size sliders. Plain, with no gradient: there is nothing to preview, and the
/// number beside it says where it stands.
/// Draws the markers as the Marker tab comes forward.
///
/// A tab of marker tools with nothing drawn is a page about nothing, and the cycle button at
/// the top of it says what is shown and puts it away again. Only on the way in, and only from
/// nothing: a tab clicked while the photographs are being looked at on their own leaves them
/// that way.
fn show_on_tab(
    tab: On<TabInFront>,
    mut markers: ResMut<PhotoMarkers>,
    editor: Option<ResMut<RailEditor>>,
) {
    if tab.0 != Tab::Marker || markers.showing != Showing::Nothing {
        return;
    }

    markers.showing = Showing::Editing;
    if let Some(mut editor) = editor {
        editor.editing = false;
    }
    info!("photo markers: {}", markers.showing.name());
}

/// Fills the panel's Marker tab the moment its body appears.
///
/// Keyed off the body rather than ordered against the tab panel's startup system, so that
/// neither plugin has to know when the other runs — the arrangement the camera buttons and the
/// F1 list already have. The body is spawned once and filled once; what changes afterwards is
/// written by the systems below rather than respawned.
fn fill_marker_tab(
    mut commands: Commands,
    bodies: Query<Entity, Added<MarkerTabBody>>,
    markers: Res<PhotoMarkers>,
) {
    for body in &bodies {
        // What a marker is drawn like on the left, what colour it is drawn in and what is
        // done to it on the right; see two_columns for why a long tab is read in two.
        let [left, right] = two_columns(&mut commands, body);

        spawn_actions(&mut commands, left, &markers, &[Action::Showing]);

        // The selection, and how the selected marker stands on screen: what the sizes below
        // are judged against.
        commands.spawn((
            Readout,
            Text::new(readout_text(None, 0)),
            ThemedText,
            ChildOf(left),
        ));
        commands
            .spawn_scene(label_dim(""))
            .insert((SizeReadout, ChildOf(left)));

        for size in [Size::Card, Size::Corner, Size::Selected] {
            spawn_size_row(&mut commands, left, size, &markers);
        }

        spawn_actions(&mut commands, left, &markers, &[Action::Tethers]);

        // The colour the sliders hold, as a square and as hex.
        let line = commands.spawn((row(), ChildOf(right))).id();
        commands.spawn((
            ColourSwatch,
            Node {
                width: Val::Px(SWATCH_SIZE),
                height: Val::Px(SWATCH_SIZE),
                ..default()
            },
            BackgroundColor(markers.colour.into()),
            ChildOf(line),
        ));
        commands.spawn((
            ColourHex,
            Text::new(hex_of(markers.colour)),
            ThemedText,
            ChildOf(line),
        ));

        commands.entity(right).with_children(|right| {
            for channel in [Channel::Hue, Channel::Saturation, Channel::Value] {
                spawn_slider(right, channel, markers.colour);
            }
        });

        // The presets, as swatches rather than captions: a colour names itself.
        let swatches = commands.spawn((row(), ChildOf(right))).id();
        for (_, preset) in PRESETS {
            commands.spawn((
                Preset(preset),
                Button,
                Node {
                    width: Val::Px(SWATCH_SIZE),
                    height: Val::Px(SWATCH_SIZE),
                    ..default()
                },
                BackgroundColor(preset.into()),
                ChildOf(swatches),
            ));
        }

        // The two testing affordances, and how many a scatter makes.
        spawn_size_row(&mut commands, right, Size::Count, &markers);
        commands
            .spawn_scene(label_dim(""))
            .insert((ScatterReadout, ChildOf(right)));
        spawn_actions(
            &mut commands,
            right,
            &markers,
            &[Action::Folder, Action::Scatter],
        );

        spawn_actions(
            &mut commands,
            right,
            &markers,
            &[Action::ClearPhoto, Action::Remove, Action::Save],
        );
    }
}

/// One size: its name, the reading of where it stands, and a feathers slider under them.
///
/// The slider draws its own number, and the reading beside the name is still here because it
/// says what the number means — pixels, whole markers, or "square" where a radius has reached
/// zero — which a bare value cannot.
fn spawn_size_row(commands: &mut Commands, body: Entity, size: Size, markers: &PhotoMarkers) {
    let line = commands.spawn((row(), ChildOf(body))).id();
    commands.spawn_scene(label_dim(size.name())).insert((
        Node {
            min_width: Val::Px(66.0),
            ..default()
        },
        ChildOf(line),
    ));
    commands.spawn((
        SizeReading(size),
        Text::new(size.reading(markers)),
        ThemedText,
        ChildOf(line),
    ));

    let range = size.range();
    let value = size.of(markers);

    commands
        .spawn_scene(bsn! {
            @FeathersSlider {
                @min: {range.start()},
                @max: {range.end()},
                @value: {value},
            }
        })
        .insert((SizeSlider(size), ChildOf(body)))
        // Whole markers for the count, as the old track's snap gave, and two decimals for the
        // width, which is the only one of the four read in fractions of a pixel.
        .insert_if(SliderStep(1.0), || size == Size::Count)
        .insert(SliderPrecision(match size {
            Size::Selected => 1,
            _ => 0,
        }))
        .observe(
            move |change: On<ValueChange<f32>>,
                  mut markers: ResMut<PhotoMarkers>,
                  mut commands: Commands| {
                size.set(&mut markers, change.value);
                commands
                    .entity(change.source)
                    .insert(SliderValue(change.value));
            },
        );
}

/// A row of feathers buttons, one per action.
///
/// Each carries the action it names and triggers [`Pressed`] when the widget says it was
/// activated, so the work itself stays in one observer however a press arrived — a click, or
/// Enter on a focused button, which the hand-drawn ones never answered.
fn spawn_actions(
    commands: &mut Commands,
    body: Entity,
    markers: &PhotoMarkers,
    actions: &[Action],
) {
    let line = commands.spawn((row(), ChildOf(body))).id();

    for &action in actions {
        commands
            .spawn_scene(bsn! {
                @FeathersButton {
                    @caption: bsn! { Text({action.caption(markers).to_string()}) ThemedText Caption },
                }
            })
            .insert((MarkerButton(action), ChildOf(line)))
            .observe(move |_: On<Activate>, mut commands: Commands| {
                commands.trigger(Pressed(action));
            });
    }
}

/// A press of one of the action buttons, which [`on_pressed`] answers.
#[derive(Event)]
struct Pressed(Action);

/// What each button does. One observer rather than a system over the buttons: the widgets say
/// when they were activated, so what is left is the work, and the work wants a good many
/// resources to be named once rather than per button.
///
/// The readiness is checked again here even though a button that cannot apply is given
/// `InteractionDisabled`, which stops the widget activating at all: the two are written in
/// different places and the cheap check keeps them from disagreeing.
#[allow(clippy::too_many_arguments)]
fn on_pressed(
    press: On<Pressed>,
    mut commands: Commands,
    mut markers: ResMut<PhotoMarkers>,
    scatter: Res<Scatter>,
    grids: big_space::prelude::Grids,
    camera: Query<super::MarkerCamera, With<bevy_terrain::prelude::OrbitalCameraController>>,
    mut cards: Query<&mut ImageNode>,
    editor: Option<ResMut<RailEditor>>,
) {
    let readiness = Readiness::of(&markers, &scatter);
    if !press.0.enabled(readiness) {
        return;
    }

    match press.0 {
        Action::Showing => {
            markers.showing = markers.showing.next();
            // Only the state that wants the keys stands the editor down. Coming back the other
            // way, to the photographs alone, gives them back without the editor having to ask.
            if let (true, Some(mut editor)) = (markers.showing.editing(), editor) {
                editor.editing = false;
            }
            info!("photo markers: {}", markers.showing.name());
        }
        Action::ClearPhoto => {
            if let Some(marker) = markers.selected_mut() {
                let card = marker.card;
                photo::clear_photo(&mut marker.photo, &mut marker.missing, &mut marker.aspect);
                super::card::clear_card(card, &mut cards);
                markers.dirty = true;
            }
        }
        Action::Remove => {
            if let Some(entity) = markers.remove_selected() {
                commands.entity(entity).despawn();
            }
        }
        Action::Tethers => {
            markers.all_tethers = !markers.all_tethers;
            info!(
                "photo markers: {}",
                match markers.all_tethers {
                    true => "every tether drawn",
                    false => "only the selected marker's tether",
                }
            );
        }
        Action::Save => file::save_markers(&mut markers),
        Action::Folder => scatter::choose_folder(&mut commands),
        Action::Scatter => {
            // Around whatever the camera is over at the moment of the press, so a scatter
            // fills the view you are looking at rather than one you were.
            let Some(position) = camera.single().ok().and_then(|camera| {
                let grid = grids.parent_grid(camera.entity)?;
                Some(grid.grid_position_double(camera.cell, camera.transform))
            }) else {
                return;
            };

            let (around, altitude) = scatter::over_the_camera(position);
            scatter::scatter_markers(&mut markers, &scatter, around, altitude, &mut commands);
        }
    }
}

/// Brings the Marker tab forward as the markers reach their editing state, and puts the panel
/// away again as they leave.
///
/// The old panel was visible exactly while that state was on, which it could be because the
/// panel was this plugin's. The F11 panel is shared, so the state change asks for the tab
/// rather than holding the panel open: F11 and the other two tabs go on working while markers
/// are being edited, and putting the tab away takes the panel with it only while the Marker tab
/// is the one in front.
fn open_marker_tab(
    markers: Res<PhotoMarkers>,
    mut commands: Commands,
    mut was_editing: Local<bool>,
) {
    let editing = markers.showing.editing();
    if editing != *was_editing {
        commands.trigger(match editing {
            true => ShowTab::Up(Tab::Marker),
            false => ShowTab::Down(Tab::Marker),
        });
    }

    *was_editing = editing;
}

/// Everything on screen that belongs to somebody else: a UI node with no parent, which is not
/// one of the markers' own cards.
type SomebodyElses<'world, 'state> = Query<
    'world,
    'state,
    (Entity, &'static mut Node, &'static mut Visibility),
    (Without<ChildOf>, Without<MarkerCard>),
>;

/// What one of those was before it was taken off the screen.
type AsItWas = (Entity, Display, Visibility);

/// Takes everything else off the screen while F10 is showing the photographs on their own.
///
/// Every root UI node that is not a card belongs to something else: the frame rate graph, the
/// VRAM readout, the trains table, the station and train names, this panel. The middle state
/// exists to look at the photographs, and a count of atlas slots lying across the top of one is
/// not part of looking at it. So the rule is the whole rule: if it is on screen and it is not a
/// photograph, it goes.
///
/// It is `display` that does the taking, and not only `Visibility`. The labels decide their own
/// visibility every frame, in PostUpdate as this does, and nothing ordered the two against each
/// other — so a station name hidden here was shown again by its own system on whichever frames
/// the scheduler happened to run it second, and the names stayed on screen. Nothing else writes
/// `display`, so a node given `Display::None` keeps it however the frame is ordered; bevy_ui
/// then lays it out to nothing, and both the node and its text are skipped for being empty.
///
/// The ordering is belt to that brace: after `UiSystems::Prepare`, which every one of those
/// label systems runs before, and ahead of the layout that reads what this writes.
///
/// Each is put back exactly as it was found. An overlay that has its own say about visibility
/// will have it again on the next frame; one that has none keeps what it is given here.
fn quieten_everything_else(
    markers: Res<PhotoMarkers>,
    mut quiet: Local<bool>,
    mut remembered: Local<Vec<AsItWas>>,
    mut nodes: SomebodyElses,
) {
    let wanted = markers.showing == Showing::Photos;

    match (wanted, *quiet) {
        // On the way in, what each was is written down before it is taken away.
        (true, false) => {
            remembered.clear();
            for (entity, mut node, mut visibility) in &mut nodes {
                remembered.push((entity, node.display, *visibility));
                node.display = Display::None;
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
        (true, true) => {
            for (_, mut node, mut visibility) in &mut nodes {
                if node.display != Display::None {
                    node.display = Display::None;
                }
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
        // And on the way out, back as they were.
        (false, true) => {
            for (entity, display, visibility) in remembered.drain(..) {
                if let Ok((_, mut node, mut was)) = nodes.get_mut(entity) {
                    node.display = display;
                    was.set_if_neq(visibility);
                }
            }
        }
        (false, false) => {}
    }

    *quiet = wanted;
}

/// A preset swatch sets the sliders, and with them the selected marker's colour.
fn press_swatches(
    mut markers: ResMut<PhotoMarkers>,
    swatches: Query<(&Preset, &Interaction), Changed<Interaction>>,
) {
    for (preset, interaction) in &swatches {
        if *interaction != Interaction::Pressed {
            continue;
        }

        markers.colour = preset.0;
        if let Some(marker) = markers.selected_mut() {
            marker.colour = preset.0;
        }
        markers.dirty = true;
    }
}

/// Hands a button that cannot apply to the widget as disabled, and takes it back when it can.
///
/// `InteractionDisabled` is what feathers reads to grey a button and what `bevy_ui_widgets`
/// reads to stop it activating, so one component does what the painting and the dimming used to
/// do between them. Every frame rather than on a change, because a button goes from able to
/// unable without the pointer moving, and the insert is skipped when it is already right.
fn enable_buttons(
    mut commands: Commands,
    markers: Res<PhotoMarkers>,
    scatter: Res<Scatter>,
    buttons: Query<(Entity, &MarkerButton, Has<InteractionDisabled>, &Children)>,
    mut captions: Query<&mut Text, With<Caption>>,
) {
    let readiness = Readiness::of(&markers, &scatter);

    for (entity, button, disabled, children) in &buttons {
        match (button.0.enabled(readiness), disabled) {
            (true, true) => {
                commands.entity(entity).remove::<InteractionDisabled>();
            }
            (false, false) => {
                commands.entity(entity).insert(InteractionDisabled);
            }
            _ => {}
        }

        // Written here rather than once at spawn, because a two-state button's caption is part
        // of the state and not part of the furniture.
        for child in children.iter() {
            if let Ok(mut caption) = captions.get_mut(child) {
                caption.set_if_neq(Text::new(button.0.caption(&markers)));
            }
        }
    }
}

/// Keeps the swatch and the hex beside the sliders showing the colour they hold.
///
/// Nothing else is painted from here. The plate the colour used to go on has gone, and
/// a card is the photograph and nothing else; the marker's own colour is drawn on its dot, and
/// on the tether once there is one, both of which read it straight off the marker each frame.
fn apply_colour(
    markers: Res<PhotoMarkers>,
    mut swatch: Single<&mut BackgroundColor, With<ColourSwatch>>,
    mut hex: Single<&mut Text, With<ColourHex>>,
) {
    let colour = markers.colour;
    swatch.set_if_neq(BackgroundColor(colour.into()));
    hex.set_if_neq(Text::new(hex_of(colour)));
}

/// Brings the sliders to what they set whenever it changes from under them: a marker selected
/// by a click, a preset pressed, or a file loaded. SliderValue is immutable, so it is reinserted
/// rather than written through, which is what the widget's own observer does too.
fn sync_sliders(
    markers: Res<PhotoMarkers>,
    mut commands: Commands,
    sliders: Query<(Entity, &SliderValue, &ChildOf), With<Slider>>,
    tracks: Query<&SliderTrack>,
    children: Query<&Children>,
    sizes: Query<(Entity, &SizeSlider, &SliderValue)>,
) {
    if !markers.is_changed() {
        return;
    }
    let colour = markers.colour;

    // The feathers sliders say which size they are outright, so there is no track to read it
    // from and no gradient to repaint afterwards.
    for (entity, slider, value) in &sizes {
        let wanted = slider.0.of(&markers);
        if (value.0 - wanted).abs() > f32::EPSILON {
            commands.entity(entity).insert(SliderValue(wanted));
        }
    }

    for (entity, value, _) in &sliders {
        // Which channel this slider is, from the track it holds.
        let Some(channel) = children
            .get(entity)
            .ok()
            .and_then(|kids| kids.iter().find_map(|kid| tracks.get(kid).ok()))
            .map(|track| track.0)
        else {
            continue;
        };

        let wanted = channel.of(colour);
        if (value.0 - wanted).abs() > f32::EPSILON {
            commands.entity(entity).insert(SliderValue(wanted));
        }
    }
}

/// Moves each thumb to its channel's value and repaints the gradient under it, since the
/// saturation and value strips are drawn from the other two channels and change as they do. The
/// colour is the truth here, not the widgets: sync_sliders has already brought their values to
/// it, and reading it directly keeps each thumb on its own channel.
fn style_sliders(
    markers: Res<PhotoMarkers>,
    mut thumbs: Query<(&ColourThumb, &mut Node)>,
    mut tracks: Query<(&SliderTrack, &mut BackgroundGradient)>,
) {
    let colour = markers.colour;

    for (track, mut gradient) in &mut tracks {
        *gradient = BackgroundGradient(vec![
            LinearGradient::to_right(track.0.gradient(colour)).into(),
        ]);
    }

    for (thumb, mut node) in &mut thumbs {
        node.left = Val::Percent(thumb.0.fraction(colour) * 100.0);
    }
}

/// Writes the readout: which marker of how many, where it is, and what is on its screen.
fn update_readout(markers: Res<PhotoMarkers>, mut readout: Single<&mut Text, With<Readout>>) {
    let text = readout_text(markers.selected, markers.markers.len());
    let text = match markers.selected() {
        Some(marker) if marker.riding.is_some() => {
            format!("{text}  riding a train  {}", photo_text(marker))
        }
        Some(marker) => format!(
            "{text}  {:.5}, {:.5}  ground {:.0} m  {}",
            marker.latitude(),
            marker.longitude(),
            marker.ground,
            photo_text(marker)
        ),
        None => text,
    };

    readout.set_if_neq(Text::new(text));
}

/// What the readout says about a marker's photo. Named either way, so that one whose file has
/// moved can be put back rather than guessed at: the path is kept on the marker and saved with it.
fn photo_text(marker: &super::PhotoMarker) -> String {
    match (&marker.photo, marker.missing) {
        (Some(path), true) => format!(
            "{} is missing",
            ascii(&path.file_name().unwrap_or_default().to_string_lossy())
        ),
        (Some(path), false) => ascii(&path.file_name().unwrap_or_default().to_string_lossy()),
        (None, _) => "no photo".to_string(),
    }
}

/// The first part of the readout: which marker of how many, or what to do when there is none.
pub(super) fn readout_text(selected: Option<usize>, total: usize) -> String {
    match (selected, total) {
        (Some(index), total) => format!("marker {} of {total}", index + 1),
        (None, 0) => "no markers: Ctrl+click the terrain to place one".to_string(),
        (None, total) => format!("{total} markers, none selected"),
    }
}

/// The colour as it would be written in CSS, for the readout beside the swatch. to_hex brings
/// its own hash, and leaves the alpha off when it is opaque, which every marker's colour is.
pub(super) fn hex_of(colour: Hsva) -> String {
    Srgba::from(Color::from(colour)).to_hex()
}

/// The line under the readout: how the selected marker stands on screen at this moment. This is
/// what a size is judged against, so it is on the panel beside the slider that sets it.
fn update_size_readout(
    markers: Res<PhotoMarkers>,
    view: Res<MarkerView>,
    mut readout: Single<&mut Text, With<SizeReadout>>,
    mut readings: Query<(&SizeReading, &mut Text), Without<SizeReadout>>,
) {
    for (reading, mut text) in &mut readings {
        text.set_if_neq(Text::new(reading.0.reading(&markers)));
    }

    readout.set_if_neq(Text::new(view_text(view.shown, view.total)));
}

/// Says which folder a scatter would take its photographs from.
fn update_scatter_readout(
    scatter: Res<Scatter>,
    mut readout: Single<&mut Text, With<ScatterReadout>>,
) {
    readout.set_if_neq(Text::new(scatter.caption()));
}

/// What the panel says about how many of the markers actually have a card on screen, which is
/// what tells you whether the card slider has gone too big: the layout drops the furthest when
/// there is no room left down the sides.
pub(super) fn view_text(shown: usize, total: usize) -> String {
    match (shown, total) {
        (_, 0) => "no markers yet: Ctrl+click the ground".to_string(),
        (0, total) => format!("none of {total} shown"),
        (shown, total) => format!("{shown} of {total} shown"),
    }
}
