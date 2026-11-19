//! Relay path of the chat (Rust TUI). Runs beside the LAN path, never instead of it.
//!
//! Room model only: settings.room_code holds the 8-character room code. This node joins the room (join-room).
//! The 1:1 relay by short ID was removed; sending to room members is not implemented yet (see room_relay).

mod room_relay;

use crate::chat::Chat;
use crate::relay_key;
use crate::relay_rest;
use crate::relay_settings::{self, Mode, Settings};
use crate::store;
use ed25519_dalek::SigningKey;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const DEAD_LEAVE_TIMEOUT: Duration = Duration::from_secs(3);

/// This PC's relay identity: the Ed25519 public key (the row key) and the X25519 Noise public key (the envelope key).
pub fn my_keys(state: &Path) -> Result<(String, String), String> {
    let seed = relay_key::load_or_create(state)?;
    let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    let noise = store::read_identity(state).map_err(|error| error.to_string())?.ok_or("no identity yet: start a chat first")?;
    Ok((store::to_hex(&public), store::to_hex(&noise.public)))
}

/// This PC's Ed25519 seed (row signing), after the identity check: the chat must have been started once.
pub(crate) fn seed(state: &Path) -> Result<[u8; 32], String> {
    store::read_identity(state).map_err(|error| error.to_string())?.ok_or("no identity yet: start a chat first")?;
    relay_key::load_or_create(state)
}

/// Delete this PC's own row (signed, pk and short id). Best effort and a few seconds at most. False when nothing was sent.
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

/// Start the relay for the saved or environment settings, replacing any relay that runs. Ok(false): relay is off.
pub fn start(chat: &Arc<Chat>, state: &Path) -> Result<bool, String> {
    start_with(chat, state, relay_settings::effective(state))
}

/// The same start with the settings given (tests use this; the environment is not read).
pub fn start_with(chat: &Arc<Chat>, state: &Path, settings: Settings) -> Result<bool, String> {
    chat.stop_relay();
    // The mode decides the path: in Local mode the LAN carries messages and no relay runs.
    chat.set_supabase_mode(settings.mode == Mode::Supabase);
    if settings.mode != Mode::Supabase {
        chat.set_relay_status("");
        return Ok(false);
    }
    if settings.url.is_empty() && settings.key.is_empty() && settings.room_code.is_empty() {
        chat.set_relay_status("");
        return Ok(false);
    }
    let Some(config) = relay_rest::config(Some(&settings.url), Some(&settings.key))? else {
        chat.set_relay_status("relay: settings incomplete");
        return Ok(false);
    };
    if !relay_rest::valid_room_code(&settings.room_code) {
        chat.set_relay_status("relay: room code missing or invalid (8 uppercase letters or digits)");
        return Ok(false);
    }
    if !relay_rest::valid_trust(&settings.trust) {
        chat.set_relay_status("relay: trust code missing or invalid (8 digits)");
        return Ok(false);
    }
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    chat.attach_relay(tx, stop.clone());
    let (chat, state) = (chat.clone(), state.to_path_buf());
    thread::spawn(move || {
        room_relay::run(&chat, &state, &config, &settings, &rx, &stop);
        // The thread is over (failed join or stopped): its inbox must not keep taking lines nobody reads.
        chat.detach_relay(&stop);
    });
    Ok(true)
}

/// Leave the relay when this node stops normally: delete this PC's own row (signed, pk and short id). Best effort and a
/// few seconds at most. Nothing is sent when the relay is off, the key files are missing, or another live node still uses
/// this folder. A failure is ignored: the row then stays until the owner removes it.
pub fn leave(state: &Path) {
    if !store::other_node_running(state) {
        leave_with(state, &relay_settings::effective(state), relay_rest::TIMEOUT);
    }
}

/// The same delete for any node folder, with that folder's settings and keys. The caller makes sure no live node uses
/// the folder (a node that ended without cleaning up, see store::remove_dead_sessions).
pub fn leave_folder(state: &Path) {
    leave_with(state, &relay_settings::effective(state), DEAD_LEAVE_TIMEOUT);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node folder (keys made when asked) and the settings that point at a one-shot local relay.
    fn leaving(name: &str, mode: Mode, keys: bool) -> (std::path::PathBuf, Settings, std::sync::mpsc::Receiver<String>) {
        let state = std::env::temp_dir().join(format!("herdr-talk-relay-leave-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        std::fs::create_dir_all(&state).unwrap();
        if keys {
            relay_key::load_or_create(&state).unwrap();
            relay_key::short_id(&state).unwrap();
        }
        let (config, received) = relay_rest::tests::serve_once("HTTP/1.1 204 No Content
Content-Length: 0
Connection: close

");
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
    fn start_is_off_without_settings_and_refuses_a_bad_url() {
        let chat = Arc::new(Chat::new(crate::chat::Me { id: "a".into(), name: "A".into(), port: 1, key: vec![], secret: vec![] }));
        let state = std::env::temp_dir().join(format!("herdr-talk-relay-start-{}", std::process::id()));
        assert_eq!(start_with(&chat, &state, Settings::default()), Ok(false));
        assert_eq!(chat.relay_status(), "");
        let bad = Settings { mode: Mode::Supabase, url: "http://example.com".into(), key: "k".into(), ..Settings::default() };
        assert!(start_with(&chat, &state, bad).is_err());
        // The chat itself still works: a command is echoed.
        assert!(chat.handle("/help"));
        assert!(chat.lines().iter().any(|line| line.text == "> /help"));
    }

    #[test]
    fn start_with_valid_settings_runs_until_stopped() {
        let chat = Arc::new(Chat::new(crate::chat::Me { id: "a".into(), name: "A".into(), port: 1, key: vec![], secret: vec![] }));
        let state = std::env::temp_dir().join(format!("herdr-talk-relay-run-{}", std::process::id()));
        let settings = Settings { mode: Mode::Supabase, url: "http://127.0.0.1:9".into(), key: "dummy-test-key".into(), name: String::new(), room_code: "ABCD1234".into(), trust: "12345678".into(), setup_code: String::new() };
        assert_eq!(start_with(&chat, &state, settings), Ok(true));
        chat.stop_relay();
    }

    #[test]
    fn local_mode_never_starts_the_relay_even_with_relay_values() {
        let chat = Arc::new(Chat::new(crate::chat::Me { id: "a".into(), name: "A".into(), port: 1, key: vec![], secret: vec![] }));
        let state = std::env::temp_dir().join(format!("herdr-talk-relay-local-{}", std::process::id()));
        let settings = Settings { mode: Mode::Local, url: "https://example.supabase.co".into(), key: "k".into(), name: "A".into(), room_code: "ABCD1234".into(), trust: "12345678".into(), setup_code: String::new() };
        assert_eq!(start_with(&chat, &state, settings), Ok(false));
        assert!(!chat.is_supabase());
        assert_eq!(chat.relay_status(), "");
    }

    #[test]
    fn supabase_mode_turns_the_lan_path_off() {
        let chat = Arc::new(Chat::new(crate::chat::Me { id: "a".into(), name: "A".into(), port: 1, key: vec![], secret: vec![] }));
        let state = std::env::temp_dir().join(format!("herdr-talk-relay-mode-{}", std::process::id()));
        let settings = Settings { mode: Mode::Supabase, url: "http://127.0.0.1:9".into(), key: "k".into(), name: "A".into(), room_code: "ABCD1234".into(), trust: "12345678".into(), setup_code: String::new() };
        assert_eq!(start_with(&chat, &state, settings), Ok(true));
        assert!(chat.is_supabase());
        assert_eq!(chat.mode_name(), "supabase");
        chat.stop_relay();
    }
}

/// True when the saved room refuses this trust code (join-room answers 403): the chat must not open the room then. Any other
/// failure (offline, no keys yet) is not a refusal: the chat starts as before and shows it.
/// The trust check gives up after this long and does not refuse (the chat then starts as before).
const ROOM_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

pub fn room_refuses(state: &Path) -> bool {
    let settings = relay_settings::effective(state);
    if settings.mode != Mode::Supabase || settings.room_code.is_empty() {
        return false;
    }
    let Ok(Some(config)) = relay_rest::config(Some(&settings.url), Some(&settings.key)) else { return false };
    let Ok(seed) = relay_key::load_or_create(state) else { return false };
    let Ok(request) = relay_rest::join_room_request(&config, &seed, &settings.room_code, &settings.name, &settings.trust) else { return false };
    let Ok((status, body)) = relay_rest::http_post_within(&request, ROOM_CHECK_TIMEOUT) else { return false };
    matches!(relay_rest::read_join_room_response(status, &body), Err(error) if error == crate::i18n::t("room.trust_wrong", &[]))
}

#[cfg(test)]
mod room_refuses_tests {
    use super::*;

    #[test]
    fn no_saved_room_is_never_refused() {
        let dir = std::env::temp_dir().join("herdr-talk-room-refuses-empty");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!room_refuses(&dir), "no relay.json means no room to refuse");
    }

    fn saved_room(dir: &Path, url: &str) {
        relay_settings::save(dir, &relay_settings::Settings {
            mode: Mode::Supabase,
            url: url.to_string(),
            key: "dummy-test-key".into(),
            name: "Alice".into(),
            room_code: "ABCD2345".into(),
            trust: "12345678".into(),
            setup_code: String::new(),
        })
        .unwrap();
    }

    #[test]
    fn a_slow_trust_check_gives_up_after_three_seconds_and_does_not_refuse() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        // The server takes the connection and never answers.
        std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(std::time::Duration::from_secs(8));
        });
        let dir = std::env::temp_dir().join("herdr-talk-room-refuses-slow");
        let _ = std::fs::remove_dir_all(&dir);
        saved_room(&dir, &url);
        let started = std::time::Instant::now();
        assert!(!room_refuses(&dir), "no answer in time is not a refusal");
        let took = started.elapsed();
        assert!(took >= std::time::Duration::from_secs(2) && took < std::time::Duration::from_secs(7), "{took:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_trust_code_403_is_refused_and_other_answers_are_not() {
        use crate::relay_rest::tests::serve_once;
        let dir = std::env::temp_dir().join("herdr-talk-room-refuses-403");
        let _ = std::fs::remove_dir_all(&dir);
        let (config, _sent) = serve_once("HTTP/1.1 403 Forbidden
Content-Length: 0
Connection: close

");
        saved_room(&dir, config.url());
        assert!(room_refuses(&dir), "403 means the trust code is wrong");
        let (config, _sent) = serve_once("HTTP/1.1 500 Internal Server Error
Content-Length: 0
Connection: close

");
        saved_room(&dir, config.url());
        assert!(!room_refuses(&dir), "a server error is not a refusal");
        saved_room(&dir, "http://127.0.0.1:9");
        assert!(!room_refuses(&dir), "no network is not a refusal");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
