//! End-to-end test of the relay path on one machine: a local HTTP+websocket server stands in for Supabase.
//! It answers GET /rest/v1/peers with the row for the asked pk, and relays broadcasts between websocket clients
//! on the same topic (not back to the sender), like Realtime with self:false. No network beyond 127.0.0.1.

#![cfg(test)]

use crate::chat::{Chat, Me};
use crate::relay::channel_name;
use crate::relay_chat::{self, Session};
use crate::relay_realtime::{self, Frame, Link};
use crate::relay_rest;
use crate::store::{self, Trusted};
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tungstenite::Message;

const PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";

pub(crate) fn noise_pair() -> snow::Keypair {
    snow::Builder::new(PATTERN.parse().unwrap()).generate_keypair().unwrap()
}

fn ed_pk_hex(seed: &[u8; 32]) -> String {
    store::to_hex(&SigningKey::from_bytes(seed).verifying_key().to_bytes())
}

/// Serves rows for the given (ed25519 pk, noise pk) pairs, and relays websocket broadcasts.
fn fake_supabase(rows: Vec<(String, String)>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hub: Arc<Mutex<Vec<(usize, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let ids = Arc::new(AtomicUsize::new(1));
    let rows = Arc::new(rows);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (hub, ids, rows) = (hub.clone(), ids.clone(), rows.clone());
            thread::spawn(move || handle(stream, &hub, &ids, &rows));
        }
    });
    port
}

fn handle(mut stream: TcpStream, hub: &Mutex<Vec<(usize, String)>>, ids: &AtomicUsize, rows: &[(String, String)]) {
    let mut head = [0u8; 4096];
    let mut seen = 0;
    while seen < head.len() {
        let read = stream.peek(&mut head[seen..]).unwrap_or(0);
        if read == 0 {
            return;
        }
        if head[..seen + read].windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        seen += read;
        thread::sleep(Duration::from_millis(5));
    }
    let text = String::from_utf8_lossy(&head).to_string();
    if text.starts_with("GET /rest/v1/peers") {
        let pk = text.split("pk=eq.").nth(1).map(|rest| rest[..64].to_string()).unwrap_or_default();
        let found: Vec<Value> = rows.iter().filter(|(row_pk, _)| *row_pk == pk).map(|(row_pk, noise)| json!({"pk": row_pk, "name": "Peer", "noise_pk": noise})).collect();
        let body = Value::Array(found).to_string();
        // Read the request head so the client sees a clean close.
        let mut consumed = Vec::new();
        let mut byte = [0u8; 1];
        while !consumed.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).unwrap_or(0) == 0 {
                break;
            }
            consumed.push(byte[0]);
        }
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let _ = stream.write_all(response.as_bytes());
        return;
    }
    stream.set_read_timeout(Some(Duration::from_millis(30))).unwrap();
    let Ok(mut socket) = tungstenite::accept(stream) else { return };
    let id = ids.fetch_add(1, Ordering::SeqCst);
    let mut cursor = 0;
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        match socket.read() {
            Ok(Message::Text(received)) => {
                if let Some(frame) = relay_realtime::decode(received.as_str()) {
                    if frame.event == "phx_join" {
                        let reply = Frame { topic: frame.topic.clone(), event: "phx_reply".into(), payload: json!({"status": "ok"}), reference: frame.reference.clone() };
                        let _ = socket.send(Message::Text(relay_realtime::encode(&reply).into()));
                    } else if frame.event == "broadcast" {
                        hub.lock().unwrap().push((id, received.as_str().to_string()));
                    }
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error)) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => return,
        }
        let pending: Vec<(usize, String)> = hub.lock().unwrap()[cursor..].to_vec();
        cursor += pending.len();
        for (origin, text) in pending {
            if origin != id {
                let _ = socket.send(Message::Text(text.into()));
            }
        }
    }
}

fn chat_for(name: &str, trusted: (&str, Vec<u8>)) -> Chat {
    let chat = Chat::new(Me { id: name.into(), name: name.into(), port: 1, key: vec![], secret: vec![] });
    chat.trusted.lock().unwrap().push(Trusted { name: trusted.0.into(), key: trusted.1, ip: "10.0.0.9".into() });
    chat
}

#[test]
fn a_message_goes_from_one_chat_to_the_other_through_the_relay() {
    let (alice_noise, bob_noise) = (noise_pair(), noise_pair());
    let (alice_seed, bob_seed) = ([7u8; 32], [8u8; 32]);
    let (alice_pk, bob_pk) = (ed_pk_hex(&alice_seed), ed_pk_hex(&bob_seed));
    let port = fake_supabase(vec![
        (alice_pk.clone(), store::to_hex(&alice_noise.public)),
        (bob_pk.clone(), store::to_hex(&bob_noise.public)),
    ]);
    let config = relay_rest::config(Some(&format!("http://127.0.0.1:{port}")), Some("dummy-test-key")).unwrap().unwrap();
    let alice_session = Session { seed: alice_seed, noise_secret: alice_noise.private.clone(), noise_public: alice_noise.public.clone() };
    let bob_session = Session { seed: bob_seed, noise_secret: bob_noise.private.clone(), noise_public: bob_noise.public.clone() };

    // Each side has verified the other with /trust.
    let alice_chat = chat_for("Alice", ("Bob", bob_noise.public.clone()));
    let bob_chat = chat_for("Bob", ("Alice", alice_noise.public.clone()));

    let channel = channel_name(&alice_noise.public, &bob_noise.public);
    let mut alice_link = Link::connect(&config, &channel).unwrap();
    let mut bob_link = Link::connect(&config, &channel).unwrap();

    relay_chat::send_text(&alice_chat, &config, &bob_pk, &alice_session, &bob_noise.public, &mut alice_link, "hello over relay").unwrap();

    let mut received = None;
    for _ in 0..200 {
        if let Some(line) = bob_link.poll_envelope().unwrap() {
            received = Some(line);
            break;
        }
    }
    let line = received.expect("Bob receives the envelope");
    let mut cache = crate::relay_replay::NonceCache::default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    assert!(relay_chat::receive(&bob_chat, &line, &bob_session, &alice_noise.public, &mut cache, now));
    assert!(!relay_chat::receive(&bob_chat, &line, &bob_session, &alice_noise.public, &mut cache, now), "the same envelope twice is refused");

    let lines = bob_chat.lines();
    let last = lines.last().unwrap();
    assert_eq!(last.sender.as_deref(), Some("Alice"));
    assert_eq!(last.text, "hello over relay");
    // Alice's own chat shows nothing from the relay (the LAN path prints her own line).
    assert!(alice_chat.lines().iter().all(|line| line.text != "hello over relay"));
}

#[test]
fn a_peer_that_is_not_verified_gets_nothing() {
    let (alice_noise, bob_noise, mallory_noise) = (noise_pair(), noise_pair(), noise_pair());
    let (alice_seed, bob_seed) = ([7u8; 32], [8u8; 32]);
    let (alice_pk, bob_pk) = (ed_pk_hex(&alice_seed), ed_pk_hex(&bob_seed));
    // Bob's row now claims a key Alice never verified.
    let port = fake_supabase(vec![
        (alice_pk, store::to_hex(&alice_noise.public)),
        (bob_pk.clone(), store::to_hex(&mallory_noise.public)),
    ]);
    let config = relay_rest::config(Some(&format!("http://127.0.0.1:{port}")), Some("dummy-test-key")).unwrap().unwrap();
    let alice_session = Session { seed: alice_seed, noise_secret: alice_noise.private.clone(), noise_public: alice_noise.public.clone() };
    let alice_chat = chat_for("Alice", ("Bob", bob_noise.public.clone()));
    let channel = channel_name(&alice_noise.public, &bob_noise.public);
    let mut alice_link = Link::connect(&config, &channel).unwrap();
    let result = relay_chat::send_text(&alice_chat, &config, &bob_pk, &alice_session, &bob_noise.public, &mut alice_link, "secret");
    assert!(result.is_err(), "not sent to an unverified row");
}
