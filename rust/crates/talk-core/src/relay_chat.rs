//! Relay path of the chat (stage 1, Rust TUI). Runs beside the LAN path, never instead of it.
//!
//! Sending: the peer's row is read with get_peer; its noise_pk must be a key the user verified with /trust. Then the
//! text is sealed into one Noise IK message (the first message of IK carries the payload, so the receiver learns and
//! checks the sender's static key) and sent as an envelope on the channel channel_name(my noise key, peer noise key).
//! Receiving: relay::open and the verified-sender check (relay_realtime::accept), then Noise decryption, then one line.
//! Nothing is stored. Failures are silent on receive; on send a short notice says why nothing went out.

use crate::chat::{Chat, Line, MAX_TEXT_CHARS};
use crate::net::noise_builder;
use crate::relay;
use crate::relay_key;
use crate::relay_realtime::{self, Link};
use crate::relay_replay::NonceCache;
use crate::relay_rest::{self, Config, Peer};
use crate::relay_settings::{self, Settings};
use crate::store;
use crate::text::clean_text;
use ed25519_dalek::SigningKey;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const RETRY: Duration = Duration::from_secs(5);
const PEER_WAIT: Duration = Duration::from_secs(30);
const SLICE: Duration = Duration::from_millis(200);
const NOISE_MAX: usize = 65535;

/// This PC's relay identity: the Ed25519 public key (the row key) and the X25519 Noise public key (the envelope key).
pub fn my_keys(state: &Path) -> Result<(String, String), String> {
    let seed = relay_key::load_or_create(state)?;
    let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    let noise = store::read_identity(state).map_err(|error| error.to_string())?.ok_or("no identity yet: start a chat first")?;
    Ok((store::to_hex(&public), store::to_hex(&noise.public)))
}

/// Start the relay for the saved or environment settings, replacing any relay that runs. Ok(false): relay is off.
pub fn start(chat: &Arc<Chat>, state: &Path) -> Result<bool, String> {
    chat.stop_relay();
    let settings = relay_settings::effective(state);
    if settings.url.is_empty() && settings.key.is_empty() && settings.peer.is_empty() {
        chat.set_relay_status("");
        return Ok(false);
    }
    let Some(config) = relay_rest::config(Some(&settings.url), Some(&settings.key))? else {
        chat.set_relay_status("");
        return Ok(false);
    };
    if !relay_rest::valid_pk(&settings.peer) {
        chat.set_relay_status("relay: peer key missing");
        return Ok(false);
    }
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    chat.attach_relay(tx, stop.clone());
    let (chat, state) = (chat.clone(), state.to_path_buf());
    thread::spawn(move || run(&chat, &state, &config, &settings, &rx, &stop));
    Ok(true)
}

/// Keys and names the relay thread needs, all read once at start.
pub(crate) struct Session {
    pub(crate) seed: [u8; 32],
    pub(crate) noise_secret: Vec<u8>,
    pub(crate) noise_public: Vec<u8>,
}

fn session(state: &Path) -> Result<Session, String> {
    let seed = relay_key::load_or_create(state)?;
    let identity = store::read_identity(state).map_err(|error| error.to_string())?.ok_or("no identity yet: start a chat first")?;
    Ok(Session { seed, noise_secret: identity.secret, noise_public: identity.public })
}

fn run(chat: &Chat, state: &Path, config: &Config, settings: &Settings, rx: &Receiver<String>, stop: &AtomicBool) {
    // Nonces seen in this run. A restart empties it (see relay_replay).
    let mut cache = NonceCache::default();
    let session = match session(state) {
        Ok(session) => session,
        Err(error) => return chat.set_relay_status(&format!("relay: {error}")),
    };
    // Publish this PC's row (name and Noise key, signed). A failure is shown, and the chat continues.
    if relay_rest::valid_name(&settings.name) {
        let published = relay_rest::put_peer(config, &session.seed, &store::to_hex(&session.noise_public), &settings.name);
        if !published {
            chat.set_relay_status("relay: could not publish this PC's row");
        }
    }
    while !stop.load(Ordering::Relaxed) {
        let Some(peer) = relay_rest::get_peer(config, &settings.peer) else {
            chat.set_relay_status("relay: peer not found on the relay");
            wait(chat, rx, stop, PEER_WAIT);
            continue;
        };
        let Some(theirs) = verified_key(chat, &peer) else {
            chat.set_relay_status("relay: peer not verified yet (use /trust)");
            wait(chat, rx, stop, PEER_WAIT);
            continue;
        };
        let channel = relay::channel_name(&session.noise_public, &theirs);
        let mut link = match Link::connect(config, &channel) {
            Ok(link) => link,
            Err(error) => {
                chat.set_relay_status(&format!("relay: {error}, retrying"));
                wait(chat, rx, stop, RETRY);
                continue;
            }
        };
        chat.set_relay_status("relay: connected");
        while !stop.load(Ordering::Relaxed) {
            while let Ok(text) = rx.try_recv() {
                if let Err(error) = send_text(chat, config, &settings.peer, &session, &theirs, &mut link, &text) {
                    chat.say(error);
                }
            }
            match link.poll_envelope() {
                Ok(Some(line)) => {
                    receive(chat, &line, &session, &theirs, &mut cache, now_secs());
                }
                Ok(None) => {}
                Err(error) => {
                    chat.set_relay_status(&format!("relay: {error}, retrying"));
                    break;
                }
            }
        }
    }
}

/// The peer's Noise key from its row, when the user verified that key with /trust.
fn verified_key(chat: &Chat, peer: &Peer) -> Option<Vec<u8>> {
    let key = store::from_hex(&peer.noise_pk)?;
    chat.trusted_name(&key).is_some().then_some(key)
}

/// Sleep for `time`, in slices so a stop is seen. Texts typed meanwhile are not sent: a notice says so.
fn wait(chat: &Chat, rx: &Receiver<String>, stop: &AtomicBool, time: Duration) {
    let until = Instant::now() + time;
    while Instant::now() < until && !stop.load(Ordering::Relaxed) {
        while let Ok(_text) = rx.try_recv() {
            chat.say("relay: not connected, the message was not sent over the relay");
        }
        thread::sleep(SLICE);
    }
}

/// Re-check the row right before sending, then seal and send one envelope.
pub(crate) fn send_text(chat: &Chat, config: &Config, peer_pk: &str, session: &Session, theirs: &[u8], link: &mut Link, text: &str) -> Result<(), String> {
    let checked = relay_rest::get_peer(config, peer_pk).and_then(|peer| verified_key(chat, &peer));
    if checked.as_deref() != Some(theirs) {
        return Err("relay: peer not verified, not sent".to_string());
    }
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|_| "relay: no random source".to_string())?;
    let nonce = store::to_hex(&random);
    let payload = encode_payload(text, now_secs(), &nonce);
    let ciphertext = seal_payload(&session.noise_secret, theirs, &payload)?;
    let envelope = relay::seal(&session.noise_public, theirs, &ciphertext).ok_or("relay: bad key length")?;
    link.send_envelope(&envelope).map_err(|error| format!("relay: {error}"))
}

/// The plaintext inside the ciphertext (format v1): one JSON line, keys in this order.
pub(crate) fn encode_payload(text: &str, ts: u64, nonce: &str) -> String {
    let text_json = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
    format!(r#"{{"v":1,"ts":{ts},"nonce":"{nonce}","text":{text_json}}}"#)
}

/// The checked fields of a payload: version 1, a number ts, a 32-hex nonce, a text. None for anything else.
pub(crate) fn parse_payload(payload: &str) -> Option<(u64, String, String)> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    if value["v"].as_u64()? != 1 {
        return None;
    }
    let ts = value["ts"].as_u64()?;
    let nonce = value["nonce"].as_str()?;
    if !relay_rest::valid_hex(nonce, 32) {
        return None;
    }
    let text = value["text"].as_str()?;
    Some((ts, nonce.to_string(), text.to_string()))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_secs())
}

/// Receive one envelope line. Order: envelope check, decryption, format (v1), time window, nonce not used before,
/// then the line. Every failure is silent. Returns true when a line was shown.
pub(crate) fn receive(chat: &Chat, line: &str, session: &Session, theirs: &[u8], cache: &mut NonceCache, now: u64) -> bool {
    let Ok(envelope) = relay_realtime::accept(line, &session.noise_public, theirs) else { return false };
    let Some(payload) = open_payload(&session.noise_secret, theirs, &envelope.ct) else { return false };
    let Some((ts, nonce, text)) = parse_payload(&payload) else { return false };
    if !cache.accept(&nonce, ts, now) {
        return false;
    }
    let text = clean_text(&text, MAX_TEXT_CHARS);
    if text.trim().is_empty() {
        return false;
    }
    let sender = chat.trusted_name(theirs).unwrap_or_else(|| "?".to_string());
    chat.add_relay_line(Line { sender: Some(sender), text, mine: false });
    true
}

/// One Noise IK message carrying the text as its payload (initiator side).
fn seal_payload(secret: &[u8], peer_key: &[u8], text: &str) -> Result<Vec<u8>, String> {
    let mut handshake = noise_builder()
        .local_private_key(secret)
        .and_then(|builder| builder.remote_public_key(peer_key))
        .and_then(|builder| builder.build_initiator())
        .map_err(|_| "relay: could not start encryption".to_string())?;
    let mut buffer = vec![0u8; NOISE_MAX];
    let length = handshake.write_message(text.as_bytes(), &mut buffer).map_err(|_| "relay: message too long or not encrypted".to_string())?;
    buffer.truncate(length);
    Ok(buffer)
}

/// The text of one IK message from `expected` (responder side). None when it does not verify.
fn open_payload(secret: &[u8], expected: &[u8], ciphertext: &[u8]) -> Option<String> {
    let mut handshake = noise_builder().local_private_key(secret).ok()?.build_responder().ok()?;
    let mut buffer = vec![0u8; NOISE_MAX];
    let length = handshake.read_message(ciphertext, &mut buffer).ok()?;
    if handshake.get_remote_static()? != expected {
        return None;
    }
    String::from_utf8(buffer[..length].to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_round_trips_between_two_keys_and_a_third_key_cannot_open_it() {
        let alice = snow::Builder::new("Noise_IK_25519_ChaChaPoly_BLAKE2s".parse().unwrap()).generate_keypair().unwrap();
        let bob = snow::Builder::new("Noise_IK_25519_ChaChaPoly_BLAKE2s".parse().unwrap()).generate_keypair().unwrap();
        let mallory = snow::Builder::new("Noise_IK_25519_ChaChaPoly_BLAKE2s".parse().unwrap()).generate_keypair().unwrap();
        let sealed = seal_payload(&alice.private, &bob.public, "hello bob").unwrap();
        assert_eq!(open_payload(&bob.private, &alice.public, &sealed).as_deref(), Some("hello bob"));
        assert_eq!(open_payload(&bob.private, &mallory.public, &sealed), None);
        assert_eq!(open_payload(&mallory.private, &alice.public, &sealed), None);
    }

    #[test]
    fn verified_key_needs_a_trusted_noise_key() {
        let chat = Chat::new(crate::chat::Me { id: "me".into(), name: "Me".into(), port: 1, key: vec![1; 32], secret: vec![2; 32] });
        let peer = Peer { pk: "11".repeat(32), name: "Bob".into(), noise_pk: "22".repeat(32) };
        assert_eq!(verified_key(&chat, &peer), None);
        chat.trusted.lock().unwrap().push(crate::store::Trusted { name: "Bob".into(), key: vec![0x22; 32], ip: "10.0.0.2".into() });
        assert_eq!(verified_key(&chat, &peer), Some(vec![0x22; 32]));
    }


    fn now() -> u64 {
        now_secs()
    }

    fn envelope_with(payload: &str, from: &snow::Keypair, to: &[u8]) -> String {
        let ciphertext = seal_payload(&from.private, to, payload).unwrap();
        relay::seal(&from.public, to, &ciphertext).unwrap()
    }

    fn session_of(keys: &snow::Keypair, seed: u8) -> Session {
        Session { seed: [seed; 32], noise_secret: keys.private.clone(), noise_public: keys.public.clone() }
    }

    fn nonce_of(byte: u8) -> String {
        format!("{byte:02x}").repeat(16)
    }

    #[test]
    fn payload_format_is_fixed() {
        let payload = encode_payload("hi \"there\"", 1700000000, &nonce_of(0xab));
        assert_eq!(payload, format!(r#"{{"v":1,"ts":1700000000,"nonce":"{}","text":"hi \"there\""}}"#, nonce_of(0xab)));
        assert_eq!(parse_payload(&payload), Some((1700000000, nonce_of(0xab), "hi \"there\"".to_string())));
    }

    #[test]
    fn payload_with_another_version_or_a_bad_nonce_is_refused() {
        assert_eq!(parse_payload(&encode_payload("x", 1, &nonce_of(1)).replace("\"v\":1", "\"v\":2")), None);
        assert_eq!(parse_payload(&encode_payload("x", 1, "ABCD")), None);
        assert_eq!(parse_payload("not json"), None);
    }

    #[test]
    fn receive_checks_version_window_and_replay_in_order() {
        let (alice, bob) = (crate::relay_e2e_test::noise_pair(), crate::relay_e2e_test::noise_pair());
        let (alice_session, bob_session) = (session_of(&alice, 1), session_of(&bob, 2));
        let chat = Chat::new(crate::chat::Me { id: "b".into(), name: "Bob".into(), port: 1, key: vec![], secret: vec![] });
        chat.trusted.lock().unwrap().push(crate::store::Trusted { name: "Alice".into(), key: alice.public.clone(), ip: "10.0.0.1".into() });
        let mut cache = crate::relay_replay::NonceCache::default();
        let now = now();
        let _ = alice_session;

        // version 2: refused
        let v2 = envelope_with(&encode_payload("v2", now, &nonce_of(1)).replace("\"v\":1", "\"v\":2"), &alice, &bob.public);
        assert!(!receive(&chat, &v2, &bob_session, &alice.public, &mut cache, now));
        // 301 seconds off: refused; 299 seconds off: accepted
        let late = envelope_with(&encode_payload("late", now - 301, &nonce_of(2)), &alice, &bob.public);
        assert!(!receive(&chat, &late, &bob_session, &alice.public, &mut cache, now));
        let early = envelope_with(&encode_payload("ok 299", now - 299, &nonce_of(3)), &alice, &bob.public);
        assert!(receive(&chat, &early, &bob_session, &alice.public, &mut cache, now));
        // the same envelope again: refused as a replay
        assert!(!receive(&chat, &early, &bob_session, &alice.public, &mut cache, now));
        // a new envelope with the same text but a new nonce: shown
        let again = envelope_with(&encode_payload("ok 299", now, &nonce_of(4)), &alice, &bob.public);
        assert!(receive(&chat, &again, &bob_session, &alice.public, &mut cache, now));
        assert_eq!(chat.lines().iter().filter(|line| line.text == "ok 299").count(), 2);
        assert!(chat.lines().iter().all(|line| line.text != "v2" && line.text != "late"));
    }
}
