//! Network side. One JSON object per UDP packet. One TCP connection carries one message:
//! - encrypted (default): a Noise IK handshake, then one encrypted JSON object. Only for verified peers.
//! - plain (only with "plaintext": true): one JSON object, as the Python 0.5.x versions sent it.
//!   Never used for a peer that has a key.

use crate::chat::{default_name, Chat, Line, Me, Peer, MAX_TEXT_CHARS};
use crate::i18n::t;
use crate::store::{self, Identity};
use crate::text::{clean_text, peer_name, plain_sender, sender_name, squeeze_spaces};
use serde_json::{json, Value};
use socket2::{Domain, Protocol, Socket, Type};
use std::io::{self, ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::path::Path;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const APP: &str = "herdr-talk";
pub const UDP_PORT: u16 = 47850; // discovery port, the same on every PC
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(2);
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_FRAME: usize = u16::MAX as usize; // a Noise frame carries a 2-byte length
const NOISE_PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
const SEND_TIMEOUT: Duration = Duration::from_secs(3);
/// Total time for one incoming connection: its first byte, the handshake and the message. Not per read.
const CONNECTION_TIME: Duration = Duration::from_secs(3);
/// Incoming connections handled at once, all addresses together. Each one beyond this is closed unread.
const MAX_CONNECTIONS: usize = 16;
/// Incoming connections handled at once from one address. Each one beyond this is closed unread.
const MAX_PER_IP: usize = 3;
const LOOP_POLL: Duration = Duration::from_millis(500); // how often loops check for the stop signal
const ACCEPT_POLL: Duration = Duration::from_millis(50);

/// Bind the sockets, then discover peers and receive messages in the background.
/// Keys and trusted peers live in the state folder; plain messages follow config.json.
pub fn start(name: Option<String>, tcp_port: u16, udp_port: u16) -> io::Result<Arc<Chat>> {
    start_in(name, tcp_port, udp_port, &store::state_dir(), store::plaintext_allowed())
}

/// start with the state folder and the plain-message choice given.
pub fn start_in(name: Option<String>, tcp_port: u16, udp_port: u16, state: &Path, plaintext: bool) -> io::Result<Arc<Chat>> {
    let server = TcpListener::bind(("0.0.0.0", tcp_port))?;
    server.set_nonblocking(true)?;
    let listener = discovery_socket(udp_port)?;
    let identity = match store::read_identity(state)? {
        Some(identity) => identity,
        None => {
            let keys = noise_builder().generate_keypair().map_err(noise_error)?;
            let identity = Identity { public: keys.public, secret: keys.private };
            store::write_identity(state, &identity)?;
            identity
        }
    };
    let me = Me {
        id: store::peer_id(&identity.public),
        name: name.unwrap_or_else(default_name),
        port: server.local_addr()?.port(),
        key: identity.public,
        secret: identity.secret,
    };
    let mut chat = Chat::new(me);
    chat.trusted = Mutex::new(store::read_trusted(state)?);
    chat.plaintext = plaintext;
    chat.state_dir = Some(state.to_path_buf());
    let chat = Arc::new(chat);
    let (receiver, announcer, peers) = (chat.clone(), chat.clone(), chat.clone());
    thread::spawn(move || receive_messages(server, receiver));
    thread::spawn(move || announce(udp_port, announcer));
    thread::spawn(move || listen_for_peers(listener, peers));
    Ok(chat)
}

/// Several chats on one PC share the discovery port, so the address is reusable.
fn discovery_socket(udp_port: u16) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::from(([0, 0, 0, 0], udp_port)).into())?;
    socket.set_read_timeout(Some(LOOP_POLL))?;
    Ok(socket.into())
}

/// Broadcast this instance's name, port and key every few seconds.
fn announce(udp_port: u16, chat: Arc<Chat>) {
    let Ok(socket) = UdpSocket::bind(("0.0.0.0", 0)) else { return };
    if socket.set_broadcast(true).is_err() {
        return;
    }
    while !chat.stop.is_set() {
        let me = chat.me.lock().unwrap().clone();
        let payload = json!({"app": APP, "id": me.id, "name": me.name, "port": me.port, "key": store::to_hex(&me.key)}).to_string();
        let _ = socket.send_to(payload.as_bytes(), ("255.255.255.255", udp_port));
        chat.stop.wait(ANNOUNCE_INTERVAL);
    }
}

/// Remember every other instance that announces itself.
fn listen_for_peers(socket: UdpSocket, chat: Arc<Chat>) {
    let mut buffer = [0; 4096];
    while !chat.stop.is_set() {
        match socket.recv_from(&mut buffer) {
            Ok((size, address)) => remember_peer(&chat, &buffer[..size], address.ip().to_string()),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return,
        }
    }
}

fn remember_peer(chat: &Chat, data: &[u8], ip: String) {
    let Ok(info) = serde_json::from_slice::<Value>(data) else { return };
    let id = info.get("id").and_then(Value::as_str).unwrap_or("");
    let port = info.get("port").and_then(Value::as_u64).and_then(|port| u16::try_from(port).ok()).unwrap_or(0);
    let me = chat.me.lock().unwrap().clone();
    // Another chat on this PC has our id and key, but another port.
    if info.get("app").and_then(Value::as_str) != Some(APP) || (id == me.id && port == me.port) {
        return;
    }
    let key = match info.get("key").and_then(Value::as_str) {
        Some(text) => match store::from_hex(text) {
            Some(key) if key.len() == 32 && store::peer_id(&key) == id => Some(key),
            _ => return, // a key that does not hash to the id is forged
        },
        // A key-derived id without its key is a peer stripping its key to force plain text.
        None if id.len() == 64 => return,
        None => None,
    };
    // This PC's own key belongs to another pane here. Only an announcement from a local address is taken.
    if key.as_deref() == Some(me.key.as_slice()) && !is_local_address(&ip) {
        return;
    }
    // A key the user trusts: an announcement may not rename its entry or move it to another address.
    let trusted = key.as_deref().is_some_and(|key| chat.trusts(key));
    let name = key.as_deref().and_then(|key| chat.trusted_name(key)).unwrap_or_else(|| peer_name(info.get("name").and_then(Value::as_str)));
    let peer = Peer { id: id.to_string(), name: name.clone(), ip: ip.clone(), port, seen: Instant::now(), key: key.clone() };
    let mut peers = chat.peers.lock().unwrap();
    if trusted {
        // Only refreshes an entry at the same address. A new address adds an entry; the old one stays as it was.
        // Another pane on this PC (own key, local address) may rename itself, since its key is ours.
        if let Some(known) = peers.iter_mut().find(|known| known.id == id && known.port == port && known.ip == ip) {
            known.seen = Instant::now();
            if key.as_deref() == Some(me.key.as_slice()) {
                known.name = name;
            }
            return;
        }
    } else if let Some(known) = peers.iter_mut().find(|known| known.id == id && known.port == port) {
        *known = peer;
        return;
    }
    peers.push(peer);
    drop(peers);
    chat.say(t("peer.joined", &[("name", &name), ("ip", &ip)]));
    if let Some(key) = key.filter(|key| !chat.trusts(key)) {
        // A new key from an address we trusted before is a changed key, not a new peer.
        let changed = chat.trusted.lock().unwrap().iter().any(|trusted| trusted.ip == ip);
        let code = store::fingerprint(&me.key, &key);
        if changed {
            chat.say(t("peer.key_changed", &[("name", &name), ("ip", &ip)]));
        } else {
            chat.say(t("peer.fingerprint", &[("name", &name), ("code", &code)]));
        }
    }
}

/// Accept TCP connections. Each connection carries one message.
fn receive_messages(server: TcpListener, chat: Arc<Chat>) {
    let open = Arc::new(Mutex::new(Open::default()));
    while !chat.stop.is_set() {
        match server.accept() {
            Ok((conn, address)) => {
                let ip = address.ip().to_string();
                // Connections from unknown addresses or over the rate limit are closed unread.
                if !chat.admit_sender(&ip) {
                    continue;
                }
                // Too many open connections, from this address or in all: close this one unread.
                let Some(slot) = Slot::take(&open, &ip) else { continue };
                let chat = chat.clone();
                thread::spawn(move || {
                    let _slot = slot; // given back when this thread ends
                    if let Ok(conn) = Deadline::new(conn, CONNECTION_TIME) {
                        receive_one(conn, &chat);
                    }
                });
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => chat.stop.wait(ACCEPT_POLL),
            Err(_) => return,
        }
    }
}

/// Open incoming connections, in all and per address.
#[derive(Default)]
struct Open {
    total: usize,
    per_ip: HashMap<String, usize>,
}

/// One open incoming connection. Given back when dropped.
struct Slot {
    open: Arc<Mutex<Open>>,
    ip: String,
}

impl Slot {
    fn take(open: &Arc<Mutex<Open>>, ip: &str) -> Option<Slot> {
        let mut counts = open.lock().unwrap();
        let from_ip = counts.per_ip.get(ip).copied().unwrap_or(0);
        if counts.total >= MAX_CONNECTIONS || from_ip >= MAX_PER_IP {
            return None;
        }
        counts.total += 1;
        counts.per_ip.insert(ip.to_string(), from_ip + 1);
        Some(Slot { open: open.clone(), ip: ip.to_string() })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut counts = self.open.lock().unwrap();
        counts.total -= 1;
        let left = counts.per_ip.get(&self.ip).copied().unwrap_or(1) - 1;
        if left == 0 {
            counts.per_ip.remove(&self.ip);
        } else {
            counts.per_ip.insert(self.ip.clone(), left);
        }
    }
}

/// A TCP connection that must finish within its time in total. Each read or write gets only the time left,
/// so a peer that drips bytes slowly cannot hold the thread.
struct Deadline {
    stream: TcpStream,
    until: Instant,
}

impl Deadline {
    fn new(stream: TcpStream, time: Duration) -> io::Result<Self> {
        stream.set_nonblocking(false)?;
        Ok(Deadline { stream, until: Instant::now() + time })
    }

    fn arm(&self) -> io::Result<()> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(ErrorKind::TimedOut, "connection took too long"));
        }
        self.stream.set_read_timeout(Some(left))?;
        self.stream.set_write_timeout(Some(left))
    }
}

impl Read for Deadline {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.arm()?;
        self.stream.read(buf)
    }
}

impl Write for Deadline {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.arm()?;
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

/// A plain message starts with '{'. Anything else is a Noise handshake.
fn receive_one(mut conn: impl Read + Write, chat: &Chat) {
    let mut first = [0; 1];
    if conn.read_exact(&mut first).is_err() {
        return;
    }
    let received = if first[0] == b'{' {
        if !chat.plaintext {
            return;
        }
        read_plain(&mut conn, first[0]).ok().map(|data| (data, None))
    } else {
        receive_encrypted(&mut conn, first[0], chat).ok().map(|(data, key)| (data, Some(key)))
    };
    if let Some((data, key)) = received {
        deliver(&data, key.as_deref(), chat);
    }
}

fn read_plain(conn: &mut impl Read, first: u8) -> io::Result<Vec<u8>> {
    let mut data = vec![first];
    let mut chunk = [0; 4096];
    while data.len() <= MAX_MESSAGE_BYTES {
        match conn.read(&mut chunk)? {
            0 => break,
            size => data.extend_from_slice(&chunk[..size]),
        }
    }
    if data.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(ErrorKind::InvalidData, "message too large"));
    }
    Ok(data)
}

/// Noise IK responder. The handshake shows the sender's key before we answer, so an unverified
/// sender gets no answer at all. Returns the message and the verified key.
fn receive_encrypted(conn: &mut (impl Read + Write), first: u8, chat: &Chat) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let mut second = [0; 1];
    conn.read_exact(&mut second)?;
    let hello = read_body(conn, usize::from(u16::from_be_bytes([first, second[0]])))?;
    let secret = chat.me.lock().unwrap().secret.clone();
    let mut handshake = noise_builder().local_private_key(&secret).map_err(noise_error)?.build_responder().map_err(noise_error)?;
    let mut buffer = vec![0; MAX_FRAME];
    handshake.read_message(&hello, &mut buffer).map_err(noise_error)?;
    let sender = handshake.get_remote_static().map(<[u8]>::to_vec).ok_or_else(|| invalid("no sender key"))?;
    if !chat.trusts(&sender) {
        return Err(invalid("sender is not verified"));
    }
    let n = handshake.write_message(&[], &mut buffer).map_err(noise_error)?;
    write_frame(conn, &buffer[..n])?;
    let mut session = handshake.into_transport_mode().map_err(noise_error)?;
    let frame = read_frame(conn)?;
    let n = session.read_message(&frame, &mut buffer).map_err(noise_error)?;
    Ok((buffer[..n].to_vec(), sender))
}

/// Show one message. With a key, the id must be that key's id. Without one, the id must not be a key-derived id.
fn deliver(data: &[u8], key: Option<&[u8]>, chat: &Chat) {
    let Ok(message) = serde_json::from_slice::<Value>(data) else { return };
    if message.get("app").and_then(Value::as_str) != Some(APP) {
        return;
    }
    let id = message.get("id").and_then(Value::as_str).unwrap_or("");
    let id_ok = match key {
        Some(key) => id == store::peer_id(key),
        None => id.len() != 64,
    };
    if !id_ok {
        return;
    }
    // A message of the wrong shape is dropped.
    let (Some(sender), Some(body)) = (message.get("from").and_then(Value::as_str), message.get("text").and_then(Value::as_str)) else {
        return;
    };
    // Rows are kept as sent, one per line, within the text limit. Each row is cleaned and spaces squeezed.
    let rows: Vec<String> = body.chars().take(MAX_TEXT_CHARS).collect::<String>()
        .split('\n')
        .map(|row| squeeze_spaces(&clean_text(row, MAX_TEXT_CHARS)))
        .collect();
    if rows.iter().all(|row| row.trim().is_empty()) {
        return;
    }
    let lines = match key {
        // Verified: the name the user verified, not the one the message claims. One line, as before.
        Some(key) => vec![Line { sender: Some(chat.trusted_name(key).unwrap_or_else(|| sender_name(sender))), text: squeeze_spaces(&rows.join(" ")), mine: false }],
        // Plain: no key to check. Every row carries the plain sender's label, so no row can pass for another sender.
        None => {
            let label = format!("{} ({})", plain_sender(sender), t("chat.plain", &[]));
            rows.into_iter().map(|row| Line { sender: Some(label.clone()), text: row, mine: false }).collect()
        }
    };
    chat.add(lines);
}

/// True for this PC's own addresses: loopback, and the address this PC uses to reach the LAN.
fn is_local_address(ip: &str) -> bool {
    let Ok(ip) = ip.parse::<IpAddr>() else { return false };
    if ip.is_loopback() {
        return true;
    }
    // Connecting a UDP socket sends nothing. It only picks the address this PC sends from.
    let Ok(probe) = UdpSocket::bind(("0.0.0.0", 0)) else { return false };
    probe.connect(("192.0.2.1", 9)).is_ok() && probe.local_addr().is_ok_and(|local| local.ip() == ip)
}

/// Send one message. A peer with a key only ever gets the encrypted form.
/// Whether a keyless peer may get plain text is decided by the caller (Chat).
pub fn send_to(peer: &Peer, me: &Me, text: &str) -> io::Result<()> {
    let address = (peer.ip.as_str(), peer.port).to_socket_addrs()?.next()
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "no address"))?;
    let mut conn = TcpStream::connect_timeout(&address, SEND_TIMEOUT)?;
    conn.set_read_timeout(Some(SEND_TIMEOUT))?;
    conn.set_write_timeout(Some(SEND_TIMEOUT))?;
    let message = json!({"app": APP, "id": me.id, "from": me.name, "text": text}).to_string();
    match &peer.key {
        Some(key) => send_encrypted(&mut conn, me, key, message.as_bytes())?,
        None => conn.write_all(message.as_bytes())?,
    }
    conn.shutdown(Shutdown::Write)
}

/// Noise IK initiator: handshake with the peer's key, then one encrypted message.
fn send_encrypted(conn: &mut (impl Read + Write), me: &Me, peer_key: &[u8], message: &[u8]) -> io::Result<()> {
    let builder = noise_builder().local_private_key(&me.secret).map_err(noise_error)?;
    let mut handshake = builder.remote_public_key(peer_key).map_err(noise_error)?.build_initiator().map_err(noise_error)?;
    let mut buffer = vec![0; MAX_FRAME];
    let n = handshake.write_message(&[], &mut buffer).map_err(noise_error)?;
    write_frame(conn, &buffer[..n])?;
    let reply = read_frame(conn)?;
    handshake.read_message(&reply, &mut buffer).map_err(noise_error)?;
    let mut session = handshake.into_transport_mode().map_err(noise_error)?;
    let n = session.write_message(message, &mut buffer).map_err(noise_error)?;
    write_frame(conn, &buffer[..n])
}

fn noise_builder<'a>() -> snow::Builder<'a> {
    snow::Builder::new(NOISE_PATTERN.parse().expect("built-in Noise pattern"))
}

fn noise_error(error: snow::Error) -> io::Error {
    io::Error::other(error.to_string())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message.to_string())
}

fn read_body(conn: &mut impl Read, length: usize) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; length];
    conn.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn read_frame(conn: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut length = [0; 2];
    conn.read_exact(&mut length)?;
    read_body(conn, usize::from(u16::from_be_bytes(length)))
}

fn write_frame(conn: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    let length = u16::try_from(bytes.len()).map_err(|_| io::Error::new(ErrorKind::InvalidInput, "frame too long"))?;
    conn.write_all(&length.to_be_bytes())?;
    conn.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Trusted;
    use std::fs;
    use std::io::Cursor;

    fn chat() -> Chat {
        Chat::new(Me { id: "me".into(), name: "Me".into(), port: 1, key: vec![], secret: vec![] })
    }

    fn texts(chat: &Chat) -> Vec<String> {
        chat.lines().into_iter().map(|line| line.render()).collect()
    }

    fn deliver_plain(chat: &Chat, payload: &str) {
        receive_one(Cursor::new(payload.as_bytes().to_vec()), chat);
    }

    fn chat_with_plain_text() -> Chat {
        let mut chat = chat();
        chat.plaintext = true;
        chat
    }

    fn folder() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("herdr-talk-net-{:016x}", fastrand::u64(..)))
    }

    #[test]
    fn valid_message_is_shown() {
        let chat = chat_with_plain_text();
        deliver_plain(&chat, r#"{"app":"herdr-talk","from":"Bob","text":"hi"}"#);
        assert_eq!(texts(&chat), [format!("Bob ({}): hi", t("chat.plain", &[]))]);
    }

    #[test]
    fn plain_messages_are_refused_unless_allowed() {
        let chat = chat();
        deliver_plain(&chat, r#"{"app":"herdr-talk","from":"Bob","text":"hi"}"#);
        assert!(texts(&chat).is_empty());
    }

    #[test]
    fn wrong_shapes_are_dropped() {
        let chat = chat_with_plain_text();
        for payload in [
            r#"{"app":"herdr-talk","from":"Bob","text":123}"#,
            r#"{"app":"herdr-talk","from":["x"],"text":"hi"}"#,
            r#"{"app":"other","from":"Bob","text":"hi"}"#,
            r#"{"app":"herdr-talk","from":"Bob","text":"   "}"#,
            "not json",
        ] {
            deliver_plain(&chat, payload);
        }
        assert!(texts(&chat).is_empty());
    }

    #[test]
    fn oversized_message_is_dropped() {
        let chat = chat_with_plain_text();
        let text = "x".repeat(MAX_MESSAGE_BYTES);
        deliver_plain(&chat, &json!({"app": APP, "from": "Bob", "text": text}).to_string());
        assert!(texts(&chat).is_empty());
    }

    #[test]
    fn control_characters_are_replaced() {
        let chat = chat_with_plain_text();
        deliver_plain(&chat, &json!({"app": APP, "from": "Bob", "text": "a\x1b[31mred\x07"}).to_string());
        let lines = texts(&chat);
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].contains(['\x1b', '\x07']));
    }

    #[test]
    fn sender_of_only_control_characters_keeps_the_spaces() {
        let chat = chat_with_plain_text();
        deliver_plain(&chat, &json!({"app": APP, "from": "\x07\x02", "text": "hi"}).to_string());
        // A sender of only blanks is unknown, as a sender of nothing is.
        assert_eq!(texts(&chat), [format!("unknown ({}): hi", t("chat.plain", &[]))]);
    }

    #[test]
    fn plain_sender_colons_and_space_runs_are_removed() {
        let chat = chat_with_plain_text();
        deliver_plain(&chat, &json!({"app": APP, "from": "Bo:b   x", "text": "hi    there"}).to_string());
        assert_eq!(texts(&chat), [format!("Bob x ({}): hi there", t("chat.plain", &[]))]);
    }

    #[test]
    fn every_row_of_a_plain_message_carries_its_sender() {
        // A second row that starts like another sender must still read as this plain sender's row.
        let chat = chat_with_plain_text();
        deliver_plain(&chat, &json!({"app": APP, "from": "Bob", "text": "first\nBob: forged"}).to_string());
        let label = format!("Bob ({})", t("chat.plain", &[]));
        assert_eq!(texts(&chat), [format!("{label}: first"), format!("{label}: Bob: forged")]);
    }

    #[test]
    fn blank_plain_message_is_dropped() {
        let chat = chat_with_plain_text();
        deliver_plain(&chat, &json!({"app": APP, "from": "Bob", "text": "\n   \n"}).to_string());
        assert!(texts(&chat).is_empty());
    }

    #[test]
    fn own_key_is_taken_only_from_a_local_address() {
        let chat = Chat::new(Me { id: "me".into(), name: "Me".into(), port: 1, key: vec![7; 32], secret: vec![] });
        let packet = json!({"app": APP, "id": store::peer_id(&[7; 32]), "name": "Other", "port": 9, "key": store::to_hex(&[7; 32])}).to_string();
        remember_peer(&chat, packet.as_bytes(), "203.0.113.7".into());
        assert!(chat.peers.lock().unwrap().is_empty());
        remember_peer(&chat, packet.as_bytes(), "127.0.0.1".into());
        assert_eq!(chat.peers.lock().unwrap().len(), 1);
    }

    #[test]
    fn another_pane_on_this_pc_can_rename_itself() {
        let chat = Chat::new(Me { id: "me".into(), name: "Me".into(), port: 1, key: vec![7; 32], secret: vec![] });
        let packet = |name: &str| json!({"app": APP, "id": store::peer_id(&[7; 32]), "name": name, "port": 9, "key": store::to_hex(&[7; 32])}).to_string();
        remember_peer(&chat, packet("Old").as_bytes(), "127.0.0.1".into());
        remember_peer(&chat, packet("New").as_bytes(), "127.0.0.1".into());
        let peers = chat.peers.lock().unwrap();
        assert_eq!((peers.len(), peers[0].name.as_str()), (1, "New"));
    }

    #[test]
    fn only_loopback_and_this_pcs_own_address_are_local() {
        assert!(is_local_address("127.0.0.1"));
        assert!(!is_local_address("203.0.113.7"));
        assert!(!is_local_address("not an address"));
    }

    #[test]
    fn plain_message_cannot_use_a_key_derived_id() {
        // A key-derived id is the id of a Noise peer. Sent in plain text, it is a downgrade.
        let chat = chat_with_plain_text();
        let id = store::peer_id(&[5u8; 32]);
        deliver_plain(&chat, &json!({"app": APP, "id": id, "from": "Bob", "text": "hi"}).to_string());
        assert!(texts(&chat).is_empty());
    }

    #[test]
    fn join_notice_uses_the_cleaned_name_and_appears_once() {
        let chat = chat();
        let raw = format!("Li\x1bam{}", "z".repeat(60));
        let packet = json!({"app": APP, "id": "p1", "name": raw, "port": 52311}).to_string();
        remember_peer(&chat, packet.as_bytes(), "10.0.0.9".into());
        remember_peer(&chat, packet.as_bytes(), "10.0.0.9".into());
        let peers = chat.peers.lock().unwrap().clone();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].name, clean_text(&raw, 32));
        assert_eq!(texts(&chat).len(), 1);
        assert!(!texts(&chat)[0].contains('\x1b'));
    }

    #[test]
    fn own_and_foreign_packets_are_ignored() {
        let chat = chat();
        remember_peer(&chat, json!({"app": APP, "id": "me", "name": "Me", "port": 1}).to_string().as_bytes(), "10.0.0.1".into());
        remember_peer(&chat, json!({"app": "x", "id": "p", "name": "P"}).to_string().as_bytes(), "10.0.0.1".into());
        assert!(chat.peers.lock().unwrap().is_empty());
    }

    #[test]
    fn announced_key_must_hash_to_the_id() {
        let chat = chat();
        let key = [7u8; 32];
        let forged = json!({"app": APP, "id": "someone", "name": "X", "port": 1, "key": store::to_hex(&key)});
        remember_peer(&chat, forged.to_string().as_bytes(), "10.0.0.5".into());
        assert!(chat.peers.lock().unwrap().is_empty());
    }

    #[test]
    fn key_derived_id_without_a_key_is_refused() {
        // Stripping the key from an announcement must not turn a Noise peer into a plain one.
        let chat = chat();
        let id = store::peer_id(&[7u8; 32]);
        remember_peer(&chat, json!({"app": APP, "id": id, "name": "X", "port": 1}).to_string().as_bytes(), "10.0.0.5".into());
        assert!(chat.peers.lock().unwrap().is_empty());
    }

    #[test]
    fn new_key_from_a_trusted_address_is_a_changed_key() {
        let chat = chat();
        let old_key = [1u8; 32];
        let new_key = [2u8; 32];
        chat.trusted.lock().unwrap().push(Trusted { name: "Bob".into(), key: old_key.to_vec(), ip: "10.0.0.5".into() });
        let announce = json!({"app": APP, "id": store::peer_id(&new_key), "name": "Bob", "port": 9, "key": store::to_hex(&new_key)});
        remember_peer(&chat, announce.to_string().as_bytes(), "10.0.0.5".into());
        assert!(!chat.trusts(&new_key));
        assert!(texts(&chat).contains(&t("peer.key_changed", &[("name", "Bob"), ("ip", "10.0.0.5")])));
    }

    /// Two chats on this PC, each with its own state folder. Each trusts the other only when asked.
    fn pair(trust: bool) -> (Arc<Chat>, Arc<Chat>, Peer, Peer, std::path::PathBuf) {
        let base = folder();
        let (first, second) = (
            start_in(Some("A".into()), 0, 0, &base.join("a"), false).unwrap(),
            start_in(Some("B".into()), 0, 0, &base.join("b"), false).unwrap(),
        );
        let (me_first, me_second) = (first.me.lock().unwrap().clone(), second.me.lock().unwrap().clone());
        let to_second = Peer { id: me_second.id.clone(), name: "B".into(), ip: "127.0.0.1".into(), port: me_second.port, seen: Instant::now(), key: Some(me_second.key.clone()) };
        let to_first = Peer { id: me_first.id.clone(), name: "A".into(), ip: "127.0.0.1".into(), port: me_first.port, seen: Instant::now(), key: Some(me_first.key.clone()) };
        // The receiver only takes messages from addresses it has heard announce themselves.
        second.peers.lock().unwrap().push(to_first.clone());
        first.peers.lock().unwrap().push(to_second.clone());
        if trust {
            first.trusted.lock().unwrap().push(Trusted { name: "B".into(), key: me_second.key.clone(), ip: "127.0.0.1".into() });
            second.trusted.lock().unwrap().push(Trusted { name: "A".into(), key: me_first.key.clone(), ip: "127.0.0.1".into() });
        }
        (first, second, to_second, to_first, base)
    }

    fn wait_for_lines(chat: &Chat, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while chat.lines().len() < count && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn verified_peers_exchange_encrypted_messages() {
        let (first, second, to_second, _, base) = pair(true);
        let me = first.me.lock().unwrap().clone();
        send_to(&to_second, &me, "안녕").unwrap();
        wait_for_lines(&second, 1);
        assert_eq!(texts(&second), ["A: 안녕"]);
        first.stop.set();
        second.stop.set();
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn unverified_sender_gets_no_answer_and_no_message() {
        let (first, second, to_second, _, base) = pair(false);
        let me = first.me.lock().unwrap().clone();
        assert!(send_to(&to_second, &me, "hi").is_err());
        thread::sleep(Duration::from_millis(200));
        assert!(second.lines().is_empty());
        first.stop.set();
        second.stop.set();
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn encrypted_message_with_a_forged_id_is_dropped() {
        let (first, second, to_second, _, base) = pair(true);
        let forged = Me { id: "someone-else".into(), ..first.me.lock().unwrap().clone() };
        send_to(&to_second, &forged, "hi").unwrap();
        thread::sleep(Duration::from_millis(300));
        assert!(second.lines().is_empty());
        first.stop.set();
        second.stop.set();
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn two_panes_on_one_pc_share_the_identity_and_still_talk() {
        // Both panes use this PC's key, so each one sees the other as its own key. That must not block the chat.
        let base = folder();
        let first = start_in(Some("A".into()), 0, 0, &base, false).unwrap();
        let second = start_in(Some("B".into()), 0, 0, &base, false).unwrap();
        let (me_first, me_second) = (first.me.lock().unwrap().clone(), second.me.lock().unwrap().clone());
        assert_eq!(me_first.id, me_second.id);
        let to_second = Peer { id: me_second.id.clone(), name: "B".into(), ip: "127.0.0.1".into(), port: me_second.port, seen: Instant::now(), key: Some(me_second.key.clone()) };
        second.peers.lock().unwrap().push(Peer { id: me_first.id.clone(), name: "A".into(), ip: "127.0.0.1".into(), port: me_first.port, seen: Instant::now(), key: Some(me_first.key.clone()) });
        send_to(&to_second, &me_first, "same pc").unwrap();
        wait_for_lines(&second, 1);
        assert_eq!(texts(&second), ["A: same pc"]);
        first.stop.set();
        second.stop.set();
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn a_peer_with_a_key_never_gets_plain_text() {
        // The key says who the peer is. With the wrong key the handshake fails, and nothing falls back to plain text.
        let (first, second, to_second, _, base) = pair(false);
        let me = first.me.lock().unwrap().clone();
        let wrong_key = Peer { key: Some(vec![9; 32]), ..to_second };
        assert!(send_to(&wrong_key, &me, "secret").is_err());
        thread::sleep(Duration::from_millis(200));
        assert!(second.lines().is_empty());
        first.stop.set();
        second.stop.set();
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn verified_message_shows_the_verified_name_not_the_claimed_one() {
        // Sender 'first' is verified as "A". Its message claims to be "Mallory" and must show as "A".
        let (first, second, to_second, _, base) = pair(true);
        let me = first.me.lock().unwrap().clone();
        let claimed = json!({"app": APP, "id": me.id, "from": "Mallory", "text": "hi"}).to_string();
        let mut conn = TcpStream::connect(("127.0.0.1", to_second.port)).unwrap();
        send_encrypted(&mut conn, &me, to_second.key.as_deref().unwrap(), claimed.as_bytes()).unwrap();
        wait_for_lines(&second, 1);
        assert_eq!(texts(&second), ["A: hi"]);
        first.stop.set();
        second.stop.set();
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn announcement_cannot_move_a_verified_peer() {
        let chat = chat();
        let key = [3u8; 32];
        let id = store::peer_id(&key);
        chat.trusted.lock().unwrap().push(Trusted { name: "Bob".into(), key: key.to_vec(), ip: "10.0.0.5".into() });
        let announce = |name: &str| json!({"app": APP, "id": id, "name": name, "port": 9, "key": store::to_hex(&key)}).to_string();
        remember_peer(&chat, announce("Bob").as_bytes(), "10.0.0.5".into());
        remember_peer(&chat, announce("Mallory").as_bytes(), "10.0.0.5".into());
        remember_peer(&chat, announce("Mallory").as_bytes(), "10.0.0.66".into());
        let peers = chat.peers.lock().unwrap().clone();
        // Same address: only refreshed. Another address: a new entry. The old entry keeps its address and name.
        assert_eq!(peers.len(), 2);
        assert!(peers.iter().all(|peer| peer.name == "Bob"));
        assert_eq!(peers[0].ip, "10.0.0.5");
        assert_eq!(peers[1].ip, "10.0.0.66");
    }

    fn stream_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    #[test]
    fn a_silent_connection_is_cut_at_its_deadline() {
        let (_client, server) = stream_pair();
        let mut conn = Deadline::new(server, Duration::from_millis(200)).unwrap();
        let started = Instant::now();
        let mut byte = [0; 1];
        assert!(conn.read_exact(&mut byte).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_silent_peer_connection_is_closed_within_the_connection_time() {
        // Through the real accept loop: a known address that sends nothing is closed by the server.
        let chat = Arc::new(chat());
        chat.peers.lock().unwrap().push(Peer { id: "p".into(), name: "P".into(), ip: "127.0.0.1".into(), port: 1, seen: Instant::now(), key: None });
        let server = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        server.set_nonblocking(true).unwrap();
        let address = server.local_addr().unwrap();
        let worker = chat.clone();
        thread::spawn(move || receive_messages(server, worker));
        let mut client = TcpStream::connect(address).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let started = Instant::now();
        let mut byte = [0; 1];
        let read = client.read(&mut byte);
        assert!(matches!(read, Ok(0)), "expected the server to close: {read:?}");
        assert!(started.elapsed() < CONNECTION_TIME + Duration::from_secs(1));
        chat.stop.set();
    }

    #[test]
    fn the_deadline_covers_the_whole_connection_not_each_read() {
        // One byte every 100 ms: every read is quick, but the connection runs past its 300 ms in total.
        let (mut client, server) = stream_pair();
        let dripper = thread::spawn(move || {
            for _ in 0..50 {
                if client.write_all(b"x").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        let mut conn = Deadline::new(server, Duration::from_millis(300)).unwrap();
        let mut bytes = [0; 20];
        let started = Instant::now();
        assert!(conn.read_exact(&mut bytes).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(conn);
        dripper.join().unwrap();
    }

    #[test]
    fn connections_per_address_and_in_all_are_limited() {
        let open = Arc::new(Mutex::new(Open::default()));
        let held: Vec<Slot> = (0..MAX_PER_IP).map(|_| Slot::take(&open, "10.0.0.1").unwrap()).collect();
        assert!(Slot::take(&open, "10.0.0.1").is_none());
        assert!(Slot::take(&open, "10.0.0.2").is_some());
        drop(held);
        assert!(Slot::take(&open, "10.0.0.1").is_some());
        // Many addresses, one each: the total limit applies.
        let many: Vec<Slot> = (0..MAX_CONNECTIONS).map(|index| Slot::take(&open, &format!("10.1.0.{index}")).unwrap()).collect();
        assert!(Slot::take(&open, "10.9.9.9").is_none());
        drop(many);
        assert_eq!(open.lock().unwrap().total, 0);
        assert!(open.lock().unwrap().per_ip.is_empty());
    }
}
