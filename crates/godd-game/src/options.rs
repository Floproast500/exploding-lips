//! Player options as console variables, the default bindings, and the in-game options overlay
//! (F10 / gamepad Select): one page per cvar category, generated from the registry, plus a
//! key-bindings page that captures keys, mouse buttons, the wheel and pad buttons.

use crate::Frame;
use crate::input::{Action, Input};
use crate::text::{GLYPH, draw_text, fill_rect, frame_rect, text_width};
use engine_core::bind::{BindTable, pretty};
use engine_core::cvar::{Category, CvarRegistry, HIDDEN};

/// Register the game's cvars. Front ends add their own (window, audio device) to the same registry.
pub fn register_cvars(c: &mut CvarRegistry) {
    use Category::*;
    c.float("r_fov", "Field of view", Video, (30.0, 120.0, 2.0), 48.9);
    c.bool("r_truecolor", "True colour", Video, false);
    c.bool("r_classic", "Classic 320x200", Video, false);
    c.choice("r_renderer", "Renderer", Video, &["software", "gpu"], 1);
    c.bool("r_freelook", "True 3D look (GPU)", Video, true);
    c.choice("r_lighting", "Lighting (GPU)", Video, &["classic", "modern"], 1);
    c.bool("r_dynlights", "Dynamic lights", Video, true);
    c.bool("r_bloom", "Bloom", Video, true);
    c.choice("r_msaa", "Anti-aliasing", Video, &["off", "4x"], 1);
    c.choice("r_tonemap", "Tonemapping", Video, &["clamp", "soft", "filmic"], 1);
    c.float("r_exposure", "Exposure", Video, (0.25, 4.0, 0.05), 1.0);
    // HD texture pack folder (built with tools/packbuilder from your own files); "" = none.
    c.string("r_texpack", "Texture pack", Video, "");
    c.float("snd_master", "Master volume", Audio, (0.0, 1.0, 0.05), 1.0);
    c.float("snd_sfx", "Effects volume", Audio, (0.0, 1.0, 0.05), 1.0);
    c.float("snd_music", "Music volume", Audio, (0.0, 1.0, 0.05), 0.8);
    c.float("m_sensitivity", "Mouse speed", Controls, (0.2, 20.0, 0.2), 2.6);
    c.float("m_yscale", "Mouse vertical scale", Controls, (0.25, 4.0, 0.05), 1.0);
    c.bool("m_invert", "Invert mouse", Controls, false);
    c.float("joy_sensitivity", "Stick look speed", Controls, (0.2, 4.0, 0.1), 1.0);
    c.bool("joy_invert", "Invert stick", Controls, false);
    c.bool("cl_autorun", "Always run", Controls, false);
    c.bool("cl_togglecrouch", "Toggle crouch", Controls, false);
    c.int("hud_scale", "HUD scale (0 = auto)", Hud, (0, 8, 1), 0);
    c.choice("crosshair", "Crosshair", Hud, &["off", "dot", "cross", "circle"], 0);
    c.bool("hud_fps", "Show FPS", Hud, false);
    c.bool("hud_highcontrast", "High-contrast HUD", Accessibility, false);
    c.bool("msg_long", "Long messages", Accessibility, false);
}

/// Snapshot of the option cvars, refreshed every tick (cheap to copy, convenient to read).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameOptions {
    /// Vertical field of view in degrees (original ≈ 48.9).
    pub fov: f32,
    pub true_color: bool,
    /// Render the original 320x200 screen (4:3) instead of native resolution.
    pub classic: bool,
    /// Radians per mouse count.
    pub mouse_sensitivity: f32,
    pub mouse_yscale: f32,
    pub invert_mouse: bool,
    pub stick_speed: f32,
    pub invert_stick: bool,
    pub autorun: bool,
    pub toggle_crouch: bool,
    /// HUD pixel scale; 0 = largest that fits.
    pub hud_scale: usize,
    pub crosshair: u8,
    /// Keep status messages on screen longer (accessibility).
    pub long_messages: bool,
    pub show_fps: bool,
    /// Brighter, higher-contrast HUD colours (accessibility).
    pub high_contrast: bool,
    pub volume: (f32, f32, f32),
}

impl GameOptions {
    pub fn from_cvars(c: &CvarRegistry) -> Self {
        let f = |n| c.get_float(n) as f32;
        Self {
            fov: f("r_fov"),
            true_color: c.get_bool("r_truecolor"),
            classic: c.get_bool("r_classic"),
            mouse_sensitivity: f("m_sensitivity") / 1000.0,
            mouse_yscale: f("m_yscale"),
            invert_mouse: c.get_bool("m_invert"),
            stick_speed: f("joy_sensitivity"),
            invert_stick: c.get_bool("joy_invert"),
            autorun: c.get_bool("cl_autorun"),
            toggle_crouch: c.get_bool("cl_togglecrouch"),
            hud_scale: c.get_int("hud_scale") as usize,
            crosshair: c.get_int("crosshair") as u8,
            long_messages: c.get_bool("msg_long"),
            show_fps: c.get_bool("hud_fps"),
            high_contrast: c.get_bool("hud_highcontrast"),
            volume: (f("snd_master"), f("snd_sfx"), f("snd_music")),
        }
    }
}

impl Default for GameOptions {
    fn default() -> Self {
        let mut c = CvarRegistry::default();
        register_cvars(&mut c);
        Self::from_cvars(&c)
    }
}

/// Gamepad buttons, the same in both profiles (gilrs names).
const PAD: &[(&str, &str)] = &[
    ("pad:South", "Jump"),
    ("pad:South", "MenuSelect"),
    ("pad:East", "Crouch"),
    ("pad:West", "Use"),
    ("pad:RightTrigger2", "Use"),
    ("pad:LeftTrigger2", "Run"),
    ("pad:LeftThumb", "Run"),
    ("pad:RightThumb", "CenterView"),
    ("pad:North", "Map"),
    ("pad:Start", "Menu"),
    ("pad:Select", "Options"),
    ("pad:DPadUp", "MenuUp"),
    ("pad:DPadUp", "LookUp"),
    ("pad:DPadDown", "MenuDown"),
    ("pad:DPadDown", "LookDown"),
    ("pad:LeftTrigger", "ToolPrev"),
    ("pad:DPadLeft", "ToolPrev"),
    ("pad:RightTrigger", "ToolNext"),
    ("pad:DPadRight", "ToolNext"),
];

/// Keys shared by both profiles: menus, function keys, tool numbers.
const COMMON: &[(&str, &str)] = &[
    ("key:Escape", "Menu"),
    ("key:ArrowUp", "MenuUp"),
    ("key:ArrowDown", "MenuDown"),
    ("key:Enter", "MenuSelect"),
    ("key:NumpadEnter", "MenuSelect"),
    ("key:F10", "Options"),
    ("key:Backquote", "Console"),
    ("key:F6", "QuickSave"),
    ("key:F7", "QuickLoad"),
    ("key:NumpadMultiply", "Brighten"),
    ("key:NumpadDivide", "Darken"),
    ("key:Digit0", "Tool0"),
    ("key:Digit1", "Tool1"),
    ("key:Digit2", "Tool2"),
    ("key:Digit3", "Tool3"),
    ("key:Digit4", "Tool4"),
    ("key:Digit5", "Tool5"),
    ("key:Digit6", "Tool6"),
    ("key:Digit7", "Tool7"),
    ("key:Digit8", "Tool8"),
    ("key:Digit9", "Tool9"),
];

/// Modern layout: WASD + mouse look, wheel switches tools.
const MODERN: &[(&str, &str)] = &[
    ("key:KeyW", "Forward"),
    ("key:ArrowUp", "Forward"),
    ("key:KeyS", "Back"),
    ("key:ArrowDown", "Back"),
    ("key:KeyA", "StrafeLeft"),
    ("key:KeyD", "StrafeRight"),
    ("key:ArrowLeft", "TurnLeft"),
    ("key:ArrowRight", "TurnRight"),
    ("key:ShiftLeft", "Run"),
    ("key:Space", "Jump"),
    ("key:ControlLeft", "Crouch"),
    ("key:KeyC", "Crouch"),
    ("mouse:Left", "Use"),
    ("key:KeyF", "Use"),
    ("key:Tab", "Map"),
    ("key:PageUp", "LookUp"),
    ("key:PageDown", "LookDown"),
    ("mouse:Middle", "CenterView"),
    ("key:End", "CenterView"),
    ("wheel:Down", "ToolPrev"),
    ("key:KeyQ", "ToolPrev"),
    ("wheel:Up", "ToolNext"),
    ("key:KeyE", "ToolNext"),
];

/// The original keyboard.txt layout (arrows, Alt strafe, Tab jump, Ctrl fires, RMB runs).
const CLASSIC: &[(&str, &str)] = &[
    ("key:ArrowUp", "Forward"),
    ("key:ArrowDown", "Back"),
    ("key:ArrowLeft", "TurnLeft"),
    ("key:ArrowRight", "TurnRight"),
    ("key:AltLeft", "StrafeModifier"),
    ("key:AltRight", "StrafeModifier"),
    ("key:ShiftLeft", "Run"),
    ("key:ShiftRight", "Run"),
    ("key:Tab", "Jump"),
    ("key:Insert", "Jump"),
    ("key:CapsLock", "Crouch"),
    ("key:Delete", "Crouch"),
    ("key:KeyW", "LookUp"),
    ("key:KeyX", "LookDown"),
    ("key:KeyS", "CenterView"),
    ("key:ControlLeft", "Use"),
    ("key:ControlRight", "Use"),
    ("key:Enter", "Use"),
    ("mouse:Left", "Use"),
    ("mouse:Right", "Forward"),
    ("key:F2", "Map"),
    ("wheel:Down", "ToolPrev"),
    ("wheel:Up", "ToolNext"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Modern,
    Classic,
}

pub fn default_binds(profile: Profile) -> BindTable {
    let layout = match profile {
        Profile::Modern => MODERN,
        Profile::Classic => CLASSIC,
    };
    let mut t = BindTable::default();
    for (s, a) in COMMON.iter().chain(layout).chain(PAD) {
        t.bind(s, a);
    }
    t
}

/// Actions offered on the key-bindings page.
pub const REBINDABLE: [(Action, &str); 20] = [
    (Action::Forward, "Forward"),
    (Action::Back, "Back"),
    (Action::StrafeLeft, "Strafe left"),
    (Action::StrafeRight, "Strafe right"),
    (Action::TurnLeft, "Turn left"),
    (Action::TurnRight, "Turn right"),
    (Action::Run, "Run"),
    (Action::Jump, "Jump"),
    (Action::Crouch, "Crouch"),
    (Action::Use, "Use tool"),
    (Action::ToolPrev, "Previous tool"),
    (Action::ToolNext, "Next tool"),
    (Action::LookUp, "Look up"),
    (Action::LookDown, "Look down"),
    (Action::CenterView, "Centre view"),
    (Action::Map, "Map"),
    (Action::StrafeModifier, "Strafe modifier"),
    (Action::QuickSave, "Quick save"),
    (Action::QuickLoad, "Quick load"),
    (Action::Console, "Console"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Page {
    #[default]
    Main,
    Category(Category),
    Bindings,
}

#[derive(Debug, Default)]
pub struct OptionsMenu {
    pub open: bool,
    page: Page,
    selected: usize,
    /// Waiting for a key, button or wheel turn to bind to this action.
    capturing: Option<Action>,
}

/// Categories that have at least one visible cvar, in menu order.
fn categories(c: &CvarRegistry) -> Vec<Category> {
    Category::ALL.into_iter().filter(|&cat| c.iter().any(|v| v.category == cat && v.flags & HIDDEN == 0)).collect()
}

fn items(c: &CvarRegistry, cat: Category) -> Vec<&'static str> {
    c.iter().filter(|v| v.category == cat && v.flags & HIDDEN == 0).map(|v| v.name).collect()
}

impl OptionsMenu {
    pub fn is_capturing(&self) -> bool {
        self.capturing.is_some()
    }

    /// Handle input while open. Returns true if the menu consumed the input (the game pauses).
    pub fn update(&mut self, input: &Input, cvars: &mut CvarRegistry, binds: &mut BindTable) -> bool {
        if let Some(action) = self.capturing {
            if let Some(source) = &input.last_source {
                if source != "key:Escape" && source != "pad:Start" {
                    binds.rebind(&action.name(), source, |a| Action::from_name(a).is_some_and(Action::is_menu));
                }
                self.capturing = None;
            }
            return true;
        }
        if input.pressed(Action::Options) {
            self.open = !self.open;
            self.page = Page::Main;
            self.selected = 0;
            return true;
        }
        if !self.open {
            return false;
        }
        let cats = categories(cvars);
        let back_to_main = |page: Page| match page {
            Page::Category(c) => cats.iter().position(|&x| x == c).unwrap_or(0),
            _ => cats.len(),
        };
        if input.pressed(Action::Menu) {
            match self.page {
                Page::Main => self.open = false,
                p => {
                    self.selected = back_to_main(p);
                    self.page = Page::Main;
                }
            }
            return true;
        }
        let count = match self.page {
            Page::Main => cats.len() + 2,
            Page::Category(cat) => items(cvars, cat).len() + 1,
            Page::Bindings => REBINDABLE.len() + 3,
        };
        if input.pressed(Action::MenuUp) {
            self.selected = (self.selected + count - 1) % count;
        }
        if input.pressed(Action::MenuDown) {
            self.selected = (self.selected + 1) % count;
        }
        let select = input.pressed(Action::MenuSelect);
        let left = input.pressed(Action::TurnLeft) || input.pressed(Action::StrafeLeft) || input.pressed(Action::ToolPrev);
        let right = input.pressed(Action::TurnRight) || input.pressed(Action::StrafeRight) || input.pressed(Action::ToolNext);
        match self.page {
            Page::Main if select => {
                self.page = match self.selected {
                    i if i < cats.len() => Page::Category(cats[i]),
                    i if i == cats.len() => Page::Bindings,
                    _ => {
                        self.open = false;
                        Page::Main
                    }
                };
                self.selected = 0;
            }
            Page::Category(cat) => {
                let names = items(cvars, cat);
                match names.get(self.selected) {
                    Some(name) if left => cvars.step(name, -1),
                    Some(name) if right || select => cvars.step(name, 1),
                    None if select => {
                        self.selected = back_to_main(self.page);
                        self.page = Page::Main;
                    }
                    _ => {}
                }
            }
            Page::Bindings if select => match self.selected {
                i if i < REBINDABLE.len() => self.capturing = Some(REBINDABLE[i].0),
                i if i == REBINDABLE.len() => *binds = default_binds(Profile::Modern),
                i if i == REBINDABLE.len() + 1 => *binds = default_binds(Profile::Classic),
                _ => {
                    self.selected = back_to_main(self.page);
                    self.page = Page::Main;
                }
            },
            _ => {}
        }
        true
    }

    pub fn draw(&self, frame: &mut Frame, cvars: &CvarRegistry, binds: &BindTable, colors: (u8, u8, u8, u8)) {
        if !self.open && self.capturing.is_none() {
            return;
        }
        let back = || "Back".to_string();
        let (title, rows): (String, Vec<String>) = match self.page {
            Page::Main => (
                "Options".into(),
                categories(cvars)
                    .iter()
                    .map(|c| format!("{}...", c.label()))
                    .chain(["Key bindings...".to_string(), back()])
                    .collect(),
            ),
            Page::Category(cat) => {
                let names = items(cvars, cat);
                let width = names.iter().filter_map(|n| cvars.find(n)).map(|v| v.label.len()).max().unwrap_or(0) + 2;
                (
                    cat.label().into(),
                    names
                        .iter()
                        .filter_map(|n| cvars.find(n))
                        .map(|v| format!("{:<width$}{:>8}", v.label, v.display()))
                        .chain(std::iter::once(back()))
                        .collect(),
                )
            }
            Page::Bindings => (
                if self.capturing.is_some() { "Press a key, button or wheel (Esc cancels)".into() } else { "Key bindings".into() },
                REBINDABLE
                    .iter()
                    .map(|(action, label)| {
                        let names: Vec<String> = binds.sources(&action.name()).iter().map(|s| pretty(s)).take(3).collect();
                        let keys = if names.is_empty() { "-".to_string() } else { names.join(", ") };
                        format!("{label:<16}{keys:>22}")
                    })
                    .chain(["Defaults: modern (WASD + mouse)".to_string(), "Defaults: classic (keyboard.txt)".to_string(), back()])
                    .collect(),
            ),
        };
        draw_panel(frame, &title, &rows, self.selected, colors);
    }
}

fn draw_panel(frame: &mut Frame, title: &str, rows: &[String], selected: usize, colors: (u8, u8, u8, u8)) {
    let (bg, border, text, highlight) = colors;
    let s = (frame.height / 200).max(1);
    let line = (GLYPH + 4) * s;
    // Long pages scroll: show a window of rows that keeps the selection visible.
    let fits = (frame.height / line).saturating_sub(3).max(3);
    let shown = rows.len().min(fits);
    let first = selected.saturating_sub(shown / 2).min(rows.len() - shown);
    let w = rows.iter().map(|r| text_width(r, s)).chain(std::iter::once(text_width(title, s))).max().unwrap_or(0) + 16 * s;
    let h = line * (shown + 2);
    let x = (frame.width.saturating_sub(w) / 2) as i32;
    let y = (frame.height.saturating_sub(h) / 2) as i32;
    fill_rect(frame, x, y, w, h, bg);
    frame_rect(frame, x, y, w, h, border);
    draw_text(frame, x + 8 * s as i32, y + 4 * s as i32, s, highlight, title);
    for (k, (i, row)) in rows.iter().enumerate().skip(first).take(shown).enumerate() {
        let color = if i == selected { highlight } else { text };
        let ry = y + ((k + 1) * line + 4 * s) as i32;
        if i == selected {
            draw_text(frame, x + 2 * s as i32, ry, s, highlight, ">");
        }
        draw_text(frame, x + 8 * s as i32, ry, s, color, row);
    }
    let arrow_x = x + w as i32 - 10 * s as i32;
    if first > 0 {
        draw_text(frame, arrow_x, y + 4 * s as i32, s, text, "^");
    }
    if first + shown < rows.len() {
        draw_text(frame, arrow_x, y + ((shown + 1) * line + 4 * s) as i32, s, text, "v");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_cover_every_rebindable_action() {
        for profile in [Profile::Modern, Profile::Classic] {
            let t = default_binds(profile);
            for (action, label) in REBINDABLE {
                let skip = match profile {
                    Profile::Modern => action == Action::StrafeModifier, // strafes with A/D
                    Profile::Classic => matches!(action, Action::StrafeLeft | Action::StrafeRight), // Alt + arrows
                };
                if skip {
                    continue;
                }
                assert!(!t.sources(&action.name()).is_empty(), "{profile:?}: {label} unbound");
            }
            for (_, a) in t.iter() {
                assert!(Action::from_name(a).is_some(), "unknown action {a}");
            }
        }
    }

    #[test]
    fn menu_edits_cvars_and_rebinds() {
        let mut c = CvarRegistry::default();
        register_cvars(&mut c);
        let mut b = default_binds(Profile::Modern);
        let mut m = OptionsMenu::default();
        let press = |a: Action| {
            let mut i = Input::default();
            i.set(a, true);
            i
        };
        assert!(m.update(&press(Action::Options), &mut c, &mut b));
        // Main page: Video, Audio, Controls, HUD, Accessibility, Key bindings, Back.
        m.update(&press(Action::MenuSelect), &mut c, &mut b); // Video
        m.update(&press(Action::MenuDown), &mut c, &mut b); // True colour
        m.update(&press(Action::MenuSelect), &mut c, &mut b);
        assert!(c.get_bool("r_truecolor"));
        m.update(&press(Action::Menu), &mut c, &mut b); // back to main
        for _ in 0..5 {
            m.update(&press(Action::MenuDown), &mut c, &mut b);
        }
        m.update(&press(Action::MenuSelect), &mut c, &mut b); // Key bindings, row 0 = Forward
        m.update(&press(Action::MenuSelect), &mut c, &mut b);
        assert!(m.is_capturing());
        let mut i = Input::default();
        i.last_source = Some("key:KeyI".into());
        m.update(&i, &mut c, &mut b);
        assert_eq!(b.sources("Forward"), ["key:KeyI"]);
        assert!(b.actions("key:ArrowUp").any(|a| a == "MenuUp"), "menu keys survive");
        assert!(!b.sources("Jump").is_empty() && b.sources("Jump").contains(&"pad:South"));
    }
}
