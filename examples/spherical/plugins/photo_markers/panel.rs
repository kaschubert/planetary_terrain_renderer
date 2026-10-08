//! The F10 panel: which marker is selected, what colour it is, and how its card is drawn.
//!
//! Bottom right, in the F5 panel's style, and F10 stands the rail editor down as F5 stands this
//! down, so the corner, Delete and Escape have one owner at a time. The buttons are Bevy's
//! Button with its Interaction, the pattern of the editor's panel and the overlay's copy button:
//! one component names the action, one system acts on a press, one colours hover and press, and
//! one dims what cannot apply.
//!
//! The colour is hue, saturation and value on three sliders, the headless slider of the sheet
//! grid's height control with a gradient under each track so a drag's effect can be seen before
//! it is made. The marker holds those three numbers rather than a colour, because a very light
//! grey, which is the default, has no hue to read back: deriving the sliders from a colour would
//! send the hue to zero the moment the saturation reached it, and the hue slider would seem dead.
//! The presets beside them are shortcuts, not the only colours: the sliders reach all of them.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::ui_widgets::{
    Slider, SliderRange, SliderThumb, SliderValue, TrackClick, ValueChange, observe,
};
use bevy::{prelude::*, text::FontSize};

use super::card::{CARD_PIXELS, CORNER_RADIUS, MAX_CARD, MAX_RADIUS, MIN_CARD, MIN_RADIUS};
use super::{DEFAULT_COLOUR, MarkerView, PhotoMarkerSystems, PhotoMarkers, file, photo};
use crate::plugins::provenance::ascii;
use crate::plugins::rail_editor::RailEditor;

/// The panel and everything on it, added by PhotoMarkersPlugin.
pub(super) struct PhotoPanelPlugin;

impl Plugin for PhotoPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_panel).add_systems(
            Update,
            (
                toggle_markers,
                show_panel,
                press_buttons,
                press_swatches,
                colour_buttons,
                dim_buttons,
                apply_colour,
                sync_sliders,
                style_sliders,
                style_size_sliders,
            )
                .chain(),
        );
        app.add_systems(
            PostUpdate,
            // After the mouse has had its say on the selection, so the readout is of the marker
            // clicked this frame rather than the one before it.
            // After the placing too, which is what measures the marker on screen.
            (update_readout, update_size_readout).after(PhotoMarkerSystems),
        );
    }
}

/// The panel's width, in pixels. Fixed rather than fitted, so its edge does not wander as the
/// readout changes. Wide enough for the readout's sentence at the 12 px font.
const PANEL_WIDTH: f32 = 360.0;

/// The gap between the window's edge and the panel, as the F5 panel and the F2 table keep.
const MARGIN: f32 = 6.0;

/// The buttons' colours, the editor panel's: the panel is black at half alpha, so a button has
/// to be lighter than that to read as one, and hover and press lighten it further.
const BUTTON_COLOUR: Color = Color::srgba(0.25, 0.25, 0.25, 0.8);
const HOVER_COLOUR: Color = Color::srgba(0.4, 0.4, 0.4, 0.9);
const PRESSED_COLOUR: Color = Color::srgba(0.6, 0.6, 0.6, 0.9);
const DISABLED_COLOUR: Color = Color::srgba(0.12, 0.12, 0.12, 0.6);
const DISABLED_TEXT: Color = Color::srgb(0.45, 0.45, 0.45);

/// The slider track's width and the thumb's size, in pixels, as the sheet grid's.
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

/// The panel itself, shown while marker mode is on.
#[derive(Component)]
pub(super) struct MarkerPanel;

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

/// A button's text, dimmed with the button.
#[derive(Component)]
struct Caption;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    ClearPhoto,
    Remove,
    Save,
}

/// What the buttons can be judged against: whether there is a marker to work on, and whether it
/// has a photo to take off it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Readiness {
    selected: bool,
    has_photo: bool,
    can_save: bool,
}

impl Readiness {
    fn of(markers: &PhotoMarkers) -> Self {
        Self {
            selected: markers.selected.is_some(),
            has_photo: markers
                .selected()
                .is_some_and(|marker| marker.photo.is_some()),
            can_save: markers.can_save(),
        }
    }
}

impl Action {
    fn caption(self) -> &'static str {
        match self {
            Self::ClearPhoto => "clear photo",
            Self::Remove => "remove",
            Self::Save => "save",
        }
    }

    /// Whether the button can apply. The two edits want a marker, and clearing wants a photo on
    /// it; saving wants something unsaved and a file it may write over.
    fn enabled(self, readiness: Readiness) -> bool {
        match self {
            Self::ClearPhoto => readiness.has_photo,
            Self::Remove => readiness.selected,
            Self::Save => readiness.can_save,
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

fn spawn_button(row: &mut RelatedSpawnerCommands<ChildOf>, action: Action) {
    row.spawn((
        MarkerButton(action),
        Button,
        Node {
            padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
            ..default()
        },
        BackgroundColor(BUTTON_COLOUR),
        children![(
            Caption,
            Text::new(action.caption()),
            font(),
            TextColor(Color::WHITE)
        )],
    ));
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
}

impl Size {
    fn name(self) -> &'static str {
        match self {
            Self::Card => "card",
            Self::Corner => "corner",
        }
    }

    fn range(self) -> SliderRange {
        match self {
            Self::Card => SliderRange::new(MIN_CARD, MAX_CARD),
            Self::Corner => SliderRange::new(MIN_RADIUS, MAX_RADIUS),
        }
    }

    fn of(self, markers: &PhotoMarkers) -> f32 {
        match self {
            Self::Card => markers.card_pixels,
            Self::Corner => markers.corner_radius,
        }
    }

    fn set(self, markers: &mut PhotoMarkers, value: f32) {
        match self {
            Self::Card => markers.card_pixels = value,
            Self::Corner => markers.corner_radius = value,
        }
    }

    /// How far along its track the value stands, 0 to 1, which is where its thumb goes.
    fn fraction(self, markers: &PhotoMarkers) -> f32 {
        let range = self.range();
        let (start, end) = (range.start(), range.end());

        ((self.of(markers) - start) / (end - start)).clamp(0.0, 1.0)
    }

    fn reading(self, markers: &PhotoMarkers) -> String {
        match self {
            Self::Card => format!("{:.0} px", markers.card_pixels),
            Self::Corner if markers.corner_radius < 0.5 => "square".to_string(),
            Self::Corner => format!("{:.0} px", markers.corner_radius),
        }
    }
}

/// A size slider's thumb, and the number beside it.
#[derive(Component)]
struct SizeThumb(Size);

#[derive(Component)]
struct SizeReading(Size);

/// One of the two size sliders. Plain, with no gradient: there is nothing to preview, and the
/// number beside it says where it stands.
fn spawn_size_slider(panel: &mut RelatedSpawnerCommands<ChildOf>, size: Size, value: f32) {
    panel.spawn(row()).with_children(|line| {
        line.spawn((
            Text::new(size.name()),
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
            SliderValue(value),
            size.range(),
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
                    size.set(&mut markers, change.value);
                    commands
                        .entity(change.source)
                        .insert(SliderValue(change.value));
                },
            ),
            children![
                (
                    Node {
                        height: Val::Px(6.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.35, 0.35, 0.35)),
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
                        SizeThumb(size),
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

        line.spawn((
            SizeReading(size),
            Text::new(String::new()),
            font(),
            TextColor(Color::WHITE),
            Node {
                min_width: Val::Px(52.0),
                ..default()
            },
        ));
    });
}

fn spawn_panel(mut commands: Commands) {
    let colour = DEFAULT_COLOUR;
    let (card, corner) = (CARD_PIXELS, CORNER_RADIUS);

    commands
        .spawn((
            MarkerPanel,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(MARGIN),
                right: Val::Px(MARGIN),
                width: Val::Px(PANEL_WIDTH),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                padding: UiRect::all(Val::Px(MARGIN)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            panel.spawn((
                Readout,
                Text::new(readout_text(None, 0)),
                font(),
                TextColor(Color::WHITE),
            ));

            // How the selected marker stands on screen right now, which is what the size is
            // being judged against and what the sample button writes down.
            panel.spawn((
                SizeReadout,
                Text::new(String::new()),
                font(),
                TextColor(Color::srgb(0.7, 0.78, 0.82)),
            ));

            spawn_size_slider(panel, Size::Card, card);
            spawn_size_slider(panel, Size::Corner, corner);

            panel.spawn(row()).with_children(|line| {
                line.spawn((
                    ColourSwatch,
                    Node {
                        width: Val::Px(SWATCH_SIZE),
                        height: Val::Px(SWATCH_SIZE),
                        ..default()
                    },
                    BackgroundColor(colour.into()),
                ));
                line.spawn((
                    ColourHex,
                    Text::new(hex_of(colour)),
                    font(),
                    TextColor(Color::WHITE),
                ));
            });

            for channel in [Channel::Hue, Channel::Saturation, Channel::Value] {
                spawn_slider(panel, channel, colour);
            }

            panel.spawn(row()).with_children(|swatches| {
                for (_, preset) in PRESETS {
                    swatches.spawn((
                        Preset(preset),
                        Button,
                        Node {
                            width: Val::Px(SWATCH_SIZE),
                            height: Val::Px(SWATCH_SIZE),
                            ..default()
                        },
                        BackgroundColor(preset.into()),
                    ));
                }
            });

            panel.spawn(row()).with_children(|actions| {
                for action in [Action::ClearPhoto, Action::Remove, Action::Save] {
                    spawn_button(actions, action);
                }
            });
        });
}

/// F10 turns marker mode on and off, and stands the rail editor down as it comes on: both panels
/// want the bottom right corner, and both want Delete and Escape. The panel follows in
/// show_panel, so the key is read once and the two cannot drift apart.
fn toggle_markers(
    keys: Res<ButtonInput<KeyCode>>,
    mut markers: ResMut<PhotoMarkers>,
    editor: Option<ResMut<RailEditor>>,
) {
    if !keys.just_pressed(KeyCode::F10) {
        return;
    }

    markers.editing = !markers.editing;
    if let (true, Some(mut editor)) = (markers.editing, editor) {
        editor.editing = false;
    }
    info!(
        "photo markers: {}",
        if markers.editing { "on" } else { "off" }
    );
}

/// The panel is visible exactly while marker mode is on.
fn show_panel(markers: Res<PhotoMarkers>, mut panel: Single<&mut Visibility, With<MarkerPanel>>) {
    let visibility = if markers.editing {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    panel.set_if_neq(visibility);
}

fn press_buttons(
    mut commands: Commands,
    mut markers: ResMut<PhotoMarkers>,
    mut cards: Query<&mut ImageNode>,
    buttons: Query<(&MarkerButton, &Interaction), Changed<Interaction>>,
) {
    let readiness = Readiness::of(&markers);

    for (button, interaction) in &buttons {
        if *interaction != Interaction::Pressed || !button.0.enabled(readiness) {
            continue;
        }

        match button.0 {
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
            Action::Save => file::save_markers(&mut markers),
        }
    }
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

fn button_colour(interaction: Interaction, enabled: bool) -> Color {
    match (enabled, interaction) {
        (false, _) => DISABLED_COLOUR,
        (true, Interaction::Pressed) => PRESSED_COLOUR,
        (true, Interaction::Hovered) => HOVER_COLOUR,
        (true, Interaction::None) => BUTTON_COLOUR,
    }
}

/// Gives the buttons their hover and press states. Every frame rather than on a change of
/// Interaction, because a button goes from able to unable without the pointer moving; the write
/// is skipped when the colour is already right.
fn colour_buttons(
    markers: Res<PhotoMarkers>,
    mut buttons: Query<(&MarkerButton, &Interaction, &mut BackgroundColor)>,
) {
    let readiness = Readiness::of(&markers);

    for (button, interaction, mut background) in &mut buttons {
        let colour = button_colour(*interaction, button.0.enabled(readiness));
        background.set_if_neq(BackgroundColor(colour));
    }
}

/// Greys the caption of a button that cannot apply, and whitens it again when it can.
fn dim_buttons(
    markers: Res<PhotoMarkers>,
    buttons: Query<&MarkerButton>,
    mut captions: Query<(&ChildOf, &mut TextColor), With<Caption>>,
) {
    let readiness = Readiness::of(&markers);

    for (child_of, mut colour) in &mut captions {
        let Ok(button) = buttons.get(child_of.parent()) else {
            continue;
        };
        let text = if button.0.enabled(readiness) {
            Color::WHITE
        } else {
            DISABLED_TEXT
        };
        colour.set_if_neq(TextColor(text));
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

/// Brings the sliders to the colour whenever it changes from under them: a marker selected by a
/// click, a preset pressed, or a file loaded. SliderValue is immutable, so it is reinserted
/// rather than written through, which is what the widget's own observer does too.
fn sync_sliders(
    markers: Res<PhotoMarkers>,
    mut commands: Commands,
    sliders: Query<(Entity, &SliderValue, &ChildOf), With<Slider>>,
    tracks: Query<&SliderTrack>,
    children: Query<&Children>,
) {
    if !markers.is_changed() {
        return;
    }
    let colour = markers.colour;

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

/// Moves each size thumb to its value. The resource is the truth, as it is for the colour.
fn style_size_sliders(markers: Res<PhotoMarkers>, mut thumbs: Query<(&SizeThumb, &mut Node)>) {
    for (thumb, mut node) in &mut thumbs {
        node.left = Val::Percent(thumb.0.fraction(&markers) * 100.0);
    }
}
