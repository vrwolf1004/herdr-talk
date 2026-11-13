//! Relay path of the chat (stage 1, Rust TUI). Runs beside the LAN path, never instead of it.
//!
//! Two relay modes:
//! - Peer-to-peer (legacy): direct connection to one peer via Supabase relay channels
//! - Room-based: multiple peers in a room, messages via Supabase database
//!
//! Peer-to-peer path: settings.peer contains the peer's short ID. Messages are sent/received through
//! Noise-encrypted envelopes on a channel named after both noise keys.
//!
//! Room-based path: settings.room_code contains the 8-character room code. This node joins the room
//! and receives messages from all members.

pub mod peer_relay;
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
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    chat.attach_relay(tx, stop.clone());
    let (chat, state) = (chat.clone(), state.to_path_buf());
    thread::spawn(move || {
        room_relay::run(&chat, &state, &config, &settings, &rx, &stop);
    });
    Ok(true)
}

/// Leave the relay when this node stops normally: delete this PC's own row (signed, pk and short id). Best effort and a
/// few seconds at most. Nothing is sent when the relay is off, the key files are missing, or another live node still uses
/// this folder. A failure is ignored: the row then stays until the owner removes it.
pub fn leave(state: &Path) {
    if !store::other_node_running(state) {
        peer_relay::leave_with(state, &relay_settings::effective(state), relay_rest::TIMEOUT);
    }
}

/// The same delete for any node folder, with that folder's settings and keys. The caller makes sure no live node uses
/// the folder (a node that ended without cleaning up, see store::remove_dead_sessions).
pub fn leave_folder(state: &Path) {
    peer_relay::leave_with(state, &relay_settings::effective(state), DEAD_LEAVE_TIMEOUT);
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let settings = Settings { mode: Mode::Supabase, url: "http://127.0.0.1:9".into(), key: "dummy-test-key".into(), name: String::new(), room_code: "ABCD1234".into() };
        assert_eq!(start_with(&chat, &state, settings), Ok(true));
        chat.stop_relay();
    }

    #[test]
    fn local_mode_never_starts_the_relay_even_with_relay_values() {
        let chat = Arc::new(Chat::new(crate::chat::Me { id: "a".into(), name: "A".into(), port: 1, key: vec![], secret: vec![] }));
        let state = std::env::temp_dir().join(format!("herdr-talk-relay-local-{}", std::process::id()));
        let settings = Settings { mode: Mode::Local, url: "https://example.supabase.co".into(), key: "k".into(), name: "A".into(), room_code: "ABCD1234".into() };
        assert_eq!(start_with(&chat, &state, settings), Ok(false));
        assert!(!chat.is_supabase());
        assert_eq!(chat.relay_status(), "");
    }

    #[test]
    fn supabase_mode_turns_the_lan_path_off() {
        let chat = Arc::new(Chat::new(crate::chat::Me { id: "a".into(), name: "A".into(), port: 1, key: vec![], secret: vec![] }));
        let state = std::env::temp_dir().join(format!("herdr-talk-relay-mode-{}", std::process::id()));
        let settings = Settings { mode: Mode::Supabase, url: "http://127.0.0.1:9".into(), key: "k".into(), name: "A".into(), room_code: "ABCD1234".into() };
        assert_eq!(start_with(&chat, &state, settings), Ok(true));
        assert!(chat.is_supabase());
        assert_eq!(chat.mode_name(), "supabase");
        chat.stop_relay();
    }
}
