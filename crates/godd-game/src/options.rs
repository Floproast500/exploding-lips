//! Player-facing options (modern additions) and the in-game options overlay (F10 / gamepad
//! Select), including a controls page for rebinding keys. Front ends read [`GameOptions`]
//! every frame, persist it, and apply rebinding requests from [`OptionsMenu::take_rebind`].

use crate::Frame;
use crate::input::{Action, Input};
use crate::text::{GLYPH, draw_text, fill_rect, frame_rect, text_width};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameOptions {
    /// Vertical field of view in degrees (original ≈ 48.9).
    pub fov: f32,
    pub true_color: bool,
    /// Render the original 320x200 screen (4:3) instead of native resolution.
    pub classic: bool,
    pub mouse_sensitivity: f32,
    pub invert_mouse: bool,
    /// Keep status messages on screen longer (accessibility).
    pub long_messages: bool,
    pub show_fps: bool,
    /// Brighter, higher-contrast HUD colours (accessibility).
    pub high_contrast: bool,
}

impl Default for GameOptions {
    fn default() -> Self {
        Self {
            fov: 48.9,
            true_color: false,
            classic: false,
            mouse_sensitivity: 0.0025,
            invert_mouse: false,
            long_messages: false,
            show_fps: false,
            high_contrast: false,
        }
    }
}

/// Actions offered on the controls page.
pub const REBINDABLE: [(Action, &str); 14] = [
    (Action::Forward, "Forward"),
    (Action::Back, "Back"),
    (Action::TurnLeft, "Turn left"),
    (Action::TurnRight, "Turn right"),
    (Action::StrafeLeft, "Strafe left"),
    (Action::StrafeRight, "Strafe right"),
    (Action::Run, "Run"),
    (Action::Jump, "Jump"),
    (Action::Crouch, "Crouch"),
    (Action::Use, "Use tool"),
    (Action::LookUp, "Look up"),
    (Action::LookDown, "Look down"),
    (Action::CenterView, "Centre view"),
    (Action::Map, "Map"),
];

const MAIN_ITEMS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Page {
    #[default]
    Main,
    Controls,
}

#[derive(Debug, Default)]
pub struct OptionsMenu {
    pub open: bool,
    page: Page,
    selected: usize,
    /// Waiting for a key to bind to this action.
    capturing: Option<Action>,
    rebind: Option<(Action, String)>,
    /// Current key names per rebindable action, provided by the front end.
    pub binding_names: Vec<(Action, String)>,
}

impl OptionsMenu {
    /// A rebinding chosen on the controls page: (action, key name from `Input::last_key`).
    pub fn take_rebind(&mut self) -> Option<(Action, String)> {
        self.rebind.take()
    }

    /// Handle input while open. Returns true if the menu consumed the input.
    pub fn update(&mut self, input: &Input, options: &mut GameOptions) -> bool {
        if let Some(action) = self.capturing {
            if let Some(key) = &input.last_key {
                if key != "Escape" {
                    self.rebind = Some((action, key.clone()));
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
        if input.pressed(Action::Menu) {
            match self.page {
                Page::Main => self.open = false,
                Page::Controls => {
                    self.page = Page::Main;
                    self.selected = 8;
                }
            }
            return true;
        }
        let count = match self.page {
            Page::Main => MAIN_ITEMS,
            Page::Controls => REBINDABLE.len() + 1,
        };
        if input.pressed(Action::MenuUp) {
            self.selected = (self.selected + count - 1) % count;
        }
        if input.pressed(Action::MenuDown) {
            self.selected = (self.selected + 1) % count;
        }
        let select = input.pressed(Action::MenuSelect);
        if self.page == Page::Controls {
            if select {
                match REBINDABLE.get(self.selected) {
                    Some(&(action, _)) => self.capturing = Some(action),
                    None => {
                        self.page = Page::Main;
                        self.selected = 8;
                    }
                }
            }
            return true;
        }
        let left = input.pressed(Action::TurnLeft) || input.pressed(Action::StrafeLeft);
        let right = input.pressed(Action::TurnRight) || input.pressed(Action::StrafeRight) || select;
        let step = if right { 1.0 } else if left { -1.0 } else { 0.0 };
        if step != 0.0 {
            match self.selected {
                0 => options.fov = (options.fov + step * 2.0).clamp(30.0, 110.0),
                1 => options.true_color = !options.true_color,
                2 => options.classic = !options.classic,
                3 => options.mouse_sensitivity = (options.mouse_sensitivity * if step > 0.0 { 1.15 } else { 1.0 / 1.15 }).clamp(0.0003, 0.02),
                4 => options.invert_mouse = !options.invert_mouse,
                5 => options.long_messages = !options.long_messages,
                6 => options.show_fps = !options.show_fps,
                7 => options.high_contrast = !options.high_contrast,
                8 if select => {
                    self.page = Page::Controls;
                    self.selected = 0;
                }
                9 if select => self.open = false,
                _ => {}
            }
        }
        true
    }

    pub fn draw(&self, frame: &mut Frame, options: &GameOptions, colors: (u8, u8, u8, u8)) {
        if !self.open && self.capturing.is_none() {
            return;
        }
        let on = |b: bool| if b { "on" } else { "off" };
        let (title, rows): (&str, Vec<String>) = match self.page {
            Page::Main => (
                "Options",
                vec![
                    format!("Field of view   {:>5.1}", options.fov),
                    format!("True colour     {:>5}", on(options.true_color)),
                    format!("Classic 320x200 {:>5}", on(options.classic)),
                    format!("Mouse speed     {:>5.2}", options.mouse_sensitivity * 1000.0),
                    format!("Invert mouse    {:>5}", on(options.invert_mouse)),
                    format!("Long messages   {:>5}", on(options.long_messages)),
                    format!("Show FPS        {:>5}", on(options.show_fps)),
                    format!("High contrast   {:>5}", on(options.high_contrast)),
                    "Controls...".to_string(),
                    "Back".to_string(),
                ],
            ),
            Page::Controls => (
                if self.capturing.is_some() { "Press a key (Esc cancels)" } else { "Controls" },
                REBINDABLE
                    .iter()
                    .map(|(action, label)| {
                        let key = self.binding_names.iter().find(|(a, _)| a == action).map_or("-", |(_, k)| k.as_str());
                        format!("{label:<13}{key:>10}")
                    })
                    .chain(std::iter::once("Back".to_string()))
                    .collect(),
            ),
        };
        draw_panel(frame, title, &rows, self.selected, colors);
    }
}

fn draw_panel(frame: &mut Frame, title: &str, rows: &[String], selected: usize, colors: (u8, u8, u8, u8)) {
    let (bg, border, text, highlight) = colors;
    let s = (frame.height / 200).max(1);
    let line = (GLYPH + 4) * s;
    let w = rows.iter().map(|r| text_width(r, s)).chain(std::iter::once(text_width(title, s))).max().unwrap_or(0) + 16 * s;
    let h = line * (rows.len() + 2);
    let x = (frame.width.saturating_sub(w) / 2) as i32;
    let y = (frame.height.saturating_sub(h) / 2) as i32;
    fill_rect(frame, x, y, w, h, bg);
    frame_rect(frame, x, y, w, h, border);
    draw_text(frame, x + 8 * s as i32, y + 4 * s as i32, s, highlight, title);
    for (i, row) in rows.iter().enumerate() {
        let color = if i == selected { highlight } else { text };
        let ry = y + ((i + 1) * line + 4 * s) as i32;
        if i == selected {
            draw_text(frame, x + 2 * s as i32, ry, s, highlight, ">");
        }
        draw_text(frame, x + 8 * s as i32, ry, s, color, row);
    }
}
