//! UDP transport: a listen server (the host also plays as player 0) and clients. The server
//! can also accept WebSocket clients (browsers), which use the same messages without the
//! sequence header since TCP is already reliable.
//!
//! Packet = `seq: u32` (0 = unreliable) followed by an encoded [`Message`]. Reliable packets
//! are resent every [`RESEND`] until the peer sends [`Message::Ack`]; receivers drop
//! duplicates. Peers that stay silent for [`TIMEOUT`] are dropped. All sockets are
//! non-blocking: call `poll` once per game tick.

use crate::{DEFAULT_PORT, Message, PROTOCOL_VERSION, PlayerId, PlayerState};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::net::{SocketAddr, TcpListener, ToSocketAddrs, UdpSocket};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RESEND: Duration = Duration::from_millis(200);
const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PLAYERS: usize = 8;
const HOST_ID: PlayerId = 0;

struct Pending {
    seq: u32,
    to: SocketAddr,
    bytes: Vec<u8>,
    last_sent: Instant,
}

/// Reliability bookkeeping shared by server and client.
#[derive(Default)]
struct Link {
    next_seq: u32,
    pending: Vec<Pending>,
    seen: HashMap<SocketAddr, (HashSet<u32>, VecDeque<u32>)>,
}

impl Link {
    fn send(&mut self, socket: &UdpSocket, to: SocketAddr, msg: &Message) {
        let seq = if msg.reliable() {
            self.next_seq = self.next_seq.wrapping_add(1).max(1);
            self.next_seq
        } else {
            0
        };
        let mut bytes = seq.to_le_bytes().to_vec();
        bytes.extend(msg.encode());
        let _ = socket.send_to(&bytes, to);
        if seq != 0 {
            self.pending.push(Pending { seq, to, bytes, last_sent: Instant::now() });
        }
    }

    fn resend(&mut self, socket: &UdpSocket) {
        let now = Instant::now();
        for p in &mut self.pending {
            if now - p.last_sent >= RESEND {
                let _ = socket.send_to(&p.bytes, p.to);
                p.last_sent = now;
            }
        }
    }

    /// Read every waiting datagram; acks and duplicates are handled here.
    fn receive(&mut self, socket: &UdpSocket) -> Vec<(SocketAddr, Message)> {
        let mut out = Vec::new();
        let mut buf = [0u8; 1500];
        while let Ok((n, from)) = socket.recv_from(&mut buf) {
            if n < 5 {
                continue;
            }
            let seq = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
            let Some(msg) = Message::decode(&buf[4..n]) else { continue };
            if let Message::Ack { seq: acked } = msg {
                self.pending.retain(|p| !(p.seq == acked && p.to == from));
                continue;
            }
            if seq != 0 {
                let ack = Message::Ack { seq };
                let mut bytes = 0u32.to_le_bytes().to_vec();
                bytes.extend(ack.encode());
                let _ = socket.send_to(&bytes, from);
                let (set, order) = self.seen.entry(from).or_default();
                if !set.insert(seq) {
                    continue; // duplicate
                }
                order.push_back(seq);
                if order.len() > 512
                    && let Some(old) = order.pop_front() {
                        set.remove(&old);
                    }
            }
            out.push((from, msg));
        }
        out
    }

    fn forget(&mut self, peer: SocketAddr) {
        self.pending.retain(|p| p.to != peer);
        self.seen.remove(&peer);
    }
}

/// What the game should react to.
#[derive(Debug, Clone, PartialEq)]
pub enum NetEvent {
    Welcome { id: PlayerId, level: String },
    Refused(String),
    Joined { id: PlayerId, name: String },
    Left(PlayerId),
    State { id: PlayerId, state: PlayerState },
    Tag(u32),
    Chat { id: PlayerId, text: String },
}

struct Peer {
    id: PlayerId,
    name: String,
    last_seen: Instant,
    websocket: bool,
}

/// WebSocket side of the server: inbound messages and per-client outbound queues.
struct WebSockets {
    inbound: Receiver<(SocketAddr, Option<Vec<u8>>)>,
    outbound: Arc<Mutex<HashMap<SocketAddr, Sender<Vec<u8>>>>>,
}

/// Listen server. The host plays as player 0.
pub struct Server {
    socket: UdpSocket,
    link: Link,
    peers: HashMap<SocketAddr, Peer>,
    level: String,
    host_name: String,
    websockets: Option<WebSockets>,
}

impl Server {
    pub fn bind(port: u16, level: &str, host_name: &str) -> io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", if port == 0 { 0 } else { port }))?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            link: Link::default(),
            peers: HashMap::new(),
            level: level.to_string(),
            host_name: host_name.to_string(),
            websockets: None,
        })
    }

    /// Also accept browser clients over WebSocket on `port` (0 = any). Returns the address.
    pub fn enable_websocket(&mut self, port: u16) -> io::Result<SocketAddr> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        let addr = listener.local_addr()?;
        let (inbound_tx, inbound) = channel();
        let outbound: Arc<Mutex<HashMap<SocketAddr, Sender<Vec<u8>>>>> = Arc::default();
        let registry = outbound.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let Ok(peer) = stream.peer_addr() else { continue };
                let (tx, rx) = channel::<Vec<u8>>();
                registry.lock().expect("ws registry").insert(peer, tx);
                let inbound_tx = inbound_tx.clone();
                std::thread::spawn(move || websocket_client(stream, peer, rx, inbound_tx));
            }
        });
        self.websockets = Some(WebSockets { inbound, outbound });
        Ok(addr)
    }

    fn send_to(&mut self, to: SocketAddr, msg: &Message) {
        if self.peers.get(&to).is_some_and(|p| p.websocket) {
            if let Some(ws) = &self.websockets
                && let Some(tx) = ws.outbound.lock().expect("ws registry").get(&to) {
                    let _ = tx.send(msg.encode());
                }
        } else {
            self.link.send(&self.socket, to, msg);
        }
    }

    pub fn bind_default(level: &str, host_name: &str) -> io::Result<Self> {
        Self::bind(DEFAULT_PORT, level, host_name)
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn player_count(&self) -> usize {
        self.peers.len() + 1
    }

    fn broadcast(&mut self, msg: &Message, except: Option<SocketAddr>) {
        let targets: Vec<SocketAddr> = self.peers.keys().copied().filter(|a| Some(*a) != except).collect();
        for to in targets {
            self.send_to(to, msg);
        }
    }

    pub fn poll(&mut self) -> Vec<NetEvent> {
        let mut events = Vec::new();
        let mut incoming: Vec<(SocketAddr, Message, bool)> = self.link.receive(&self.socket).into_iter().map(|(a, m)| (a, m, false)).collect();
        if let Some(ws) = &self.websockets {
            while let Ok((from, data)) = ws.inbound.try_recv() {
                match data.as_deref().and_then(Message::decode) {
                    Some(msg) => incoming.push((from, msg, true)),
                    None if data.is_none() => {
                        // Closed connection.
                        if let Some(peer) = self.peers.get_mut(&from) {
                            peer.last_seen = Instant::now() - TIMEOUT - Duration::from_secs(1);
                        }
                    }
                    None => {}
                }
            }
        }
        for (from, msg, websocket) in incoming {
            if let Some(peer) = self.peers.get_mut(&from) {
                peer.last_seen = Instant::now();
            }
            match msg {
                Message::Hello { version, name } => {
                    if let Some(peer) = self.peers.get(&from) {
                        let welcome = Message::Welcome { id: peer.id, level: self.level.clone() };
                        self.send_to(from, &welcome);
                        continue;
                    }
                    // Register first so replies take the right transport.
                    self.peers.insert(from, Peer { id: 0, name: name.clone(), last_seen: Instant::now(), websocket });
                    if version != PROTOCOL_VERSION {
                        self.send_to(from, &Message::Refused { reason: format!("protocol {version}, server {PROTOCOL_VERSION}") });
                        self.peers.remove(&from);
                        continue;
                    }
                    let used: HashSet<PlayerId> = self.peers.iter().filter(|(a, _)| **a != from).map(|(_, p)| p.id).collect();
                    let Some(id) = (1..MAX_PLAYERS as PlayerId).find(|i| !used.contains(i)) else {
                        self.send_to(from, &Message::Refused { reason: "server full".into() });
                        self.peers.remove(&from);
                        continue;
                    };
                    self.peers.get_mut(&from).expect("registered").id = id;
                    self.send_to(from, &Message::Welcome { id, level: self.level.clone() });
                    // Tell the newcomer about everyone already here, then announce it.
                    self.send_to(from, &Message::Joined { id: HOST_ID, name: self.host_name.clone() });
                    let others: Vec<(PlayerId, String)> = self.peers.iter().filter(|(a, _)| **a != from).map(|(_, p)| (p.id, p.name.clone())).collect();
                    for (oid, oname) in others {
                        self.send_to(from, &Message::Joined { id: oid, name: oname });
                    }
                    self.broadcast(&Message::Joined { id, name: name.clone() }, Some(from));
                    events.push(NetEvent::Joined { id, name });
                }
                Message::State { tick, state, .. } => {
                    let Some(id) = self.peers.get(&from).map(|p| p.id) else { continue };
                    self.broadcast(&Message::State { id, tick, state }, Some(from));
                    events.push(NetEvent::State { id, state });
                }
                Message::Tag { tag } => {
                    if self.peers.contains_key(&from) {
                        self.broadcast(&Message::Tag { tag }, Some(from));
                        events.push(NetEvent::Tag(tag));
                    }
                }
                Message::Chat { text, .. } => {
                    let Some(id) = self.peers.get(&from).map(|p| p.id) else { continue };
                    self.broadcast(&Message::Chat { id, text: text.clone() }, Some(from));
                    events.push(NetEvent::Chat { id, text });
                }
                _ => {}
            }
        }
        // Drop silent peers.
        let now = Instant::now();
        let gone: Vec<SocketAddr> = self.peers.iter().filter(|(_, p)| now - p.last_seen > TIMEOUT).map(|(a, _)| *a).collect();
        for addr in gone {
            if let Some(peer) = self.peers.remove(&addr) {
                self.link.forget(addr);
                if let Some(ws) = &self.websockets {
                    ws.outbound.lock().expect("ws registry").remove(&addr);
                }
                self.broadcast(&Message::Left { id: peer.id }, None);
                events.push(NetEvent::Left(peer.id));
            }
        }
        self.link.resend(&self.socket);
        events
    }

    /// Send the host player's state to every client.
    pub fn send_state(&mut self, tick: u32, state: PlayerState) {
        self.broadcast(&Message::State { id: HOST_ID, tick, state }, None);
    }

    /// Broadcast a tag fired on the host.
    pub fn send_tag(&mut self, tag: u32) {
        self.broadcast(&Message::Tag { tag }, None);
    }

    pub fn send_chat(&mut self, text: &str) {
        self.broadcast(&Message::Chat { id: HOST_ID, text: text.to_string() }, None);
    }
}

/// One WebSocket client: forwards inbound binary messages and sends queued outbound ones.
fn websocket_client(stream: std::net::TcpStream, peer: SocketAddr, outbound: Receiver<Vec<u8>>, inbound: Sender<(SocketAddr, Option<Vec<u8>>)>) {
    let Ok(mut ws) = tungstenite::accept(stream) else {
        let _ = inbound.send((peer, None));
        return;
    };
    let _ = ws.get_mut().set_read_timeout(Some(Duration::from_millis(5)));
    loop {
        while let Ok(data) = outbound.try_recv() {
            if ws.send(tungstenite::Message::binary(data)).is_err() {
                let _ = inbound.send((peer, None));
                return;
            }
        }
        match ws.read() {
            Ok(tungstenite::Message::Binary(data)) => {
                let _ = inbound.send((peer, Some(data.to_vec())));
            }
            Ok(tungstenite::Message::Close(_)) => {
                let _ = inbound.send((peer, None));
                return;
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(_) => {
                let _ = inbound.send((peer, None));
                return;
            }
        }
    }
}

/// Client connected to a [`Server`].
pub struct Client {
    socket: UdpSocket,
    link: Link,
    server: SocketAddr,
    pub id: Option<PlayerId>,
    name: String,
    hello_sent: Option<Instant>,
}

impl Client {
    pub fn connect(address: &str, name: &str) -> io::Result<Self> {
        let with_port = if address.contains(':') { address.to_string() } else { format!("{address}:{DEFAULT_PORT}") };
        let server = with_port.to_socket_addrs()?.next().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no address"))?;
        let socket = UdpSocket::bind(if server.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" })?;
        socket.set_nonblocking(true)?;
        let mut client = Self { socket, link: Link::default(), server, id: None, name: name.to_string(), hello_sent: None };
        client.hello();
        Ok(client)
    }

    fn hello(&mut self) {
        let msg = Message::Hello { version: PROTOCOL_VERSION, name: self.name.clone() };
        self.link.send(&self.socket, self.server, &msg);
        self.hello_sent = Some(Instant::now());
    }

    pub fn poll(&mut self) -> Vec<NetEvent> {
        let mut events = Vec::new();
        for (from, msg) in self.link.receive(&self.socket) {
            if from != self.server {
                continue;
            }
            match msg {
                Message::Welcome { id, level } => {
                    if self.id.is_none() {
                        self.id = Some(id);
                        events.push(NetEvent::Welcome { id, level });
                    }
                }
                Message::Refused { reason } => events.push(NetEvent::Refused(reason)),
                Message::Joined { id, name } => events.push(NetEvent::Joined { id, name }),
                Message::Left { id } => events.push(NetEvent::Left(id)),
                Message::State { id, state, .. } => events.push(NetEvent::State { id, state }),
                Message::Tag { tag } => events.push(NetEvent::Tag(tag)),
                Message::Chat { id, text } => events.push(NetEvent::Chat { id, text }),
                _ => {}
            }
        }
        self.link.resend(&self.socket);
        events
    }

    pub fn send_state(&mut self, tick: u32, state: PlayerState) {
        if let Some(id) = self.id {
            self.link.send(&self.socket, self.server, &Message::State { id, tick, state });
        }
    }

    pub fn send_tag(&mut self, tag: u32) {
        self.link.send(&self.socket, self.server, &Message::Tag { tag });
    }

    pub fn send_chat(&mut self, text: &str) {
        let id = self.id.unwrap_or(0);
        self.link.send(&self.socket, self.server, &Message::Chat { id, text: text.to_string() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pump(server: &mut Server, clients: &mut [&mut Client]) -> (Vec<NetEvent>, Vec<Vec<NetEvent>>) {
        let mut s = Vec::new();
        let mut c = vec![Vec::new(); clients.len()];
        for _ in 0..50 {
            s.extend(server.poll());
            for (i, client) in clients.iter_mut().enumerate() {
                c[i].extend(client.poll());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        (s, c)
    }

    #[test]
    fn join_relay_states_and_tags_over_loopback() {
        let mut server = Server::bind(0, "LIPS00", "host").unwrap();
        let port = server.local_addr().unwrap().port();
        let mut a = Client::connect(&format!("127.0.0.1:{port}"), "alice").unwrap();
        let mut b = Client::connect(&format!("127.0.0.1:{port}"), "bob").unwrap();
        let (server_events, client_events) = pump(&mut server, &mut [&mut a, &mut b]);
        assert_eq!(server_events.iter().filter(|e| matches!(e, NetEvent::Joined { .. })).count(), 2);
        assert!(client_events[0].iter().any(|e| matches!(e, NetEvent::Welcome { level, .. } if level == "LIPS00")));
        assert_eq!(server.player_count(), 3);
        let (a_id, b_id) = (a.id.unwrap(), b.id.unwrap());
        assert_ne!(a_id, b_id);

        let state = PlayerState { x: 10.0, y: 20.0, z: 0.0, angle: 1.0, region: 5, action: 1 };
        a.send_state(1, state);
        a.send_tag(401);
        server.send_state(1, PlayerState { x: 1.0, ..state });
        let (server_events, client_events) = pump(&mut server, &mut [&mut a, &mut b]);
        assert!(server_events.contains(&NetEvent::State { id: a_id, state }));
        assert!(server_events.contains(&NetEvent::Tag(401)));
        assert!(client_events[1].contains(&NetEvent::State { id: a_id, state }), "b sees a");
        assert!(client_events[1].contains(&NetEvent::Tag(401)), "tag broadcast to b");
        assert!(client_events[0].iter().any(|e| matches!(e, NetEvent::State { id: 0, .. })), "a sees host");
        assert_eq!(client_events[1].iter().filter(|e| **e == NetEvent::Tag(401)).count(), 1, "reliable messages arrive once");
    }

    #[test]
    fn websocket_clients_join_and_relay() {
        let mut server = Server::bind(0, "LIPS00", "host").unwrap();
        let addr = server.enable_websocket(0).unwrap();
        let (mut ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{}", addr.port())).unwrap();
        if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
            s.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        }
        ws.send(tungstenite::Message::binary(Message::Hello { version: PROTOCOL_VERSION, name: "browser".into() }.encode())).unwrap();
        let mut joined = false;
        for _ in 0..200 {
            joined |= server.poll().iter().any(|e| matches!(e, NetEvent::Joined { name, .. } if name == "browser"));
            if joined {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(joined);
        let reply = ws.read().unwrap();
        let welcome = Message::decode(&reply.into_data()).unwrap();
        assert!(matches!(welcome, Message::Welcome { ref level, .. } if level == "LIPS00"));
        server.send_tag(77);
        for _ in 0..50 {
            server.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut got_tag = false;
        for _ in 0..10 {
            if let Ok(m) = ws.read() {
                got_tag |= Message::decode(&m.into_data()) == Some(Message::Tag { tag: 77 });
            }
        }
        assert!(got_tag, "tag relayed to the WebSocket client");
    }
}
