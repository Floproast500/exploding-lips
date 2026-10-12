//! Gamepad input (gilrs), with a layout that suits the Steam Deck's built-in controls.

use godd_game::{Action, Input};
use gilrs::{Axis, Button, EventType, Gilrs};

const DEADZONE: f32 = 0.15;
/// Radians per tick at full right-stick deflection.
const LOOK_SPEED: (f32, f32) = (0.06, 0.035);

pub struct Gamepads {
    gilrs: Gilrs,
}

impl Gamepads {
    pub fn new() -> Option<Self> {
        Gilrs::new().map(|gilrs| Self { gilrs }).map_err(|e| log::warn!("gamepads disabled: {e}")).ok()
    }

    /// Apply pending events and current stick positions to `input`.
    pub fn update(&mut self, input: &mut Input) {
        while let Some(event) = self.gilrs.next_event() {
            let (button, down) = match event.event {
                EventType::ButtonPressed(b, _) => (b, true),
                EventType::ButtonReleased(b, _) => (b, false),
                _ => continue,
            };
            log::debug!("gamepad {} {button:?} {}", event.id, if down { "down" } else { "up" });
            for &action in bindings(button) {
                input.set(action, down);
            }
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
fn apply_axes(input: &mut Input, axes: (f32, f32, f32, f32)) {
    input.move_axis = (axes.0.clamp(-1.0, 1.0), axes.1.clamp(-1.0, 1.0));
    input.look_delta.0 += axes.2 * LOOK_SPEED.0;
    input.look_delta.1 -= axes.3 * LOOK_SPEED.1;
}

fn deadzone(v: f32) -> f32 {
    if v.abs() < DEADZONE { 0.0 } else { (v - DEADZONE * v.signum()) / (1.0 - DEADZONE) }
}

fn bindings(button: Button) -> &'static [Action] {
    use Action::*;
    match button {
        Button::South => &[Jump, MenuSelect],
        Button::East => &[Crouch],
        Button::West => &[Use],
        Button::North => &[Map],
        Button::RightTrigger2 => &[Use],
        Button::LeftTrigger2 => &[Run],
        Button::LeftThumb => &[Run],
        Button::RightThumb => &[CenterView],
        Button::Start => &[Menu],
        Button::Select => &[Options],
        Button::DPadUp => &[MenuUp, LookUp],
        Button::DPadDown => &[MenuDown, LookDown],
        Button::LeftTrigger | Button::DPadLeft => &[ToolPrev],
        Button::RightTrigger | Button::DPadRight => &[ToolNext],
        _ => &[],
    }
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
        apply_axes(&mut input, (0.5, -2.0, 1.0, 1.0));
        assert_eq!(input.move_axis, (0.5, -1.0), "two pads pushing together clamp to full");
        assert_eq!(input.look_delta, (LOOK_SPEED.0, -LOOK_SPEED.1), "stick up looks up");
        apply_axes(&mut input, (0.0, 0.0, 0.0, 0.0));
        assert_eq!(input.move_axis, (0.0, 0.0), "released stick stops");
    }

    #[test]
    fn steam_deck_layout() {
        assert!(bindings(Button::South).contains(&Action::Jump));
        assert!(bindings(Button::South).contains(&Action::MenuSelect));
        assert!(bindings(Button::RightTrigger2).contains(&Action::Use));
        assert!(bindings(Button::West).contains(&Action::Use));
        assert_eq!(bindings(Button::Start), &[Action::Menu]);
        assert_eq!(bindings(Button::Select), &[Action::Options]);
        assert!(bindings(Button::DPadDown).contains(&Action::MenuDown));
        assert_eq!(bindings(Button::RightTrigger), &[Action::ToolNext], "bumpers switch tools");
        assert_eq!(bindings(Button::DPadLeft), &[Action::ToolPrev]);
        assert!(bindings(Button::Mode).is_empty());
    }
}
