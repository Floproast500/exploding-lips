//! Netplay for the reimplementation (replaces the dead galaxygames.org host list).
//!
//! Model: client–server. Every peer simulates its own player and sends [`Message::State`]
//! about 20 times a second; the server relays states and is the authority for world tags
//! ([`Message::Tag`]) so doors, switches and pickups stay consistent. Messages are small
//! binary packets; reliable messages (join, tags, chat) are resent until acknowledged.
//!
//! The protocol is transport-agnostic ([`Message::encode`]/[`Message::decode`]); [`udp`]
//! implements it over UDP for native builds.

#[cfg(all(feature = "game", not(target_arch = "wasm32")))]
pub mod session;
#[cfg(not(target_arch = "wasm32"))]
pub mod udp;

/// Protocol version; peers with a different version are refused.
pub const PROTOCOL_VERSION: u16 = 1;
pub const DEFAULT_PORT: u16 = 27_960;

pub type PlayerId = u8;

/// A player's replicated state.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlayerState {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub angle: f32,
    pub region: u16,
    /// Animation action (0 idle, 1 walk, …) for remote sprite selection.
    pub action: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// Client → server: request to join.
    Hello { version: u16, name: String },
    /// Server → client: accepted, with the client's id and the level to load.
    Welcome { id: PlayerId, level: String },
    /// Server → client: refused (version mismatch, full).
    Refused { reason: String },
    /// Player state (client → server, relayed server → clients).
    State { id: PlayerId, tick: u32, state: PlayerState },
    /// A world tag fired (client → server request, server → all broadcast).
    Tag { tag: u32 },
    Chat { id: PlayerId, text: String },
    /// A peer joined (server → clients) with its name.
    Joined { id: PlayerId, name: String },
    Left { id: PlayerId },
    /// Acknowledges reliable packet `seq`.
    Ack { seq: u32 },
}

impl Message {
    /// Whether the transport must deliver this message reliably.
    pub fn reliable(&self) -> bool {
        !matches!(self, Message::State { .. } | Message::Ack { .. })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        match self {
            Message::Hello { version, name } => {
                w.u8(1);
                w.u16(*version);
                w.str(name);
            }
            Message::Welcome { id, level } => {
                w.u8(2);
                w.u8(*id);
                w.str(level);
            }
            Message::Refused { reason } => {
                w.u8(3);
                w.str(reason);
            }
            Message::State { id, tick, state } => {
                w.u8(4);
                w.u8(*id);
                w.u32(*tick);
                for v in [state.x, state.y, state.z, state.angle] {
                    w.f32(v);
                }
                w.u16(state.region);
                w.u8(state.action);
            }
            Message::Tag { tag } => {
                w.u8(5);
                w.u32(*tag);
            }
            Message::Chat { id, text } => {
                w.u8(6);
                w.u8(*id);
                w.str(text);
            }
            Message::Joined { id, name } => {
                w.u8(7);
                w.u8(*id);
                w.str(name);
            }
            Message::Left { id } => {
                w.u8(8);
                w.u8(*id);
            }
            Message::Ack { seq } => {
                w.u8(9);
                w.u32(*seq);
            }
        }
        w.0
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        let mut r = Reader(data);
        Some(match r.u8()? {
            1 => Message::Hello { version: r.u16()?, name: r.str()? },
            2 => Message::Welcome { id: r.u8()?, level: r.str()? },
            3 => Message::Refused { reason: r.str()? },
            4 => Message::State {
                id: r.u8()?,
                tick: r.u32()?,
                state: PlayerState { x: r.f32()?, y: r.f32()?, z: r.f32()?, angle: r.f32()?, region: r.u16()?, action: r.u8()? },
            },
            5 => Message::Tag { tag: r.u32()? },
            6 => Message::Chat { id: r.u8()?, text: r.str()? },
            7 => Message::Joined { id: r.u8()?, name: r.str()? },
            8 => Message::Left { id: r.u8()? },
            9 => Message::Ack { seq: r.u32()? },
            _ => return None,
        })
    }
}

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        let bytes = &s.as_bytes()[..s.len().min(255)];
        self.u8(bytes.len() as u8);
        self.0.extend_from_slice(bytes);
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f32(&mut self) -> Option<f32> {
        self.take(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        self.take(n).map(|b| String::from_utf8_lossy(b).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let msgs = [
            Message::Hello { version: PROTOCOL_VERSION, name: "tester".into() },
            Message::Welcome { id: 2, level: "LIPS00".into() },
            Message::Refused { reason: "full".into() },
            Message::State { id: 1, tick: 99, state: PlayerState { x: 1.5, y: -2.0, z: 160.0, angle: 3.0, region: 640, action: 1 } },
            Message::Tag { tag: 401 },
            Message::Chat { id: 1, text: "hi".into() },
            Message::Joined { id: 3, name: "bob".into() },
            Message::Left { id: 3 },
            Message::Ack { seq: 7 },
        ];
        for m in msgs {
            assert_eq!(Message::decode(&m.encode()), Some(m));
        }
        assert_eq!(Message::decode(&[200]), None);
        assert_eq!(Message::decode(&[4, 1]), None, "truncated");
    }
}
