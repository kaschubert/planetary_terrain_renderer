//! The table of where each terrain's pixels came from, toggled with F2.
//!
//! It loads its own files rather than reading the terrains', because the terrains are not
//! there to read. They stream in by camera distance, so Wellington's config is never loaded
//! at all unless you fly to it, and one that does load is dropped as soon as the atlas has
//! copied out of it. A provenance file is a few kilobytes, so this holds all of them for
//! the life of the process and the table is complete from the first frame.

use super::sheet_grid::SheetGridControls;
use crate::STREAMED_TERRAINS;
use bevy::asset::LoadState;
use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;
use bevy::text::FontSize;
use bevy_terrain::prelude::*;

pub struct ProvenancePlugin;

impl Plugin for ProvenancePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProvenanceTable>()
            .add_systems(Startup, (load_provenance, spawn_provenance_table))
            .add_systems(
                Update,
                (
                    settle_provenance.run_if(provenance_pending),
                    rebuild_provenance_table.run_if(provenance_dirty),
                    toggle_provenance_table,
                )
                    .chain(),
            );
    }
}

/// The F2 panel: a row of controls, then the table.
#[derive(Component)]
struct ProvenancePanel;

/// The table itself, rebuilt as provenance arrives.
#[derive(Component)]
struct ProvenanceGrid;

#[derive(Resource, Default)]
struct ProvenanceTable {
    entries: Vec<ProvenanceEntry>,
    /// Set when an entry settles, cleared when the rows are rebuilt.
    dirty: bool,
}

struct ProvenanceEntry {
    name: String,
    /// Held for the life of the app. Dropping it would unload the asset, which is exactly
    /// how the streamed terrain configs disappear.
    handle: Handle<TerrainProvenance>,
    state: ProvenanceState,
}

#[derive(PartialEq)]
enum ProvenanceState {
    Pending,
    Loaded,
    /// The terrain was preprocessed before its sources were recorded. Backfill it with
    /// `preprocess_<name> --provenance-only`.
    Missing,
}

/// The columns, in order. The first is the terrain, left blank on continuation rows so the
/// name reads as a heading without needing a spanning row.
const PROVENANCE_COLUMNS: [&str; 8] = [
    "terrain", "layer", "dataset", "res", "captured", "sheets", "tiles", "size",
];

fn load_provenance(asset_server: Res<AssetServer>, mut table: ResMut<ProvenanceTable>) {
    table.entries = STREAMED_TERRAINS
        .iter()
        .map(|terrain| {
            // "terrains/auckland/config.tc.ron" -> "terrains/auckland" -> "auckland"
            let directory = terrain.path.rsplit_once('/').map_or("", |(dir, _)| dir);

            ProvenanceEntry {
                name: directory
                    .rsplit('/')
                    .next()
                    .unwrap_or(directory)
                    .to_string(),
                handle: asset_server.load(format!("{directory}/provenance.tp.ron")),
                state: ProvenanceState::Pending,
            }
        })
        .collect();

    table.dirty = true;
}

fn provenance_pending(table: Res<ProvenanceTable>) -> bool {
    table
        .entries
        .iter()
        .any(|entry| entry.state == ProvenanceState::Pending)
}

fn provenance_dirty(table: Res<ProvenanceTable>) -> bool {
    table.dirty
}

/// Moves entries out of Pending as the asset server finishes with them.
///
/// Polled rather than driven by AssetEvent because a file that is not there raises no
/// event at all, and a terrain built before provenance existed is the ordinary case. The
/// polling stops for good once every entry has settled.
fn settle_provenance(asset_server: Res<AssetServer>, mut table: ResMut<ProvenanceTable>) {
    let mut settled = false;

    for entry in &mut table.entries {
        entry.state = match asset_server.load_state(&entry.handle) {
            LoadState::Loaded => ProvenanceState::Loaded,
            LoadState::Failed(_) => ProvenanceState::Missing,
            _ => continue,
        };

        settled = true;
    }

    table.dirty |= settled;
}

pub(super) fn spawn_provenance_table(mut commands: Commands) {
    commands.spawn((
        ProvenancePanel,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(6.0),
            // Opposite side to the hotkey list, which is long enough to run most of the
            // way down the left.
            right: Val::Px(6.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(8.0),
            padding: UiRect::all(Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
        Visibility::Hidden,
        children![
            // The sheet grid's height controls, filled in by its plugin.
            (
                SheetGridControls,
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(10.0),
                    ..default()
                },
            ),
            (
                ProvenanceGrid,
                Node {
                    display: Display::Grid,
                    grid_template_columns: RepeatedGridTrack::auto(PROVENANCE_COLUMNS.len() as u16),
                    column_gap: Val::Px(12.0),
                    row_gap: Val::Px(2.0),
                    ..default()
                },
            ),
        ],
    ));
}

fn rebuild_provenance_table(
    mut commands: Commands,
    mut table: ResMut<ProvenanceTable>,
    provenance: Res<Assets<TerrainProvenance>>,
    panel: Single<Entity, With<ProvenanceGrid>>,
) {
    table.dirty = false;

    let mut panel = commands.entity(*panel);
    panel.despawn_children();

    panel.with_children(|panel| {
        for column in PROVENANCE_COLUMNS {
            provenance_cell(panel, column, Color::srgb(0.65, 0.8, 1.0));
        }

        // A rule under the headings, spanning every column. Grid lines are 1 indexed.
        panel.spawn((
            Node {
                grid_column: GridPlacement::start_span(1, PROVENANCE_COLUMNS.len() as u16),
                height: Val::Px(1.0),
                ..default()
            },
            BackgroundColor(Color::srgb(0.4, 0.4, 0.4)),
        ));

        for entry in &table.entries {
            let sources = match entry.state {
                ProvenanceState::Loaded => provenance.get(&entry.handle),
                _ => None,
            };

            let Some(sources) = sources else {
                let note = match entry.state {
                    ProvenanceState::Pending => "loading",
                    // Not an error: every terrain built before this existed has none.
                    _ => "no provenance recorded",
                };

                provenance_cell(panel, &entry.name, Color::WHITE);
                provenance_cell(panel, "", Color::WHITE);
                provenance_cell(panel, note, Color::srgb(0.6, 0.6, 0.6));

                for _ in 3..PROVENANCE_COLUMNS.len() {
                    provenance_cell(panel, "", Color::WHITE);
                }

                continue;
            };

            // A HashMap has no order of its own, and a table that reshuffles between
            // rebuilds is unreadable. Height first, it being the layer the rest sits on.
            let mut layers = sources.sources.iter().collect::<Vec<_>>();
            layers.sort_by_key(|(label, _)| {
                (**label != AttachmentLabel::Height, String::from(*label))
            });

            let mut first = true;

            for (label, records) in layers {
                for record in records {
                    // The terrain names itself once, on its first row.
                    provenance_cell(panel, if first { &entry.name } else { "" }, Color::WHITE);
                    first = false;

                    provenance_cell(panel, &String::from(label), Color::WHITE);

                    let Some(manifest) = &record.manifest else {
                        // A source no download script fetched, as the example terrains use.
                        provenance_cell(panel, &record.path, Color::srgb(0.6, 0.6, 0.6));

                        for _ in 3..PROVENANCE_COLUMNS.len() {
                            provenance_cell(panel, "", Color::WHITE);
                        }

                        continue;
                    };

                    let colour = if manifest.gains.is_some() {
                        // Colour matched, so what you are looking at is not quite what the
                        // survey published.
                        Color::srgb(1.0, 0.85, 0.5)
                    } else {
                        Color::WHITE
                    };

                    provenance_cell(panel, &manifest.dataset, colour);
                    provenance_cell(panel, &manifest.resolution, Color::WHITE);
                    // The national elevation mosaic is stitched from surveys spanning
                    // years and publishes no single date.
                    provenance_cell(
                        panel,
                        manifest.captured.as_deref().unwrap_or("-"),
                        Color::WHITE,
                    );
                    provenance_cell(panel, &manifest.sheets.len().to_string(), Color::WHITE);
                    provenance_cell(panel, &manifest.tiles.to_string(), Color::WHITE);
                    provenance_cell(panel, &gibibytes(manifest.bytes), Color::WHITE);
                }
            }
        }
    });
}

fn provenance_cell(panel: &mut RelatedSpawnerCommands<ChildOf>, text: &str, colour: Color) {
    panel.spawn((
        // Columns are sized to their content, so a cell that wrapped would size its column
        // to a word instead of the whole string.
        TextLayout::new(Justify::Left, LineBreak::NoWrap),
        Text::new(ascii(text)),
        TextFont {
            font_size: FontSize::Px(12.0),
            ..default()
        },
        TextColor(colour),
    ));
}

/// The bundled font covers printable ASCII and nothing else, so anything outside it would
/// render as a gap. These strings come from data files rather than from this source, so
/// fold rather than trust.
fn ascii(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\u{2013}' | '\u{2014}' | '\u{2022}' | '\u{00b7}' => '-',
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            '\u{00a0}' => ' ',
            character if character.is_ascii_graphic() || character == ' ' => character,
            _ => '?',
        })
        .collect()
}

fn gibibytes(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / (1u64 << 30) as f64)
}

fn toggle_provenance_table(
    input: Res<ButtonInput<KeyCode>>,
    mut panel: Query<&mut Visibility, With<ProvenancePanel>>,
) {
    if !input.just_pressed(KeyCode::F2) {
        return;
    }

    for mut visibility in &mut panel {
        *visibility = match *visibility {
            Visibility::Hidden => Visibility::Visible,
            _ => Visibility::Hidden,
        };
    }
}
