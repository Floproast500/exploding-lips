//! Platform-neutral input state. Front ends report physical sources by name (`key:KeyW`,
//! `mouse:Left`, `pad:South`, see [`engine_core::bind`]) through [`Input::source`]; the bind
//! table turns them into these actions.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Forward,
    Back,
    TurnLeft,
    TurnRight,
    StrafeLeft,
    StrafeRight,
    /// Held modifier that turns TurnLeft/TurnRight into strafes (original: Alt).
    StrafeModifier,
    Run,
    Jump,
    Crouch,
    LookUp,
    LookDown,
    CenterView,
    Use,
    Menu,
    MenuUp,
    MenuDown,
    MenuSelect,
    Map,
    Brighten,
    Darken,
    /// Open the options overlay (F10 / gamepad Select).
    Options,
    /// Tool selection keys 0–9.
    Tool0,
    Tool1,
    Tool2,
    Tool3,
    Tool4,
    Tool5,
    Tool6,
    Tool7,
    Tool8,
    Tool9,
    /// Previous / next selectable tool (gamepad bumpers and D-pad).
    ToolPrev,
    ToolNext,
    /// Engine console (~).
    Console,
    QuickSave,
    QuickLoad,
}

const ACTION_COUNT: usize = Action::QuickLoad as usize + 1;

impl Action {
    pub const ALL: [Action; ACTION_COUNT] = {
        use Action::*;
        [
            Forward, Back, TurnLeft, TurnRight, StrafeLeft, StrafeRight, StrafeModifier, Run, Jump, Crouch, LookUp,
            LookDown, CenterView, Use, Menu, MenuUp, MenuDown, MenuSelect, Map, Brighten, Darken, Options, Tool0, Tool1,
            Tool2, Tool3, Tool4, Tool5, Tool6, Tool7, Tool8, Tool9, ToolPrev, ToolNext, Console, QuickSave, QuickLoad,
        ]
    };

    /// Name used in bind tables and config files (the variant name).
    pub fn name(self) -> String {
        format!("{self:?}")
    }

    pub fn from_name(name: &str) -> Option<Action> {
        Self::ALL.into_iter().find(|a| a.name().eq_ignore_ascii_case(name))
    }

    /// Menu navigation actions share keys with gameplay actions; rebinding a gameplay action
    /// must not take its key away from them.
    pub fn is_menu(self) -> bool {
        matches!(self, Action::Menu | Action::MenuUp | Action::MenuDown | Action::MenuSelect)
    }

    pub const TOOLS: [Action; 10] = [
        Action::Tool0,
        Action::Tool1,
        Action::Tool2,
        Action::Tool3,
        Action::Tool4,
        Action::Tool5,
        Action::Tool6,
        Action::Tool7,
        Action::Tool8,
        Action::Tool9,
    ];
}

#[derive(Debug, Clone)]
pub struct Input {
    held: [bool; ACTION_COUNT],
    pressed: [bool; ACTION_COUNT],
    /// Analog movement from a gamepad stick, each in -1..=1 (x = strafe, y = forward).
    pub move_axis: (f32, f32),
    /// Accumulated look delta since the last tick, in radians (x = yaw, y = pitch).
    pub look_delta: (f32, f32),
    /// Raw mouse motion (counts) since the last tick; the game applies sensitivity and invert.
    pub mouse: (f32, f32),
    /// Right-stick look, each in -1..=1 (y up positive), held state like `move_axis`.
    pub look_axis: (f32, f32),
    /// Characters typed since the last tick (backspace = '\u{8}'), for name prompts.
    pub text: String,
    /// Name of the last source pressed this tick (`key:KeyW`, `mouse:Left`, ...), for rebinding.
    pub last_source: Option<String>,
    /// Sources currently held, so an action stays held while any of its sources is down.
    held_sources: Vec<String>,
}

impl Default for Input {
    fn default() -> Self {
        Self { held: [false; ACTION_COUNT], pressed: [false; ACTION_COUNT], move_axis: (0.0, 0.0), look_delta: (0.0, 0.0), mouse: (0.0, 0.0), look_axis: (0.0, 0.0), text: String::new(), last_source: None, held_sources: Vec::new() }
    }
}

impl Input {
    pub fn set(&mut self, action: Action, down: bool) {
        let i = action as usize;
        if down && !self.held[i] {
            self.pressed[i] = true;
        }
        self.held[i] = down;
    }

    /// A physical source went down or up: update every action bound to it.
    pub fn source(&mut self, binds: &engine_core::BindTable, source: &str, down: bool) {
        let was = self.held_sources.iter().any(|s| s == source);
        match (down, was) {
            (true, false) => {
                self.held_sources.push(source.to_string());
                self.last_source = Some(source.to_string());
            }
            (false, true) => self.held_sources.retain(|s| s != source),
            _ => {}
        }
        for action in binds.actions(source).filter_map(Action::from_name) {
            let any = self.held_sources.iter().any(|s| binds.actions(s).any(|a| a.eq_ignore_ascii_case(&action.name())));
            self.set(action, any);
        }
    }

    /// One-shot source without a release (mouse wheel): pressed for one tick.
    pub fn pulse(&mut self, binds: &engine_core::BindTable, source: &str) {
        self.last_source = Some(source.to_string());
        for action in binds.actions(source).filter_map(Action::from_name) {
            self.pressed[action as usize] = true;
        }
    }

    /// Release everything (focus loss).
    pub fn release_all(&mut self) {
        self.held_sources.clear();
        self.held = [false; ACTION_COUNT];
    }

    pub fn held(&self, action: Action) -> bool {
        self.held[action as usize]
    }

    /// True once per press, until [`Input::end_tick`].
    pub fn pressed(&self, action: Action) -> bool {
        self.pressed[action as usize]
    }

    /// Clear per-tick edge state and consumed deltas.
    pub fn end_tick(&mut self) {
        self.pressed = [false; ACTION_COUNT];
        self.look_delta = (0.0, 0.0);
        self.mouse = (0.0, 0.0);
        self.text.clear();
        self.last_source = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for a in Action::ALL {
            assert_eq!(Action::from_name(&a.name()), Some(a));
        }
        assert_eq!(Action::ALL.len(), Action::QuickLoad as usize + 1);
    }

    #[test]
    fn action_held_while_any_source_is_down() {
        let binds = engine_core::BindTable::from_pairs(&[("key:ControlLeft", "Use"), ("mouse:Left", "Use"), ("wheel:Up", "ToolNext")]);
        let mut input = Input::default();
        input.source(&binds, "key:ControlLeft", true);
        input.source(&binds, "mouse:Left", true);
        input.source(&binds, "key:ControlLeft", false);
        assert!(input.held(Action::Use), "mouse still holds it");
        input.source(&binds, "mouse:Left", false);
        assert!(!input.held(Action::Use));
        assert!(input.pressed(Action::Use));
        input.end_tick();
        input.pulse(&binds, "wheel:Up");
        assert!(input.pressed(Action::ToolNext) && !input.held(Action::ToolNext));
        assert_eq!(input.last_source.as_deref(), Some("wheel:Up"));
    }
}
