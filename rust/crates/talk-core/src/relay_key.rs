//! The Ed25519 signing key for relay row writes. Kept apart from the Noise key (identity.json) on purpose.
//! Same file and rules as python/src/relay_key.py: relay-signing.key in the state folder, 64 hex characters.
//! The file is created new, never overwritten: a damaged file is an error, not a reason to make a new key.

use std::fs;
use std::io::Write;
use std::path::Path;

pub const KEY_FILE: &str = "relay-signing.key";

/// The 32-byte seed (the private key). Makes a new key only when there is no file yet.
pub fn load_or_create(state: &Path) -> Result<[u8; 32], String> {
    let path = state.join(KEY_FILE);
    match fs::read_to_string(&path) {
        Ok(text) => parse(text.trim()).ok_or_else(|| format!("{KEY_FILE} is damaged; it was not replaced")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => create(state, &path),
        Err(error) => Err(format!("cannot read {KEY_FILE}: {error}")),
    }
}

fn parse(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return None;
    }
    let mut seed = [0u8; 32];
    for (index, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(seed)
}

fn create(state: &Path, path: &Path) -> Result<[u8; 32], String> {
    fs::create_dir_all(state).map_err(|error| error.to_string())?;
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|error| error.to_string())?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    match options.open(path) {
        Ok(mut file) => {
            let hex: String = seed.iter().map(|byte| format!("{byte:02x}")).collect();
            file.write_all(format!("{hex}\n").as_bytes()).map_err(|error| error.to_string())?;
            Ok(seed)
        }
        // Another process made the file first: use that one.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => load_or_create(state),
        Err(error) => Err(format!("cannot create {KEY_FILE}: {error}")),
    }
}

pub const SHORT_FILE: &str = "short-id.txt";
const SHORT_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// This PC's short ID (8 uppercase letters or digits). Made once and kept in short-id.txt; a damaged file is an error, not replaced.
pub fn short_id(state: &Path) -> Result<String, String> {
    let path = state.join(SHORT_FILE);
    match fs::read_to_string(&path) {
        Ok(text) => {
            // A file written by an older version may hold lowercase: it is read as uppercase (the input rule).
            let text = text.trim().to_ascii_uppercase();
            if crate::relay_rest::valid_short_id(&text) { Ok(text) } else { Err(format!("{SHORT_FILE} is damaged; it was not replaced")) }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let id = new_short_id()?;
            fs::create_dir_all(state).map_err(|error| error.to_string())?;
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            match options.open(&path) {
                Ok(mut file) => {
                    file.write_all(format!("{id}\n").as_bytes()).map_err(|error| error.to_string())?;
                    Ok(id)
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => short_id(state),
                Err(error) => Err(format!("cannot create {SHORT_FILE}: {error}")),
            }
        }
        Err(error) => Err(format!("cannot read {SHORT_FILE}: {error}")),
    }
}

/// A new random short ID. Bytes at or above 252 are skipped, so every character is equally likely (36 divides 252).
pub fn new_short_id() -> Result<String, String> {
    let mut id = String::new();
    let mut byte = [0u8; 1];
    while id.len() < 8 {
        getrandom::getrandom(&mut byte).map_err(|error| error.to_string())?;
        if byte[0] < 252 {
            id.push(SHORT_ALPHABET[usize::from(byte[0]) % SHORT_ALPHABET.len()] as char);
        }
    }
    Ok(id)
}

/// Replace the short ID after the server said it belongs to another key (409). Returns the new one, and saves it.
pub fn replace_short_id(state: &Path) -> Result<String, String> {
    let id = new_short_id()?;
    let temp = state.join(format!("{SHORT_FILE}.{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&temp).map_err(|error| error.to_string())?;
    file.write_all(format!("{id}\n").as_bytes()).map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(&temp, state.join(SHORT_FILE)).map_err(|error| error.to_string())?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("herdr-talk-relay-key-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn created_once_then_read_back() {
        let dir = folder("once");
        let first = load_or_create(&dir).unwrap();
        assert_eq!(load_or_create(&dir).unwrap(), first);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn damaged_file_is_an_error_and_is_kept() {
        let dir = folder("damaged");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(KEY_FILE), "not a key\n").unwrap();
        assert!(load_or_create(&dir).is_err());
        assert_eq!(fs::read_to_string(dir.join(KEY_FILE)).unwrap(), "not a key\n");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_a_file_written_by_the_python_edition_format() {
        let dir = folder("format");
        fs::create_dir_all(&dir).unwrap();
        let hex: String = (0u8..32).map(|byte| format!("{byte:02x}")).collect();
        fs::write(dir.join(KEY_FILE), format!("{hex}\n")).unwrap();
        let expected: [u8; 32] = std::array::from_fn(|index| index as u8);
        assert_eq!(load_or_create(&dir).unwrap(), expected);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn short_id_is_made_once_and_kept() {
        let dir = folder("short");
        let first = short_id(&dir).unwrap();
        assert!(crate::relay_rest::valid_short_id(&first));
        assert_eq!(short_id(&dir).unwrap(), first);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replaced_short_id_is_saved() {
        let dir = folder("short-replace");
        let first = short_id(&dir).unwrap();
        let next = replace_short_id(&dir).unwrap();
        assert!(crate::relay_rest::valid_short_id(&next));
        assert_eq!(short_id(&dir).unwrap(), next);
        assert_ne!(next, first);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn damaged_short_id_file_is_kept() {
        let dir = folder("short-damaged");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(SHORT_FILE), "ABCD\n").unwrap();
        assert!(short_id(&dir).is_err());
        assert_eq!(fs::read_to_string(dir.join(SHORT_FILE)).unwrap(), "ABCD\n");
        fs::remove_dir_all(&dir).unwrap();
    }
}
