//! Screen text in several languages. Sentences live in locales/<language>.json as "key": "text",
//! and a {name} inside a sentence is filled from the values given to t().
//!
//! Language order (the first one set wins):
//!   1. HERDR_TALK_LANG
//!   2. "lang" in HERDR_PLUGIN_CONFIG_DIR/config.json
//!   3. the OS language
//!   4. en
//!
//! A key missing from the language falls back to en, then to the key itself.

use serde_json::{Map, Value};
use std::path::Path;
use std::sync::{OnceLock, RwLock};

const DEFAULT: &str = "en";

/// The language chosen while the app runs (F3); None until one is chosen. It wins over the detected language.
static CHOSEN: RwLock<Option<&'static str>> = RwLock::new(None);

/// Language code and file. Every supported language is listed here.
const LOCALES: [(&str, &str); 7] = [
    ("en", include_str!("../locales/en.json")),
    ("ko", include_str!("../locales/ko.json")),
    ("ru", include_str!("../locales/ru.json")),
    ("fr", include_str!("../locales/fr.json")),
    ("es", include_str!("../locales/es.json")),
    ("de", include_str!("../locales/de.json")),
    ("ja", include_str!("../locales/ja.json")),
];

/// 'ko_KR.UTF-8', 'ko-KR', 'KO' become 'ko'. None when not supported.
fn normalize(value: &str) -> Option<&'static str> {
    let code = value.trim().replace('-', "_");
    let code = code.split(['.', '_']).next().unwrap_or("").to_lowercase();
    LOCALES.iter().map(|(name, _)| *name).find(|name| *name == code)
}

fn from_plugin_config() -> Option<&'static str> {
    let folder = std::env::var("HERDR_PLUGIN_CONFIG_DIR").ok()?;
    let text = std::fs::read_to_string(Path::new(&folder).join("config.json")).ok()?;
    let config: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    normalize(config.get("lang")?.as_str()?)
}

fn detect() -> &'static str {
    std::env::var("HERDR_TALK_LANG").ok().and_then(|value| normalize(&value))
        .or_else(from_plugin_config)
        .or_else(|| sys_locale::get_locale().and_then(|value| normalize(&value)))
        .unwrap_or(DEFAULT)
}

fn load(language: &str) -> Map<String, Value> {
    let text = LOCALES.iter().find(|(name, _)| *name == language).map_or("{}", |(_, text)| text);
    serde_json::from_str(text).unwrap_or_default()
}

/// The language in use now: the one chosen with F3, else the detected one.
pub fn language() -> &'static str {
    static DETECTED: OnceLock<&'static str> = OnceLock::new();
    CHOSEN.read().ok().and_then(|chosen| *chosen).unwrap_or_else(|| *DETECTED.get_or_init(detect))
}

/// The language after `current` in LOCALES order (F3 cycles through them).
fn after(current: &str) -> &'static str {
    let position = LOCALES.iter().position(|(name, _)| *name == current).unwrap_or(0);
    LOCALES[(position + 1) % LOCALES.len()].0
}

/// Write "lang" into config.json in `folder`, keeping the other keys. A file that is not a JSON object is left alone.
fn write_lang(folder: &Path, language: &str) -> Result<(), String> {
    let path = folder.join("config.json");
    let mut config = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
            Ok(Value::Object(map)) => map,
            _ => return Err(format!("{} is not a JSON object", path.display())),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(error) => return Err(error.to_string()),
    };
    config.insert("lang".to_string(), Value::String(language.to_string()));
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    std::fs::write(&path, Value::Object(config).to_string()).map_err(|error| error.to_string())
}

/// F3: switch to the next language for this run. Ok(true): saved to config.json (HERDR_TALK_LANG still wins at the next start).
/// Ok(false): no config folder (HERDR_PLUGIN_CONFIG_DIR unset), so the switch lasts this run only. Err: the save failed.
pub fn switch_language() -> Result<bool, String> {
    let next = after(language());
    *CHOSEN.write().map_err(|error| error.to_string())? = Some(next);
    save_to_folder(std::env::var("HERDR_PLUGIN_CONFIG_DIR").ok().as_deref(), next)
}

/// The save part of F3: no folder means Ok(false); otherwise the language goes into the folder's config.json.
fn save_to_folder(folder: Option<&str>, language: &str) -> Result<bool, String> {
    let Some(folder) = folder.filter(|folder| !folder.is_empty()) else { return Ok(false) };
    write_lang(Path::new(folder), language).map(|()| true)
}

/// What the screens show after F3: nothing when saved, the not-saved line when there is no config folder, else the save error.
pub fn switch_language_notice() -> String {
    notice(switch_language())
}

fn notice(result: Result<bool, String>) -> String {
    match result {
        Ok(true) => String::new(),
        Ok(false) => t("tui.lang.not_saved", &[]),
        Err(error) => error,
    }
}

fn lookup(key: &str) -> String {
    static MESSAGES: OnceLock<Vec<(&'static str, Map<String, Value>)>> = OnceLock::new();
    let all = MESSAGES.get_or_init(|| LOCALES.iter().map(|(name, _)| (*name, load(name))).collect());
    let messages = |language: &str| all.iter().find(|(name, _)| *name == language).map(|(_, messages)| messages);
    [messages(language()), messages(DEFAULT)]
        .into_iter()
        .flatten()
        .find_map(|messages| messages.get(key)?.as_str().filter(|text| !text.is_empty()))
        .unwrap_or(key)
        .to_string()
}

/// Fill each {name} once. An unknown {name} stays as it is, so a translation slip cannot break a line,
/// and a {name} inside a peer's text is not filled again.
fn fill(text: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let value = rest.find('}').and_then(|end| {
            let (_, value) = values.iter().find(|(name, _)| *name == &rest[1..end])?;
            Some((end, *value))
        });
        match value {
            Some((end, value)) => {
                out.push_str(value);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('{');
                rest = &rest[1..];
            }
        }
    }
    out + rest
}

pub fn t(key: &str, values: &[(&str, &str)]) -> String {
    fill(&lookup(key), values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn normalizes_locale_names() {
        assert_eq!(normalize("ko_KR.UTF-8"), Some("ko"));
        assert_eq!(normalize("ko-KR"), Some("ko"));
        assert_eq!(normalize("KO"), Some("ko"));
        assert_eq!(normalize("vi"), None);
    }

    #[test]
    fn fill_keeps_unknown_names_and_does_not_fill_twice() {
        assert_eq!(fill("{a} {b}", &[("a", "x")]), "x {b}");
        assert_eq!(fill("{a} {b}", &[("a", "{b}"), ("b", "y")]), "{b} y");
    }

    #[test]
    fn every_locale_has_the_same_keys_as_english() {
        let english = load("en");
        let keys: BTreeSet<_> = english.keys().collect();
        for (name, _) in LOCALES {
            let messages = load(name);
            assert_eq!(messages.keys().collect::<BTreeSet<_>>(), keys, "{name}");
        }
    }

    #[test]
    fn f3_cycles_through_every_language_and_back() {
        let mut seen = vec!["en"];
        let mut current = "en";
        for _ in 0..LOCALES.len() {
            current = after(current);
            seen.push(current);
        }
        assert_eq!(seen, ["en", "ko", "ru", "fr", "es", "de", "ja", "en"]);
    }

    #[test]
    fn saving_the_language_keeps_the_other_config_keys() {
        let dir = std::env::temp_dir().join(format!("herdr-talk-lang-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), "\u{feff}{\"mode\":\"supabase\",\"lang\":\"en\"}").unwrap();
        write_lang(&dir, "ko").unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).unwrap()).unwrap();
        assert_eq!(saved["lang"], "ko");
        assert_eq!(saved["mode"], "supabase");
        std::fs::write(dir.join("config.json"), "[1, 2]").unwrap();
        assert!(write_lang(&dir, "ja").is_err(), "a file that is not an object is left alone");
        assert_eq!(std::fs::read_to_string(dir.join("config.json")).unwrap(), "[1, 2]");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_config_folder_is_a_notice_and_a_failed_save_is_an_error() {
        // Only the save decision is tested here (no global language or environment change, so parallel tests are not affected).
        let base = std::env::temp_dir().join(format!("herdr-talk-lang-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        assert_eq!(save_to_folder(None, "ko"), Ok(false), "no folder: the switch lasts this run");
        assert_eq!(save_to_folder(Some(""), "ko"), Ok(false), "an empty folder is no folder");
        assert_eq!(notice(Ok(false)), t("tui.lang.not_saved", &[]));
        assert_eq!(save_to_folder(base.to_str(), "ko"), Ok(true));
        assert!(notice(Ok(true)).is_empty(), "a saved switch shows nothing");
        assert!(base.join("config.json").exists());
        // A config folder that is a file: the save fails and the error is what the screens show.
        let file = base.join("not-a-folder");
        std::fs::write(&file, "x").unwrap();
        let failed = save_to_folder(file.to_str(), "ko");
        assert!(failed.is_err());
        assert_eq!(notice(failed.clone()), failed.unwrap_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn every_first_screen_has_the_language_key_and_english_mode_names() {
        // F3 is named in the first-run footer, and the mode names stay in English in every language.
        for (name, _) in LOCALES {
            let messages = load(name);
            let footer = messages["tui.first_run.keys"].as_str().unwrap();
            assert!(footer.contains("F3"), "{name}: {footer}");
            assert_eq!(messages["mode.local"].as_str(), Some("BASE STATION"), "{name}");
            assert_eq!(messages["mode.supabase"].as_str(), Some("CLOUD SERVICE"), "{name}");
            for key in ["mode.hint_local", "mode.hint_supabase", "mode.first_run_title"] {
                assert!(messages[key].as_str().is_some_and(|text| !text.is_empty()), "{name}: {key}");
            }
        }
    }

    #[test]
    fn every_status_line_leaves_the_key_names_to_the_guide() {
        // The keyboard guide row already shows the keys; the status line shows the app name, name and peer count only.
        for (name, _) in LOCALES {
            let line = load(name)["status.line"].as_str().unwrap().to_string();
            for key in ["Ctrl+", "/help"] {
                assert!(!line.contains(key), "{name}: {line}");
            }
        }
    }
}
