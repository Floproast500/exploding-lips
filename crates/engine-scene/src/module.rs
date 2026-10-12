//! The contract between the engine's front ends and a game module (the G.O.D.D. games, the
//! Quake-family BSP viewer, future revival targets). A runner owns the window, input devices,
//! audio and the GPU; it ticks the module at a fixed rate, asks it for a view each frame and
//! draws that with the shared renderer.

use crate::scene::{IndexedImage, SceneCamera, SceneFrame, Shading};
use engine_core::{AudioEvent, Frame};

/// Fixed simulation rate the runner ticks modules at.
pub const TICK_RATE: f64 = 60.0;

/// Movement and look for one tick, already mapped from keys, mouse and pads.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ModuleInput {
    /// -1..=1 each.
    pub forward: f32,
    pub strafe: f32,
    pub up: f32,
    /// Look change this tick in radians (yaw, pitch); positive yaw turns left, positive pitch up.
    pub look: (f32, f32),
    pub run: bool,
    /// The "use" / fire button was pressed this tick.
    pub use_pressed: bool,
}

/// What to draw this frame.
pub struct ModuleView {
    pub camera: SceneCamera,
    pub shading: Shading,
    pub scene: SceneFrame,
    /// Changes when the module's images change wholesale (new level): re-upload them.
    pub epoch: u64,
}

pub trait GameModule {
    /// Window title.
    fn title(&self) -> String;
    /// Advance one fixed tick.
    fn tick(&mut self, input: &ModuleInput);
    /// The view at `alpha` between the last two ticks; the module may draw HUD and menus into
    /// `ui` (palette indices with coverage). `None`: nothing to show in 3D.
    fn view(&mut self, ui: &mut Frame, alpha: f32) -> Option<ModuleView>;
    /// The images vertices refer to (re-read when the view's epoch changes).
    fn images(&self) -> Vec<IndexedImage<'_>>;
    fn palette(&self) -> &[[u8; 3]; 256];
    /// Shade tables, `rows[level][index]` with level 0 brightest.
    fn shade_rows(&self) -> &[[u8; 256]];
    /// Audio requests since the last call.
    fn drain_audio(&mut self) -> Vec<AudioEvent> {
        Vec::new()
    }
}

/// Fit modern lighting to a palette and its shade tables: the linear brightness of each shade
/// level (how much the tables darken colours on average) and the colours no level darkens
/// (fullbright: lamps, lava).
pub fn fit_light(palette: &[[u8; 3]; 256], rows: &[[u8; 256]]) -> ([f32; 16], [u32; 8]) {
    let lin = |c: u8| {
        let v = c as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let lum = |i: u8| {
        let [r, g, b] = palette[i as usize];
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    };
    let mut brightness = [0.0f32; 16];
    let base: f32 = (0..=255u8).map(lum).sum::<f32>().max(1e-6);
    for (level, b) in brightness.iter_mut().enumerate() {
        *b = rows.get(level).map_or(0.0, |row| (0..256).map(|i| lum(row[i])).sum::<f32>() / base);
    }
    let mut emissive = [0u32; 8];
    for i in 1..256usize {
        // Steady through the bright half of the table (the darkest rows may still fade them).
        let half = &rows[..rows.len().div_ceil(2)];
        let steady = !half.is_empty() && half.iter().all(|row| palette[row[i] as usize] == palette[i]);
        if steady && lum(i as u8) > 0.05 {
            emissive[i / 32] |= 1 << (i % 32);
        }
    }
    (brightness, emissive)
}
