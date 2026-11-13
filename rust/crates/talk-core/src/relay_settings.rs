//! Relay settings: the project URL, the publishable key, this PC's display name, and the peer's Ed25519 public key.
//! Saved in relay.json in the state folder (its own file: identity.json and trusted.json are not touched).
//! The environment variables HERDR_TALK_RELAY_URL and HERDR_TALK_RELAY_KEY win over the saved values when set.
//! The key is never printed: the screen shows its length only.

use crate::relay_rest::{self, URL_VARIABLE, KEY_VARIABLE};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const FILE: &str = "relay.json";

/// How messages travel. Exactly one of the two is used: Local (LAN discovery and chat, the default) or Supabase
/// (messages only through the relay; nothing goes over the LAN).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Local,
    Supabase,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Local => "local",
            Mode::Supabase => "supabase",
        }
    }

    /// Only the exact word "supabase" selects Supabase. A missing or unknown value means Local.
    pub fn from_saved(text: &str) -> Mode {
        if text == "supabase" { Mode::Supabase } else { Mode::Local }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub mode: Mode,
    pub url: String,
    pub key: String,
    pub name: String,
    /// Room code (8 uppercase letters or digits). All peers in the room share messages.
    pub room_code: String,
}

/// True before the connection mode has been chosen: no relay.json, a damaged one, or one without a `mode` key.
pub fn needs_first_run(state: &Path) -> bool {
    let Ok(text) = fs::read_to_string(state.join(FILE)) else { return true };
    let text = text.trim_start_matches('\u{feff}');
    match serde_json::from_str::<Value>(text) {
        Ok(value) => value.get("mode").is_none(),
        Err(_) => true,
    }
}

/// The settings as saved. Missing or damaged file: empty settings (the relay stays off).
pub fn load_saved(state: &Path) -> Settings {
    let Ok(text) = fs::read_to_string(state.join(FILE)) else { return Settings::default() };
    // A UTF-8 byte order mark (written by Windows PowerShell) is allowed and dropped.
    let text = text.trim_start_matches('\u{feff}');
    let Ok(value) = serde_json::from_str::<Value>(text) else { return Settings::default() };
    let field = |name: &str| value[name].as_str().unwrap_or("").to_string();
    let room_code = field("room_code");
    Settings { mode: Mode::from_saved(&field("mode")), url: field("url"), key: field("key"), name: field("name"), room_code }
}

/// The settings the relay uses: the environment wins for URL and key when it is set and not empty.
pub fn effective(state: &Path) -> Settings {
    effective_with(state, non_empty_env(URL_VARIABLE), non_empty_env(KEY_VARIABLE))
}

/// The same rule with the environment values given: a non-empty value replaces the saved one.
pub fn effective_with(state: &Path, env_url: Option<String>, env_key: Option<String>) -> Settings {
    let mut settings = load_saved(state);
    if let Some(url) = env_url {
        settings.url = url;
    }
    if let Some(key) = env_key {
        settings.key = key;
    }
    settings
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

/// Whether URL and key come from the environment (the screen says so).
pub fn from_environment() -> (bool, bool) {
    (non_empty_env(URL_VARIABLE).is_some(), non_empty_env(KEY_VARIABLE).is_some())
}

/// Checks before saving. Empty fields are allowed (they turn the relay off); a filled field must be valid.
pub fn validate(settings: &Settings) -> Result<(), String> {
    if settings.mode == Mode::Supabase && [&settings.url, &settings.key, &settings.name, &settings.room_code].iter().any(|field| field.is_empty()) {
        return Err("Supabase mode needs the URL, the key, the display name and the room code".to_string());
    }
    if !settings.name.is_empty() && !relay_rest::valid_name(&settings.name) {
        return Err("name must be 1 to 32 characters without control characters".to_string());
    }
    if !settings.room_code.is_empty() && !relay_rest::valid_room_code(&settings.room_code) {
        return Err("room code must be 8 uppercase letters or digits".to_string());
    }
    // Local mode keeps whatever was typed; the URL and key are checked when Supabase is in use.
    if settings.mode == Mode::Supabase {
        relay_rest::config(Some(&settings.url), Some(&settings.key))?.ok_or("URL and key are both needed")?;
    }
    Ok(())
}

/// Save to relay.json. The file is written whole through a temporary file and renamed. Unix: 0600.
pub fn save(state: &Path, settings: &Settings) -> Result<(), String> {
    validate(settings)?;
    fs::create_dir_all(state).map_err(|error| error.to_string())?;
    let text = json!({"mode": settings.mode.as_str(), "url": settings.url, "key": settings.key, "name": settings.name, "room_code": settings.room_code}).to_string();
    let temp = state.join(format!("{FILE}.{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&temp).map_err(|error| error.to_string())?;
    file.write_all(text.as_bytes()).map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(&temp, state.join(FILE)).map_err(|error| error.to_string())
}

/// Set only the connection mode and keep every other saved value (the mode switch in the windows).
pub fn set_mode(state: &Path, mode: Mode) -> Result<(), String> {
    let mut settings = load_saved(state);
    settings.mode = mode;
    save(state, &settings)
}

/// What the screen shows for the key: its length only, never the value.
pub fn key_label(key: &str) -> String {
    if key.is_empty() {
        "-".to_string()
    } else {
        format!("({} chars)", key.chars().count())
    }
}

pub fn file_path(state: &Path) -> PathBuf {
    state.join(FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("herdr-talk-relay-settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn sample() -> Settings {
        Settings {
            mode: Mode::Supabase,
            url: "https://example.supabase.co".into(),
            key: "dummy-test-key".into(),
            name: "Alice".into(),
            room_code: "ABCD2345".into(),
        }
    }

    #[test]
    fn saved_settings_come_back_as_they_were() {
        let dir = folder("roundtrip");
        save(&dir, &sample()).unwrap();
        assert_eq!(load_saved(&dir), sample());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_or_damaged_file_means_no_settings() {
        let dir = folder("missing");
        assert_eq!(load_saved(&dir), Settings::default());
        fs::create_dir_all(&dir).unwrap();
        fs::write(file_path(&dir), "{not json").unwrap();
        assert_eq!(load_saved(&dir), Settings::default());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_fields_are_allowed_and_bad_ones_are_refused() {
        assert!(validate(&Settings::default()).is_ok());
        assert!(validate(&Settings { name: "x".repeat(33), ..sample() }).is_err());
        assert!(validate(&Settings { room_code: "AB".repeat(32), ..sample() }).is_err());
        assert!(validate(&Settings { url: "http://example.com".into(), ..sample() }).is_err());
        assert!(validate(&Settings { key: String::new(), ..sample() }).is_err());
    }

    #[test]
    fn a_saved_file_with_a_byte_order_mark_is_read() {
        let dir = folder("bom");
        fs::create_dir_all(&dir).unwrap();
        let text = format!("{}{}", '\u{feff}', r#"{"url":"https://example.supabase.co","key":"k","name":"Bob","peer":""}"#);
        fs::write(file_path(&dir), text).unwrap();
        assert_eq!(load_saved(&dir).name, "Bob");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_peer_id_is_eight_lowercase_characters_only() {
        let base = Settings { mode: Mode::Supabase, url: "https://example.supabase.co".into(), key: "k".into(), name: "A".into(), room_code: String::new() };
        assert!(validate(&Settings { room_code: "ABCD2345".into(), ..base.clone() }).is_ok());
        for bad in ["abcdefgh", "ABCDEFGHI", "abcd2345", "AB".repeat(32).as_str()] {
            assert!(validate(&Settings { room_code: bad.into(), ..base.clone() }).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_saved_long_peer_key_is_dropped() {
        let dir = folder("old-peer");
        fs::create_dir_all(&dir).unwrap();
        fs::write(file_path(&dir), format!(r#"{{"mode":"supabase","url":"https://example.supabase.co","key":"k","name":"A","peer":"{}"}}"#, "ab".repeat(32))).unwrap();
        assert_eq!(load_saved(&dir).room_code, "");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn key_is_shown_by_length_only() {
        assert_eq!(key_label("dummy-test-key"), "(14 chars)");
        assert_eq!(key_label(""), "-");
        assert!(!key_label("dummy-test-key").contains("dummy"));
    }

    #[test]
    fn environment_wins_over_saved_values() {
        let dir = folder("env");
        save(&dir, &sample()).unwrap();
        let effective = effective_with(&dir, Some("https://env.example.supabase.co".into()), None);
        assert_eq!(effective.url, "https://env.example.supabase.co");
        assert_eq!(effective.key, "dummy-test-key");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn first_run_until_a_mode_is_saved() {
        let dir = folder("first-run");
        assert!(needs_first_run(&dir), "no file");
        fs::create_dir_all(&dir).unwrap();
        fs::write(file_path(&dir), r#"{"url":"https://example.supabase.co"}"#).unwrap();
        assert!(needs_first_run(&dir), "no mode key");
        save(&dir, &Settings { mode: Mode::Local, ..Settings::default() }).unwrap();
        assert!(!needs_first_run(&dir), "mode saved");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_mode_keeps_the_other_values() {
        let dir = folder("set-mode");
        save(&dir, &sample()).unwrap();
        set_mode(&dir, Mode::Local).unwrap();
        let saved = load_saved(&dir);
        assert_eq!(saved.mode, Mode::Local);
        assert_eq!(saved.name, "Alice");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mode_defaults_to_local_and_only_supabase_selects_supabase() {
        let dir = folder("mode-file");
        fs::create_dir_all(&dir).unwrap();
        fs::write(file_path(&dir), r#"{"url":"https://example.supabase.co","key":"k","name":"A","peer":""}"#).unwrap();
        assert_eq!(load_saved(&dir).mode, Mode::Local, "no mode key: local, even with relay values present");
        fs::write(file_path(&dir), r#"{"mode":"something else"}"#).unwrap();
        assert_eq!(load_saved(&dir).mode, Mode::Local);
        fs::write(file_path(&dir), r#"{"mode":"supabase"}"#).unwrap();
        assert_eq!(load_saved(&dir).mode, Mode::Supabase);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn supabase_mode_needs_every_field() {
        let full = Settings { mode: Mode::Supabase, url: "https://example.supabase.co".into(), key: "k".into(), name: "A".into(), room_code: "ABCD2345".into() };
        assert!(validate(&full).is_ok());
        assert!(validate(&Settings { name: String::new(), ..full.clone() }).is_err());
        assert!(validate(&Settings { room_code: String::new(), ..full.clone() }).is_err());
        assert!(validate(&Settings { url: String::new(), ..full.clone() }).is_err());
        assert!(validate(&Settings { key: String::new(), ..full.clone() }).is_err());
        assert!(validate(&Settings { mode: Mode::Local, name: String::new(), ..full }).is_ok(), "local mode keeps partial values");
    }

    #[test]
    fn the_mode_is_written_every_time() {
        let dir = folder("mode-write");
        save(&dir, &Settings { mode: Mode::Local, ..Settings::default() }).unwrap();
        let text = fs::read_to_string(file_path(&dir)).unwrap();
        assert!(text.contains(r#""mode":"local""#), "{text}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
