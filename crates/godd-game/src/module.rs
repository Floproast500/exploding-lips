//! Exploding Lips as an engine [`GameModule`]: the same interface the Quake-family viewer
//! implements, so generic runners can host it. (The desktop and web front ends use the richer
//! [`Game`] API directly: menus, saves, options, netplay.)

use crate::{Action, Game, Input, Vfs};
use engine_core::{AudioEvent, Frame};
use engine_scene::module::{GameModule, ModuleInput, ModuleView};
use engine_scene::scene::IndexedImage;

pub struct GoddModule {
    pub game: Game,
    pub vfs: Vfs,
    input: Input,
}

impl GoddModule {
    pub fn new(game: Game, vfs: Vfs) -> Self {
        Self { game, vfs, input: Input::default() }
    }
}

impl GameModule for GoddModule {
    fn title(&self) -> String {
        match self.game.session() {
            Some(s) => format!("Exploding Lips — {}", s.level.title.trim()),
            None => "Exploding Lips".into(),
        }
    }

    fn tick(&mut self, m: &ModuleInput) {
        let i = &mut self.input;
        i.set(Action::Forward, m.forward > 0.3);
        i.set(Action::Back, m.forward < -0.3);
        i.set(Action::StrafeRight, m.strafe > 0.3);
        i.set(Action::StrafeLeft, m.strafe < -0.3);
        i.set(Action::Jump, m.up > 0.3);
        i.set(Action::Crouch, m.up < -0.3);
        i.set(Action::Run, m.run);
        i.set(Action::Use, m.use_pressed);
        // Module look: positive yaw turns left, positive pitch looks up; the game's is the other way.
        i.look_delta = (-m.look.0, -m.look.1);
        self.game.tick(&self.vfs, i);
        i.end_tick();
    }

    fn view(&mut self, ui: &mut Frame, alpha: f32) -> Option<ModuleView> {
        let v = self.game.gpu_frame(ui, alpha)?;
        Some(ModuleView { camera: v.camera, shading: v.shading, scene: v.scene, epoch: v.epoch })
    }

    fn images(&self) -> Vec<IndexedImage<'_>> {
        self.game.world_images()
    }

    fn palette(&self) -> &[[u8; 3]; 256] {
        self.game.palette()
    }

    fn shade_rows(&self) -> &[[u8; 256]] {
        self.game.shade_rows()
    }

    fn drain_audio(&mut self) -> Vec<AudioEvent> {
        self.game.drain_audio()
    }
}
