//! Files that outlive one chat: this PC's identity key and the peers the user has verified.
//! Both sit in the herdr-talk state folder. The identity is readable by the user only.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::relay_key;
use crate::relay_rest;

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

/// The first 8 hex digits of the peer id: enough to tell peers apart on screen, never the full id.
pub fn key_label(key: &[u8]) -> String {
    peer_id(key)[..8].to_string()
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

/// The session this process uses: its id and its folder. Chosen once, at start (see choose_session).
static SESSION: OnceLock<(String, PathBuf)> = OnceLock::new();

/// The id of the session this process uses ("default" for the base folder). None until one is chosen.
pub fn session_id() -> Option<String> {
    SESSION.get().map(|(id, _)| id.clone())
}

/// The folder for everything this process keeps: the session folder once chosen, else the base folder.
pub fn state_dir() -> PathBuf {
    match SESSION.get() {
        Some((_, dir)) => dir.clone(),
        None => base_dir(),
    }
}

/// herdr's plugin state folder, else a herdr-talk folder under the user's local data. Sessions live under it.
pub fn base_dir() -> PathBuf {
    if let Some(dir) = env::var_os("HERDR_PLUGIN_STATE_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    let base = env::var_os("LOCALAPPDATA").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".local").join("state"));
    base.join("herdr-talk")
}

/// The session id that means the base folder itself (the state of earlier versions).
pub const DEFAULT_SESSION: &str = "default";

/// Session folders live in base/sessions/<id>.
pub fn sessions_dir(base: &Path) -> PathBuf {
    base.join("sessions")
}

/// A new session: a fresh short id and its new folder, created now. Its keys, identity and short ID are made later.
pub fn new_session(base: &Path) -> Result<(String, PathBuf), String> {
    loop {
        let id = relay_key::new_short_id()?;
        let dir = sessions_dir(base).join(&id);
        if dir.exists() {
            continue;
        }
        fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        return Ok((id, dir));
    }
}

/// An existing session by id. "default" is the base folder; any other id must name a folder that exists.
pub fn existing_session(base: &Path, id: &str) -> Result<(String, PathBuf), String> {
    if id == DEFAULT_SESSION {
        return Ok((id.to_string(), base.to_path_buf()));
    }
    if !relay_rest::valid_short_id(id) {
        return Err("session id is 8 lowercase letters or digits, or default".to_string());
    }
    let dir = sessions_dir(base).join(id);
    if !dir.is_dir() {
        return Err(format!("no session {id} (see herdr-talk sessions)"));
    }
    Ok((id.to_string(), dir))
}

/// Use the session chosen by the command line (--session ID), or a new one when none was given. Call it once, first.
pub fn choose_session(requested: Option<&str>) -> Result<(), String> {
    let base = base_dir();
    let (id, dir) = match requested {
        None => new_session(&base)?,
        Some(id) => existing_session(&base, id)?,
    };
    // Only the first choice counts; a second call keeps the first session.
    let _ = SESSION.set((id, dir));
    Ok(())
}

/// Whether a process with this pid exists. Windows asks the OS; other Unix systems send signal 0 (which sends
/// nothing and only checks): "no such process" means gone, and any other error (EPERM) means it exists.
#[cfg(windows)]
pub fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    const STILL_ACTIVE: u32 = 259;
    // SAFETY: the handle is checked before use and closed once; the code pointer is a local u32.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            // Access denied means the process exists but belongs to someone else.
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut code = 0u32;
        let known = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        !known || code == STILL_ACTIVE
    }
}

#[cfg(unix)]
pub fn process_alive(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    // A pid above i32::MAX cannot be a live process on Unix.
    let Ok(pid) = i32::try_from(pid) else { return false };
    // ESRCH: "no such process". Linux and macOS both use 3. Other errors (EPERM) mean the process exists.
    const ESRCH: i32 = 3;
    // SAFETY: signal 0 is only a permission and existence check; it delivers nothing.
    let result = unsafe { kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() != Some(ESRCH)
}

#[cfg(not(any(windows, unix)))]
pub fn process_alive(_pid: u32) -> bool {
    true // unknown system: keep the folder
}

/// Remove the node folders under base whose node has ended without cleaning up: each has control files, and every one of
/// them is for a process that has ended. A folder without a control file is kept, since its node may still be starting.
/// This process's own session is kept: with --session it may be such a folder, about to be used again.
/// Before a folder goes, `leave` gets it when its key files are there, to delete that node's relay row (best effort: the
/// folder is removed whatever `leave` does). Returns the ids that could not be removed (they are skipped).
pub fn remove_dead_sessions(base: &Path, leave: &dyn Fn(&Path)) -> Vec<String> {
    remove_dead_sessions_with(base, session_id().as_deref(), &process_alive, leave, &|dir| fs::remove_dir_all(dir))
}

pub(crate) fn remove_dead_sessions_with(
    base: &Path,
    own: Option<&str>,
    alive: &dyn Fn(u32) -> bool,
    leave: &dyn Fn(&Path),
    remove: &dyn Fn(&Path) -> io::Result<()>,
) -> Vec<String> {
    list_sessions(base)
        .into_iter()
        .filter(|(id, dir)| Some(id.as_str()) != own && ended_without_cleanup(dir) && !has_live_node(dir, alive, None))
        .filter_map(|(id, dir)| {
            // The keys are in the folder, so the row goes first.
            if dir.join(relay_key::KEY_FILE).is_file() && dir.join(relay_key::SHORT_FILE).is_file() {
                leave(&dir);
            }
            remove(&dir).err().map(|_| id)
        })
        .collect()
}

/// Its control files are all for processes that have ended (and there is at least one).
fn ended_without_cleanup(dir: &Path) -> bool {
    control_pids(dir).is_some_and(|pids| !pids.is_empty())
}

/// Whether another live node (not this process) has a control file in this folder.
pub fn other_node_running(dir: &Path) -> bool {
    has_live_node(dir, &process_alive, Some(std::process::id()))
}

/// This node's own folder, removed when the node stops. The default folder is never removed. A folder that another live
/// node still uses is kept. Returns the id when the removal failed (the stop goes on).
pub fn remove_own_session() -> Option<String> {
    let (id, dir) = SESSION.get()?;
    remove_session_dir(id, dir, std::process::id(), &process_alive, &|dir| fs::remove_dir_all(dir))
}

pub(crate) fn remove_session_dir(
    id: &str,
    dir: &Path,
    own_pid: u32,
    alive: &dyn Fn(u32) -> bool,
    remove: &dyn Fn(&Path) -> io::Result<()>,
) -> Option<String> {
    if id == DEFAULT_SESSION || has_live_node(dir, alive, Some(own_pid)) {
        return None;
    }
    remove(dir).err().map(|_| id.to_string())
}

/// The pids of the control-<pid>.json files in a folder; None when the folder cannot be read.
fn control_pids(dir: &Path) -> Option<Vec<u32>> {
    let entries = fs::read_dir(dir).ok()?;
    Some(
        entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.strip_prefix("control-").and_then(|rest| rest.strip_suffix(".json")).and_then(|pid| pid.parse::<u32>().ok())
            })
            .collect(),
    )
}

/// A control file of a live process, other than `except`. An unreadable folder counts as live, so it is kept.
fn has_live_node(dir: &Path, alive: &dyn Fn(u32) -> bool, except: Option<u32>) -> bool {
    match control_pids(dir) {
        None => true,
        Some(pids) => pids.into_iter().any(|pid| Some(pid) != except && alive(pid)),
    }
}

/// The session folders under base (not the default folder): id and folder, sorted by id.
pub fn list_sessions(base: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(sessions_dir(base)) else { return Vec::new() };
    let mut found: Vec<(String, PathBuf)> = entries.filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok().map(|id| (id, entry.path())))
        .filter(|(id, _)| relay_rest::valid_short_id(id))
        .collect();
    found.sort();
    found
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
    fn key_label_is_the_first_eight_digits_of_the_id() {
        let key = [3u8; 32];
        assert_eq!(key_label(&key), &peer_id(&key)[..8]);
        assert_eq!(key_label(&key).len(), 8);
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

    fn scratch(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("herdr-talk-session-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn two_new_sessions_have_different_ids_and_keys() {
        let base = scratch("two");
        let (first_id, first) = new_session(&base).unwrap();
        let (second_id, second) = new_session(&base).unwrap();
        assert_ne!(first_id, second_id);
        assert_ne!(relay_key::short_id(&first).unwrap(), relay_key::short_id(&second).unwrap());
        assert_ne!(relay_key::load_or_create(&first).unwrap(), relay_key::load_or_create(&second).unwrap());
        assert!(!base.join(relay_key::KEY_FILE).exists(), "the default folder is not touched");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn an_existing_session_must_exist_and_default_is_the_base_folder() {
        let base = scratch("existing");
        let (id, dir) = new_session(&base).unwrap();
        assert_eq!(existing_session(&base, &id).unwrap(), (id.clone(), dir));
        assert_eq!(existing_session(&base, DEFAULT_SESSION).unwrap(), (DEFAULT_SESSION.to_string(), base.clone()));
        assert!(existing_session(&base, "abcd2345").is_err());
        assert!(existing_session(&base, "../x").is_err());
        fs::remove_dir_all(&base).unwrap();
    }

    /// A node folder, with a control file for `pid` when given.
    fn node(base: &Path, id: &str, pid: Option<u32>) -> PathBuf {
        let dir = sessions_dir(base).join(id);
        fs::create_dir_all(&dir).unwrap();
        if let Some(pid) = pid {
            fs::write(dir.join(format!("control-{pid}.json")), b"{}").unwrap();
        }
        dir
    }

    #[test]
    fn dead_nodes_go_and_live_or_unknown_ones_stay() {
        let base = scratch("cleanup-dead");
        let dead = node(&base, "abcd2345", Some(4000001));
        let live = node(&base, "abcd2346", Some(4242));
        let unknown = node(&base, "abcd2347", None);
        let failed = remove_dead_sessions_with(&base, None, &|pid| pid == 4242, &|_| {}, &|dir| fs::remove_dir_all(dir));
        assert!(failed.is_empty());
        assert!(!dead.exists());
        assert!(live.exists(), "a live node keeps its folder");
        assert!(unknown.exists(), "a folder without a control file may still be starting");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_dead_node_with_keys_leaves_once_before_its_folder_goes() {
        let base = scratch("cleanup-leave");
        let (keyed_id, keyed) = new_session(&base).unwrap();
        relay_key::load_or_create(&keyed).unwrap();
        relay_key::short_id(&keyed).unwrap();
        fs::write(keyed.join("control-4000001.json"), b"{}").unwrap();
        let keyless = node(&base, "abcd2345", Some(4000001));
        let (live_id, live) = new_session(&base).unwrap();
        relay_key::load_or_create(&live).unwrap();
        relay_key::short_id(&live).unwrap();
        fs::write(live.join("control-4242.json"), b"{}").unwrap();
        assert_ne!(keyed_id, live_id);
        let left = std::cell::RefCell::new(Vec::new());
        let leave = |dir: &Path| {
            assert!(dir.join(relay_key::KEY_FILE).is_file(), "the folder is still there when the row goes");
            left.borrow_mut().push(dir.to_path_buf());
        };
        let failed = remove_dead_sessions_with(&base, None, &|pid| pid == 4242, &leave, &|dir| fs::remove_dir_all(dir));
        assert!(failed.is_empty());
        assert_eq!(*left.borrow(), vec![keyed.clone()], "once for the dead node with keys, never for a keyless or live one");
        assert!(!keyed.exists() && !keyless.exists() && live.exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_own_session_is_kept_even_when_its_old_node_is_dead() {
        let base = scratch("cleanup-own");
        let mine = node(&base, "abcd2345", Some(4000001));
        let other = node(&base, "abcd2346", Some(4000001));
        assert!(remove_dead_sessions_with(&base, Some("abcd2345"), &|_| false, &|_| {}, &|dir| fs::remove_dir_all(dir)).is_empty());
        assert!(mine.exists() && !other.exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_folder_that_cannot_be_removed_is_skipped_and_named() {
        let base = scratch("cleanup-fail");
        let dead = node(&base, "abcd2345", Some(4000001));
        let failed = remove_dead_sessions_with(&base, None, &|_| false, &|_| {}, &|_| Err(io::Error::other("locked")));
        assert_eq!(failed, vec!["abcd2345".to_string()]);
        assert!(dead.exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_default_folder_is_never_removed() {
        let base = scratch("cleanup-default");
        fs::create_dir_all(&base).unwrap();
        let control = base.join("control-4000001.json");
        fs::write(&control, b"{}").unwrap();
        assert!(remove_dead_sessions_with(&base, None, &|_| false, &|_| {}, &|dir| fs::remove_dir_all(dir)).is_empty());
        assert!(control.exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn an_own_folder_goes_when_no_other_live_node_is_in_it() {
        let base = scratch("own");
        let mine = node(&base, "abcd2345", Some(1));
        assert_eq!(remove_session_dir("abcd2345", &mine, 1, &|_| false, &|dir| fs::remove_dir_all(dir)), None);
        assert!(!mine.exists());
        let shared = node(&base, "abcd2346", Some(4242));
        assert_eq!(remove_session_dir("abcd2346", &shared, 1, &|pid| pid == 4242, &|dir| fs::remove_dir_all(dir)), None);
        assert!(shared.exists(), "another live node still uses it");
        let refused = remove_session_dir("abcd2346", &shared, 1, &|_| false, &|_| Err(io::Error::other("locked")));
        assert_eq!(refused, Some("abcd2346".to_string()));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_default_folder_is_never_removed_on_stop() {
        let base = scratch("own-default");
        fs::create_dir_all(&base).unwrap();
        assert_eq!(remove_session_dir(DEFAULT_SESSION, &base, 1, &|_| false, &|dir| fs::remove_dir_all(dir)), None);
        assert!(base.exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn list_shows_only_session_folders() {
        let base = scratch("list");
        let (id, _) = new_session(&base).unwrap();
        fs::create_dir_all(sessions_dir(&base).join("NotASession")).unwrap();
        let listed: Vec<String> = list_sessions(&base).into_iter().map(|(name, _)| name).collect();
        assert_eq!(listed, vec![id]);
        fs::remove_dir_all(&base).unwrap();
    }
}
