//! Gamepad input (gilrs), with a layout that suits the Steam Deck's built-in controls.

use engine_core::BindTable;
use gilrs::{Axis, EventType, Gilrs};
use godd_game::Input;

const DEADZONE: f32 = 0.15;

pub struct Gamepads {
    gilrs: Gilrs,
}

impl Gamepads {
    pub fn new() -> Option<Self> {
        Gilrs::new().map(|gilrs| Self { gilrs }).map_err(|e| log::warn!("gamepads disabled: {e}")).ok()
    }

    /// Apply pending button events (as `pad:<Button>` sources through the bind table) and the
    /// current stick positions to `input`.
    pub fn update(&mut self, input: &mut Input, binds: &BindTable) {
        while let Some(event) = self.gilrs.next_event() {
            let (button, down) = match event.event {
                EventType::ButtonPressed(b, _) => (b, true),
                EventType::ButtonReleased(b, _) => (b, false),
                _ => continue,
            };
            log::debug!("gamepad {} {button:?} {}", event.id, if down { "down" } else { "up" });
            input.source(binds, &format!("pad:{button:?}"), down);
        }
        let mut axes = (0.0, 0.0, 0.0, 0.0);
        for (_, pad) in self.gilrs.gamepads() {
            let v = |a| deadzone(pad.value(a));
            axes.0 += v(Axis::LeftStickX);
            axes.1 += v(Axis::LeftStickY);
            axes.2 += v(Axis::RightStickX);
            axes.3 += v(Axis::RightStickY);
        }
        if axes != (0.0, 0.0, 0.0, 0.0) {
            log::trace!("gamepad axes {axes:?}");
        }
        apply_axes(input, axes);
    }
}

/// Sticks (left x/y, right x/y, after the deadzone; summed over pads) to movement and look.
/// The game applies look speed and invert (`joy_sensitivity`, `joy_invert`).
fn apply_axes(input: &mut Input, axes: (f32, f32, f32, f32)) {
    let c = |v: f32| v.clamp(-1.0, 1.0);
    input.move_axis = (c(axes.0), c(axes.1));
    input.look_axis = (c(axes.2), c(axes.3));
}

fn deadzone(v: f32) -> f32 {
    if v.abs() < DEADZONE { 0.0 } else { (v - DEADZONE * v.signum()) / (1.0 - DEADZONE) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadzone_is_continuous_and_reaches_full_scale() {
        assert_eq!(deadzone(0.1), 0.0);
        assert_eq!(deadzone(-0.149), 0.0);
        assert!(deadzone(0.151).abs() < 0.01, "no jump at the edge");
        assert_eq!(deadzone(1.0), 1.0);
        assert_eq!(deadzone(-1.0), -1.0);
        assert!((deadzone(0.575) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn sticks_move_and_look() {
        let mut input = Input::default();
        apply_axes(&mut input, (0.5, -2.0, 1.0, -0.5));
        assert_eq!(input.move_axis, (0.5, -1.0), "two pads pushing together clamp to full");
        assert_eq!(input.look_axis, (1.0, -0.5));
        apply_axes(&mut input, (0.0, 0.0, 0.0, 0.0));
        assert_eq!(input.move_axis, (0.0, 0.0), "released stick stops");
    }

    #[test]
    fn pad_buttons_reach_actions_through_the_binds() {
        use godd_game::Action;
        let binds = godd_game::options::default_binds(godd_game::options::Profile::Modern);
        let mut input = Input::default();
        for (button, action) in [("South", Action::Jump), ("RightTrigger2", Action::Use), ("Start", Action::Menu), ("RightTrigger", Action::ToolNext)] {
            input.source(&binds, &format!("pad:{button}"), true);
            assert!(input.held(action), "{button}");
        }
    }
}
