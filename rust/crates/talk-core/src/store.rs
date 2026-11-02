//! Files that outlive one chat: this PC's identity key and the peers the user has verified.
//! Both sit in the herdr-talk state folder. The identity is readable by the user only.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

const IDENTITY_FILE: &str = "identity.json";
const TRUSTED_FILE: &str = "trusted.json";

/// This PC's Noise key pair. The secret half is never shown or logged.
pub struct Identity {
    pub public: Vec<u8>,
    pub secret: Vec<u8>,
}

/// A peer the user verified: its key, and the name and address it had when verified.
#[derive(Clone)]
pub struct Trusted {
    pub name: String,
    pub key: Vec<u8>,
    pub ip: String,
}

/// A peer's id is the SHA-256 of its public key, in hex. Nobody can claim another peer's id without its key.
pub fn peer_id(key: &[u8]) -> String {
    to_hex(&Sha256::digest(key))
}

/// Code both users compare on their screens: 128 bits of SHA-256 over the two public keys, sorted,
/// so both PCs compute the same value. Each side has its own key in the hash, so one attacker cannot
/// make the two screens show the same code for two different keys (a short code could be collided).
pub fn fingerprint(own: &[u8], peer: &[u8]) -> String {
    let (first, second) = if own <= peer { (own, peer) } else { (peer, own) };
    let mut hasher = Sha256::new();
    hasher.update(first);
    hasher.update(second);
    let hex = to_hex(&hasher.finalize()[..16]);
    (0..32).step_by(4).map(|index| &hex[index..index + 4]).collect::<Vec<_>>().join("-")
}

/// What the user types for a code: no dashes, spaces or capitals.
pub fn normalize_code(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace() && *ch != '-').collect::<String>().to_lowercase()
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    (0..text.len()).step_by(2)
        .map(|index| text.get(index..index + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok()))
        .collect()
}

/// herdr's plugin state folder, else a herdr-talk folder under the user's local data.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = env::var_os("HERDR_PLUGIN_STATE_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    let base = env::var_os("LOCALAPPDATA").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".local").join("state"));
    base.join("herdr-talk")
}

/// Plain messages to peers without a key are off unless config.json sets "plaintext": true.
pub fn plaintext_allowed() -> bool {
    let Some(folder) = env::var_os("HERDR_PLUGIN_CONFIG_DIR") else { return false };
    let Ok(text) = fs::read_to_string(Path::new(&folder).join("config.json")) else { return false };
    serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).ok()
        .and_then(|config| config.get("plaintext")?.as_bool())
        .unwrap_or(false)
}

pub fn read_identity(dir: &Path) -> io::Result<Option<Identity>> {
    let Some(value) = read_json(&dir.join(IDENTITY_FILE))? else { return Ok(None) };
    let field = |name: &str| value.get(name).and_then(Value::as_str).and_then(from_hex).ok_or_else(damaged);
    Ok(Some(Identity { public: field("public")?, secret: field("secret")? }))
}

/// Stored once. A damaged file is an error, never a reason to make a new key: a new key would make every peer distrust this PC.
pub fn write_identity(dir: &Path, identity: &Identity) -> io::Result<()> {
    let text = json!({"public": to_hex(&identity.public), "secret": to_hex(&identity.secret)}).to_string();
    write_private(&dir.join(IDENTITY_FILE), &text)
}

pub fn read_trusted(dir: &Path) -> io::Result<Vec<Trusted>> {
    let Some(value) = read_json(&dir.join(TRUSTED_FILE))? else { return Ok(Vec::new()) };
    let mut trusted = Vec::new();
    for entry in value.as_array().ok_or_else(damaged)? {
        let key = entry.get("key").and_then(Value::as_str).and_then(from_hex).ok_or_else(damaged)?;
        let name = entry.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        let ip = entry.get("ip").and_then(Value::as_str).unwrap_or("").to_string();
        trusted.push(Trusted { name, key, ip });
    }
    Ok(trusted)
}

pub fn write_trusted(dir: &Path, trusted: &[Trusted]) -> io::Result<()> {
    let list: Vec<Value> = trusted.iter()
        .map(|peer| json!({"name": peer.name, "key": to_hex(&peer.key), "ip": peer.ip}))
        .collect();
    write_private(&dir.join(TRUSTED_FILE), &Value::Array(list).to_string())
}

fn read_json(path: &Path) -> io::Result<Option<Value>> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|error| io::Error::new(ErrorKind::InvalidData, error.to_string())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Owner read/write only on Unix. On Windows the file keeps the default permissions of the user's profile folder.
fn write_private(path: &Path, text: &str) -> io::Result<()> {
    fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

fn damaged() -> io::Error {
    io::Error::new(ErrorKind::InvalidData, "herdr-talk state file is damaged")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> PathBuf {
        env::temp_dir().join(format!("herdr-talk-store-{:016x}", fastrand::u64(..)))
    }

    #[test]
    fn identity_is_stored_once_and_read_back() {
        let dir = folder();
        assert!(read_identity(&dir).unwrap().is_none());
        write_identity(&dir, &Identity { public: vec![1; 32], secret: vec![2; 32] }).unwrap();
        let read = read_identity(&dir).unwrap().unwrap();
        assert_eq!((read.public, read.secret), (vec![1; 32], vec![2; 32]));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn damaged_identity_is_an_error_not_a_new_key() {
        let dir = folder();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(IDENTITY_FILE), "{\"public\": \"zz\"}").unwrap();
        assert!(read_identity(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn trusted_peers_round_trip() {
        let dir = folder();
        let peer = Trusted { name: "Bob".into(), key: vec![9; 32], ip: "10.0.0.2".into() };
        write_trusted(&dir, std::slice::from_ref(&peer)).unwrap();
        let read = read_trusted(&dir).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!((read[0].name.as_str(), read[0].key.clone(), read[0].ip.as_str()), ("Bob", vec![9; 32], "10.0.0.2"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn codes_ignore_dashes_spaces_and_case() {
        let (own, peer) = ([3u8; 32], [4u8; 32]);
        let code = fingerprint(&own, &peer);
        assert_eq!(normalize_code(&format!(" {} ", code.to_uppercase())), normalize_code(&code));
        assert_eq!(normalize_code("A B-C"), "abc");
    }

    #[test]
    fn both_sides_compute_the_same_code() {
        let (a, b) = ([3u8; 32], [4u8; 32]);
        assert_eq!(fingerprint(&a, &b), fingerprint(&b, &a));
        assert_eq!(fingerprint(&a, &b).len(), 32 + 7);
        assert_ne!(fingerprint(&a, &b), fingerprint(&a, &[5u8; 32]));
    }
}
