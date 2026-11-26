//! The host (방장) of this PC: the node that holds `phone.lock` in the state folder. The lock is an OS file lock, taken
//! atomically and given back by the OS when the process ends, even when it crashes. So of nodes that start together only
//! one wins. The host writes `phone.json` (its own pid and chat port) and every node on this PC can read it.
//! The last node to leave removes `phone.json`. Nothing here is sent over the network.

use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const LOCK_FILE: &str = "phone.lock";
const PHONE_FILE: &str = "phone.json";
const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

/// The host recorded in `phone.json`: its process and its chat port.
#[derive(Clone, Debug, PartialEq)]
pub struct Phone {
    pub owner_pid: u32,
    /// The host's chat port: how another node on this PC tells which announcement is the host.
    pub owner_port: u16,
}

pub fn phone_file(state: &Path) -> PathBuf {
    state.join(PHONE_FILE)
}

pub fn read(state: &Path) -> Option<Phone> {
    let value: Value = serde_json::from_str(&fs::read_to_string(phone_file(state)).ok()?).ok()?;
    Some(Phone {
        owner_pid: u32::try_from(value["owner_pid"].as_u64().unwrap_or(0)).unwrap_or(0),
        owner_port: u16::try_from(value["owner_port"].as_u64().unwrap_or(0)).unwrap_or(0),
    })
}

fn write(state: &Path, phone: &Phone) -> io::Result<()> {
    let text = json!({"owner_pid": phone.owner_pid, "owner_port": phone.owner_port});
    // A new file each time, so it is never half written; only this user may read it (0o600; on Windows the profile ACL applies).
    let temp = state.join(format!("{PHONE_FILE}.{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&temp)?.write_all(text.to_string().as_bytes())?;
    fs::rename(&temp, phone_file(state))
}

/// Control files of other live nodes in this state folder. A node is live when its control port accepts a connection.
pub fn other_live_nodes(state: &Path) -> usize {
    let me = std::process::id();
    let Ok(entries) = fs::read_dir(state) else { return 0 };
    entries.flatten().filter(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name.strip_prefix("control-").and_then(|rest| rest.strip_suffix(".json")).and_then(|pid| pid.parse::<u32>().ok()) else {
            return false;
        };
        let port = fs::read_to_string(entry.path()).ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value["port"].as_u64()).and_then(|port| u16::try_from(port).ok());
        pid != me && port.is_some_and(|port| TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), PROBE_TIMEOUT).is_ok())
    }).count()
}

/// Holds the host lock. Dropping it gives the lock back and, when no other node is left, removes `phone.json`.
pub struct Host {
    _lock: File,
    state: PathBuf,
}

impl Host {
    /// Take the host lock if it is free. `chat_port` is this node's chat port. None when another node is the host.
    pub fn try_take(state: &Path, chat_port: u16) -> io::Result<Option<Host>> {
        fs::create_dir_all(state)?;
        let lock = OpenOptions::new().create(true).read(true).write(true).truncate(false).open(state.join(LOCK_FILE))?;
        if lock.try_lock().is_err() {
            return Ok(None);
        }
        write(state, &Phone { owner_pid: std::process::id(), owner_port: chat_port })?;
        Ok(Some(Host { _lock: lock, state: state.to_path_buf() }))
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // The file stays while another node is alive: it is rewritten by the successor when it takes over.
        if other_live_nodes(&self.state) == 0 {
            let _ = fs::remove_file(phone_file(&self.state));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> PathBuf {
        std::env::temp_dir().join(format!("herdr-talk-phone-{:016x}", fastrand::u64(..)))
    }

    #[test]
    fn only_one_node_is_host_and_the_next_takes_over() {
        let dir = folder();
        let first = Host::try_take(&dir, 1111).unwrap().expect("first node is the host");
        let phone = read(&dir).unwrap();
        assert_eq!((phone.owner_pid, phone.owner_port), (std::process::id(), 1111));
        assert!(Host::try_take(&dir, 2222).unwrap().is_none(), "a second node is not the host");
        assert_eq!(read(&dir).unwrap(), phone, "and does not touch phone.json");
        drop(first);
        // Nobody else is alive, so the file went with the host.
        assert!(read(&dir).is_none());
        let again = Host::try_take(&dir, 2222).unwrap().expect("the lock is free after the host ends");
        assert_eq!(read(&dir).unwrap().owner_port, 2222);
        drop(again);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn phone_json_holds_only_the_owner() {
        let dir = folder();
        let host = Host::try_take(&dir, 7).unwrap().unwrap();
        let value: Value = serde_json::from_str(&fs::read_to_string(phone_file(&dir)).unwrap()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 2, "no token, no address: only the owner fields");
        assert!(value.get("owner_pid").is_some() && value.get("owner_port").is_some());
        drop(host);
        fs::remove_dir_all(dir).ok();
    }
}
