//! SDL3 gamepads (feature `sdl`): the same buttons and sticks as the gilrs backend (button
//! names match, so bindings carry over), plus gyro aiming for pads with motion sensors (Steam
//! Deck, DualShock/DualSense, Switch Pro) and rumble. Polled each frame; no SDL window or event
//! loop is involved.

use engine_core::{BindTable, HapticEvent};
use godd_game::Input;
use sdl3_sys::gamepad::*;
use sdl3_sys::joystick::SDL_JoystickID;
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

fn deadzone(v: f32) -> f32 {
    if v.abs() < DEADZONE { 0.0 } else { (v - DEADZONE * v.signum()) / (1.0 - DEADZONE) }
}

impl SdlPads {
    pub fn new() -> Option<Self> {
        // SAFETY: plain SDL calls on the main thread; SDL owns no Rust memory here.
        if !unsafe { SDL_Init(SDL_INIT_GAMEPAD) } {
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
                        // rad/s: x pitches (up positive), y yaws (left positive).
                        input.look_delta.0 -= g[1] * dt * speed;
                        input.look_delta.1 -= g[0] * dt * speed;
                    }
                }
            }
        }
        let c = |v: f32| v.clamp(-1.0, 1.0);
        input.move_axis = (c(axes.0), c(axes.1));
        input.look_axis = (c(axes.2), c(axes.3));
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
