//! The example's plugins run together, as only the whole example otherwise runs them.

use super::*;

/// Bevy refuses a system whose parameters could reach one component two ways (its error
/// B0001) the first time the system's schedule runs, which an example otherwise finds out
/// only when it opens a window. Running the example's own seven plugins through one update
/// with no window, no input and no renderer trips the same check here; the library's
/// plugins and the gizmo crate's want a renderer and are not the example's to check. A
/// system whose resources are missing fails Bevy's parameter validation, which the warning
/// error handler turns into a skip instead of a panic, and its parameters are initialised
/// before that check, so nothing but a conflict can bring the test down.
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
        TrainsPlugin,
    ));
    app.update();
}
