//! The phone's address and token, owned by one node per PC: the host (방장).
//!
//! The host is the node that holds `phone.lock` in the state folder. The lock is an OS file lock, taken atomically and
//! given back by the OS when the process ends, even when it crashes. So of nodes that start together only one wins.
//! The host writes `phone.json` (port, token, URL candidates, its own pid and chat port) and every node on this PC reads it.
//! When the host ends, a node that finds the lock free takes over and keeps the token (the phone need not be set up again).
//! The last node to leave removes `phone.json`. Nothing here is sent over the network.

use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 47860;
const LOCK_FILE: &str = "phone.lock";
const PHONE_FILE: &str = "phone.json";
const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

/// What the phone needs, as stored in `phone.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Phone {
    pub port: u16,
    pub token: String,
    pub urls: Vec<String>,
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
        port: u16::try_from(value["port"].as_u64()?).ok()?,
        token: value["token"].as_str()?.to_string(),
        urls: value["urls"].as_array()?.iter().filter_map(|url| url.as_str().map(str::to_string)).collect(),
        owner_pid: u32::try_from(value["owner_pid"].as_u64().unwrap_or(0)).unwrap_or(0),
        owner_port: u16::try_from(value["owner_port"].as_u64().unwrap_or(0)).unwrap_or(0),
    })
}

fn write(state: &Path, phone: &Phone) -> io::Result<()> {
    let text = json!({"port": phone.port, "token": phone.token, "urls": phone.urls, "owner_pid": phone.owner_pid, "owner_port": phone.owner_port});
    // A new file each time, so the token is never half written; only this user may read it (0o600; on Windows the profile ACL applies).
    let temp = state.join(format!("{PHONE_FILE}.{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&temp)?.write_all(text.to_string().as_bytes())?;
    fs::rename(&temp, phone_file(state))
}

/// Six digits, 000000 to 999999, from the OS random source. Values above the largest multiple of a million are redrawn, so no digit is likelier.
pub fn new_token() -> io::Result<String> {
    loop {
        let mut bytes = [0u8; 4];
        getrandom::getrandom(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
        let value = u32::from_le_bytes(bytes);
        if value < 4_294_000_000 {
            return Ok(format!("{:06}", value % 1_000_000));
        }
    }
}

/// A new token in `phone.json`, kept for the other fields. None when there is no `phone.json`.
pub fn regenerate_token(state: &Path) -> io::Result<Option<()>> {
    let Some(mut phone) = read(state) else { return Ok(None) };
    phone.token = new_token()?;
    write(state, &phone)?;
    Ok(Some(()))
}

/// Phone port: TALK_BRIDGE_PORT as the bridge reads it, else the default.
pub fn bridge_port() -> u16 {
    std::env::var("TALK_BRIDGE_PORT").ok().and_then(|port| port.parse().ok()).unwrap_or(DEFAULT_PORT)
}

/// Addresses the phone can try, best first: this PC's default-route address, then other private addresses,
/// and virtual adapters (VirtualBox host-only 192.168.56.x, link-local, Docker) last.
pub fn url_candidates(port: u16) -> Vec<String> {
    let default = UdpSocket::bind(("0.0.0.0", 0)).ok()
        .and_then(|probe| probe.connect(("192.0.2.1", 9)).ok().and_then(|()| probe.local_addr().ok()))
        .and_then(|address| match address.ip() { IpAddr::V4(ip) => Some(ip), IpAddr::V6(_) => None });
    let mut all: Vec<Ipv4Addr> = default.into_iter().collect();
    let host = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME"))
        .or_else(|_| fs::read_to_string("/etc/hostname").map(|name| name.trim().to_string())).unwrap_or_default();
    if !host.is_empty() {
        for address in (host.as_str(), 0u16).to_socket_addrs().into_iter().flatten() {
            if let IpAddr::V4(ip) = address.ip() {
                all.push(ip);
            }
        }
    }
    rank(all, default, port)
}

fn rank(addresses: Vec<Ipv4Addr>, default: Option<Ipv4Addr>, port: u16) -> Vec<String> {
    let mut found: Vec<Ipv4Addr> = Vec::new();
    for ip in addresses {
        if !ip.is_loopback() && !ip.is_unspecified() && !found.contains(&ip) {
            found.push(ip);
        }
    }
    let virtual_adapter = |ip: &Ipv4Addr| {
        let o = ip.octets();
        ip.is_link_local() || (o[0] == 192 && o[1] == 168 && o[2] == 56) || (o[0] == 172 && o[1] == 17)
    };
    found.sort_by_key(|ip| (virtual_adapter(ip), Some(*ip) != default, !ip.is_private()));
    found.into_iter().map(|ip| format!("http://{ip}:{port}")).collect()
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
        // Others still alive and a file left by the old host: this is a take-over, so the phone keeps its token.
        let kept = if other_live_nodes(state) > 0 { read(state) } else { None };
        let port = kept.as_ref().map_or_else(bridge_port, |phone| phone.port);
        let token = match kept {
            Some(phone) => phone.token,
            None => new_token()?,
        };
        write(state, &Phone { port, token, urls: url_candidates(port), owner_pid: std::process::id(), owner_port: chat_port })?;
        Ok(Some(Host { _lock: lock, state: state.to_path_buf() }))
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // The pid in the file is ours until a successor rewrites it; the file stays for the successor's token.
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
    fn token_is_six_digits_with_leading_zeros_kept() {
        for _ in 0..200 {
            let token = new_token().unwrap();
            assert_eq!(token.len(), 6);
            assert!(token.bytes().all(|byte| byte.is_ascii_digit()));
        }
        assert_eq!(format!("{:06}", 42), "000042");
    }

    #[test]
    fn only_one_node_is_host_and_the_next_takes_over() {
        let dir = folder();
        let first = Host::try_take(&dir, 1111).unwrap().expect("first node is the host");
        let phone = read(&dir).unwrap();
        assert_eq!((phone.owner_pid, phone.owner_port, phone.port), (std::process::id(), 1111, bridge_port()));
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
    fn phone_json_has_the_agreed_fields_and_regenerate_keeps_them() {
        let dir = folder();
        let host = Host::try_take(&dir, 7).unwrap().unwrap();
        let value: Value = serde_json::from_str(&fs::read_to_string(phone_file(&dir)).unwrap()).unwrap();
        for field in ["port", "token", "urls", "owner_pid", "owner_port"] {
            assert!(value.get(field).is_some(), "{field}");
        }
        let before = read(&dir).unwrap();
        assert_eq!(regenerate_token(&dir).unwrap(), Some(()));
        let after = read(&dir).unwrap();
        assert_eq!((after.port, &after.urls, after.owner_pid), (before.port, &before.urls, before.owner_pid));
        assert_eq!(after.token.len(), 6);
        drop(host);
        assert_eq!(regenerate_token(&dir).unwrap(), None);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn addresses_are_ranked_for_a_phone() {
        let ip = |text: &str| text.parse::<Ipv4Addr>().unwrap();
        let ranked = rank(
            vec![ip("192.168.56.1"), ip("169.254.1.2"), ip("10.0.0.5"), ip("192.168.0.181"), ip("127.0.0.1"), ip("192.168.0.181"), ip("203.0.113.9")],
            Some(ip("192.168.0.181")), 47860,
        );
        assert_eq!(ranked, [
            "http://192.168.0.181:47860", "http://10.0.0.5:47860", "http://203.0.113.9:47860",
            "http://192.168.56.1:47860", "http://169.254.1.2:47860",
        ]);
    }
}
