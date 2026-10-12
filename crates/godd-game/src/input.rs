//! Platform-neutral input state. Front ends translate keyboard, mouse and gamepad events into
//! these actions; the default bindings follow the original keyboard.txt.

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
    /// Characters typed since the last tick (backspace = '\u{8}'), for name prompts.
    pub text: String,
    /// Name of the last physical key pressed this tick (for rebinding).
    pub last_key: Option<String>,
}

impl Default for Input {
    fn default() -> Self {
        Self { held: [false; ACTION_COUNT], pressed: [false; ACTION_COUNT], move_axis: (0.0, 0.0), look_delta: (0.0, 0.0), text: String::new(), last_key: None }
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
        self.text.clear();
        self.last_key = None;
    }
}
