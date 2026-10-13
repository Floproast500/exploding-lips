//! A running level: world, player, script VM and the transient UI state (status messages,
//! text overlays). Tags fired by scripts are dispatched to the world here.

use crate::audio::AudioEvent;
use crate::effects::{EffectRequest, Effects};
use crate::input::{Action, Input};
use crate::level::Level;
use crate::menus::{MenuRequest, Menus};
use crate::objects::{ObjRequest, Objects, PlayerView};
use crate::player::{PhysicsParams, Player, PlayerEvent};
use crate::script::{STARTUP_TAG, Script, ScriptEvent};
use crate::{BoxError, TICK_RATE, Vfs};
use godd_formats::xrb::{RecordExt, Section};

/// The startup INI read before the first level (GODDRUN.DAT names it).
pub const STARTUP_INI: &str = "LIPSTART.INI";
/// Seconds a status message (XM) stays on screen.
const MESSAGE_SECONDS: f64 = 3.0;
/// Seconds an OTEXT overlay stays on screen.
const TEXT_SECONDS: f64 = 5.0;

#[derive(Debug, Clone)]
pub struct Message {
    pub color: u8,
    pub text: String,
    ticks_left: u32,
}

pub struct Session {
    pub name: String,
    /// The level's BSP world (`[startup] loadbspfilename`), and whether the player is in it.
    pub bsp: Option<Box<crate::bsp_world::BspWorld>>,
    pub in_bsp: bool,
    /// The background now playing: the level's own (its music, MIDI or a recording) until a
    /// region or the BSP stay names another; one at a time, as in the original.
    background: Option<String>,
    /// The level's default background (restored by a region sound of `BACKSND`).
    default_background: Option<String>,
    /// Sound names that exist only as MIDI (background "sounds" such as TOCATTA are music).
    midi_only: std::collections::HashSet<String>,
    pub level: Level,
    pub player: Player,
    /// Player state at the previous tick, for render interpolation.
    pub previous: Player,
    pub script: Script,
    pub effects: Effects,
    pub objects: Objects,
    pub menus: Menus,
    pub save: godd_formats::gam::SaveFile,
    pub save_dirty: bool,
    pub pending_goto: Option<Goto>,
    /// Level change requested by an OSTRAP exit: (orb, start object).
    pub pending_level: Option<(String, Option<String>)>,
    /// Player view spin from a script (PTL/PTR): radians per tick, ticks left.
    spin: (f32, u32),
    /// Status-message duration multiplier (accessibility "long messages").
    pub message_scale: f64,
    /// World tags fired by local interactions, to share with netplay peers.
    pub outgoing_tags: Vec<u32>,
    /// Netplay peers: id → sprite index.
    remote_players: std::collections::HashMap<u8, usize>,
    /// Movement overrides from morphs (OOTOO on the player).
    pub physics_override: Option<PhysicsParams>,
    /// Cheat: enemy shots do not fire their hit tag (which usually sends the player back).
    pub god: bool,
    /// Original-game bug fixes on (cvar `g_fixes`; off = classic behaviour).
    pub fixes: bool,
    /// Portal hops from the player's region, for bot earshot: ((region, half-second), hops).
    hops_cache: ((usize, u64), Vec<u8>),
    /// The menu's "Options" item was chosen (the game opens the options screen).
    pub wants_options: bool,
    /// Whether the level data fixes (`crate::fixes`) are applied now.
    fixes_applied: bool,
    /// g_fixes trap pits: (region, starting floor, empty since).
    trap_floors: Vec<(usize, f32, Option<f64>)>,
    /// Lightning: everything full bright while set (OFLASH).
    pub flash: bool,
    /// Screen overlays (OTEXOVER): Tex, 320x200 position, animation.
    pub overlays: Vec<(usize, i32, i32, Option<crate::anim::Player>)>,
    /// TV screens (WTV walls showing secondary views).
    pub tvs: Vec<Tv>,
    pub messages: Vec<Message>,
    pub overlay_text: Option<Message>,
    /// Elapsed play time in ticks (HUD "Time").
    pub ticks: u64,
    pub quit_requested: bool,
    /// Currently selected tool (HUD "Tool").
    pub tool: usize,
    pub audio: Vec<AudioEvent>,
}

/// A secondary view shown on walls (WTV): rendered from its camera object into an image.
#[derive(Debug, Clone)]
pub struct Tv {
    pub name: String,
    pub size: (usize, usize),
    /// Object whose position/angle the view uses.
    pub camera: Option<usize>,
    /// World image that receives the picture.
    pub image: usize,
    pub on: bool,
    /// Walls showing it, with their normal middle texture.
    pub walls: Vec<(usize, Option<crate::world::Surface>)>,
}

/// GAMEGOTO request: level to start and the character slot to use.
#[derive(Debug, Clone)]
pub struct Goto {
    pub level: String,
    pub slot: Option<usize>,
    /// Restore position/inventory from the slot (it was saved with GAMESAVE).
    pub restore: bool,
}

/// Whether a level is entered fresh (new game) or from another level (states carry over).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Fresh,
    Continue,
}

impl Session {
    /// Load `name` and run its scripts in the original order (script.md §5).
    pub fn start(
        vfs: &Vfs,
        name: &str,
        entry: Entry,
        carried: Option<Script>,
        save: godd_formats::gam::SaveFile,
        current_slot: Option<usize>,
    ) -> Result<Self, BoxError> {
        let level = Level::load(vfs.read(&format!("{name}.XRB"))?, vfs.read(&format!("{name}.XPG"))?)?;
        let (x, y, angle, region) = level.start;
        let player = Player::in_region(&level.world, x, y, angle, region);
        log::info!(
            "loaded {name}: \"{}\" ({} regions, {} walls, {} sprites)",
            level.title,
            level.world.regions.len(),
            level.world.walls.len(),
            level.world.sprites.len()
        );
        let effects = Effects::new(&level);
        let level_ini = read_text(vfs, &format!("{name}.INI"));
        let objects = Objects::new(&level, &level_ini);
        let ini = godd_formats::ini::Ini::parse(&level_ini);
        let cursor_pic = |name: &str| -> Option<crate::world::Image> {
            let pic = godd_formats::pic::Pic::parse(vfs.read(&format!("gfx/{name}.pic")).ok()?).ok()?;
            Some(crate::world::Image { width: pic.width as usize, height: pic.height as usize, pixels: pic.pixels })
        };
        let mut objects = objects;
        objects.sound_lengths = vfs_sound_lengths(vfs);
        let mut level = level;
        let tvs = setup_tvs(&mut level);
        let mut menus = Menus::new(&level, &ini, &cursor_pic, "lipstart");
        menus.current_slot = current_slot;
        let bsp = ini.section("startup").and_then(|s| s.get("loadbspfilename")).and_then(|file| load_bsp(vfs, file, &objects));
        let mut session = Self {
            bsp,
            in_bsp: false,
            background: None,
            default_background: None,
            midi_only: midi_only_names(vfs),
            menus,
            save,
            save_dirty: false,
            pending_goto: None,
            name: name.to_ascii_uppercase(),
            effects,
            objects,
            pending_level: None,
            spin: (0.0, 0),
            message_scale: 1.0,
            outgoing_tags: Vec::new(),
            remote_players: std::collections::HashMap::new(),
            physics_override: None,
            god: false,
            fixes: false,
            hops_cache: ((usize::MAX, 0), Vec::new()),
            wants_options: false,
            fixes_applied: false,
            trap_floors: Vec::new(),
            flash: false,
            overlays: Vec::new(),
            tvs,
            level,
            previous: player,
            player,
            script: carried.unwrap_or_else(|| Script::new(seed_from(name))),
            messages: Vec::new(),
            overlay_text: None,
            ticks: 0,
            quit_requested: false,
            tool: 0,
            audio: Vec::new(),
        };
        session.script.activate_tag(STARTUP_TAG);
        match entry {
            Entry::Fresh => {
                let startup = read_text(vfs, STARTUP_INI);
                for (ini, section) in [(&startup, "nostrap9999"), (&startup, "commands9999")] {
                    session.script.run_section(ini, section);
                }
                if !name.eq_ignore_ascii_case("LIPSTART") {
                    session.script.run_section(&level_ini, "nostrap9999");
                    session.script.run_section(&level_ini, "commands9999");
                }
            }
            Entry::Continue => {
                // States carry over; only [commands9999] of the new orb runs.
                let states = session.script.states;
                session.script.run_section(&level_ini, "commands9999");
                session.script.states = states;
                session.script.activate_tag(STARTUP_TAG);
            }
        }
        // A new level: its own background (music) replaces whatever played before.
        session.audio.push(AudioEvent::Stop { channel: BACKGROUND_CHANNEL });
        session.audio.push(AudioEvent::Music { name: None, looped: false });
        session.default_background = session.level.music_name();
        let default = session.default_background.clone();
        session.play_background(default);
        session.process_script_events();
        session.apply_tv_textures();
        let available = session.script.available_tools;
        if available != 0 {
            session.objects.tools.available = available;
        }
        let sequences = session.level.sequences.clone();
        session.objects.select_tool(0, true, &sequences);
        session.tool = session.objects.tools.current;
        Ok(session)
    }

    /// How much the space around `region` reverberates (0 dry .. 1 huge hall): the floor area
    /// reachable through open portals within a radius, its height, and whether it is open sky.
    pub fn reverb_at(&self, region: usize) -> f32 {
        let w = &self.level.world;
        let Some(start) = w.regions.get(region) else { return 0.0 };
        let centre = |r: &crate::world::Region| {
            let ((x0, y0), (x1, y1)) = r.bounds;
            ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
        };
        let origin = centre(start);
        let (mut area, mut volume, mut sky) = (0.0f32, 0.0f32, 0.0f32);
        let mut seen = vec![false; w.regions.len()];
        let mut stack = vec![region];
        seen[region] = true;
        while let Some(r) = stack.pop() {
            let reg = &w.regions[r];
            let ((x0, y0), (x1, y1)) = reg.bounds;
            let a = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
            area += a;
            volume += a * (reg.ceiling - reg.floor).max(0.0);
            if reg.ceiling_surface.is_none() {
                sky += a;
            }
            for &wi in &reg.walls {
                let wall = &w.walls[wi];
                let Some(b) = wall.back.filter(|&b| b != wall.front) else { continue };
                let other = if wall.front == r { b } else { wall.front };
                let Some(o) = w.regions.get(other) else { continue };
                let open = reg.ceiling.min(o.ceiling) - reg.floor.max(o.floor);
                let (cx, cy) = centre(o);
                if !seen[other] && open > 48.0 && ((cx - origin.0).powi(2) + (cy - origin.1).powi(2)).sqrt() < 1024.0 {
                    seen[other] = true;
                    stack.push(other);
                }
            }
        }
        if area <= 0.0 {
            return 0.0;
        }
        // ~0.15 for a small room, ~0.6 for a large hall; open sky mostly removes it.
        let size = ((volume / 4.0e6).max(1.0).log2() / 10.0).clamp(0.0, 0.7);
        size * (1.0 - 0.8 * sky / area)
    }

    /// Open portals (walls with room to pass sound) from `region` to every region: 0 for the
    /// region itself, 255 when unreachable.
    /// g_fixes: whether a bot sound at `pos` (in `region`) is heard: close by, or a few open
    /// portals from the player (not across solid walls or shut doors).
    fn in_earshot(&mut self, pos: [f32; 3], region: Option<usize>) -> bool {
        let p = self.player;
        if (pos[0] - p.x).hypot(pos[1] - p.y) < EARSHOT_NEAR {
            return true;
        }
        // (Doors open and shut: recomputed every half second.)
        let key = (p.region, self.ticks / (TICK_RATE as u64 / 2).max(1));
        if self.hops_cache.0 != key {
            self.hops_cache = (key, self.portal_hops(p.region));
        }
        region.and_then(|r| self.hops_cache.1.get(r)).is_some_and(|&h| h <= EARSHOT_HOPS)
    }

    pub fn portal_hops(&self, region: usize) -> Vec<u8> {
        let w = &self.level.world;
        let mut hops = vec![u8::MAX; w.regions.len()];
        let Some(slot) = hops.get_mut(region) else { return hops };
        *slot = 0;
        // Portals are listed by the region in front only, so link both sides from every wall;
        // stacked regions are open to each other too.
        let mut links = vec![Vec::new(); w.regions.len()];
        for wall in &w.walls {
            let Some(b) = wall.back.filter(|&b| b != wall.front && b < w.regions.len()) else { continue };
            let (f, k) = (&w.regions[wall.front], &w.regions[b]);
            if f.ceiling.min(k.ceiling) - f.floor.max(k.floor) > 8.0 {
                links[wall.front].push(b);
                links[b].push(wall.front);
            }
        }
        for (r, reg) in w.regions.iter().enumerate() {
            for o in [reg.above, reg.below].into_iter().flatten().filter(|&o| o < w.regions.len()) {
                links[r].push(o);
                links[o].push(r);
            }
        }
        let mut queue = std::collections::VecDeque::from([region]);
        while let Some(r) = queue.pop_front() {
            for &o in &links[r] {
                if hops[o] == u8::MAX {
                    hops[o] = hops[r].saturating_add(1);
                    queue.push_back(o);
                }
            }
        }
        hops
    }

    /// Positions and facings of other players (netplay), for the automap.
    pub fn remote_positions(&self) -> Vec<(f32, f32, f32)> {
        self.remote_players.values().filter_map(|&i| self.level.world.sprites.get(i)).map(|s| (s.x, s.y, s.angle)).collect()
    }

    pub fn drain_audio(&mut self) -> std::vec::Drain<'_, AudioEvent> {
        self.audio.drain(..)
    }

    pub fn tick(&mut self, input: &Input, physics: &PhysicsParams) {
        let overridden = self.physics_override;
        let physics = overridden.as_ref().unwrap_or(physics);
        self.ticks += 1;
        // Menus and prompts take the input and freeze the player.
        let states = self.script.states;
        let menu_input = self.menus.update(input, &mut self.save, &states);
        self.process_menu_requests();
        if !menu_input && input.pressed(Action::Menu) && self.menus.has_menu_for(620) {
            self.fire_tag(620);
        }
        let idle = Input::default();
        let input = if menu_input || self.menus.is_open() { &idle } else { input };
        self.previous = self.player;
        let dt = 1.0 / TICK_RATE as f32;
        if self.spin.1 > 0 {
            self.spin.1 -= 1;
            self.player.angle = (self.player.angle + self.spin.0).rem_euclid(std::f32::consts::TAU);
        }
        for (k, action) in Action::TOOLS.iter().enumerate() {
            if input.pressed(*action) {
                let sequences = std::mem::take(&mut self.level.sequences);
                self.objects.select_tool(k, false, &sequences);
                self.level.sequences = sequences;
            }
        }
        for (action, delta) in [(Action::ToolPrev, -1), (Action::ToolNext, 1)] {
            if input.pressed(action) {
                let sequences = std::mem::take(&mut self.level.sequences);
                self.objects.cycle_tool(delta, &sequences);
                self.level.sequences = sequences;
            }
        }
        if self.in_bsp {
            if self.fixes {
                self.objects.dry_fire_tick(&self.level, 1.0 / TICK_RATE as f32, input.held(Action::Use));
            }
            self.bsp_tick(input);
            return;
        }
        if input.held(Action::Use) {
            let mut states = self.script.states;
            self.objects.use_tool(&mut states, &self.level);
            self.script.states = states;
        }
        self.tool = self.objects.tools.current;
        let mut contacts: Vec<PlayerEvent> = Vec::new();
        let pad_before = (self.player.on_ground && !self.player.flying).then(|| self.effects.bounce_speed(self.level.world.solid_floor(self.player.region).1)).flatten();
        self.player.tick_with_events(&self.level.world, input, physics, &mut contacts);
        // A jump started on a jump pad also gets the pad's speed (world-effects.md 3.3).
        if let Some(a) = pad_before
            && input.held(Action::Jump)
            && self.player.vz > 0.0
            && !self.player.on_ground
        {
            self.player.vz += a;
        }
        if self.player.region != self.previous.region {
            self.region_background(self.player.region);
        }
        // Ramps (RSLANT): stand on the slope; never above its top unless flying.
        if let Some((low, high)) = self.effects.ramp_limits(self.player.region, self.player.x, self.player.y) {
            if self.player.z < low {
                self.player.z = low;
                self.player.vz = self.player.vz.max(0.0);
                self.player.on_ground = true;
            }
            if self.player.z > high && !self.player.flying {
                self.player.z = high;
            }
        }
        self.effects.on_player_events(&contacts);
        // Jump pads relaunch a body on their floor that is not rising (world-effects.md 3.3);
        // jumping as it touches the pad adds the jump's launch speed (a jump started on a pad
        // gets the pad's speed on top).
        let pad = self.level.world.solid_floor(self.player.region).1;
        if self.player.on_ground && self.player.vz <= 0.0 && !self.player.flying
            && let Some(vz) = self.effects.bounce(pad)
        {
            self.player.vz = vz + if input.held(Action::Jump) { physics.jump_speed } else { 0.0 };
            self.player.on_ground = false;
            // Off the floor at once, as the original's sub-step does: a jump pressed on the
            // next tick must not restart from the floor at the plain jump speed.
            self.player.z += 0.05;
        }
        self.level.animate(dt);
        let occupancy = self.occupancy(physics);
        let now = self.ticks as f64 / TICK_RATE;
        self.effects.fixes = self.fixes;
        self.objects.fixes = self.fixes;
        self.effects.tick(&mut self.level, now, dt, &|r| occupancy.get(r).copied().unwrap_or(0.0));
        self.process_effect_requests();
        if self.fixes != self.fixes_applied {
            crate::fixes::apply(&self.name, &mut self.level.world, self.fixes);
            self.fixes_applied = self.fixes;
        }
        if self.fixes {
            self.reset_traps(now, dt);
        }
        let view = PlayerView {
            x: self.player.x,
            y: self.player.y,
            z: self.player.z,
            eye: self.player.eye_z(physics),
            angle: self.player.angle,
            pitch: self.player.pitch(),
            radius: physics.radius,
            height: physics.height,
            region: self.player.region,
        };
        self.objects.tick(&mut self.level, &view, now, dt);
        for o in self.objects.list.iter_mut().filter(|o| o.mover.is_some() && !o.destroyed) {
            if let Some(r) = o.region
                && let Some((low, _)) = self.effects.ramp_limits(r, o.x, o.y) {
                    o.z = o.z.max(low);
                }
        }
        let seqs = &self.level.sequences;
        for (tex, _, _, anim) in &mut self.overlays {
            if let (Some(a), Some(seq)) = (anim.as_mut(), seqs.get(*tex).and_then(Option::as_ref)) {
                a.advance(seq, dt);
            }
        }
        self.process_object_requests();
        self.process_menu_requests();
        self.messages.retain_mut(|m| {
            m.ticks_left = m.ticks_left.saturating_sub(1);
            m.ticks_left > 0
        });
        if let Some(t) = &mut self.overlay_text {
            t.ticks_left = t.ticks_left.saturating_sub(1);
            if t.ticks_left == 0 {
                self.overlay_text = None;
            }
        }
        self.process_script_events();
    }

    /// Tallest object height per region (player included), for doors and lifts.
    fn occupancy(&self, physics: &PhysicsParams) -> Vec<f32> {
        let mut tallest = vec![0.0f32; self.level.world.regions.len()];
        if let Some(t) = tallest.get_mut(self.player.region) {
            *t = t.max(physics.height);
        }
        for s in self.level.world.sprites.iter().filter(|s| s.visible) {
            if let Some(t) = tallest.get_mut(s.region) {
                *t = t.max(s.height);
            }
        }
        tallest
    }

    fn process_menu_requests(&mut self) {
        for _ in 0..8 {
            let requests = self.menus.take_requests();
            if requests.is_empty() {
                return;
            }
            for r in requests {
                match r {
                    MenuRequest::Tag(t) => self.script.activate_tag(t),
                    MenuRequest::Sound(s) => self.audio.push(AudioEvent::sound(s)),
                    MenuRequest::Message(text) => self.show_message(crate::script::DEFAULT_MESSAGE_COLOR, text),
                    MenuRequest::SaveCurrent => self.save_current(),
                    MenuRequest::Goto { level, slot, restore } => self.pending_goto = Some(Goto { level, slot, restore }),
                    MenuRequest::Options => self.wants_options = true,
                    MenuRequest::Quit => self.quit_requested = true,
                    MenuRequest::SaveFileChanged => self.save_dirty = true,
                }
            }
            self.process_script_events();
        }
    }

    /// GAMESAVE: snapshot the player into the selected slot.
    fn save_current(&mut self) {
        let Some(slot) = self.menus.current_slot else { return };
        let inventory: Vec<String> = self.objects.inventory.iter().map(|&i| self.objects.list[i].name.clone()).collect();
        let name = self.save.slots[slot].name();
        crate::menus::snapshot(
            &mut self.save.slots[slot],
            &self.name,
            (self.player.x, self.player.y, self.player.z),
            self.player.angle,
            &self.script.states,
            &inventory,
        );
        self.save_dirty = true;
        self.show_message(crate::script::DEFAULT_MESSAGE_COLOR, format!("Game saved for {name}"));
    }

    /// Restore a saved position (GAMEGOTO of a saved slot).
    pub fn restore_position(&mut self, x: f32, y: f32, z: f32, angle: f32) {
        self.teleport(x, y, Some(z), Some(angle), None);
    }

    /// Re-take saved inventory items by name.
    pub fn restore_inventory(&mut self, names: &[String]) {
        for name in names {
            if let Some(i) = self.objects.list.iter().position(|o| o.name.eq_ignore_ascii_case(name) && !o.hidden) {
                self.objects.list[i].hidden = true;
                self.objects.inventory.push(i);
            }
        }
    }

    fn process_object_requests(&mut self) {
        for _ in 0..16 {
            let requests = self.objects.take_requests();
            if requests.is_empty() {
                return;
            }
            for r in requests {
                self.object_request(r);
            }
            self.process_script_events();
            self.process_effect_requests();
        }
    }

    fn object_request(&mut self, r: ObjRequest) {
        match r {
            ObjRequest::Tag(t) => self.script.activate_tag(t),
            ObjRequest::PlayerHit(_) if self.god => {}
            ObjRequest::PlayerHit(t) => self.script.activate_tag(t),
            ObjRequest::SharedTag(t) => {
                self.outgoing_tags.push(t);
                self.script.activate_tag(t);
            }
            ObjRequest::TagRange(a, b) => self.script.run_line(&format!("XT({a}-{b})")),
            ObjRequest::Ini(line) => {
                if let Some(rest) = line.strip_prefix("@seltool:") {
                    let mut parts = rest.split(':');
                    let idx = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                    let force = parts.next() == Some("true");
                    let sequences = std::mem::take(&mut self.level.sequences);
                    self.objects.select_tool(idx, force, &sequences);
                    self.level.sequences = sequences;
                } else {
                    self.script.run_line(&line);
                }
            }
            ObjRequest::Sound(s) => self.audio.push(AudioEvent::sound(s)),
            ObjRequest::Message { color, text } => self.show_message(color, text),
            ObjRequest::Text { text, .. } => {
                let ticks_left = (TEXT_SECONDS * TICK_RATE) as u32;
                self.overlay_text = Some(Message { color: crate::script::DEFAULT_MESSAGE_COLOR, text, ticks_left });
            }
            ObjRequest::PlayerState { index, op, value } => {
                if let Some(v) = self.script.states.get_mut(index) {
                    match op {
                        '=' => *v = value,
                        '+' => *v += value,
                        '-' => *v -= value,
                        '*' => *v *= value,
                        '/' if value != 0 => *v /= value,
                        _ => {}
                    }
                }
            }
            ObjRequest::IncState { index, delta } => self.script.change_state(index, delta),
            ObjRequest::Teleport { x, y, z, angle, region, template } => {
                self.teleport(x, y, z, angle, region);
                if let Some(t) = template {
                    self.take_on_template(t);
                }
            }
            ObjRequest::EnterBsp { pos, heading, sound } => {
                if let Some(world) = &mut self.bsp {
                    world.place(pos, heading);
                    world.reset_route();
                    self.player.angle = world.player.orb_angle();
                    if sound.is_some() {
                        self.set_background(sound);
                    }
                    self.in_bsp = true;
                    log::info!("entering the BSP world at {pos:?}");
                }
            }
            ObjRequest::LevelExit { level, start } => self.pending_level = Some((level, start)),
            ObjRequest::TurnPlayer { units_per_frame, frames } => {
                // Original units per 30 Hz frame → radians per simulation tick.
                let per_tick = units_per_frame as f32 / 2048.0 * std::f32::consts::TAU * (30.0 / TICK_RATE as f32);
                self.spin = (per_tick, (frames as f64 * TICK_RATE / 30.0) as u32);
            }
            ObjRequest::TriggerNamed(name) => self.trigger_named(&name),
            ObjRequest::Audio(event) => self.audio.push(event),
            ObjRequest::BotSound { name, pos, region } => {
                if !self.fixes || self.in_earshot(pos, region) {
                    self.audio.push(AudioEvent::sound_at(name, pos));
                }
            }
            ObjRequest::CopyWall { target, source } => self.copy_wall(&target, &source),
            ObjRequest::MorphPlayer { height, speed } => {
                let mut p = self.physics_override.unwrap_or_default();
                p.height = height.max(8.0);
                if speed > 0.0 {
                    p.walk_speed = speed;
                }
                self.physics_override = Some(p);
            }
            ObjRequest::Fly { on, height } => {
                self.player.flying = on;
                self.player.vz = 0.0;
                if let Some(h) = height {
                    let mut p = self.physics_override.unwrap_or_default();
                    p.height = h;
                    self.physics_override = Some(p);
                }
            }
            ObjRequest::View { name, on } => {
                if let Some(tv) = self.tvs.iter_mut().find(|t| t.name.eq_ignore_ascii_case(&name)) {
                    tv.on = on;
                }
                self.apply_tv_textures();
            }
            ObjRequest::Overlay { tex, x, y, on } => {
                let Some(tex) = tex else { return };
                self.overlays.retain(|o| o.0 != tex);
                if on {
                    let anim = self.level.sequences.get(tex).and_then(Option::as_ref).map(|s| crate::anim::Player::start(s, 0));
                    self.overlays.push((tex, x, y, anim));
                }
            }
            ObjRequest::Flash(on) => self.flash = on,
            ObjRequest::CarryRegion { region, dx, dy } => self.carry_translation(region, dx, dy),
        }
    }

    /// One game tick in the BSP world: the orb is frozen; tags fired there still reach orb
    /// objects at once; the orb player's facing follows the BSP view.
    fn bsp_tick(&mut self, input: &Input) {
        use crate::bsp_world::{BspEvent, BspInput};
        let Some(world) = &mut self.bsp else {
            self.in_bsp = false;
            return;
        };
        let axis = |pos: Action, neg: Action, analog: f32| (input.held(pos) as i32 - input.held(neg) as i32) as f32 + analog;
        let bsp_input = BspInput {
            forward: axis(Action::Forward, Action::Back, input.move_axis.1),
            strafe: axis(Action::StrafeRight, Action::StrafeLeft, input.move_axis.0),
            turn: axis(Action::TurnLeft, Action::TurnRight, 0.0),
            jump: input.held(Action::Jump),
            fire: input.held(Action::Use),
        };
        world.look(-input.look_delta.0, -input.look_delta.1);
        let events = world.tick(1.0 / TICK_RATE as f32, &bsp_input);
        self.player.angle = world.player.orb_angle();
        self.previous = self.player;
        for e in events {
            match e {
                BspEvent::Tag(t) => self.fire_tag(t),
                BspEvent::Exit(object) => self.leave_bsp(object),
            }
        }
    }

    /// ORORB: back to the orb onto the exit object (as OTELE moves to an object).
    fn leave_bsp(&mut self, object: usize) {
        let Some(o) = self.objects.list.get(object) else { return };
        let (x, y, z, angle, region) = (o.x, o.y, o.z, o.angle, o.region);
        self.teleport(x, y, Some(z), Some(angle), region);
        self.in_bsp = false;
        self.region_background(self.player.region);
        log::info!("back from the BSP world at ({x}, {y}, {z})");
    }

    /// The background sound of `region` (region record +0x100), as on entering it.
    pub fn region_background(&mut self, region: usize) {
        let sound = self
            .level
            .orb
            .records(Section::Regions)
            .get(region)
            .and_then(|r| usize::try_from(r.le_i32(0x100)).ok())
            .and_then(|s| self.level.orb.records(Section::Sounds).get(s))
            .map(|rec| c_name(&rec[0x0c..0x18]))
            .filter(|n| !n.is_empty());
        if sound.is_some() {
            self.set_background(sound);
        }
    }

    /// A region's (or the BSP stay's) background sound: `BACKSND` goes back to the level's
    /// own, `BSNDOFF` silences it, any other name replaces it.
    fn set_background(&mut self, name: Option<String>) {
        let Some(name) = name else { return };
        let target = if name.eq_ignore_ascii_case("BACKSND") {
            self.default_background.clone()
        } else if name.eq_ignore_ascii_case("BSNDOFF") {
            None
        } else {
            Some(name)
        };
        self.play_background(target);
    }

    /// Make `target` the one background playing (MIDI pieces through the music player,
    /// recordings looped), stopping the previous one.
    fn play_background(&mut self, target: Option<String>) {
        let same = match (&self.background, &target) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.audio.push(AudioEvent::Stop { channel: BACKGROUND_CHANNEL });
        self.audio.push(AudioEvent::Music { name: None, looped: false });
        if let Some(name) = &target {
            if self.midi_only.contains(&name.to_ascii_uppercase()) {
                self.audio.push(AudioEvent::Music { name: Some(name.clone()), looped: true });
            } else {
                self.audio.push(AudioEvent::Loop { channel: BACKGROUND_CHANNEL, name: name.clone(), volume: 1.0, pan: 0.0, pos: None });
            }
        }
        self.background = target;
    }

    /// g_fixes: a floor where landing kills (a trap pit like LIPS00's FALLZ) that was lowered
    /// rises back to its starting height once it has sat empty for 3 s, so stepping into the
    /// trap's trigger and back out does not leave the way on deadly for good.
    fn reset_traps(&mut self, now: f64, dt: f32) {
        if self.trap_floors.is_empty() {
            let triggers = self.effects.triggers();
            // Landing fires a tag that teleports the player (back to the start).
            let teleport_tags: Vec<i32> = self.objects.list.iter().filter(|o| o.effect == crate::objects::fx::OTELE && o.tag > 0).map(|o| o.tag).collect();
            self.trap_floors = triggers.iter().filter(|t| teleport_tags.contains(&t.1[2])).map(|t| (t.0, self.level.world.regions[t.0].floor, None)).collect();
            if self.trap_floors.is_empty() {
                self.trap_floors.push((usize::MAX, 0.0, None));
            }
        }
        for k in 0..self.trap_floors.len() {
            let (r, home, _) = self.trap_floors[k];
            if r == usize::MAX {
                continue;
            }
            let floor = self.level.world.regions[r].floor;
            let busy = self.player.region == r || self.objects.region_moving(r) || floor >= home - 0.01;
            if busy {
                self.trap_floors[k].2 = None;
                continue;
            }
            let since = *self.trap_floors[k].2.get_or_insert(now);
            if now - since >= 3.0 {
                let reg = &mut self.level.world.regions[r];
                reg.floor = (reg.floor + 200.0 / 1.2 * dt).min(home);
            }
        }
    }

    /// MoveObjToObject (object-effects.md OTELE): the player takes on the destination template's
    /// body: its flying flag (both ways: LIPS00's SWIMMER1 swims, SWIMMER2 walks again), and its
    /// height, radius and forward max speed unless they are "keep" (-1.0).
    fn take_on_template(&mut self, t: usize) {
        let Some(o) = self.objects.list.get(t) else { return };
        let keep = |v: f32| v < 0.0;
        let mut p = self.physics_override.unwrap_or_default();
        if !keep(o.height) && o.height > 0.0 {
            p.height = o.height;
        }
        if !keep(o.radius) && o.radius > 0.0 {
            p.radius = o.radius;
        }
        // (The vertical max speed is not taken: flying up and down goes at the player's fly
        // speeds (physics.md 5.5, ext+0x204/+0x208, 200), which the port keeps in fall_speed;
        // the swimmer's 150 would slow swimming below the original's.)
        let forward = o.max_speeds[0];
        if !keep(forward) && forward > 0.0 {
            p.walk_speed = forward;
        }
        let flying = o.flags & 0x2 != 0;
        self.physics_override = Some(p);
        if flying != self.player.flying {
            self.player.flying = flying;
            self.player.vz = 0.0;
        }
        self.previous = self.player;
    }

    fn teleport(&mut self, x: f32, y: f32, z: Option<f32>, angle: Option<f32>, region: Option<usize>) {
        let region = region.or_else(|| self.level.world.region_at_z(x, y, z)).unwrap_or(self.player.region);
        let left = self.player.region;
        self.player.x = x;
        self.player.y = y;
        self.player.region = region;
        self.player.z = z.unwrap_or_else(|| self.level.world.solid_floor(region).0).max(self.level.world.solid_floor(region).0);
        self.player.vz = 0.0;
        if let Some(a) = angle {
            self.player.angle = a;
        }
        self.previous = self.player;
        // Re-homing the player sends the leave and enter events like walking across
        // (world-effects.md §1: FUN_004d85a1), e.g. LIPS00's pit alcove opens SECRET2 on leave.
        if region != left {
            self.effects.on_player_events(&[PlayerEvent::LeftRegion(left), PlayerEvent::EnteredRegion(region)]);
        }
    }

    /// Run a console command line (INI command syntax).
    pub fn run_command(&mut self, line: &str) {
        self.script.run_line(line);
        self.process_script_events();
        self.process_effect_requests();
    }

    /// A world tag fired by a netplay peer (not re-shared).
    pub fn apply_shared_tag(&mut self, tag: u32) {
        self.script.activate_tag(tag);
        self.process_script_events();
        self.process_effect_requests();
        self.outgoing_tags.retain(|&t| t != tag);
    }

    /// Show or move a netplay peer, drawn with the level's NETPLAYR template.
    pub fn set_remote_player(&mut self, id: u8, x: f32, y: f32, z: f32, angle: f32, region: usize) {
        let sprite = match self.remote_players.get(&id) {
            Some(&s) => Some(s),
            None => {
                let template = self.objects.list.iter().find(|o| o.name == "NETPLAYR").and_then(|o| o.tex.map(|t| (t, o.height)));
                let fallback = self.objects.list.first().and_then(|o| o.tex.map(|t| (t, 48.0)));
                template.or(fallback).and_then(|(tex, height)| {
                    let sprite = crate::world::Sprite { x, y, z, angle, region, frames: Vec::new(), scale: 1.0, visible: true, height };
                    let index = self.level.add_sprite(sprite, tex)?;
                    self.remote_players.insert(id, index);
                    Some(index)
                })
            }
        };
        if let Some(s) = sprite.and_then(|s| self.level.world.sprites.get_mut(s)) {
            (s.x, s.y, s.z, s.angle, s.region) = (x, y, z, angle, region.min(self.level.world.regions.len().saturating_sub(1)));
            s.visible = true;
        }
    }

    pub fn remove_remote_player(&mut self, id: u8) {
        if let Some(s) = self.remote_players.remove(&id).and_then(|s| self.level.world.sprites.get_mut(s)) {
            s.visible = false;
        }
    }

    /// Objects (and the player) standing on `region`'s floor.
    fn on_floor_of(&self, region: usize) -> (Vec<usize>, bool) {
        let floor = self.level.world.regions[region].floor;
        let objects = (0..self.objects.list.len())
            .filter(|&i| {
                let o = &self.objects.list[i];
                o.region == Some(region) && o.z <= floor + 1.0 && !o.destroyed && o.carries.is_none()
            })
            .collect();
        let player = self.player.region == region && self.player.z <= floor + 1.0;
        (objects, player)
    }

    /// RROTATE carries what stands on the floor around the pivot.
    fn carry_rotation(&mut self, region: usize, pivot: (f32, f32), angle: f32) {
        let (objects, player) = self.on_floor_of(region);
        let (sin, cos) = angle.sin_cos();
        let turn = |x: f32, y: f32| {
            let (dx, dy) = (x - pivot.0, y - pivot.1);
            (pivot.0 + dx * cos - dy * sin, pivot.1 + dy * cos + dx * sin)
        };
        for i in objects {
            let o = &mut self.objects.list[i];
            (o.x, o.y) = turn(o.x, o.y);
            o.angle += angle;
        }
        if player {
            (self.player.x, self.player.y) = turn(self.player.x, self.player.y);
            self.player.angle = (self.player.angle + angle).rem_euclid(std::f32::consts::TAU);
            self.previous = self.player;
        }
    }

    /// RMOVE: the raft region moved with its carrier; carry its walls and riders.
    fn carry_translation(&mut self, region: usize, dx: f32, dy: f32) {
        let (objects, player) = self.on_floor_of(region);
        crate::effects::translate_region(&mut self.level.world, region, dx, dy);
        for i in objects {
            let o = &mut self.objects.list[i];
            o.x += dx;
            o.y += dy;
        }
        if player {
            self.player.x += dx;
            self.player.y += dy;
        }
    }

    /// A wall by its editor name (OWTOW targets and sources).
    pub fn wall_named(&self, name: &str) -> Option<usize> {
        use godd_formats::xrb::Section;
        self.level.orb.records(Section::Walls).iter().position(|w| c_name(&w[0xb0..0xb8]).eq_ignore_ascii_case(name))
    }

    /// OWTOW: copy look and behaviour (textures, solidity, light, effect) of one wall onto
    /// another, keeping the target's geometry and regions.
    fn copy_wall(&mut self, target: &str, source: &str) {
        let (Some(t), Some(s)) = (self.wall_named(target), self.wall_named(source)) else { return };
        let src = self.level.world.walls[s].clone();
        let w = &mut self.level.world.walls[t];
        w.upper = src.upper;
        w.middle = src.middle;
        w.lower = src.lower;
        w.blocking = src.blocking;
        w.masked = w.back.is_some() && src.middle.is_some();
        w.light = src.light;
        self.effects.copy_wall_effect(t, s);
    }

    /// Put TV pictures on (or take them off) their walls according to each view's state.
    pub fn apply_tv_textures(&mut self) {
        for tv in &self.tvs {
            for (wall, normal) in &tv.walls {
                let w = &mut self.level.world.walls[*wall];
                w.middle = if tv.on { Some(crate::world::Surface::new(tv.image)) } else { *normal };
            }
        }
    }

    /// Put the player at the named object (level start after an exit).
    pub fn place_player_at(&mut self, name: &str) {
        let upper = name.to_ascii_uppercase();
        if let Some(o) = self.objects.list.iter().find(|o| o.name == upper) {
            let (x, y, z, angle, region) = (o.x, o.y, o.z, o.angle, o.region);
            self.teleport(x, y, Some(z), Some(angle), region);
        }
    }

    fn region_named(&self, name: &str) -> Option<usize> {
        use godd_formats::xrb::Section;
        self.level.orb.records(Section::Regions).iter().position(|r| c_name(&r[0xa4..0xac]).eq_ignore_ascii_case(name))
    }

    /// A trigger (event 0x80) addressed to a wall or region by name, or an internal request.
    fn trigger_named(&mut self, name: &str) {
        if let Some(rest) = name.strip_prefix("@setheight:") {
            let (obj, region) = rest.split_once(':').unwrap_or(("0", ""));
            if let (Ok(obj), Some(r)) = (obj.parse::<usize>(), self.region_named(region)) {
                let ceiling = self.objects.list[obj].effect == 49;
                let reg = &self.level.world.regions[r];
                let current = if ceiling { reg.ceiling } else { reg.floor };
                self.objects.start_height_motion(obj, r, current);
            }
            return;
        }
        if let Some(region_name) = name.strip_prefix("@teleport:") {
            if let Some(r) = self.region_named(region_name) {
                let ((x0, y0), (x1, y1)) = self.level.world.regions[r].bounds;
                self.teleport((x0 + x1) / 2.0, (y0 + y1) / 2.0, None, None, Some(r));
            }
            return;
        }
        if let Some(r) = self.region_named(name) {
            self.effects.trigger_region(r);
        }
    }

    fn process_effect_requests(&mut self) {
        for _ in 0..16 {
            let requests = self.effects.take_requests();
            if requests.is_empty() {
                return;
            }
            for r in requests {
                match r {
                    EffectRequest::Tag(t) => {
                        // Wall switches and region triggers are world interactions: share them.
                        self.outgoing_tags.push(t);
                        self.script.activate_tag(t);
                    }
                    EffectRequest::Sound(name, region) => self.audio.push(match region.and_then(|r| self.level.world.regions.get(r)) {
                        Some(reg) => {
                            let ((x0, y0), (x1, y1)) = reg.bounds;
                            AudioEvent::sound_at(name, [(x0 + x1) / 2.0, (y0 + y1) / 2.0, (reg.floor + reg.ceiling) / 2.0])
                        }
                        None => AudioEvent::sound(name),
                    }),
                    EffectRequest::Rotated { region, pivot, angle } => self.carry_rotation(region, pivot, angle),
                }
            }
            self.process_script_events();
        }
    }

    /// Fire a tag from game code (effects, movers, combat).
    pub fn fire_tag(&mut self, tag: u32) {
        self.script.activate_tag(tag);
        self.process_script_events();
        self.process_effect_requests();
        self.process_object_requests();
    }

    pub fn show_message(&mut self, color: u8, text: impl Into<String>) {
        let ticks_left = (MESSAGE_SECONDS * self.message_scale * TICK_RATE) as u32;
        // Queue of 20; the HUD draws the five oldest.
        if self.messages.len() < 20 {
            self.messages.push(Message { color, text: text.into(), ticks_left });
        }
    }

    fn process_script_events(&mut self) {
        // Handling an event can fire more tags; loop until the queue is empty.
        for _ in 0..64 {
            let events = self.script.take_events();
            if events.is_empty() {
                return;
            }
            for event in events {
                match event {
                    ScriptEvent::Message { color, text } => self.show_message(color, text),
                    ScriptEvent::Text(text) => {
                        let ticks_left = (TEXT_SECONDS * TICK_RATE) as u32;
                        self.overlay_text = Some(Message { color: crate::script::DEFAULT_MESSAGE_COLOR, text, ticks_left });
                    }
                    ScriptEvent::Sound(name) => self.audio.push(AudioEvent::sound(name)),
                    ScriptEvent::WorldTag(tag) => self.dispatch_world_tag(tag),
                    ScriptEvent::Quit => self.quit_requested = true,
                    ScriptEvent::InitMover { object, script } => self.objects.init_mover(&object, &script),
                    ScriptEvent::ToolDef { kind, index, args } => self.objects.define_tool(&kind, index, &args),
                    ScriptEvent::ToolCost { index, args } => self.objects.define_tool_cost(index, &args),
                    ScriptEvent::InitParticles(_) => {}
                }
            }
        }
    }

    /// Objects, walls and regions listening on `tag` react (event 0x40). Effects are added
    /// as they are reverse engineered (re/engine/*-effects.md).
    fn dispatch_world_tag(&mut self, tag: u32) {
        self.menus.on_tag(tag, &mut self.save);
        self.effects.on_tag(tag);
        let now = self.ticks as f64 / TICK_RATE;
        self.objects.on_tag(tag, now);
    }
}

/// Playing time of every sound sample in the game folder (for repeats and sequences).
fn vfs_sound_lengths(vfs: &Vfs) -> std::collections::HashMap<String, f32> {
    vfs.names()
        .filter_map(|path| {
            let name = path.strip_prefix("snd/")?.strip_suffix(".wav")?.to_ascii_uppercase();
            Some((name, crate::audio::wav_seconds(vfs.read(path).ok()?)?))
        })
        .collect()
}

/// Find WTV walls (effect 202) and the views they show; each view gets an image.
fn setup_tvs(level: &mut Level) -> Vec<Tv> {
    use godd_formats::xrb::{RecordExt, Section};
    // (name, image size, camera object)
    let views: Vec<(String, (usize, usize), Option<usize>)> = level
        .orb
        .records(Section::Views)
        .iter()
        .map(|v| {
            (
                c_name(&v[0x5c..0x64]),
                ((v.le_u16(0x10) as usize).clamp(8, 512), (v.le_u16(0x12) as usize).clamp(8, 512)),
                usize::try_from(v.le_i32(0x2c)).ok(),
            )
        })
        .collect();
    let mut tvs: Vec<Tv> = Vec::new();
    let auxs = level.orb.records(Section::Auxs).to_vec();
    for (wi, w) in level.orb.records(Section::Walls).to_vec().iter().enumerate() {
        if w.le_i32(0xd0) != 202 {
            continue;
        }
        let Some(aux) = usize::try_from(w.le_i32(0xdc)).ok().and_then(|a| auxs.get(a)) else { continue };
        let view_name = c_name(&aux[0x0e + 12 * 9..0x0e + 12 * 10]);
        let Some((name, size, camera)) = views.iter().find(|v| v.0.eq_ignore_ascii_case(&view_name)).cloned() else { continue };
        let normal = level.world.walls[wi].middle;
        match tvs.iter_mut().find(|t| t.name == name) {
            Some(tv) => tv.walls.push((wi, normal)),
            None => {
                level.world.images.push(crate::world::Image::solid(size.0, size.1, 8));
                let image = level.world.images.len() - 1;
                tvs.push(Tv { name, size, camera, image, on: aux.le_i32(0x88) != 0, walls: vec![(wi, normal)] });
            }
        }
    }
    tvs
}

fn c_name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..end].iter().map(|&b| b as char).collect::<String>().trim().to_string()
}

fn read_text(vfs: &Vfs, path: &str) -> String {
    vfs.read(path).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default()
}

fn seed_from(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Audio channel of the background sound (object loops use 1000 and up).
const BACKGROUND_CHANNEL: u32 = 999;
/// g_fixes bot earshot: always within this distance, else within this many open portals.
const EARSHOT_NEAR: f32 = 320.0;
const EARSHOT_HOPS: u8 = 4;

/// Load the level's BSP world (`[startup] loadbspfilename`) with the level's ORORB exit cubes and
/// OPTAGSW tag cubes.
fn load_bsp(vfs: &Vfs, file: &str, objects: &Objects) -> Option<Box<crate::bsp_world::BspWorld>> {
    use crate::bsp_world::{BspWorld, Cube, goddpoly_palette};
    use crate::objects::fx;
    let path = if file.contains('.') { file.to_string() } else { format!("{file}.BSP") };
    let data = vfs.read(&path).map_err(|e| log::warn!("BSP {path}: {e}")).ok()?;
    let bsp = quake_bsp::Bsp::parse(data).map_err(|e| log::warn!("BSP {path}: {e}")).ok()?;
    let cubes = |effect: i32| -> Vec<Cube> {
        objects
            .list
            .iter()
            .enumerate()
            .filter(|(_, o)| o.effect == effect)
            .filter_map(|(i, o)| {
                let a = o.aux.as_ref()?;
                Some(Cube::new([a.ints[0], a.ints[1], a.ints[2], a.ints[3]], i, o.tag))
            })
            .collect()
    };
    log::info!("BSP world {path}: {} faces", bsp.world_faces.len());
    let mut world = BspWorld::new(bsp, goddpoly_palette(vfs), cubes(fx::ORORB), cubes(fx::OPTAGSW));
    world.colormap = crate::bsp_world::goddpoly_colormap(vfs);
    Some(Box::new(world))
}

/// Upper-case names of `snd/*.mid` files that have no `.wav` of the same name.
fn midi_only_names(vfs: &Vfs) -> std::collections::HashSet<String> {
    let upper: Vec<String> = vfs.names().map(str::to_ascii_uppercase).collect();
    let stem = |n: &str, ext: &str| n.strip_prefix("SND/").and_then(|r| r.strip_suffix(ext)).map(str::to_string);
    let wavs: std::collections::HashSet<String> = upper.iter().filter_map(|n| stem(n, ".WAV")).collect();
    upper.iter().filter_map(|n| stem(n, ".MID")).filter(|n| !wavs.contains(n)).collect()
}
