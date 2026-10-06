//! The table of the trains in the bottom left corner: one row per train with where it is
//! along its line, which way it is running and how fast, and a camera icon that puts the
//! chase camera behind it.
//!
//! The table is built in the style of the F2 provenance table, a grid of text cells under a
//! row of headings with a rule beneath them, and shows exactly while the carriages do, see
//! Trains::drawn, so F7 takes the two away together and F4, which hides the network, takes
//! them with it. The rows are rebuilt whenever the count of trains is not the count of rows,
//! which is once, on the first frame, unless a later driver adds a train. The text cells are
//! refreshed every frame the table shows, each written only when its string has changed, as
//! the editor's readout is.
//!
//! The bundled font covers printable ASCII only, so the camera icon is drawn from UI nodes
//! rather than a glyph: a rounded body, a round lens in it and a viewfinder bump on its top
//! edge, in one function so the look can be changed in one place. The body and the bump take
//! the colour, grey when idle, brighter under the pointer and the line's own colour while
//! that train is followed; the lens stays dark against whichever.

use super::Trains;
use super::chase::ChaseCamera;
use crate::plugins::auckland_rail::{AucklandRail, line_colour};
use crate::plugins::provenance::ascii;
use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::{color::Luminance, prelude::*, text::FontSize};

/// The table's systems, added by TrainsPlugin.
pub(super) struct TrainsTablePlugin;

impl Plugin for TrainsTablePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_table).add_systems(
            Update,
            (
                show_table,
                build_rows.run_if(rows_stale),
                // After the trains have moved this frame, so the distance read is where
                // the carriage is drawn.
                refresh_table.after(super::drive_trains),
                colour_follow_buttons,
            )
                .chain(),
        );
    }
}

/// The gap between the window's edge and the table, in pixels, as the other panels keep.
const MARGIN: f32 = 6.0;

/// The column headings, in order. The last is the camera icon's, which needs no word: the
/// icon says what it is, and the README says what it does.
const HEADINGS: [&str; 5] = ["line", "km", "dir", "km/h", ""];

/// The icon's geometry, in pixels: the body with its corner radius, the lens across, the
/// viewfinder bump above the body and how far in from the body's left edge it sits, and the
/// clear space round the whole that makes the click target a little bigger than the drawing.
const BODY: Vec2 = Vec2::new(16.0, 11.0);
const BODY_RADIUS: f32 = 2.0;
const LENS: f32 = 7.0;
const BUMP: Vec2 = Vec2::new(6.0, 3.0);
const BUMP_INSET: f32 = 3.0;
const ICON_PADDING: f32 = 2.0;

/// The icon's colours: a grey that reads against the black panel without shouting, lighter
/// under the pointer and lighter again while pressed, and for the lens a dark ring and a
/// darker fill that stay dark over any body colour.
const IDLE_COLOUR: Color = Color::srgb(0.55, 0.55, 0.55);
const HOVER_COLOUR: Color = Color::srgb(0.8, 0.8, 0.8);
const PRESSED_COLOUR: Color = Color::WHITE;
const LENS_RING: Color = Color::srgba(0.0, 0.0, 0.0, 0.8);
const LENS_FILL: Color = Color::srgba(0.0, 0.0, 0.0, 0.35);

/// How much lighter a followed train's icon goes under the pointer, so it still answers.
const FOLLOWED_HOVER_LIGHTER: f32 = 0.15;

/// The panel, shown while the carriages are.
#[derive(Component)]
struct TrainsTable;

/// The grid the rows are built into.
#[derive(Component)]
struct TrainsGrid;

/// A text cell: which train's and which column.
#[derive(Component, Clone, Copy)]
struct TrainCell {
    train: usize,
    column: Column,
}

/// The camera icon on a train's row, a button that follows that train.
#[derive(Component, Clone, Copy)]
pub struct FollowButton(pub usize);

/// A part of the icon that takes the icon's colour: the body and the bump.
#[derive(Component)]
struct IconFill;

/// The text columns, in the order of HEADINGS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Line,
    Km,
    Dir,
    Speed,
}

/// Kilometres along the line to one decimal: a hundred metres is as fine as the eye tells
/// on a line of tens of kilometres, and it ticks over every five seconds at line speed.
pub fn km_text(distance: f64) -> String {
    format!("{:.1}", distance / 1000.0)
}

/// Which way the train runs: ">" in the file's point order, towards the line's last point,
/// and "<" back towards its first, as the README's legend has it.
pub fn dir_text(direction: f64) -> &'static str {
    if direction < 0.0 { "<" } else { ">" }
}

/// Metres per second as whole kilometres per hour, the unit the README quotes the speed in.
pub fn speed_text(speed: f64) -> String {
    format!("{:.0}", speed * 3.6)
}

/// A text cell's string, the line's name folded to the font as the editor's panel folds it.
pub fn cell_text(column: Column, name: &str, distance: f64, direction: f64, speed: f64) -> String {
    match column {
        Column::Line => ascii(name),
        Column::Km => km_text(distance),
        Column::Dir => dir_text(direction).to_string(),
        Column::Speed => speed_text(speed),
    }
}

/// The icon's colour from what the pointer is doing to it and, while its train is followed,
/// the line's colour: that colour at rest, lighter under the pointer so the icon still
/// answers, and otherwise the greys.
pub fn icon_colour(interaction: Interaction, followed: Option<Color>) -> Color {
    match (followed, interaction) {
        (Some(colour), Interaction::None) => colour,
        (Some(colour), _) => colour.lighter(FOLLOWED_HOVER_LIGHTER),
        (None, Interaction::None) => IDLE_COLOUR,
        (None, Interaction::Hovered) => HOVER_COLOUR,
        (None, Interaction::Pressed) => PRESSED_COLOUR,
    }
}

fn font() -> TextFont {
    TextFont {
        font_size: FontSize::Px(12.0),
        ..default()
    }
}

fn spawn_table(mut commands: Commands) {
    commands.spawn((
        TrainsTable,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(MARGIN),
            left: Val::Px(MARGIN),
            padding: UiRect::all(Val::Px(MARGIN)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
        Visibility::Hidden,
        children![(
            TrainsGrid,
            Node {
                display: Display::Grid,
                grid_template_columns: RepeatedGridTrack::auto(HEADINGS.len() as u16),
                column_gap: Val::Px(12.0),
                row_gap: Val::Px(2.0),
                align_items: AlignItems::Center,
                ..default()
            },
        )],
    ));
}

/// A text cell, in one line however long the name: columns are sized to their content, so
/// a cell that wrapped would size its column to a word.
fn spawn_cell(grid: &mut RelatedSpawnerCommands<ChildOf>, text: &str, colour: Color) -> Entity {
    grid.spawn((
        TextLayout::new(Justify::Left, LineBreak::NoWrap),
        Text::new(text),
        font(),
        TextColor(colour),
    ))
    .id()
}

/// The camera icon, a button with the drawing as its children: the bump on the body's top
/// edge left of centre, the body below it with its corners rounded, and the lens centred in
/// the body as a ring. The parts are placed absolutely within the button, which is the
/// drawing plus ICON_PADDING all round, so the button is what the pointer hits and the parts
/// are what it colours.
fn spawn_follow_button(grid: &mut RelatedSpawnerCommands<ChildOf>, train: usize) {
    let part = |top: f32, left: f32, size: Vec2, radius: BorderRadius| Node {
        position_type: PositionType::Absolute,
        top: Val::Px(ICON_PADDING + top),
        left: Val::Px(ICON_PADDING + left),
        width: Val::Px(size.x),
        height: Val::Px(size.y),
        border_radius: radius,
        ..default()
    };
    // The bump's top corners are softened and its bottom ones meet the body square.
    let bump = part(
        0.0,
        BUMP_INSET,
        BUMP,
        BorderRadius::new(Val::Px(1.0), Val::Px(1.0), Val::ZERO, Val::ZERO),
    );
    let mut body = part(BUMP.y, 0.0, BODY, BorderRadius::all(Val::Px(BODY_RADIUS)));
    body.justify_content = JustifyContent::Center;
    body.align_items = AlignItems::Center;
    let lens = Node {
        width: Val::Px(LENS),
        height: Val::Px(LENS),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::MAX,
        ..default()
    };

    grid.spawn((
        FollowButton(train),
        Button,
        Node {
            width: Val::Px(BODY.x + 2.0 * ICON_PADDING),
            height: Val::Px(BODY.y + BUMP.y + 2.0 * ICON_PADDING),
            align_self: AlignSelf::Center,
            ..default()
        },
        BackgroundColor(Color::NONE),
        children![
            (IconFill, bump, BackgroundColor(IDLE_COLOUR)),
            (
                IconFill,
                body,
                BackgroundColor(IDLE_COLOUR),
                children![(
                    lens,
                    BorderColor::all(LENS_RING),
                    BackgroundColor(LENS_FILL)
                )],
            ),
        ],
    ));
}

/// The table is visible exactly while the carriages are, see Trains::drawn.
fn show_table(
    trains: Res<Trains>,
    rail: Res<AucklandRail>,
    mut panel: Query<&mut Visibility, With<TrainsTable>>,
) {
    let visibility = if trains.drawn(&rail) {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut panel in &mut panel {
        panel.set_if_neq(visibility);
    }
}

/// Whether there is a row for every train and no more: one icon per row.
fn rows_stale(trains: Res<Trains>, buttons: Query<&FollowButton>) -> bool {
    trains.trains.len() != buttons.iter().len()
}

/// Builds the headings, the rule under them and a row per train, from scratch: the rows are
/// few and a rebuild is rare, see the module doc.
fn build_rows(
    mut commands: Commands,
    trains: Res<Trains>,
    rail: Res<AucklandRail>,
    grid: Query<Entity, With<TrainsGrid>>,
) {
    let Ok(grid) = grid.single() else {
        return;
    };
    let mut grid = commands.entity(grid);
    grid.despawn_children();

    grid.with_children(|grid| {
        for heading in HEADINGS {
            spawn_cell(grid, heading, Color::srgb(0.65, 0.8, 1.0));
        }
        // A rule under the headings, spanning every column. Grid lines are 1 indexed.
        grid.spawn((
            Node {
                grid_column: GridPlacement::start_span(1, HEADINGS.len() as u16),
                height: Val::Px(1.0),
                ..default()
            },
            BackgroundColor(Color::srgb(0.4, 0.4, 0.4)),
        ));

        for (index, train) in trains.trains.iter().enumerate() {
            let name = train.line_name(&rail);
            let colour = line_colour(name);

            for column in [Column::Line, Column::Km, Column::Dir, Column::Speed] {
                let text = cell_text(column, name, train.distance, train.direction, trains.speed);
                let text_colour = if column == Column::Line {
                    colour
                } else {
                    Color::WHITE
                };
                let cell = spawn_cell(grid, &text, text_colour);
                grid.commands().entity(cell).insert(TrainCell {
                    train: index,
                    column,
                });
            }
            spawn_follow_button(grid, index);
        }
    });
}

/// Rewrites every text cell whose string has changed, while the table shows. A cell for a
/// train that is gone is left as it was; the rows are rebuilt next frame.
fn refresh_table(
    trains: Res<Trains>,
    rail: Res<AucklandRail>,
    mut cells: Query<(&TrainCell, &mut Text)>,
) {
    if !trains.drawn(&rail) {
        return;
    }

    for (cell, mut text) in &mut cells {
        let Some(train) = trains.trains.get(cell.train) else {
            continue;
        };
        let wanted = cell_text(
            cell.column,
            train.line_name(&rail),
            train.distance,
            train.direction,
            trains.speed,
        );
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
}

/// Colours the icons by the pointer and by which train is followed. Every frame rather than
/// on a change of Interaction, since the followed train changes without the pointer moving,
/// when the camera is let go of by a key or a press on the terrain; the write is skipped
/// when the colour is already right.
fn colour_follow_buttons(
    chase: Res<ChaseCamera>,
    trains: Res<Trains>,
    rail: Res<AucklandRail>,
    buttons: Query<(&FollowButton, &Interaction)>,
    mut fills: Query<(&ChildOf, &mut BackgroundColor), With<IconFill>>,
) {
    for (child_of, mut background) in &mut fills {
        let Ok((button, interaction)) = buttons.get(child_of.parent()) else {
            continue;
        };
        let followed = (chase.following == Some(button.0)).then(|| {
            trains
                .trains
                .get(button.0)
                .map_or(Color::WHITE, |train| line_colour(train.line_name(&rail)))
        });
        background.set_if_neq(BackgroundColor(icon_colour(*interaction, followed)));
    }
}

#[cfg(test)]
mod tests;
