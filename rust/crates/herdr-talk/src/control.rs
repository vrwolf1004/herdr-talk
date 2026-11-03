//! Local control channel for a GUI: newline-delimited JSON over 127.0.0.1 with a per-run token.
//! The port and token go into `control-<pid>.json` in the state folder, readable only by this user:
//! on Unix the folder is made 0o700 and the file 0o600; on Windows the default ACL of %LOCALAPPDATA% already
//! admits only this user, SYSTEM and Administrators, and the file inherits it.

use serde_json::{json, Value};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use talk_core::{Chat, Line};

const MAX_REQUEST_BYTES: u64 = 64 * 1024;
const MAX_CLIENTS: usize = 4;
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const LINES_SHOWN: usize = 300;

/// Owns the control file; removes it when the chat ends.
pub struct Control {
    file: PathBuf,
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.file);
    }
}

pub fn control_file(state: &Path) -> PathBuf {
    state.join(format!("control-{}.json", std::process::id()))
}

/// Start the listener on a free loopback port and write the control file.
/// With `allow_shutdown`, a client may end the chat (the headless `serve` mode, which has no terminal to close).
pub fn serve(chat: Arc<Chat>, state: &Path, allow_shutdown: bool) -> io::Result<Control> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let token = new_token()?;
    create_private_dir(state)?;
    let file = control_file(state);
    write_private(&file, &json!({"port": port, "token": token, "pid": std::process::id()}).to_string())?;
    let active = Arc::new(AtomicUsize::new(0));
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if active.load(Ordering::SeqCst) >= MAX_CLIENTS {
                continue;
            }
            active.fetch_add(1, Ordering::SeqCst);
            let (chat, token, active) = (chat.clone(), token.clone(), active.clone());
            thread::spawn(move || {
                let _ = client(stream, &chat, &token, allow_shutdown);
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Ok(Control { file })
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)
}

/// A new file only this user can read. A leftover file of a reused pid is replaced, not reopened.
fn write_private(file: &Path, contents: &str) -> io::Result<()> {
    match fs::remove_file(file) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(file)?.write_all(contents.as_bytes())
}

fn new_token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn client(stream: TcpStream, chat: &Chat, token: &str, allow_shutdown: bool) -> io::Result<()> {
    stream.set_read_timeout(Some(IDLE_TIMEOUT))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let first = read_request(&mut reader)?;
    if first.get("token").and_then(Value::as_str) != Some(token) {
        return reply(&mut writer, json!({"ok": false, "error": "bad token"}));
    }
    reply(&mut writer, json!({"ok": true}))?;
    loop {
        let request = read_request(&mut reader)?;
        let answer = answer(chat, &request, allow_shutdown);
        reply(&mut writer, answer)?;
    }
}

/// One request line. Anything longer than the limit closes the connection.
fn read_request(reader: &mut impl BufRead) -> io::Result<Value> {
    let mut line = Vec::new();
    loop {
        let buffered = reader.fill_buf()?;
        if buffered.is_empty() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed"));
        }
        let end = buffered.iter().position(|&byte| byte == b'\n').map_or(buffered.len(), |at| at + 1);
        line.extend_from_slice(&buffered[..end]);
        reader.consume(end);
        if line.last() == Some(&b'\n') {
            break;
        }
        if line.len() as u64 > MAX_REQUEST_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "request too long"));
        }
    }
    serde_json::from_slice(&line).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn answer(chat: &Chat, request: &Value, allow_shutdown: bool) -> Value {
    match request.get("cmd").and_then(Value::as_str) {
        Some("lines") => {
            let lines: Vec<Line> = chat.lines();
            let start = lines.len().saturating_sub(LINES_SHOWN);
            let rows: Vec<Value> = lines[start..]
                .iter()
                .map(|line| json!({"text": line.render(), "mine": line.mine}))
                .collect();
            json!({"ok": true, "lines": rows})
        }
        Some("peers") => {
            let peers: Vec<Value> = chat
                .fresh_peers()
                .iter()
                .map(|peer| json!({"name": peer.name, "ip": peer.ip, "port": peer.port}))
                .collect();
            // This chat's own name and port, so a GUI can list it above the others.
            let me = chat.me.lock().unwrap().clone();
            json!({"ok": true, "peers": peers, "me": {"name": me.name, "port": me.port}})
        }
        Some("send") => match request.get("text").and_then(Value::as_str) {
            // Same path as typing in the terminal: commands and plain messages both work.
            Some(text) => {
                chat.handle(text);
                json!({"ok": true})
            }
            None => json!({"ok": false, "error": "text missing"}),
        },
        Some("shutdown") if allow_shutdown => {
            chat.stop.set();
            json!({"ok": true})
        }
        _ => json!({"ok": false, "error": "unknown cmd"}),
    }
}

fn reply(writer: &mut impl Write, value: Value) -> io::Result<()> {
    writer.write_all(format!("{value}\n").as_bytes())?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_file_is_private_and_replaced() {
        let dir = std::env::temp_dir().join(format!("herdr-talk-control-test-{}", std::process::id()));
        create_private_dir(&dir).unwrap();
        let file = dir.join("c.json");
        write_private(&file, "old").unwrap();
        write_private(&file, "new").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
