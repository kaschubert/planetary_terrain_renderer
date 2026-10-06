//! Whether the pointer belongs to the UI or to the scene.
//!
//! The camera controllers read the mouse directly, so without this a drag on a slider was
//! also a pan, and a click on a checkbox also planted a pan anchor. Bevy's UI already
//! knows what is under the pointer; this turns that into one question the controllers can
//! ask before they start anything. The hover map comes from bevy_picking, which an app
//! without the default plugins' picking may not have; then nothing is ever over the UI and
//! the controllers run unhindered, rather than the app panicking on its first frame for want
//! of a resource.

use bevy::{
    picking::{hover::HoverMap, pointer::PointerId},
    prelude::*,
    ui::ComputedNode,
};

/// Whether the UI has the pointer.
///
/// Over a node it does, plainly. Less plainly, a button pressed over a node stays the UI's
/// until it is released, wherever the pointer goes meanwhile: a slider drag that wanders
/// off its panel must not turn into a camera pan halfway. The reverse holds too, and needs
/// no state here: a camera drag begun over the scene keeps going across a panel, because
/// the controllers only consult this when starting one.
#[derive(Resource, Default)]
pub struct PointerCapture {
    over_ui: bool,
    /// Left, middle, right: pressed while over the UI and not yet released.
    held: [bool; 3],
}

const BUTTONS: [MouseButton; 3] = [MouseButton::Left, MouseButton::Middle, MouseButton::Right];

impl PointerCapture {
    /// True while the scene should leave the mouse alone.
    pub fn blocks_pointer(&self) -> bool {
        self.over_ui || self.held.iter().any(|&held| held)
    }
}

pub(crate) fn track_pointer_capture(
    hover: Option<Res<HoverMap>>,
    nodes: Query<(), With<ComputedNode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut capture: ResMut<PointerCapture>,
) {
    // Every node is picked by default, whether or not it is interactive, and blocks what is
    // under it. The filter is only insurance against another backend feeding this map.
    capture.over_ui = hover
        .as_deref()
        .and_then(|hover| hover.get(&PointerId::Mouse))
        .is_some_and(|hits| hits.keys().any(|entity| nodes.contains(*entity)));

    let over_ui = capture.over_ui;

    for (held, button) in capture.held.iter_mut().zip(BUTTONS) {
        if buttons.just_pressed(button) {
            *held = over_ui;
        } else if !buttons.pressed(button) {
            *held = false;
        }
    }
}
