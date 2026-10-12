//! Menus and character save slots (re/engine/script.md §8, object-effects.md OMENU/OHOTSPOT).
//!
//! Menus are XRB objects: an OMENU listening on a tag (600 main menu, 610 character menu,
//! 620 quit menu) owns hotspot objects named `<prefix>1..n`; the INI's `[namedmessages]`
//! supplies the strings. Selecting a hotspot fires its tag, which reaches the GAME* objects
//! (new / select / copy / delete / save / go to game) and OQUIT.

use crate::Frame;
use crate::level::Level;
use crate::text::{GLYPH, draw_text, fill_rect, frame_rect};
use crate::world::Image;
use godd_formats::gam::{FLAG_ACTIVE, FLAG_CREATED, FLAG_SAVED, SaveFile};
use godd_formats::ini::Ini;
use godd_formats::xrb::{RecordExt, Section};
use std::collections::HashMap;

mod effect {
    pub const OQUIT: i32 = 64;
    pub const OCURSOR: i32 = 83;
    pub const OHOTSPOT: i32 = 84;
    pub const OMENU: i32 = 85;
    pub const GAMENEW: i32 = 259;
    pub const GAMEDEL: i32 = 260;
    pub const GAMECOPY: i32 = 261;
    pub const GAMESAVE: i32 = 262;
    pub const GAMECUR: i32 = 263;
    pub const GAMEQUIT: i32 = 264;
    pub const GAMEGOTO: i32 = 265;
}

const ITEM_COLOR: u8 = 92;
const SELECTED_COLOR: u8 = 176;
const PROMPT_BORDER: u8 = 176;
const PROMPT_TEXT: u8 = 92;

#[derive(Debug, Clone)]
struct Hotspot {
    key: String,
    x: i32,
    y: i32,
    slot: usize,
    tag: i32,
    /// 1 text, 4 save slot (shows the character name), 0 picture.
    kind: i32,
    sound: Option<String>,
}

#[derive(Debug, Clone)]
struct MenuDef {
    tag: i32,
    title: String,
    subtitle: String,
    /// x, y, w, h, tl_y, il_y, border colour, background colour.
    info: [i32; 8],
    title_pos: (i32, i32),
    title_color: u8,
    cursor: Option<String>,
    items: Vec<Hotspot>,
}

/// An action object (GAME*, OQUIT) listening on a tag.
#[derive(Debug, Clone)]
struct Action {
    effect: i32,
    tag: i32,
    kind: i32,
    ok_tag: i32,
    fail_tag: i32,
    orb: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Prompt {
    NewName { ok: i32, fail: i32 },
    CopyName { ok: i32, fail: i32 },
    ConfirmDelete { ok: i32, fail: i32 },
}

/// What the session must do after a menu action.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuRequest {
    Tag(u32),
    Sound(String),
    Message(String),
    /// Snapshot the player into the current slot and write the save file.
    SaveCurrent,
    /// Go to a level; `restore` = take position/states from the slot (saved game).
    Goto { level: String, slot: Option<usize>, restore: bool },
    Quit,
    SaveFileChanged,
}

pub struct Menus {
    menus: Vec<MenuDef>,
    actions: Vec<Action>,
    open: Option<usize>,
    selected: usize,
    last_slot: usize,
    pub current_slot: Option<usize>,
    pub prompt: Option<(Prompt, String)>,
    cursors: HashMap<String, Image>,
    requests: Vec<MenuRequest>,
    start_orb: String,
}

impl Menus {
    pub fn new(level: &Level, ini: &Ini, cursor_pics: &dyn Fn(&str) -> Option<Image>, start_orb: &str) -> Self {
        let messages: HashMap<String, String> = ini
            .section("namedmessages")
            .map(|s| s.entries.iter().map(|e| (e.key.to_ascii_uppercase(), e.value.clone())).collect())
            .unwrap_or_default();
        let lookup = |key: &str| messages.get(&key.to_ascii_uppercase()).cloned().unwrap_or_else(|| key.to_string());
        let auxs = level.orb.records(Section::Auxs);
        let slot_of = |aux: &[u8], k: usize| c_string(&aux[0x0e + 12 * k..0x0e + 12 * (k + 1)]);
        let objects = level.orb.records(Section::Objects);
        let named: HashMap<String, usize> = objects.iter().enumerate().map(|(i, o)| (c_string(&o[0x58..0x60]).to_ascii_uppercase(), i)).collect();
        let aux_of = |o: &[u8]| usize::try_from(o.le_i32(0x1b0)).ok().and_then(|a| auxs.get(a));
        let sound_name = |aux: &[u8]| {
            let s = slot_of(aux, 4);
            (!s.is_empty()).then_some(s)
        };

        let mut menus = Vec::new();
        let mut actions = Vec::new();
        let mut cursors = HashMap::new();
        for o in objects {
            let effect = o.le_i32(0x140);
            let tag = o.le_u16(0x46) as i16 as i32;
            let aux = aux_of(o);
            match effect {
                effect::OMENU => {
                    let Some(aux) = aux else { continue };
                    let info_text = lookup(&slot_of(aux, 7));
                    let mut info = [12, 4, 230, 146, 32, -1, 199, 207];
                    for (d, v) in info.iter_mut().zip(info_text.split(',')) {
                        *d = v.trim().parse().unwrap_or(*d);
                    }
                    let prefix = slot_of(aux, 9).to_ascii_uppercase();
                    let mut items = Vec::new();
                    for n in 1.. {
                        let Some(&h) = named.get(&format!("{prefix}{n}")) else { break };
                        let ho = &objects[h];
                        let Some(ha) = aux_of(ho) else { continue };
                        items.push(Hotspot {
                            key: lookup(&slot_of(ha, 5)),
                            x: ha.le_i32(0x88),
                            y: ha.le_i32(0x8c),
                            slot: ha.le_i32(0x90).max(0) as usize,
                            tag: ha.le_i32(0x94),
                            kind: ho.le_i32(0x54),
                            sound: sound_name(ha),
                        });
                    }
                    let cursor_name = slot_of(aux, 8).to_ascii_uppercase();
                    let cursor = named.get(&cursor_name).and_then(|&c| aux_of(&objects[c])).map(|a| slot_of(a, 9));
                    if let Some(pic) = &cursor
                        && !cursors.contains_key(pic)
                            && let Some(img) = cursor_pics(pic) {
                                cursors.insert(pic.clone(), img);
                            }
                    menus.push(MenuDef {
                        tag,
                        title: lookup(&slot_of(aux, 5)),
                        subtitle: lookup(&slot_of(aux, 6)),
                        info,
                        title_pos: (aux.le_i32(0x8c), aux.le_i32(0x90)),
                        title_color: aux.le_i32(0x94).clamp(0, 255) as u8,
                        cursor,
                        items,
                    });
                }
                effect::GAMENEW | effect::GAMEDEL | effect::GAMECOPY | effect::GAMESAVE | effect::GAMECUR | effect::GAMEGOTO
                | effect::GAMEQUIT | effect::OQUIT => {
                    actions.push(Action {
                        effect,
                        tag,
                        kind: o.le_i32(0x54),
                        ok_tag: aux.map_or(0, |a| a.le_i32(0x94)),
                        fail_tag: aux.map_or(0, |a| a.le_i32(0x98)),
                        orb: aux.map(|a| slot_of(a, 9)).filter(|s| !s.is_empty()),
                    });
                }
                effect::OCURSOR | effect::OHOTSPOT => {}
                _ => {}
            }
        }
        Menus {
            menus,
            actions,
            open: None,
            selected: 0,
            last_slot: 0,
            current_slot: None,
            prompt: None,
            cursors,
            requests: Vec::new(),
            start_orb: start_orb.to_string(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some() || self.prompt.is_some()
    }

    pub fn has_menu_for(&self, tag: i32) -> bool {
        self.menus.iter().any(|m| m.tag == tag)
    }

    pub fn take_requests(&mut self) -> Vec<MenuRequest> {
        std::mem::take(&mut self.requests)
    }

    fn tag(&mut self, t: i32) {
        if t > 0 {
            self.requests.push(MenuRequest::Tag(t as u32));
        }
    }

    /// A tag reached the world: open a menu or run GAME* actions.
    pub fn on_tag(&mut self, tag: u32, save: &mut SaveFile) {
        if let Some(i) = self.menus.iter().position(|m| m.tag == tag as i32) {
            self.open = Some(i);
            self.selected = 0;
        }
        let actions: Vec<Action> = self.actions.iter().filter(|a| a.tag == tag as i32).cloned().collect();
        for a in actions {
            self.run_action(&a, save);
        }
    }

    fn run_action(&mut self, a: &Action, save: &mut SaveFile) {
        match a.effect {
            effect::OQUIT | effect::GAMEQUIT => self.requests.push(MenuRequest::Quit),
            effect::GAMENEW => match save.free_slot() {
                Some(_) => self.prompt = Some((Prompt::NewName { ok: a.ok_tag, fail: a.fail_tag }, String::new())),
                None => {
                    self.requests.push(MenuRequest::Message("No Available Game Slots".into()));
                    self.tag(a.fail_tag);
                }
            },
            effect::GAMECUR => {
                let slot = self.last_slot;
                if (1..=save.slots.len()).contains(&slot) {
                    self.current_slot = Some(slot - 1);
                    let s = &save.slots[slot - 1];
                    if s.is_active() {
                        self.requests.push(MenuRequest::Message(format!("{}'s game Selected", s.name())));
                        self.tag(a.ok_tag);
                    } else {
                        self.requests.push(MenuRequest::Message("Open new game selected".into()));
                        self.tag(a.fail_tag);
                    }
                }
            }
            effect::GAMECOPY => {
                if self.current_slot.is_some() && save.free_slot().is_some() {
                    self.prompt = Some((Prompt::CopyName { ok: a.ok_tag, fail: a.fail_tag }, String::new()));
                }
            }
            effect::GAMEDEL => {
                if self.current_slot.is_some() {
                    self.prompt = Some((Prompt::ConfirmDelete { ok: a.ok_tag, fail: a.fail_tag }, String::new()));
                }
            }
            effect::GAMESAVE => {
                if self.current_slot.is_some() {
                    self.requests.push(MenuRequest::SaveCurrent);
                    self.tag(a.ok_tag);
                }
            }
            effect::GAMEGOTO => {
                if a.kind == 1 {
                    // Back to the game-select orb.
                    self.requests.push(MenuRequest::Goto { level: self.start_orb.to_ascii_uppercase(), slot: None, restore: false });
                    return;
                }
                let slot = self.current_slot;
                let saved = slot.is_some_and(|s| save.slots[s].is_saved());
                let level = match slot {
                    Some(s) if saved => save.slots[s].orb(),
                    _ => a.orb.clone().unwrap_or_else(|| "LIPS00".into()),
                };
                if let Some(s) = slot {
                    self.requests.push(MenuRequest::Message(format!("Going to game for {}", save.slots[s].name())));
                }
                self.requests.push(MenuRequest::Goto { level: level.to_ascii_uppercase(), slot, restore: saved });
            }
            _ => {}
        }
    }

    /// Handle input while a menu or prompt is open. Returns true when input was consumed.
    pub fn update(&mut self, input: &crate::Input, save: &mut SaveFile, states: &[i32; 16]) -> bool {
        use crate::Action as A;
        if let Some((prompt, text)) = &mut self.prompt {
            let prompt = prompt.clone();
            if let Prompt::ConfirmDelete { ok, fail } = prompt {
                if input.text.chars().any(|c| c.eq_ignore_ascii_case(&'y')) {
                    if let Some(s) = self.current_slot.take() {
                        let index = save.slots[s].index();
                        save.slots[s] = godd_formats::gam::Slot::empty(index);
                        self.requests.push(MenuRequest::SaveFileChanged);
                    }
                    self.prompt = None;
                    self.tag(ok);
                } else if !input.text.is_empty() || input.pressed(A::Menu) || input.pressed(A::MenuSelect) {
                    self.prompt = None;
                    self.tag(fail);
                }
                return true;
            }
            for c in input.text.chars() {
                match c {
                    '\u{8}' => {
                        text.pop();
                    }
                    c if !c.is_control() && text.chars().count() < 19 => text.push(c),
                    _ => {}
                }
            }
            if input.pressed(A::Menu) {
                let fail = match prompt {
                    Prompt::NewName { fail, .. } | Prompt::CopyName { fail, .. } => fail,
                    Prompt::ConfirmDelete { fail, .. } => fail,
                };
                self.prompt = None;
                self.tag(fail);
            } else if input.pressed(A::MenuSelect) && !text.trim().is_empty() {
                let name = text.trim().to_string();
                self.prompt = None;
                match prompt {
                    Prompt::NewName { ok, .. } => {
                        if let Some(s) = save.free_slot() {
                            let slot = &mut save.slots[s];
                            slot.set_flags(FLAG_ACTIVE | FLAG_CREATED);
                            slot.set_name(&name);
                            slot.set_orb(&self.start_orb);
                            slot.set_states(states);
                            self.current_slot = Some(s);
                            self.requests.push(MenuRequest::SaveFileChanged);
                            self.requests.push(MenuRequest::Message(format!("New game for {name}")));
                            self.tag(ok);
                        }
                    }
                    Prompt::CopyName { ok, .. } => {
                        if let (Some(src), Some(dst)) = (self.current_slot, save.free_slot()) {
                            let index = save.slots[dst].index();
                            let mut copy = save.slots[src].clone();
                            copy.raw[4..8].copy_from_slice(&index.to_le_bytes());
                            copy.set_name(&name);
                            save.slots[dst] = copy;
                            self.requests.push(MenuRequest::SaveFileChanged);
                            self.tag(ok);
                        }
                    }
                    Prompt::ConfirmDelete { .. } => {}
                }
            }
            return true;
        }
        let Some(m) = self.open else { return false };
        let count = self.menus[m].items.len();
        if count == 0 || input.pressed(A::Menu) {
            self.open = None;
            return true;
        }
        if input.pressed(A::MenuUp) {
            self.selected = (self.selected + count - 1) % count;
        }
        if input.pressed(A::MenuDown) {
            self.selected = (self.selected + 1) % count;
        }
        if input.pressed(A::MenuSelect) || input.pressed(A::Use) || input.pressed(A::Jump) {
            let item = self.menus[m].items[self.selected].clone();
            self.open = None;
            self.last_slot = item.slot;
            if let Some(s) = item.sound {
                self.requests.push(MenuRequest::Sound(s));
            }
            self.tag(item.tag);
        }
        true
    }

    pub fn draw(&self, frame: &mut Frame, save: &SaveFile) {
        let s = (frame.height / crate::CLASSIC_HEIGHT).max(1);
        let si = s as i32;
        // Centre the 320x200 layout horizontally on wide frames.
        let x0 = (frame.width as i32 - 320 * si) / 2;
        let at = |frame: &mut Frame, x: i32, y: i32, color: u8, text: &str| {
            draw_text(frame, x0 + x * si, y * si, s, color, text);
        };
        if let Some(m) = self.open {
            let menu = &self.menus[m];
            let [x, y, w, h, tl_y, il_y, border, bg] = menu.info;
            let longest = menu.title.chars().count().max(menu.subtitle.chars().count()) as i32;
            let w = w.max(8 * longest + 6);
            if bg > 0 {
                fill_rect(frame, x0 + x * si, y * si, (w * si) as usize, (h * si) as usize, bg as u8);
            }
            for k in 0..si {
                frame_rect(frame, x0 + x * si + k, y * si + k, (w * si - 2 * k) as usize, (h * si - 2 * k) as usize, border as u8);
            }
            if tl_y > 0 {
                fill_rect(frame, x0 + x * si, tl_y * si, (w * si) as usize, (2 * s).max(1), border as u8);
            }
            if il_y > 0 {
                fill_rect(frame, x0 + x * si, il_y * si, (w * si) as usize, (2 * s).max(1), border as u8);
            }
            let (tx, ty) = menu.title_pos;
            at(frame, tx, ty, menu.title_color, &menu.title);
            at(frame, tx, ty + 12, menu.title_color, &menu.subtitle);
            for (i, item) in menu.items.iter().enumerate() {
                let label = match item.kind {
                    4 if (1..=save.slots.len()).contains(&item.slot) && save.slots[item.slot - 1].is_active() => save.slots[item.slot - 1].name(),
                    _ => item.key.clone(),
                };
                let color = if i == self.selected { SELECTED_COLOR } else { ITEM_COLOR };
                at(frame, item.x, item.y, color, &label);
                if i == self.selected {
                    let cursor = menu.cursor.as_ref().and_then(|c| self.cursors.get(c));
                    let cx = item.x + (label.chars().count() * GLYPH) as i32 + 6;
                    match cursor {
                        Some(img) => blit(frame, img, x0 + cx * si, (item.y - 4) * si, s),
                        None => at(frame, cx, item.y, SELECTED_COLOR, "<"),
                    }
                }
            }
        }
        if let Some((prompt, text)) = &self.prompt {
            let label = match prompt {
                Prompt::NewName { .. } => "Enter Character Name:",
                Prompt::CopyName { .. } => "Character Name:",
                Prompt::ConfirmDelete { .. } => "Confirm: touch Y to Delete",
            };
            let line = format!("{label}{text}_");
            let width = (line.chars().count() * GLYPH + 6) * s;
            fill_rect(frame, x0 + si, 163 * si, width.min(318 * s), 12 * s, 0);
            frame_rect(frame, x0 + si, 163 * si, width.min(318 * s), 12 * s, PROMPT_BORDER);
            at(frame, 4, 166, PROMPT_TEXT, &line);
        }
    }
}

/// Draw an image with index-0 transparency at integer scale.
fn blit(frame: &mut Frame, img: &Image, x: i32, y: i32, s: usize) {
    for v in 0..img.height * s {
        for u in 0..img.width * s {
            let texel = img.pixels[(v / s) * img.width + u / s];
            let (px, py) = (x + u as i32, y + v as i32);
            if texel != 0 && px >= 0 && py >= 0 && (px as usize) < frame.width && (py as usize) < frame.height {
                frame.put(py as usize * frame.width + px as usize, texel);
            }
        }
    }
}

fn c_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..end].iter().map(|&b| b as char).collect::<String>().trim().to_string()
}

/// Snapshot the player into a save slot (GAMESAVE).
pub fn snapshot(slot: &mut godd_formats::gam::Slot, orb: &str, pos: (f32, f32, f32), angle: f32, states: &[i32; 16], inventory: &[String]) {
    slot.set_flags(slot.flags() | FLAG_SAVED | FLAG_ACTIVE);
    slot.set_orb(&orb.to_ascii_lowercase());
    slot.set_position(pos);
    slot.set_angle((angle.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU * 2048.0) as u16 & 0x7ff);
    slot.set_states(states);
    slot.set_inventory(inventory);
}
