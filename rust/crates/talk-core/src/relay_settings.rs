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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub url: String,
    pub key: String,
    pub name: String,
    /// The peer's Ed25519 public key (64 lowercase hex). Used to look up the peer's row.
    pub peer: String,
}

/// The settings as saved. Missing or damaged file: empty settings (the relay stays off).
pub fn load_saved(state: &Path) -> Settings {
    let Ok(text) = fs::read_to_string(state.join(FILE)) else { return Settings::default() };
    let Ok(value) = serde_json::from_str::<Value>(&text) else { return Settings::default() };
    let field = |name: &str| value[name].as_str().unwrap_or("").to_string();
    Settings { url: field("url"), key: field("key"), name: field("name"), peer: field("peer") }
}

/// The settings the relay uses: the environment wins for URL and key when it is set and not empty.
pub fn effective(state: &Path) -> Settings {
    let mut settings = load_saved(state);
    if let Some(url) = non_empty_env(URL_VARIABLE) {
        settings.url = url;
    }
    if let Some(key) = non_empty_env(KEY_VARIABLE) {
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
    if !settings.name.is_empty() && !relay_rest::valid_name(&settings.name) {
        return Err("name must be 1 to 32 characters without control characters".to_string());
    }
    if !settings.peer.is_empty() && !relay_rest::valid_pk(&settings.peer) {
        return Err("peer key must be 64 lowercase hex characters".to_string());
    }
    if !settings.url.is_empty() || !settings.key.is_empty() {
        relay_rest::config(Some(&settings.url), Some(&settings.key))?.ok_or("URL and key are both needed")?;
    }
    Ok(())
}

/// Save to relay.json. The file is written whole through a temporary file and renamed. Unix: 0600.
pub fn save(state: &Path, settings: &Settings) -> Result<(), String> {
    validate(settings)?;
    fs::create_dir_all(state).map_err(|error| error.to_string())?;
    let text = json!({"url": settings.url, "key": settings.key, "name": settings.name, "peer": settings.peer}).to_string();
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
            url: "https://example.supabase.co".into(),
            key: "dummy-test-key".into(),
            name: "Alice".into(),
            peer: "11".repeat(32),
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
        assert!(validate(&Settings { peer: "AB".repeat(32), ..sample() }).is_err());
        assert!(validate(&Settings { url: "http://example.com".into(), ..sample() }).is_err());
        assert!(validate(&Settings { key: String::new(), ..sample() }).is_err());
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
        std::env::set_var(URL_VARIABLE, "https://env.example.supabase.co");
        std::env::remove_var(KEY_VARIABLE);
        let effective = effective(&dir);
        std::env::remove_var(URL_VARIABLE);
        assert_eq!(effective.url, "https://env.example.supabase.co");
        assert_eq!(effective.key, "dummy-test-key");
        fs::remove_dir_all(&dir).unwrap();
    }
}
