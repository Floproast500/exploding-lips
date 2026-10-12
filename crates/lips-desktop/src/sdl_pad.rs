//! SDL3 gamepads (feature `sdl`): the same buttons and sticks as the gilrs backend (button
//! names match, so bindings carry over), plus gyro aiming for pads with motion sensors (Steam
//! Deck, DualShock/DualSense, Switch Pro) and rumble. Polled each frame; no SDL window or event
//! loop is involved.

use engine_core::{BindTable, HapticEvent};
use godd_game::Input;
use sdl3_sys::gamepad::*;
use sdl3_sys::joystick::SDL_JoystickID;
use sdl3_sys::hints::{SDL_HINT_NO_SIGNAL_HANDLERS, SDL_SetHint};
use sdl3_sys::init::{SDL_INIT_GAMEPAD, SDL_Init};
use sdl3_sys::sensor::SDL_SENSOR_GYRO;

const DEADZONE: f32 = 0.15;

/// SDL button → the gilrs name used in bind tables.
const BUTTONS: [(SDL_GamepadButton, &str); 15] = [
    (SDL_GAMEPAD_BUTTON_SOUTH, "South"),
    (SDL_GAMEPAD_BUTTON_EAST, "East"),
    (SDL_GAMEPAD_BUTTON_WEST, "West"),
    (SDL_GAMEPAD_BUTTON_NORTH, "North"),
    (SDL_GAMEPAD_BUTTON_BACK, "Select"),
    (SDL_GAMEPAD_BUTTON_GUIDE, "Mode"),
    (SDL_GAMEPAD_BUTTON_START, "Start"),
    (SDL_GAMEPAD_BUTTON_LEFT_STICK, "LeftThumb"),
    (SDL_GAMEPAD_BUTTON_RIGHT_STICK, "RightThumb"),
    (SDL_GAMEPAD_BUTTON_LEFT_SHOULDER, "LeftTrigger"),
    (SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER, "RightTrigger"),
    (SDL_GAMEPAD_BUTTON_DPAD_UP, "DPadUp"),
    (SDL_GAMEPAD_BUTTON_DPAD_DOWN, "DPadDown"),
    (SDL_GAMEPAD_BUTTON_DPAD_LEFT, "DPadLeft"),
    (SDL_GAMEPAD_BUTTON_DPAD_RIGHT, "DPadRight"),
];

struct Pad {
    handle: *mut SDL_Gamepad,
    id: SDL_JoystickID,
    gyro: bool,
    held: Vec<bool>,
    triggers: [bool; 2],
}

pub struct SdlPads {
    pads: Vec<Pad>,
    frames: u32,
}

/// Look change (radians, in `Input::look_delta`'s convention: positive turns right / looks
/// down) from the gyro's angular velocity `g` (rad/s; x pitches up, y yaws left) over `dt`:
/// the view turns as far as the controller did, times `speed`.
pub fn gyro_look(g: [f32; 3], dt: f32, speed: f32) -> (f32, f32) {
    (-g[1] * dt * speed, -g[0] * dt * speed)
}

fn deadzone(v: f32) -> f32 {
    if v.abs() < DEADZONE { 0.0 } else { (v - DEADZONE * v.signum()) / (1.0 - DEADZONE) }
}

impl SdlPads {
    pub fn new() -> Option<Self> {
        // SAFETY: plain SDL calls on the main thread; SDL owns no Rust memory here. SDL would
        // otherwise turn SIGINT/SIGTERM into quit events nobody reads, so the game would
        // ignore Ctrl+C and `kill`.
        if !unsafe {
            SDL_SetHint(SDL_HINT_NO_SIGNAL_HANDLERS, c"1".as_ptr());
            SDL_Init(SDL_INIT_GAMEPAD)
        } {
            log::warn!("SDL gamepads unavailable");
            return None;
        }
        let mut s = Self { pads: Vec::new(), frames: 0 };
        s.rescan();
        log::info!("SDL gamepads: {} connected", s.pads.len());
        Some(s)
    }

    /// Open newly connected pads, forget disconnected ones.
    fn rescan(&mut self) {
        // SAFETY: SDL returns a malloc'd id array we free; handles stay valid until closed.
        unsafe {
            self.pads.retain(|p| SDL_GamepadConnected(p.handle));
            let mut count = 0;
            let ids = SDL_GetGamepads(&mut count);
            if ids.is_null() {
                return;
            }
            for k in 0..count.max(0) as usize {
                let id = *ids.add(k);
                if self.pads.iter().any(|p| p.id == id) {
                    continue;
                }
                let handle = SDL_OpenGamepad(id);
                if handle.is_null() {
                    continue;
                }
                let gyro = SDL_GamepadHasSensor(handle, SDL_SENSOR_GYRO) && SDL_SetGamepadSensorEnabled(handle, SDL_SENSOR_GYRO, true);
                if gyro {
                    log::info!("gamepad {}: gyro available", id.0);
                }
                self.pads.push(Pad { handle, id, gyro, held: vec![false; BUTTONS.len()], triggers: [false; 2] });
            }
            sdl3_sys::stdinc::SDL_free(ids.cast());
        }
    }

    /// Buttons through the bind table, sticks to movement/look, and (when `gyro_speed` is set)
    /// the gyro's angular velocity over `dt` seconds as look.
    pub fn update(&mut self, input: &mut Input, binds: &BindTable, gyro_speed: Option<f32>, dt: f32) {
        self.frames += 1;
        if self.frames % 120 == 1 {
            self.rescan();
        }
        let mut axes = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        // SAFETY: handles are open pads (rescan drops disconnected ones).
        unsafe {
            SDL_UpdateGamepads();
            for pad in &mut self.pads {
                for (k, &(button, name)) in BUTTONS.iter().enumerate() {
                    let down = SDL_GetGamepadButton(pad.handle, button);
                    if down != pad.held[k] {
                        pad.held[k] = down;
                        input.source(binds, &format!("pad:{name}"), down);
                    }
                }
                for (k, (axis, name)) in [(SDL_GAMEPAD_AXIS_LEFT_TRIGGER, "LeftTrigger2"), (SDL_GAMEPAD_AXIS_RIGHT_TRIGGER, "RightTrigger2")].into_iter().enumerate() {
                    let down = SDL_GetGamepadAxis(pad.handle, axis) as f32 / 32767.0 > 0.5;
                    if down != pad.triggers[k] {
                        pad.triggers[k] = down;
                        input.source(binds, &format!("pad:{name}"), down);
                    }
                }
                let a = |axis| deadzone(SDL_GetGamepadAxis(pad.handle, axis) as f32 / 32767.0);
                axes.0 += a(SDL_GAMEPAD_AXIS_LEFTX);
                axes.1 -= a(SDL_GAMEPAD_AXIS_LEFTY); // SDL: down is positive
                axes.2 += a(SDL_GAMEPAD_AXIS_RIGHTX);
                axes.3 -= a(SDL_GAMEPAD_AXIS_RIGHTY);
                if let (true, Some(speed)) = (pad.gyro, gyro_speed) {
                    let mut g = [0.0f32; 3];
                    if SDL_GetGamepadSensorData(pad.handle, SDL_SENSOR_GYRO, g.as_mut_ptr(), 3) {
                        let (yaw, pitch) = gyro_look(g, dt, speed);
                        input.look_delta.0 += yaw;
                        input.look_delta.1 += pitch;
                    }
                }
            }
        }
        let c = |v: f32| v.clamp(-1.0, 1.0);
        input.move_axis = (c(axes.0), c(axes.1));
        input.look_axis = (c(axes.2), c(axes.3));
    }

    /// Controller check (`--pad-test`): rumble each pad, then report gyro motion for
    /// `seconds` (move the controller). Returns one line per finding.
    pub fn pad_test(&mut self, seconds: f32) -> Vec<String> {
        let mut report = Vec::new();
        // SAFETY: open handles; SDL calls on this thread only.
        unsafe {
            for pad in &self.pads {
                let name = SDL_GetGamepadName(pad.handle);
                let name = if name.is_null() { "?".into() } else { std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned() };
                let rumble = SDL_RumbleGamepad(pad.handle, 0xC000, 0x8000, 400);
                report.push(format!("pad {} ({name}): rumble {}, gyro {}", pad.id.0, if rumble { "sent" } else { "not supported" }, if pad.gyro { "present" } else { "absent" }));
            }
            let started = std::time::Instant::now();
            let mut peak = vec![[0.0f32; 3]; self.pads.len()];
            while started.elapsed().as_secs_f32() < seconds {
                SDL_UpdateGamepads();
                for (k, pad) in self.pads.iter().enumerate().filter(|(_, p)| p.gyro) {
                    let mut g = [0.0f32; 3];
                    if SDL_GetGamepadSensorData(pad.handle, SDL_SENSOR_GYRO, g.as_mut_ptr(), 3) {
                        for (p, v) in peak[k].iter_mut().zip(g) {
                            *p = p.max(v.abs());
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            for (k, pad) in self.pads.iter().enumerate().filter(|(_, p)| p.gyro) {
                let [x, y, z] = peak[k];
                let moved = x.max(y).max(z) > 0.5;
                report.push(format!("pad {}: peak gyro pitch {x:.2} yaw {y:.2} roll {z:.2} rad/s ({})", pad.id.0, if moved { "motion seen" } else { "no motion seen" }));
            }
        }
        report
    }

    pub fn rumble(&mut self, h: &HapticEvent) {
        let (low, high) = ((h.low.clamp(0.0, 1.0) * 65535.0) as u16, (h.high.clamp(0.0, 1.0) * 65535.0) as u16);
        for pad in &self.pads {
            // SAFETY: open handle.
            unsafe {
                SDL_RumbleGamepad(pad.handle, low, high, (h.seconds * 1000.0) as u32);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::gyro_look;

    /// Turning the controller a quarter turn left over a second (120 frames) turns the player a
    /// quarter turn left in the real game; tilting it up looks up.
    #[test]
    fn gyro_turns_the_player_like_the_controller() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../original");
        if !dir.join("LIPS00.XRB").exists() {
            return;
        }
        let vfs = godd_game::Vfs::from_dir(&dir).unwrap();
        let mut game = godd_game::Game::new(&vfs, &godd_game::Options { start_level: Some("LIPS00".into()), save: None }).unwrap();
        let idle = godd_game::Input::default();
        for _ in 0..10 {
            game.tick(&vfs, &idle);
        }
        let before = game.session().unwrap().player;
        let mut input = godd_game::Input::default();
        for _ in 0..120 {
            let (yaw, pitch) = gyro_look([0.3, std::f32::consts::FRAC_PI_2, 0.0], 1.0 / 120.0, 1.0);
            input.look_delta.0 += yaw;
            input.look_delta.1 += pitch;
        }
        game.tick(&vfs, &input);
        let after = game.session().unwrap().player;
        let turned = (after.angle - before.angle).rem_euclid(std::f32::consts::TAU);
        assert!((turned - std::f32::consts::FRAC_PI_2).abs() < 1e-3, "turned {turned} rad");
        assert!(after.look > before.look, "tilting up looks up ({} -> {})", before.look, after.look);
    }
}
