//! The example's plugins run together, as only the whole example otherwise runs them.

use super::*;

/// Bevy refuses a system whose parameters could reach one component two ways (its error
/// B0001) the first time the system's schedule runs, which an example otherwise finds out
/// only when it opens a window. Running the example's own ten plugins through one update
/// with no window, no input and no renderer trips the same check here; the library's
/// plugins and the gizmo crate's want a renderer and are not the example's to check. A
/// system whose resources are missing fails Bevy's parameter validation, which the warning
/// error handler turns into a skip instead of a panic, and its parameters are initialised
/// before that check, so nothing but a conflict can bring the test down. The live trains
/// are added without a key, so the test never reaches for the network.
#[test]
fn every_system_initialises_without_a_query_conflict() {
    let mut app = App::new();
    app.set_error_handler(bevy::ecs::error::warn);
    app.add_plugins((
        MinimalPlugins,
        VramUsagePlugin,
        ProvenancePlugin,
        SheetGridPlugin,
        AucklandRailPlugin,
        RailEditorPlugin,
        TrackFramesPlugin,
        StationsPlugin,
        PhotoMarkersPlugin,
        TrainsPlugin,
        LiveTrainsPlugin::offline(),
        TabPanelPlugin,
    ));
    app.update();
}

/// Every line of the help is broken at a word, and no line runs past the hundredth character
/// unless one word does on its own.
#[test]
fn the_help_breaks_its_lines_at_a_word_within_a_hundred_characters() {
    let long = "- ".to_string() + &"word ".repeat(60);
    let broken = wrapped(&long);

    for line in broken.lines() {
        let width = line.chars().count();
        assert!(width <= HELP_COLUMNS, "{width}: {line}");
    }
    assert_eq!(
        broken.replace('\n', " ").split_whitespace().count(),
        long.split_whitespace().count(),
        "a word was lost or gained"
    );

    // A word of its own that cannot fit is left whole, overlong, rather than cut.
    let path = "x".repeat(HELP_COLUMNS + 20);
    assert_eq!(wrapped(&path), path);
}
