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
use std::sync::OnceLock;

const DEFAULT: &str = "en";

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

fn lookup(key: &str) -> String {
    static MESSAGES: OnceLock<(Map<String, Value>, Map<String, Value>)> = OnceLock::new();
    let (chosen, fallback) = MESSAGES.get_or_init(|| (load(detect()), load(DEFAULT)));
    [chosen, fallback].iter()
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
    fn every_status_line_keeps_the_key_names() {
        for (name, _) in LOCALES {
            let line = load(name)["status.line"].as_str().unwrap().to_string();
            for key in ["Ctrl+N", "Ctrl+C", "/help"] {
                assert!(line.contains(key), "{name}");
            }
        }
    }
}
