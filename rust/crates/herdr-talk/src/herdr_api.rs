//! herdr calls: the CLI (herdr binary) and the socket API.
//! The socket is a Unix domain socket on Unix and a named pipe with the same path on Windows.

use serde_json::{json, Value};
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CLI_TIMEOUT_SECONDS: f64 = 10.0; // override with HERDR_CLI_TIMEOUT_SECONDS

fn cli_timeout() -> Duration {
    let seconds = env::var("HERDR_CLI_TIMEOUT_SECONDS").ok().and_then(|value| value.parse::<f64>().ok());
    Duration::from_secs_f64(seconds.filter(|value| *value > 0.0 && value.is_finite()).unwrap_or(CLI_TIMEOUT_SECONDS))
}

fn herdr_bin() -> String {
    env::var("HERDR_BIN_PATH").ok().filter(|path| !path.is_empty()).unwrap_or_else(|| "herdr".to_string())
}

fn socket_path() -> io::Result<String> {
    if let Some(path) = env::var("HERDR_SOCKET_PATH").ok().filter(|path| !path.is_empty()) {
        return Ok(path);
    }
    if cfg!(windows) {
        let appdata = env::var("APPDATA")
            .map_err(|_| io::Error::other("APPDATA is not set; set HERDR_SOCKET_PATH to the herdr socket"))?;
        return Ok(Path::new(&appdata).join("herdr").join("herdr.sock").to_string_lossy().into_owned());
    }
    let home = env::var("HOME").map_err(|_| io::Error::other("HOME is not set; set HERDR_SOCKET_PATH to the herdr socket"))?;
    Ok(format!("{home}/.config/herdr/herdr.sock"))
}

struct TempFile(PathBuf);

impl TempFile {
    fn new(suffix: &str) -> Self {
        TempFile(env::temp_dir().join(format!("herdr-talk-{:016x}.{suffix}", fastrand::u64(..))))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Run the herdr CLI and return the result of its JSON answer.
/// Output goes to files, not pipes: a killed child's own children can keep a pipe open and hang the read.
pub fn cli(args: &[&str]) -> Result<Value, String> {
    let label = format!("herdr {}", args.join(" "));
    let (out, err) = (TempFile::new("out"), TempFile::new("err"));
    let open = |file: &TempFile| File::create(&file.0).map_err(|error| format!("{label}: {error}"));
    let mut child = Command::new(herdr_bin()).args(args)
        .stdin(Stdio::null()).stdout(open(&out)?).stderr(open(&err)?)
        .spawn().map_err(|error| format!("{label}: {error}"))?;
    let timeout = cli_timeout();
    let started = Instant::now();
    let status = loop {
        match child.try_wait().map_err(|error| format!("{label}: {error}"))? {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{label}: no response within {} s", timeout.as_secs_f64()));
            }
            None => thread::sleep(Duration::from_millis(10)),
        }
    };
    let read = |file: &TempFile| String::from_utf8_lossy(&fs::read(&file.0).unwrap_or_default()).into_owned();
    let stdout = read(&out);
    if !status.success() {
        return Err(format!("{label}: {}", (stdout + &read(&err)).trim()));
    }
    let output = stdout.trim().trim_start_matches('\u{feff}');
    if output.is_empty() {
        return Ok(json!({}));
    }
    let answer: Value = serde_json::from_str(output).map_err(|error| format!("{label}: {error}"))?;
    Ok(answer.get("result").cloned().unwrap_or_else(|| json!({})))
}

#[cfg(windows)]
fn exchange(path: &str, payload: &[u8]) -> io::Result<String> {
    let mut pipe = fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\{path}"))?;
    pipe.write_all(payload)?;
    read_line(pipe)
}

#[cfg(unix)]
fn exchange(path: &str, payload: &[u8]) -> io::Result<String> {
    const SOCKET_TIMEOUT: Duration = Duration::from_secs(3);
    let mut client = std::os::unix::net::UnixStream::connect(path)?;
    client.set_read_timeout(Some(SOCKET_TIMEOUT))?;
    client.set_write_timeout(Some(SOCKET_TIMEOUT))?;
    client.write_all(payload)?;
    read_line(client)
}

fn read_line(stream: impl io::Read) -> io::Result<String> {
    let mut line = Vec::new();
    BufReader::new(stream).read_until(b'\n', &mut line)?;
    Ok(String::from_utf8_lossy(&line).into_owned())
}

fn request(method: &str, params: Value) -> Result<(), String> {
    let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let message = json!({"id": format!("timekit:{time}"), "method": method, "params": params});
    let raw = socket_path().and_then(|path| exchange(&path, format!("{message}\n").as_bytes()))
        .map_err(|error| format!("{method}: {error}"))?;
    if raw.trim().is_empty() {
        return Ok(());
    }
    let response: Value = serde_json::from_str(&raw).map_err(|error| format!("{method}: {error}"))?;
    response.get("error").map_or(Ok(()), |error| Err(format!("{method}: {error}")))
}

/// A request that must not stop the loop when it fails.
pub fn try_request(method: &str, params: Value) -> bool {
    request(method, params).is_ok()
}
