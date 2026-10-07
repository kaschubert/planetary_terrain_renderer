//! The F5 panel: what the selected point is, and buttons for the edits the keys have no room
//! for.
//!
//! The debug plugin has nearly every letter, so beyond Delete, Escape and the Control chords
//! the editor's operations are buttons. The panel sits in the bottom right corner, clear of
//! the F2 table above it whose height varies, and shows while editing is on: F5 flips
//! editor.editing and the panel follows it, so the two cannot disagree. Its rows are a
//! readout of the anchor, the point clicked last; the three modes; the actions; and, when the
//! startup sampling found the ground moved under points since the file was saved, a row that
//! steps the camera through them. Saving is here too, from the button and from Ctrl+S, with a
//! toast beside the panel saying how it went.
//!
//! The buttons are Bevy's Button with its Interaction, the pattern of the copy button in
//! vram_usage.rs: one component names the action, one system acts on a press, one colours
//! hover and press, and one dims the buttons that cannot apply. What is shown is put into
//! words by pure functions, which the tests check.

use super::{MoveDrag, RailEditor, RailEditorSystems};
use crate::plugins::auckland_rail::{
    AucklandRail, RAIL_PATH, RailSampler, WARNING_COLOUR, resolve_rail_heights,
};
use crate::plugins::provenance::ascii;
use crate::plugins::shared::rail_network::{MAX_GRADE, PointMode};
use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::{prelude::*, text::FontSize};
use bevy_terrain::prelude::*;
use big_space::prelude::{CellCoord, Grids};
use fly_to::{fly_to, step_moved};
use readout::{anchor_mode, moved_ground_text, readout_text};

mod fly_to;
mod readout;

/// The panel and everything on it, added by RailEditorPlugin.
pub(super) struct RailPanelPlugin;

impl Plugin for RailPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MovedCursor>()
            .add_systems(Startup, spawn_panel)
            .add_systems(
                Update,
                (
                    toggle_editing,
                    show_panel,
                    save_on_ctrl_s,
                    press_buttons,
                    colour_buttons,
                    dim_buttons,
                    expire_toasts,
                )
                    .chain()
                    // Before the positions are recomputed, so an edit made with a button is
                    // drawn the frame it is made, as a drag is.
                    .before(resolve_rail_heights),
            )
            .add_systems(
                PostUpdate,
                // After the mouse has had its say on the anchor, so the readout is of the
                // point clicked this frame.
                update_readout.after(RailEditorSystems),
            );
    }
}

/// The panel's width, in pixels. Fixed rather than fitted to its text, so its edge does not
/// wander as the readout changes and the toast knows where to sit beside it. Wide enough for
/// the moved-ground sentence and its two buttons on one line at the 12 px font.
const PANEL_WIDTH: f32 = 520.0;

/// The gap between the window's edge and the panel, and between the panel and the toast, in
/// pixels; the F2 table keeps the same from its corner.
const MARGIN: f32 = 6.0;

/// How long a toast stays, in seconds: long enough to read a count, and twice that for an
/// error, which says what to do about it.
const TOAST_SECONDS: f32 = 3.0;
const ERROR_TOAST_SECONDS: f32 = 6.0;

/// The buttons' colours. The panel is black at half alpha, so a button has to be lighter than
/// that to be seen as one; hover and press lighten it further, as the copy button does.
const BUTTON_COLOUR: Color = Color::srgba(0.25, 0.25, 0.25, 0.8);
const HOVER_COLOUR: Color = Color::srgba(0.4, 0.4, 0.4, 0.9);
const PRESSED_COLOUR: Color = Color::srgba(0.6, 0.6, 0.6, 0.9);

/// The mode button of the anchor's own mode, so the row reads as a state as well as three
/// actions. A blue no line, disc or warning is coloured in.
const CURRENT_COLOUR: Color = Color::srgba(0.2, 0.42, 0.65, 0.9);

/// A button that cannot apply: all but the panel's own black, with its caption greyed, and
/// neither hover nor press lights it.
const DISABLED_COLOUR: Color = Color::srgba(0.12, 0.12, 0.12, 0.6);
const DISABLED_TEXT: Color = Color::srgb(0.45, 0.45, 0.45);

/// The panel itself, shown while editing is on.
#[derive(Component)]
struct EditorPanel;

/// The text describing the anchor.
#[derive(Component)]
struct Readout;

/// The row about the ground having moved, laid out only while there is something to say.
#[derive(Component)]
struct MovedRow;

#[derive(Component)]
struct MovedText;

/// A button on the panel and what pressing it does.
#[derive(Component, Clone, Copy)]
struct EditorButton(Action);

/// A button's text, dimmed with the button.
#[derive(Component)]
struct Caption;

/// What a save said, gone again once the timer runs out.
#[derive(Component)]
struct SaveToast(Timer);

/// Which of the moved points the camera was last flown to, so prev and next step on from it.
#[derive(Resource, Default)]
struct MovedCursor(Option<usize>);

/// What a button does. The plan's one button, "span the steep run", is two here, select
/// steep run and then span selection, on purpose: the run finder stops at a flat top, see
/// RailEditor::select_steep_run, so what it selects is worth a look before it is spanned, and
/// a span is as often wanted on a selection made by hand, a portal clicked on each flank, as
/// on a run it found. Three clicks a tunnel either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Mode(PointMode),
    DropToGround,
    SpanSelection,
    SelectSteepRun,
    Undo,
    Redo,
    Save,
    PreviousMoved,
    NextMoved,
}

/// What the buttons can be judged against: whether there is a selection to work on, and on
/// one line the three points a span needs, an edit to undo or redo, a file that may be
/// written, and whether F4 has hidden the lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Readiness {
    selection_empty: bool,
    can_span: bool,
    undo_empty: bool,
    redo_empty: bool,
    can_save: bool,
    lines_hidden: bool,
}

impl Readiness {
    fn of(editor: &RailEditor, rail: &AucklandRail) -> Self {
        Self {
            selection_empty: editor.selection.is_empty(),
            can_span: editor.can_span(&rail.network),
            undo_empty: !editor.can_undo(),
            redo_empty: !editor.can_redo(),
            can_save: rail.can_save(),
            lines_hidden: !rail.visible,
        }
    }
}

impl Action {
    fn caption(self) -> &'static str {
        match self {
            Self::Mode(mode) => mode.name(),
            Self::DropToGround => "drop to ground",
            Self::SpanSelection => "span selection",
            Self::SelectSteepRun => "select steep run",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Save => "save",
            Self::PreviousMoved => "prev",
            Self::NextMoved => "next",
        }
    }

    /// Whether the button can apply. The edits need something selected, and a span three
    /// points of it on one line; undo and redo need something on their stack, and save a
    /// file it may write over. The moved-ground buttons are always ready, since their row is
    /// only laid out while there are moved points to step through. While F4 hides the lines
    /// every button but save is dim, as the mouse, the discs and the handle are inert then: an
    /// edit to lines that are not drawn would be a surprise, and a fly-to would show nothing.
    /// Save is of the file and not of the view.
    fn enabled(self, readiness: Readiness) -> bool {
        if readiness.lines_hidden && self != Self::Save {
            return false;
        }

        match self {
            Self::Mode(_) | Self::DropToGround | Self::SelectSteepRun => !readiness.selection_empty,
            Self::SpanSelection => readiness.can_span,
            Self::Undo => !readiness.undo_empty,
            Self::Redo => !readiness.redo_empty,
            Self::Save => readiness.can_save,
            Self::PreviousMoved | Self::NextMoved => true,
        }
    }
}

/// Saves the network and says how it went: a toast by the panel either way, and for a failure
/// the log too, since a toast is gone in seconds and the error says what to do. One toast at a
/// time; a second save while one shows replaces it.
fn save_network(
    commands: &mut Commands,
    rail: &AucklandRail,
    toasts: &mut Query<(&mut SaveToast, &mut Text, &mut TextColor)>,
) {
    let (message, seconds, colour) = match rail.save() {
        Ok(()) => {
            let count = rail.network.point_count();
            info!("auckland rail: saved {count} points to {RAIL_PATH}");
            (format!("saved {count} points"), TOAST_SECONDS, Color::WHITE)
        }
        Err(error) => {
            error!("auckland rail: save failed: {error}");
            // In the red the steep stretches draw in, the one warning colour there is.
            (
                ascii(&format!("save failed: {error}")),
                ERROR_TOAST_SECONDS,
                WARNING_COLOUR,
            )
        }
    };

    if let Ok((mut toast, mut text, mut text_colour)) = toasts.single_mut() {
        toast.0 = Timer::from_seconds(seconds, TimerMode::Once);
        text.0 = message;
        text_colour.0 = colour;
        return;
    }

    commands.spawn((
        SaveToast(Timer::from_seconds(seconds, TimerMode::Once)),
        Text::new(message),
        font(),
        TextColor(colour),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(MARGIN),
            // Just left of the panel, whether or not the panel is showing.
            right: Val::Px(2.0 * MARGIN + PANEL_WIDTH),
            // An error names the file by its path; this keeps it to a few lines.
            max_width: Val::Px(360.0),
            padding: UiRect::all(Val::Px(MARGIN)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
    ));
}

fn font() -> TextFont {
    TextFont {
        font_size: FontSize::Px(12.0),
        ..default()
    }
}

fn spawn_button(row: &mut RelatedSpawnerCommands<ChildOf>, action: Action) {
    row.spawn((
        EditorButton(action),
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

/// A row of buttons, wrapping onto a second line when the panel is too narrow for them all.
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

fn spawn_panel(mut commands: Commands) {
    commands
        .spawn((
            EditorPanel,
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
                Text::new("no point selected"),
                font(),
                TextColor(Color::WHITE),
            ));

            panel.spawn(row()).with_children(|modes| {
                for mode in [PointMode::Ground, PointMode::Fixed, PointMode::Between] {
                    spawn_button(modes, Action::Mode(mode));
                }
            });

            panel.spawn(row()).with_children(|actions| {
                for action in [
                    Action::DropToGround,
                    Action::SpanSelection,
                    Action::SelectSteepRun,
                    Action::Undo,
                    Action::Redo,
                    Action::Save,
                ] {
                    spawn_button(actions, action);
                }
            });

            // Laid out only while there is something to report, see update_readout.
            let mut moved = row();
            moved.display = Display::None;
            panel.spawn((MovedRow, moved)).with_children(|moved| {
                moved.spawn((
                    MovedText,
                    Text::new(moved_ground_text(0, None)),
                    font(),
                    TextColor(Color::WHITE),
                ));
                spawn_button(moved, Action::PreviousMoved);
                spawn_button(moved, Action::NextMoved);
            });
        });
}

/// F5 turns editing on and off. The panel follows in show_panel, so the key is read once and
/// the two cannot drift apart.
fn toggle_editing(keys: Res<ButtonInput<KeyCode>>, mut editor: ResMut<RailEditor>) {
    if keys.just_pressed(KeyCode::F5) {
        editor.editing = !editor.editing;
        info!(
            "rail editor: editing {}",
            if editor.editing { "on" } else { "off" }
        );
    }
}

/// The panel is visible exactly while editing is on.
fn show_panel(editor: Res<RailEditor>, mut panel: Single<&mut Visibility, With<EditorPanel>>) {
    let visibility = if editor.editing {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    panel.set_if_neq(visibility);
}

/// Ctrl+S, either Control key, whether or not the editor is on: the save is of the lines and
/// not of the editing of them, and the toast shows beside where the panel would be.
fn save_on_ctrl_s(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    rail: Res<AucklandRail>,
    mut toasts: Query<(&mut SaveToast, &mut Text, &mut TextColor)>,
) {
    let control = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if control && keys.just_pressed(KeyCode::KeyS) {
        save_network(&mut commands, &rail, &mut toasts);
    }
}

/// A press does what the button says, unless the button is disabled, in which case it is
/// inert as well as dim. The operations return whether they changed a point, and the caller
/// marks the network dirty when they did, as for every edit. Undo, redo and a fly-to wait for
/// a drag to end, as the keys do: mid-drag an undo would pop the drag's own snapshot, and a
/// camera moved into another cell ends the drag where it is.
#[allow(clippy::too_many_arguments)]
fn press_buttons(
    mut commands: Commands,
    buttons: Query<(&EditorButton, &Interaction), Changed<Interaction>>,
    mut editor: ResMut<RailEditor>,
    mut rail: ResMut<AucklandRail>,
    mut sampler: Option<ResMut<RailSampler>>,
    drag: Res<MoveDrag>,
    mut cursor: ResMut<MovedCursor>,
    mut toasts: Query<(&mut SaveToast, &mut Text, &mut TextColor)>,
    grids: Grids,
    mut camera: Query<(Entity, &mut Transform, &mut CellCoord), With<OrbitalCameraController>>,
) {
    for (button, interaction) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let action = button.0;
        if !action.enabled(Readiness::of(&editor, &rail)) {
            continue;
        }

        match action {
            Action::Mode(mode) => {
                if editor.set_mode(&mut rail.network, mode) {
                    rail.dirty = true;
                    info!(
                        "rail editor: {} points made {}",
                        editor.selection.len(),
                        mode.name()
                    );
                }
            }
            Action::DropToGround => {
                if editor.drop_to_ground(&mut rail.network) {
                    rail.dirty = true;
                    info!(
                        "rail editor: {} points dropped to the ground",
                        editor.selection.len()
                    );
                }
            }
            Action::SpanSelection => {
                if editor.span_selection(&mut rail.network) {
                    rail.dirty = true;
                    info!("rail editor: spanned the selection");
                }
            }
            Action::SelectSteepRun => {
                let added = editor.select_steep_run(&rail.network, MAX_GRADE);
                info!("rail editor: {added} points added along the steep ground");
            }
            Action::Undo | Action::Redo => {
                if drag.in_progress() {
                    continue;
                }
                let (done, what) = if action == Action::Undo {
                    (
                        editor.undo(&mut rail.network, sampler.as_deref_mut()),
                        "undone",
                    )
                } else {
                    (
                        editor.redo(&mut rail.network, sampler.as_deref_mut()),
                        "redone",
                    )
                };
                if done {
                    rail.dirty = true;
                    info!("rail editor: {what}");
                }
            }
            Action::Save => save_network(&mut commands, &rail, &mut toasts),
            Action::PreviousMoved | Action::NextMoved => {
                if drag.in_progress() {
                    continue;
                }
                let target = step_moved(
                    action == Action::NextMoved,
                    &mut cursor.0,
                    &rail.moved_ground,
                    &rail.network,
                    &mut editor,
                );
                match target {
                    Some(target) => fly_to(target, &grids, &mut camera),
                    None => warn!(
                        "rail editor: moved point {} of {} has been moved or removed since the \
                         startup sampling listed it",
                        cursor.0.map_or(0, |at| at + 1),
                        rail.moved_ground.len()
                    ),
                }
            }
        }
    }
}

/// A button's background by what the pointer is doing to it, whether it can apply, and
/// whether it is the mode the anchor has.
fn button_colour(interaction: Interaction, enabled: bool, current: bool) -> Color {
    match (enabled, interaction) {
        (false, _) => DISABLED_COLOUR,
        (true, Interaction::Pressed) => PRESSED_COLOUR,
        (true, Interaction::Hovered) => HOVER_COLOUR,
        (true, Interaction::None) if current => CURRENT_COLOUR,
        (true, Interaction::None) => BUTTON_COLOUR,
    }
}

/// Gives the buttons their hover and press states, and the anchor's mode its highlight. Every
/// frame rather than on a change of Interaction, because a button goes from able to unable,
/// and a mode from current to not, without the pointer moving; the write is skipped when the
/// colour is already right.
fn colour_buttons(
    editor: Res<RailEditor>,
    rail: Res<AucklandRail>,
    mut buttons: Query<(&EditorButton, &Interaction, &mut BackgroundColor)>,
) {
    let readiness = Readiness::of(&editor, &rail);
    let current = anchor_mode(&editor, &rail.network);

    for (button, interaction, mut background) in &mut buttons {
        let action = button.0;
        let is_current = matches!(action, Action::Mode(mode) if Some(mode) == current);
        let colour = button_colour(*interaction, action.enabled(readiness), is_current);
        background.set_if_neq(BackgroundColor(colour));
    }
}

/// Greys the caption of a button that cannot apply, and whitens it again when it can.
fn dim_buttons(
    editor: Res<RailEditor>,
    rail: Res<AucklandRail>,
    buttons: Query<&EditorButton>,
    mut captions: Query<(&ChildOf, &mut TextColor), With<Caption>>,
) {
    let readiness = Readiness::of(&editor, &rail);

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

/// Writes the readout and the moved-ground row. Not on a frame the network is dirty: the
/// positions are then a mutation behind the points, and though the readout reads the points
/// and not the positions, it would describe a point the discs and the lines do not yet show
/// where it is; the discs skip the frame, and so does this.
fn update_readout(
    editor: Res<RailEditor>,
    rail: Res<AucklandRail>,
    cursor: Res<MovedCursor>,
    mut readout: Query<&mut Text, With<Readout>>,
    mut moved_text: Query<&mut Text, (With<MovedText>, Without<Readout>)>,
    mut moved_row: Query<&mut Node, With<MovedRow>>,
) {
    if !editor.editing || rail.dirty {
        return;
    }

    let text = readout_text(&editor, &rail.network);
    for mut readout in &mut readout {
        if readout.0 != text {
            readout.0 = text.clone();
        }
    }

    let count = rail.moved_ground.len();
    let display = if count > 0 {
        Display::Flex
    } else {
        Display::None
    };
    for mut row in &mut moved_row {
        if row.display != display {
            row.display = display;
        }
    }
    if count > 0 {
        let text = moved_ground_text(count, cursor.0);
        for mut moved in &mut moved_text {
            if moved.0 != text {
                moved.0 = text.clone();
            }
        }
    }
}

fn expire_toasts(
    mut commands: Commands,
    time: Res<Time>,
    mut toasts: Query<(Entity, &mut SaveToast)>,
) {
    for (entity, mut toast) in &mut toasts {
        if toast.0.tick(time.delta()).is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests;
