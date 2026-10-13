//! The track's controls: what the selected point is, and buttons for the edits the keys have no
//! room for.
//!
//! The debug plugin has nearly every letter, so beyond Delete, Escape and the Control chords
//! the editor's operations are buttons. They were a panel of their own in the bottom right
//! corner, shown while editing was on; they are the AT trains tab's Tracks page now, under the
//! four switches over the network — the lines, the editing, the frames and the station names —
//! since the track
//! is one thing whether it is being looked at or worked on. There is nothing left for a panel's
//! visibility to follow: the switch that turns editing on is on the page with the buttons, and
//! `enable_buttons` hands them all to the widget as disabled while it is off.
//!
//! The page's rows are a readout of the anchor, the point clicked last; the three modes; the
//! actions; and, when the startup sampling found the ground moved under points since the file
//! was saved, a row that steps the camera through them. Saving is here too, from the button and
//! from Ctrl+S, with a toast in the bottom right corner — which the editor's own panel used to
//! hold — saying how it went.
//!
//! The buttons are feathers ones, which carry their own hover, press, focus and disabled
//! states: one component names the action, one observer does the work of a press, and
//! `InteractionDisabled` says what cannot apply, where a painting system and a dimming system
//! used to. What is shown is put into words by pure functions, which the tests check.

use super::{MoveDrag, RailEditor, RailEditorSystems};
use crate::plugins::auckland_rail::{
    AucklandRail, RAIL_PATH, RailSampler, WARNING_COLOUR, resolve_rail_heights,
};
use crate::plugins::provenance::ascii;
use crate::plugins::shared::rail_network::{MAX_GRADE, PointMode};
use crate::plugins::stations::RailStations;
use crate::plugins::tab_panel::{SubTab, SubTabBody};
use crate::plugins::track_frames::TrackFrames;
use bevy::feathers::controls::{ButtonVariant, FeathersButton, FeathersCheckbox};
use bevy::feathers::theme::ThemedText;
use bevy::ui::{Checked, InteractionDisabled};
use bevy::ui_widgets::{Activate, ValueChange};
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
            .add_observer(on_pressed)
            .add_systems(
                Update,
                (
                    fill_tracks_page,
                    save_on_ctrl_s,
                    sync_boxes,
                    enable_buttons,
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

/// The gap between the window's edge and the panel, and between the panel and the toast, in
/// pixels; the F2 table keeps the same from its corner.
const MARGIN: f32 = 6.0;

/// How long a toast stays, in seconds: long enough to read a count, and twice that for an
/// error, which says what to do about it.
const TOAST_SECONDS: f32 = 3.0;
const ERROR_TOAST_SECONDS: f32 = 6.0;

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

/// A button's text. Spawned as the feathers button's own caption, so the widget owns how it is
/// laid out and coloured, dimming included.
#[derive(Component, Default, Clone)]
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
            // The bottom right corner, which the editor's panel used to hold and the F11
            // panel does not: it stands in the bottom left.
            right: Val::Px(MARGIN),
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

/// Which switch a checkbox drives on the Tracks page. The resource is the truth for each, so a
/// box and its key can never disagree: both write the field, and `sync_boxes` brings the box
/// back to it.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Switch {
    /// `AucklandRail::visible`: the lines over the city.
    Lines,
    /// `RailEditor::editing`: the discs, the handle and these buttons.
    Editing,
    /// `TrackFrames::shown`: an arrow every 25 m along the line.
    Frames,
    /// `RailStations::shown`: the station names over the lines.
    Stations,
}

/// Fills the AT trains tab's Tracks page: the four switches over the network, then the
/// editor's own controls under them.
///
/// The editor was a panel of its own in the bottom right corner, shown exactly while editing
/// was on. It is a page now, and the switch that turns editing on is on the page with it, so
/// there is nothing left for the panel's visibility to follow: the controls are laid out
/// whether or not editing is on, and `enable_buttons` hands every one of them to the widget as
/// disabled while it is off, as it does for a button whose edit has nothing to apply to.
fn fill_tracks_page(
    mut commands: Commands,
    bodies: Query<(Entity, &SubTabBody), Added<SubTabBody>>,
    editor: Res<RailEditor>,
    rail: Res<AucklandRail>,
    frames: Res<TrackFrames>,
    stations: Res<RailStations>,
) {
    for (body, page) in bodies.iter().filter(|(_, page)| page.0 == SubTab::Tracks) {
        spawn_box(
            &mut commands,
            body,
            Switch::Lines,
            "show the lines",
            rail.visible,
        );
        spawn_box(
            &mut commands,
            body,
            Switch::Editing,
            "edit the track",
            editor.editing,
        );
        spawn_box(
            &mut commands,
            body,
            Switch::Frames,
            "show the frames",
            frames.shown,
        );
        spawn_box(
            &mut commands,
            body,
            Switch::Stations,
            "show the station names",
            stations.shown,
        );
        let _ = page;

        commands.spawn((
            Readout,
            Text::new(readout_text(&editor, &rail.network)),
            ThemedText,
            ChildOf(body),
        ));

        let modes = commands.spawn((row(), ChildOf(body))).id();
        for mode in [PointMode::Ground, PointMode::Fixed, PointMode::Between] {
            spawn_button(&mut commands, modes, Action::Mode(mode));
        }

        let actions = commands.spawn((row(), ChildOf(body))).id();
        for action in [
            Action::DropToGround,
            Action::SpanSelection,
            Action::SelectSteepRun,
            Action::Undo,
            Action::Redo,
            Action::Save,
        ] {
            spawn_button(&mut commands, actions, action);
        }

        // Laid out only while there is something to report, see update_readout.
        let mut moved = row();
        moved.display = Display::None;
        let moved = commands.spawn((MovedRow, moved, ChildOf(body))).id();
        commands.spawn((
            MovedText,
            Text::new(moved_ground_text(0, None)),
            ThemedText,
            ChildOf(moved),
        ));
        for action in [Action::PreviousMoved, Action::NextMoved] {
            spawn_button(&mut commands, moved, action);
        }
    }
}

/// A checkbox over one of the three switches.
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
                  mut editor: ResMut<RailEditor>,
                  mut rail: ResMut<AucklandRail>,
                  mut frames: ResMut<TrackFrames>,
                  mut stations: ResMut<RailStations>| {
                match switch {
                    Switch::Lines => rail.visible = change.value,
                    Switch::Editing => editor.editing = change.value,
                    Switch::Frames => {
                        frames.shown = change.value;
                        // Nothing was recomputed while hidden, as F6 has it.
                        frames.stale = true;
                    }
                    Switch::Stations => stations.shown = change.value,
                }
            },
        );
}

/// One of the editor's buttons, which triggers [`Pressed`] when the widget says it was
/// activated, so the work itself stays in one observer however a press arrived.
fn spawn_button(commands: &mut Commands, row: Entity, action: Action) {
    commands
        .spawn_scene(bsn! {
            @FeathersButton {
                @caption: bsn! { Text({action.caption().to_string()}) ThemedText Caption },
            }
        })
        .insert((EditorButton(action), ChildOf(row)))
        .observe(move |_: On<Activate>, mut commands: Commands| {
            commands.trigger(Pressed(action));
        });
}

/// Brings each box back to its switch, for a key pressed since it was drawn.
fn sync_boxes(
    mut commands: Commands,
    editor: Res<RailEditor>,
    rail: Res<AucklandRail>,
    frames: Res<TrackFrames>,
    stations: Res<RailStations>,
    boxes: Query<(Entity, &Switch, Has<Checked>)>,
) {
    for (entity, switch, checked) in &boxes {
        let on = match switch {
            Switch::Lines => rail.visible,
            Switch::Editing => editor.editing,
            Switch::Frames => frames.shown,
            Switch::Stations => stations.shown,
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

/// Hands a button that cannot apply to the widget as disabled, and holds the anchor's own mode
/// down.
///
/// `InteractionDisabled` is what feathers reads to grey a button and what `bevy_ui_widgets`
/// reads to stop it activating, so one component does what the painting and the dimming used to
/// do between them, and `ButtonVariant::Primary` does what the current mode's own colour did.
/// Every frame rather than on a change, because a button goes from able to unable, and a mode
/// from current to not, without the pointer moving.
fn enable_buttons(
    mut commands: Commands,
    editor: Res<RailEditor>,
    rail: Res<AucklandRail>,
    mut buttons: Query<(
        Entity,
        &EditorButton,
        Has<InteractionDisabled>,
        &mut ButtonVariant,
    )>,
) {
    let readiness = Readiness::of(&editor, &rail);
    let current = anchor_mode(&editor, &rail.network);

    for (entity, button, disabled, mut variant) in &mut buttons {
        let action = button.0;
        // Editing off is every edit unavailable: the mouse, the discs and the handle are inert
        // then, and a button that acted would edit a network nothing was showing you.
        let enabled = editor.editing && action.enabled(readiness);

        match (enabled, disabled) {
            (true, true) => {
                commands.entity(entity).remove::<InteractionDisabled>();
            }
            (false, false) => {
                commands.entity(entity).insert(InteractionDisabled);
            }
            _ => {}
        }

        let wanted = match action {
            Action::Mode(mode) if Some(mode) == current => ButtonVariant::Primary,
            _ => ButtonVariant::Normal,
        };
        variant.set_if_neq(wanted);
    }
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
/// A press of one of the editor's buttons, which [`on_pressed`] answers.
#[derive(Event)]
struct Pressed(Action);

/// What each button does. One observer rather than a system over the buttons: the widgets say
/// when they were activated, so what is left is the work, and the work wants a good many
/// resources to be named once rather than per button.
#[allow(clippy::too_many_arguments)]
fn on_pressed(
    press: On<Pressed>,
    mut commands: Commands,
    mut editor: ResMut<RailEditor>,
    mut rail: ResMut<AucklandRail>,
    mut sampler: Option<ResMut<RailSampler>>,
    drag: Res<MoveDrag>,
    mut cursor: ResMut<MovedCursor>,
    mut toasts: Query<(&mut SaveToast, &mut Text, &mut TextColor)>,
    grids: Grids,
    mut camera: Query<(Entity, &mut Transform, &mut CellCoord), With<OrbitalCameraController>>,
) {
    let action = press.0;
    if !action.enabled(Readiness::of(&editor, &rail)) {
        return;
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
                return;
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
                return;
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
