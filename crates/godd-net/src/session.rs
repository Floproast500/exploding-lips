//! Glue between the UDP transport and a running [`godd_game::Game`]: peers' states become
//! remote player sprites, shared world tags are exchanged, the local state is sent ~20 Hz.

use crate::PlayerState;
use crate::udp::{Client, NetEvent, Server};
use godd_game::{Game, Vfs, session::Entry};

/// Seconds between state packets (~20 Hz).
const SEND_INTERVAL: f64 = 0.05;

pub enum Net {
    Host(Server),
    Join(Client),
}

impl NetSession {
    /// Also accept browser players over WebSocket (host only).
    pub fn enable_websocket(&mut self, port: u16) -> std::io::Result<std::net::SocketAddr> {
        match &mut self.net {
            Net::Host(s) => {
                let addr = s.enable_websocket(port)?;
                log::info!("accepting WebSocket players on {addr}");
                Ok(addr)
            }
            Net::Join(_) => Err(std::io::Error::other("only the host accepts WebSocket players")),
        }
    }

    /// The bound address when hosting (tests connect to it).
    pub fn local_addr(&self) -> Option<std::net::SocketAddr> {
        match &self.net {
            Net::Host(s) => s.local_addr().ok(),
            Net::Join(_) => None,
        }
    }
}

pub struct NetSession {
    net: Net,
    since_send: f64,
    tick: u32,
}

impl NetSession {
    pub fn host(port: u16, level: &str, name: &str) -> std::io::Result<Self> {
        let server = Server::bind(port, level, name)?;
        log::info!("hosting {level} on {}", server.local_addr()?);
        Ok(Self { net: Net::Host(server), since_send: 0.0, tick: 0 })
    }

    pub fn join(address: &str, name: &str) -> std::io::Result<Self> {
        log::info!("joining {address}");
        Ok(Self { net: Net::Join(Client::connect(address, name)?), since_send: 0.0, tick: 0 })
    }

    /// Exchange packets; call once per rendered frame with the elapsed time.
    pub fn update(&mut self, game: &mut Game, vfs: &Vfs, elapsed: f64) {
        let events = match &mut self.net {
            Net::Host(s) => s.poll(),
            Net::Join(c) => c.poll(),
        };
        for event in events {
            match event {
                NetEvent::Welcome { id, level } => {
                    log::info!("joined as player {id}, level {level}");
                    let current = game.session().map(|s| s.name.clone());
                    if current.as_deref() != Some(level.as_str())
                        && let Err(e) = game.start_level(vfs, &level, Entry::Fresh) {
                            log::error!("cannot load {level}: {e}");
                        }
                }
                NetEvent::Refused(reason) => log::error!("server refused: {reason}"),
                NetEvent::Joined { id, name } => {
                    if let Some(s) = game.session_mut() {
                        s.show_message(godd_game::script::DEFAULT_MESSAGE_COLOR, format!("{name} joined"));
                    }
                    log::info!("player {id} ({name}) joined");
                }
                NetEvent::Left(id) => {
                    if let Some(s) = game.session_mut() {
                        s.remove_remote_player(id);
                    }
                }
                NetEvent::State { id, state } => {
                    if let Some(s) = game.session_mut() {
                        s.set_remote_player(id, state.x, state.y, state.z, state.angle, state.region as usize);
                    }
                }
                NetEvent::Tag(tag) => {
                    if let Some(s) = game.session_mut() {
                        s.apply_shared_tag(tag);
                    }
                }
                NetEvent::Chat { text, .. } => {
                    if let Some(s) = game.session_mut() {
                        s.show_message(195, text);
                    }
                }
            }
        }

        let Some(session) = game.session_mut() else { return };
        let tags = std::mem::take(&mut session.outgoing_tags);
        let p = session.player;
        for tag in tags {
            match &mut self.net {
                Net::Host(s) => s.send_tag(tag),
                Net::Join(c) => c.send_tag(tag),
            }
        }
        self.since_send += elapsed;
        if self.since_send >= SEND_INTERVAL {
            self.since_send = 0.0;
            self.tick = self.tick.wrapping_add(1);
            let state = PlayerState { x: p.x, y: p.y, z: p.z, angle: p.angle, region: p.region as u16, action: 0 };
            match &mut self.net {
                Net::Host(s) => s.send_state(self.tick, state),
                Net::Join(c) => c.send_state(self.tick, state),
            }
        }
    }
}
