//! Supabase Realtime side of the relay: the websocket, the Phoenix messages, and the rules for sending and receiving.
//! Protocol (Phoenix channels, v1.0.0 JSON text frames): https://supabase.com/docs/guides/realtime/protocol
//! Public channels need `private: false` in the join config: https://supabase.com/docs/guides/realtime/broadcast
//! The channel is the envelope's `channel` (relay::channel_name). Messages are relayed and never stored.

use crate::relay::{self, Envelope, Reject};
use crate::relay_rest::{self, Config, Peer};
use serde_json::{json, Value};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

/// The event name for envelopes on the channel. Fixed.
pub const EVENT: &str = "envelope";
pub const VERSION: &str = "1.0.0";
/// The server drops idle sockets; the protocol asks for a heartbeat at least every 25 seconds.
const HEARTBEAT: Duration = Duration::from_secs(25);
const POLL: Duration = Duration::from_millis(500);
const JOIN_WAIT: Duration = Duration::from_secs(10);
const CONNECT_WAIT: Duration = Duration::from_secs(5);

/// Host and port of a websocket URL (default port 443 for wss, 80 for ws). Errors never contain the URL.
fn endpoint(url: &str) -> Result<(String, u16), String> {
    let (default_port, rest) = if let Some(rest) = url.strip_prefix("wss://") {
        (443, rest)
    } else if let Some(rest) = url.strip_prefix("ws://") {
        (80, rest)
    } else {
        return Err("websocket URL has no scheme".to_string());
    };
    let authority = rest.split('/').next().unwrap_or("");
    match authority.rsplit_once(':') {
        Some((host, port)) => Ok((host.to_string(), port.parse().map_err(|_| "websocket port is not a number".to_string())?)),
        None => Ok((authority.to_string(), default_port)),
    }
}

/// One Phoenix message (v1.0.0).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub topic: String,
    pub event: String,
    pub payload: Value,
    pub reference: Option<String>,
}

/// The channel topic on the wire: "realtime:" followed by the channel name (the SDK's convention).
pub fn topic_for(channel: &str) -> String {
    format!("realtime:{channel}")
}

/// The websocket URL: wss for https (ws for loopback http, tests only), with the publishable key in the query.
/// The key is checked for URL-safe characters first. Errors never contain the URL.
pub fn socket_url(config: &Config) -> Result<String, String> {
    let url = config.url();
    let rest = if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        return Err("relay URL has no scheme".to_string());
    };
    let key = config.key();
    if key.is_empty() || !key.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-') {
        return Err("relay key has characters that are not allowed in the URL".to_string());
    }
    Ok(format!("{rest}/realtime/v1/websocket?apikey={key}&vsn={VERSION}"))
}

pub fn encode(frame: &Frame) -> String {
    json!({
        "topic": frame.topic,
        "event": frame.event,
        "payload": frame.payload,
        "ref": frame.reference,
    })
    .to_string()
}

pub fn decode(text: &str) -> Option<Frame> {
    let value: Value = serde_json::from_str(text).ok()?;
    Some(Frame {
        topic: value["topic"].as_str()?.to_string(),
        event: value["event"].as_str()?.to_string(),
        payload: value.get("payload").cloned().unwrap_or(Value::Null),
        reference: match &value["ref"] {
            Value::String(text) => Some(text.clone()),
            Value::Null => None,
            _ => return None,
        },
    })
}

pub fn join_frame(topic: &str, reference: &str) -> Frame {
    Frame {
        topic: topic.to_string(),
        event: "phx_join".to_string(),
        payload: json!({
            "config": {
                "broadcast": {"ack": false, "self": false},
                "presence": {"key": ""},
                "private": false,
            }
        }),
        reference: Some(reference.to_string()),
    }
}

pub fn heartbeat_frame(reference: &str) -> Frame {
    Frame {
        topic: "phoenix".to_string(),
        event: "heartbeat".to_string(),
        payload: json!({}),
        reference: Some(reference.to_string()),
    }
}

/// A broadcast carrying one envelope line. The envelope is embedded as JSON, not as a string.
pub fn broadcast_frame(topic: &str, envelope_line: &str, reference: &str) -> Result<Frame, String> {
    let envelope: Value = serde_json::from_str(envelope_line).map_err(|_| "envelope is not JSON".to_string())?;
    Ok(Frame {
        topic: topic.to_string(),
        event: "broadcast".to_string(),
        payload: json!({"type": "broadcast", "event": EVENT, "payload": envelope}),
        reference: Some(reference.to_string()),
    })
}

/// The envelope line carried by a received frame on `topic`, if it is one (any other frame gives None).
pub fn envelope_of(frame: &Frame, topic: &str) -> Option<String> {
    if frame.topic != topic || frame.event != "broadcast" {
        return None;
    }
    if frame.payload["type"] != "broadcast" || frame.payload["event"] != EVENT {
        return None;
    }
    let envelope = &frame.payload["payload"];
    envelope.is_object().then(|| envelope.to_string())
}

/// May a message go to this peer? Only when the row's noise_pk is the key the user verified with /trust.
pub fn may_send(peer: Option<&Peer>, trusted_noise_pk: Option<&[u8]>) -> bool {
    match (peer, trusted_noise_pk) {
        (Some(peer), Some(trusted)) => relay_rest::valid_hex(&peer.noise_pk, 64) && peer.noise_pk == to_hex(trusted),
        _ => false,
    }
}

/// Accept a received envelope line only from the verified peer. `mine` and `theirs` are X25519 keys.
pub fn accept(line: &str, mine: &[u8], theirs: &[u8]) -> Result<Envelope, Reject> {
    let envelope = relay::open(line, (mine, theirs))?;
    if envelope.from != theirs {
        return Err(Reject::Party);
    }
    Ok(envelope)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// An open channel: the websocket, the joined topic, and the heartbeat timer.
pub struct Link {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    topic: String,
    next_ref: u64,
    last_beat: Instant,
}

impl Link {
    /// Connect, join the channel for `channel`, and wait for the join to be acknowledged.
    pub fn connect(config: &Config, channel: &str) -> Result<Link, String> {
        let url = socket_url(config)?;
        let (host, port) = endpoint(&url)?;
        // The TCP connect and the handshake each get CONNECT_WAIT; tungstenite::connect would wait as long as the OS does.
        let address = (host.as_str(), port).to_socket_addrs().ok().and_then(|mut found| found.next()).ok_or("websocket host not found")?;
        let stream = TcpStream::connect_timeout(&address, CONNECT_WAIT).map_err(|_| "websocket connect failed".to_string())?;
        stream.set_read_timeout(Some(CONNECT_WAIT)).and_then(|()| stream.set_write_timeout(Some(CONNECT_WAIT))).map_err(|_| "websocket timeout setup failed".to_string())?;
        let (socket, _response) = tungstenite::client_tls(url.as_str(), stream).map_err(|_| "websocket handshake failed".to_string())?;
        let mut link = Link { socket, topic: topic_for(channel), next_ref: 1, last_beat: Instant::now() };
        link.set_timeouts(POLL)?;
        let reference = link.reference();
        link.send(&join_frame(&link.topic, &reference))?;
        let deadline = Instant::now() + JOIN_WAIT;
        while Instant::now() < deadline {
            if let Some(frame) = link.read_frame()? {
                if frame.event == "phx_reply" && frame.reference.as_deref() == Some(reference.as_str()) {
                    return if frame.payload["status"] == "ok" { Ok(link) } else { Err("join refused".to_string()) };
                }
            }
        }
        Err("join not acknowledged".to_string())
    }

    /// Send one envelope line on the channel.
    pub fn send_envelope(&mut self, envelope_line: &str) -> Result<(), String> {
        let reference = self.reference();
        let frame = broadcast_frame(&self.topic, envelope_line, &reference)?;
        self.send(&frame)
    }

    /// Read at most one envelope for this channel, sending a heartbeat when one is due. None when nothing arrived.
    pub fn poll_envelope(&mut self) -> Result<Option<String>, String> {
        if self.last_beat.elapsed() >= HEARTBEAT {
            let reference = self.reference();
            self.send(&heartbeat_frame(&reference))?;
            self.last_beat = Instant::now();
        }
        Ok(self.read_frame()?.and_then(|frame| envelope_of(&frame, &self.topic)))
    }

    fn reference(&mut self) -> String {
        self.next_ref += 1;
        self.next_ref.to_string()
    }

    fn send(&mut self, frame: &Frame) -> Result<(), String> {
        self.socket.send(Message::Text(encode(frame).into())).map_err(|_| "websocket send failed".to_string())?;
        self.socket.flush().map_err(|_| "websocket send failed".to_string())
    }

    /// One frame, or None on a read timeout. A closed or broken socket is an error.
    fn read_frame(&mut self) -> Result<Option<Frame>, String> {
        match self.socket.read() {
            Ok(Message::Text(text)) => Ok(decode(text.as_str())),
            Ok(Message::Close(_)) => Err("connection closed".to_string()),
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(error)) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(None),
            Err(_) => Err("websocket read failed".to_string()),
        }
    }

    fn set_timeouts(&mut self, timeout: Duration) -> Result<(), String> {
        let stream = match self.socket.get_mut() {
            MaybeTlsStream::Plain(tcp) => tcp,
            MaybeTlsStream::Rustls(tls) => tls.get_mut(),
            _ => return Err("unsupported websocket stream".to_string()),
        };
        stream.set_read_timeout(Some(timeout)).map_err(|_| "websocket timeout setup failed".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Dummy values only. The loopback test uses a local listener and sends nothing to Supabase.
    const KEY_A: [u8; 32] = [1; 32];
    const KEY_B: [u8; 32] = [2; 32];

    fn config_at(url: &str) -> Config {
        relay_rest::config(Some(url), Some("dummy-test-key")).unwrap().unwrap()
    }

    fn envelope_line() -> String {
        relay::seal(&KEY_A, &KEY_B, &[0, 1, 2, 255]).unwrap()
    }

    #[test]
    fn socket_url_uses_wss_for_https_and_ws_for_loopback() {
        assert_eq!(
            socket_url(&config_at("https://example.supabase.co")).unwrap(),
            "wss://example.supabase.co/realtime/v1/websocket?apikey=dummy-test-key&vsn=1.0.0"
        );
        assert_eq!(socket_url(&config_at("http://127.0.0.1:9")).unwrap(), "ws://127.0.0.1:9/realtime/v1/websocket?apikey=dummy-test-key&vsn=1.0.0");
    }

    #[test]
    fn socket_url_refuses_keys_that_need_escaping() {
        let config = relay_rest::config(Some("https://example.supabase.co"), Some("a&b")).unwrap().unwrap();
        let error = socket_url(&config).unwrap_err();
        assert!(!error.contains("a&b") && !error.contains("example"), "{error}");
    }

    #[test]
    fn topic_is_realtime_and_the_channel_name() {
        assert_eq!(topic_for("abc"), "realtime:abc");
    }

    #[test]
    fn join_frame_has_the_public_channel_config() {
        let frame = join_frame("realtime:abc", "1");
        let value: Value = serde_json::from_str(&encode(&frame)).unwrap();
        assert_eq!(value["event"], "phx_join");
        assert_eq!(value["payload"]["config"]["private"], false);
        assert_eq!(value["payload"]["config"]["broadcast"]["self"], false);
        assert_eq!(value["ref"], "1");
    }

    #[test]
    fn heartbeat_frame_is_the_protocol_message() {
        let value: Value = serde_json::from_str(&encode(&heartbeat_frame("7"))).unwrap();
        assert_eq!(value, json!({"topic": "phoenix", "event": "heartbeat", "payload": {}, "ref": "7"}));
    }

    #[test]
    fn encode_then_decode_gives_the_same_frame() {
        let frame = broadcast_frame("realtime:abc", &envelope_line(), "3").unwrap();
        assert_eq!(decode(&encode(&frame)), Some(frame));
        assert_eq!(decode("not json"), None);
        assert_eq!(decode(r#"{"topic":"t"}"#), None);
    }

    #[test]
    fn a_broadcast_gives_back_the_envelope() {
        let line = envelope_line();
        let frame = broadcast_frame("realtime:abc", &line, "3").unwrap();
        let received = decode(&encode(&frame)).unwrap();
        let back = envelope_of(&received, "realtime:abc").unwrap();
        assert_eq!(serde_json::from_str::<Value>(&back).unwrap(), serde_json::from_str::<Value>(&line).unwrap());
        assert_eq!(envelope_of(&received, "realtime:other"), None);
        assert_eq!(envelope_of(&join_frame("realtime:abc", "1"), "realtime:abc"), None);
    }

    #[test]
    fn send_rule_needs_the_verified_noise_key() {
        let verified = [9u8; 32];
        let peer = Peer { pk: "11".repeat(32), name: "Bob".into(), noise_pk: to_hex(&verified) };
        assert!(may_send(Some(&peer), Some(&verified)));
        assert!(!may_send(Some(&peer), Some(&[8u8; 32])));
        assert!(!may_send(Some(&peer), None));
        assert!(!may_send(None, Some(&verified)));
    }

    #[test]
    fn receive_rule_needs_the_verified_sender() {
        let line = envelope_line();
        assert!(accept(&line, &KEY_B, &KEY_A).is_ok());
        // A different expected sender gives a different channel name, so open() refuses it first.
        assert_eq!(accept(&line, &KEY_B, &[3u8; 32]), Err(Reject::Channel));
        assert_eq!(accept("not json", &KEY_B, &KEY_A), Err(Reject::Format));
    }

    #[test]
    fn join_and_send_over_a_local_websocket() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let line = envelope_line();
        let server_line = line.clone();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let joined = decode(match socket.read().unwrap() {
                Message::Text(text) => text.as_str().to_string(),
                other => panic!("{other:?}"),
            }.as_str())
            .unwrap();
            assert_eq!(joined.event, "phx_join");
            let reply = Frame { topic: joined.topic.clone(), event: "phx_reply".into(), payload: json!({"status": "ok"}), reference: joined.reference.clone() };
            socket.send(Message::Text(encode(&reply).into())).unwrap();
            let sent = decode(match socket.read().unwrap() {
                Message::Text(text) => text.as_str().to_string(),
                other => panic!("{other:?}"),
            }
            .as_str())
            .unwrap();
            assert_eq!(envelope_of(&sent, &joined.topic).unwrap(), serde_json::from_str::<Value>(&server_line).unwrap().to_string());
            let back = broadcast_frame(&joined.topic, &server_line, "99").unwrap();
            socket.send(Message::Text(encode(&back).into())).unwrap();
            let _ = socket.close(None);
        });
        let mut link = Link::connect(&config_at(&format!("http://127.0.0.1:{port}")), "abc").unwrap();
        link.send_envelope(&line).unwrap();
        let mut got = None;
        for _ in 0..40 {
            if let Some(envelope) = link.poll_envelope().unwrap() {
                got = Some(envelope);
                break;
            }
        }
        server.join().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&got.unwrap()).unwrap(), serde_json::from_str::<Value>(&line).unwrap());
    }
}
