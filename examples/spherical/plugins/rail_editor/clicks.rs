//! Clicks and double-clicks told apart from the start of a camera pan, from the presses and
//! releases the mouse system feeds in. Pure, with no window or time of its own, so the rules
//! are tested in the editor's tests with numbers.

use bevy::math::Vec2;

/// A press and release are a click when the pointer moved less than this many pixels and
/// took less than this long between them. Further or longer is the start of a camera pan,
/// which the orbital camera owns.
const CLICK_PIXELS: f32 = 4.0;
const CLICK_SECONDS: f64 = 0.3;

/// Two clicks are a double-click when the second press comes within this long of the first
/// release and within CLICK_PIXELS of it.
const DOUBLE_CLICK_SECONDS: f64 = 0.4;

/// Tells a click from the start of a drag, and a double-click from two clicks, from the
/// presses and releases it is shown. Pure, so the rules can be tested without a window.
#[derive(Default)]
pub(crate) struct ClickDetector {
    /// Where and when the button went down, when it went down over the scene.
    pressed: Option<(Vec2, f64)>,
    /// Where and when the last click was released, which the next press is measured from.
    last_click: Option<(Vec2, f64)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Click {
    Single(Vec2),
    Double(Vec2),
}

impl Click {
    pub(crate) fn position(self) -> Vec2 {
        match self {
            Self::Single(at) | Self::Double(at) => at,
        }
    }
}

impl ClickDetector {
    /// The left button went down. Blocked, the pointer was the UI's, and nothing that
    /// follows is a click.
    pub(crate) fn press(&mut self, at: Vec2, time: f64, blocked: bool) {
        self.pressed = (!blocked).then_some((at, time));
    }

    /// The left button came up. A click if the press was short and still, a double-click
    /// if the one before was close in time and place. A drag forgets the click before it,
    /// so a drag and a click are not a double.
    pub(crate) fn release(&mut self, at: Vec2, time: f64) -> Option<Click> {
        let (from, since) = self.pressed.take()?;

        if at.distance(from) > CLICK_PIXELS || time - since > CLICK_SECONDS {
            self.last_click = None;
            return None;
        }

        let double = self.last_click.is_some_and(|(last_at, last_time)| {
            since - last_time <= DOUBLE_CLICK_SECONDS && from.distance(last_at) <= CLICK_PIXELS
        });

        if double {
            // A third click starts over rather than making a second double.
            self.last_click = None;
            Some(Click::Double(at))
        } else {
            self.last_click = Some((at, time));
            Some(Click::Single(at))
        }
    }
}
