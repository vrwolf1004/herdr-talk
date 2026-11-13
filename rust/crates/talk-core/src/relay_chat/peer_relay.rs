//! Peer-to-peer relay logic: direct connection to one peer via Supabase relay channels.
//!
//! Messages are sent/received through Noise-encrypted envelopes on a channel named after both noise keys.

use crate::chat::{Chat, Line, MAX_TEXT_CHARS};
use crate::relay;
use crate::relay_key;
use crate::relay_realtime::Link;
use crate::relay_replay::NonceCache;
use crate::relay_rest::{self, Config, Peer};
use crate::relay_settings::{Mode, Settings};
use crate::store;
use crate::text::clean_text;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant};

const SLICE: Duration = Duration::from_millis(200);
pub(crate) const RETRY: Duration = Duration::from_secs(10);
pub(crate) const PEER_WAIT: Duration = Duration::from_secs(30);
const NOISE_MAX: usize = 65535;

/// Keys and names the relay thread needs, all read once at start.
pub(crate) struct Session {
    pub(crate) seed: [u8; 32],
    pub(crate) short_id: String,
    pub(crate) noise_secret: Vec<u8>,
    pub(crate) noise_public: Vec<u8>,
}

/// Create a Session from the state folder.
pub(crate) fn session(state: &Path) -> Result<Session, String> {
    let seed = relay_key::load_or_create(state)?;
    let identity = store::read_identity(state).map_err(|error| error.to_string())?.ok_or("no identity yet: start a chat first")?;
    let short_id = relay_key::short_id(state)?;
    Ok(Session { seed, short_id, noise_secret: identity.secret, noise_public: identity.public })
}

/// Delete this PC's own row (signed, pk and short id). Best effort and a few seconds at most.
pub(crate) fn leave_with(state: &Path, settings: &Settings, timeout: Duration) -> bool {
    if settings.mode != Mode::Supabase {
        return false;
    }
    let Ok(Some(config)) = relay_rest::config(Some(&settings.url), Some(&settings.key)) else { return false };
    if !state.join(relay_key::KEY_FILE).is_file() || !state.join(relay_key::SHORT_FILE).is_file() {
        return false;
    }
    let (Ok(seed), Ok(short)) = (relay_key::load_or_create(state), relay_key::short_id(state)) else { return false };
    relay_rest::delete_peer_within(&config, &seed, &short, timeout)
}

/// The peer's Noise key from its row, when the user verified that key with /trust.
fn verified_key(chat: &Chat, peer: &Peer) -> Option<Vec<u8>> {
    let key = store::from_hex(&peer.noise_pk)?;
    chat.trusted_name(&key).is_some().then_some(key)
}

/// Sleep for `time`, in slices so a stop is seen. Texts typed meanwhile are not sent: a notice says so.
pub(crate) fn wait(chat: &Chat, rx: &Receiver<String>, stop: &AtomicBool, time: Duration) {
    let until = Instant::now() + time;
    while Instant::now() < until && !stop.load(Ordering::Relaxed) && !chat.take_relay_wake() {
        while let Ok(_text) = rx.try_recv() {
            chat.say("relay: not connected, the message was not sent over the relay");
        }
        thread::sleep(SLICE);
    }
}

/// Re-check the row right before sending, then seal and send one envelope.
pub(crate) fn send_text(chat: &Chat, config: &Config, peer_pk: &str, session: &Session, theirs: &[u8], link: &mut Link, text: &str) -> Result<(), String> {
    let row = relay_rest::get_peer(config, peer_pk);
    // The row is read again here, so its current name is kept for the chat too.
    if let Some(peer) = &row {
        if let Some(key) = store::from_hex(&peer.noise_pk) {
            chat.set_relay_candidate(&peer.name, key);
        }
    }
    let checked = row.and_then(|peer| verified_key(chat, &peer));
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
    let Ok(envelope) = crate::relay_realtime::accept(line, &session.noise_public, theirs) else { return false };
    let Some(payload) = open_payload(&session.noise_secret, theirs, &envelope.ct) else { return false };
    let Some((ts, nonce, text)) = parse_payload(&payload) else { return false };
    if !cache.accept(&nonce, ts, now) {
        return false;
    }
    let text = clean_text(&text, MAX_TEXT_CHARS);
    if text.trim().is_empty() {
        return false;
    }
    let sender = chat.relay_display_name(theirs).unwrap_or_else(|| "?".to_string());
    chat.add_relay_line(Line { sender: Some(sender), text, mine: false });
    true
}

/// One Noise IK message carrying the text as its payload (initiator side).
fn seal_payload(secret: &[u8], peer_key: &[u8], text: &str) -> Result<Vec<u8>, String> {
    let mut handshake = crate::net::noise_builder()
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
    let mut handshake = crate::net::noise_builder().local_private_key(secret).ok()?.build_responder().ok()?;
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

    /// A node folder (keys made when asked) and the settings that point at a one-shot local relay.
    fn leaving(name: &str, mode: crate::relay_settings::Mode, keys: bool) -> (std::path::PathBuf, Settings, std::sync::mpsc::Receiver<String>) {
        let state = std::env::temp_dir().join(format!("herdr-talk-relay-leave-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        std::fs::create_dir_all(&state).unwrap();
        if keys {
            relay_key::load_or_create(&state).unwrap();
            relay_key::short_id(&state).unwrap();
        }
        let (config, received) = relay_rest::tests::serve_once("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let settings = Settings { mode, url: config.url().to_string(), key: "dummy-test-key".to_string(), ..Default::default() };
        (state, settings, received)
    }

    #[test]
    fn leaving_a_folder_with_keys_sends_one_signed_delete() {
        let (state, settings, received) = leaving("keys", Mode::Supabase, true);
        assert!(leave_with(&state, &settings, Duration::from_secs(3)));
        let raw = received.recv().unwrap();
        assert!(raw.starts_with("POST /functions/v1/delete-peer HTTP/1.1\r\n"), "{raw}");
        assert!(raw.contains(&format!(r#""short_id":"{}""#, relay_key::short_id(&state).unwrap())), "{raw}");
        std::fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn leaving_sends_nothing_when_the_relay_is_off_or_the_keys_are_missing() {
        for (name, mode, keys) in [("local", Mode::Local, true), ("keyless", Mode::Supabase, false)] {
            let (state, settings, received) = leaving(name, mode, keys);
            assert!(!leave_with(&state, &settings, Duration::from_secs(3)));
            assert!(received.try_recv().is_err(), "{name}: no request");
            assert_eq!(state.join(relay_key::KEY_FILE).exists(), keys, "{name}: no key is made on the way out");
            std::fs::remove_dir_all(&state).unwrap();
        }
    }

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
        Session { seed: [seed; 32], short_id: "abcd2345".into(), noise_secret: keys.private.clone(), noise_public: keys.public.clone() }
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
