//! Logic of the window: the state shared with the worker, the control-channel link to herdr-talk,
//! the bottom-line and status texts, and the panic log. Nothing here draws.

use talk_core::clock::Stamps;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub(crate) const POLL: Duration = Duration::from_millis(500);
pub(crate) const IO_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const RETRY: Duration = Duration::from_secs(2);
/// Height of the message box and the send button.
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub(crate) const MAX_STARTS: u32 = 3;
// Status phases, in the order they show: no server of ours yet, our own serve connected, or the chat built into this window.
// The status texts are compared by their start (see status_label), so each one is read through the same function.
pub(crate) fn status_starting() -> String {
    talk_core::t("gui.status.starting", &[])
}

pub(crate) fn status_connected() -> String {
    talk_core::t("gui.status.connected", &[])
}

pub(crate) fn status_embedded() -> String {
    talk_core::t("gui.status.embedded", &[])
}
pub(crate) const START_WAIT: Duration = Duration::from_secs(10);
pub(crate) const STOP_WAIT: Duration = Duration::from_secs(3);

/// The `herdr-talk serve` this window started, if any.
pub(crate) type Started = Arc<Mutex<Option<Child>>>;

#[derive(Clone)]
pub(crate) struct Control {
    pub(crate) port: u16,
    pub(crate) token: String,
}

#[derive(Default)]
pub(crate) struct Shared {
    pub(crate) status: String,
    pub(crate) lines: Vec<(String, bool)>,
    pub(crate) peers: Vec<String>,
    /// The other people in the room (Supabase mode), by name.
    pub(crate) room_names: Vec<String>,
    /// The chat's host flag: in Supabase mode the room owner only (talk-core's is_host).
    pub(crate) host: bool,
    /// This chat's own name and port. None until the chat says it (an older herdr-talk never does).
    pub(crate) me: Option<(String, u16)>,
    /// The relay state as the chat reports it ("" when the relay is off).
    pub(crate) relay: String,
    /// The room ID and setup code as the chat reports them ("" when no room is saved or in Local mode).
    pub(crate) room: String,
    /// The connection mode as the chat reports it: "local" or "supabase".
    pub(crate) mode: String,
    /// Ctrl+L was pressed: the chat clears its message lines (in the node it runs in).
    pub(crate) clear: bool,
    /// Peers waiting for a /join, in the terminal's numbering.
    pub(crate) pending: Vec<Waiting>,
}

/// The address this PC uses to reach the network. Connecting a UDP socket sends nothing; it only picks the address.
pub(crate) fn local_ip() -> String {
    std::net::UdpSocket::bind(("0.0.0.0", 0))
        .and_then(|socket| socket.connect(("192.0.2.1", 9)).and_then(|()| socket.local_addr()))
        .map_or_else(|_| "127.0.0.1".to_string(), |address| address.ip().to_string())
}

/// This window's session folder (chosen at start, see main).
pub(crate) fn state_dir() -> PathBuf {
    talk_core::store::state_dir()
}

pub(crate) fn read_control(file: &Path) -> Option<Control> {
    let value: Value = serde_json::from_str(&fs::read_to_string(file).ok()?).ok()?;
    Some(Control {
        port: u16::try_from(value["port"].as_u64()?).ok()?,
        token: value["token"].as_str()?.to_string(),
    })
}

pub(crate) struct Link {
    pub(crate) writer: TcpStream,
    pub(crate) reader: BufReader<TcpStream>,
}

impl Link {
    pub(crate) fn open(control: &Control) -> std::io::Result<Self> {
        let address = SocketAddr::from(([127, 0, 0, 1], control.port));
        let stream = TcpStream::connect_timeout(&address, IO_TIMEOUT)?;
        stream.set_read_timeout(Some(IO_TIMEOUT))?;
        stream.set_write_timeout(Some(IO_TIMEOUT))?;
        let mut link = Link { writer: stream.try_clone()?, reader: BufReader::new(stream) };
        let answer = link.call(&json!({"token": control.token}))?;
        if answer["ok"] != true {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "token refused"));
        }
        Ok(link)
    }

    pub(crate) fn call(&mut self, request: &Value) -> std::io::Result<Value> {
        self.writer.write_all(format!("{request}\n").as_bytes())?;
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "herdr-talk closed the connection"));
        }
        serde_json::from_str(&line).map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }
}

/// Where herdr-talk.exe may be, in the order they are tried: next to this program, HERDR_TALK_EXE,
/// the repository build (rust/target/release), then the herdr plugin install: the newest of
/// %APPDATA%/herdr/plugins/github-installations/herdr-talk-*/*/checkout/rust/bin.
pub(crate) fn talk_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
        paths.push(dir.join("herdr-talk.exe"));
    }
    if let Some(path) = std::env::var_os("HERDR_TALK_EXE").filter(|path| !path.is_empty()) {
        paths.push(PathBuf::from(path));
    }
    paths.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("rust").join("target").join("release").join("herdr-talk.exe"));
    let installs = std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join("herdr").join("plugins").join("github-installations"));
    let mut installed: Vec<PathBuf> = installs.iter()
        .flat_map(|dir| fs::read_dir(dir).into_iter().flatten().flatten())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("herdr-talk-"))
        .flat_map(|entry| fs::read_dir(entry.path()).into_iter().flatten().flatten())
        .map(|version| version.path().join("checkout").join("rust").join("bin").join("herdr-talk.exe"))
        .collect();
    installed.sort_by_key(|path| std::cmp::Reverse(fs::metadata(path).and_then(|meta| meta.modified()).ok()));
    paths.extend(installed);
    paths
}

/// The first candidate that exists. Err names every path that was looked at.
pub(crate) fn talk_program() -> Result<PathBuf, String> {
    let candidates = talk_candidates();
    candidates.iter().find(|path| path.is_file()).cloned().ok_or_else(|| {
        let looked: Vec<String> = candidates.iter().map(|path| path.display().to_string()).collect();
        talk_core::t("gui.err.exe_not_found", &[("paths", &looked.join(" | "))])
    })
}

/// Ask the child to end, which removes its control file, and kill it only if it does not.
pub(crate) fn stop_serve(started: &Started) {
    let Some(mut child) = locked(&**started, "stop_serve").take() else { return };
    let file = control_file(&state_dir(), child.id());
    if let Some(Ok(mut link)) = read_control(&file).map(|control| Link::open(&control)) {
        let _ = link.call(&json!({"cmd": "shutdown"}));
    }
    let deadline = std::time::Instant::now() + STOP_WAIT;
    while std::time::Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Start `herdr-talk serve` unless one of ours is alive. Err is the reason it cannot be started.
pub(crate) fn ensure_serve(started: &Started, starts: &mut u32) -> Result<(), String> {
    let mut child = locked(&**started, "ensure_serve");
    if child.as_mut().is_some_and(|child| matches!(child.try_wait(), Ok(Some(_)))) {
        *child = None;
    }
    if child.is_some() {
        return Ok(());
    }
    if *starts >= MAX_STARTS {
        return Err(talk_core::t("gui.err.serve_exits", &[("max", &MAX_STARTS.to_string())]));
    }
    *starts += 1;
    let program = talk_program()?;
    // No --name: the serve node picks its automatic person name for the language, like the TUI does.
    // The serve node uses this window's session, so both share the same identity, trust and relay keys.
    let session = talk_core::store::session_id().unwrap_or_else(|| talk_core::store::DEFAULT_SESSION.to_string());
    let spawned = Command::new(&program).arg("serve").arg("--session").arg(session).creation_flags(CREATE_NO_WINDOW).spawn()
        .map_err(|error| talk_core::t("gui.err.spawn", &[("error", &error.to_string())]))?;
    *child = Some(spawned);
    Ok(())
}

pub(crate) fn set_status(shared: &Arc<Mutex<Shared>>, text: &str) {
    locked(&**shared, "set_status").status = text.to_string();
}

/// Connect to the serve this window started, through its own control file. Another window's serve or a
/// herdr-talk pane is never used, so every window is a node of its own.
pub(crate) fn connect_own(dir: &Path, started: &Started) -> Option<(Link, Control)> {
    let pid = locked(&**started, "connect_own").as_ref()?.id();
    let control = read_control(&control_file(dir, pid))?;
    Link::open(&control).ok().map(|link| (link, control))
}

pub(crate) fn control_file(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("control-{pid}.json"))
}

/// Worker, first choice: start `herdr-talk serve` hidden and connect to it.
/// When that fails (program missing, no control file in time), the chat runs inside this window instead.
pub(crate) fn worker(shared: Arc<Mutex<Shared>>, outgoing: Receiver<String>, started: Started, reload: Arc<AtomicBool>) {
    let dir = state_dir();
    let (mut starts, mut waiting_since) = (0, None);
    loop {
        let Some((mut link, control)) = connect_own(&dir, &started) else {
            let since = *waiting_since.get_or_insert_with(std::time::Instant::now);
            let failure = match ensure_serve(&started, &mut starts) {
                Err(reason) => Some(reason),
                Ok(()) if since.elapsed() > START_WAIT => Some(talk_core::t("gui.err.connect_timeout", &[("secs", &START_WAIT.as_secs().to_string())])),
                Ok(()) => None,
            };
            if let Some(reason) = failure {
                run_embedded(&shared, &outgoing, &started, &reload, &reason);
            }
            set_status(&shared, &status_starting());
            thread::sleep(Duration::from_millis(500));
            continue;
        };
        waiting_since = None;
        set_status(&shared, &format!("{} (port {})", status_connected(), control.port));
        if let Err(error) = session(&mut link, &shared, &outgoing, &reload) {
            set_status(&shared, &talk_core::t("gui.status.connection_lost", &[("error", &error.to_string())]));
        }
        thread::sleep(RETRY);
    }
}

/// Second choice: the chat runs in this process through talk-core. It does not go back to herdr-talk
/// later (a control file that appears afterwards is not picked up).
pub(crate) fn run_embedded(shared: &Arc<Mutex<Shared>>, outgoing: &Receiver<String>, started: &Started, reload: &AtomicBool, reason: &str) -> ! {
    stop_serve(started); // a serve that is only late would be a second chat on this PC
    let mut stamps = Stamps::default();
    let chat = loop {
        let state = talk_core::store::state_dir();
        let attempt = talk_core::udp_port().map_err(|error| error.to_string()).and_then(|udp| {
            talk_core::net::start_in(None, 0, udp, &state, talk_core::store::plaintext_allowed()).map_err(|error| error.to_string())
        });
        match attempt {
            Ok(chat) => break chat,
            Err(error) => {
                set_status(shared, &talk_core::t("gui.err.embedded_failed", &[("error", &error.to_string()), ("reason", reason)]));
                thread::sleep(RETRY);
            }
        }
    };
    chat.watch_host();
    chat.set_lan_listening(false); // no LAN peer is remembered until the mode is known
    // The relay in this window: a bad setting shows in the status bar, the chat goes on.
    if let Err(error) = talk_core::relay_chat::start(&chat, &talk_core::store::state_dir()) {
        chat.set_relay_status(&format!("relay: {error}"));
    }
    if talk_core::relay_settings::needs_first_run(&talk_core::store::state_dir()) {
        chat.set_lan_listening(false); // the first-run choice is still to come: saving the settings starts the listening
    }
    set_status(shared, &talk_core::t("gui.status.embedded_after", &[("reason", reason)]));
    loop {
        // Settings were saved in this window: restart the embedded relay with them.
        if reload.swap(false, Ordering::SeqCst) {
            if let Err(error) = talk_core::relay_chat::start(&chat, &talk_core::store::state_dir()) {
                chat.set_relay_status(&format!("relay: {error}"));
            }
        }
        if std::mem::take(&mut locked(shared, "run_embedded.clear").clear) {
            chat.clear_lines();
        }
        while let Ok(text) = outgoing.try_recv() {
            chat.handle(&text);
        }
        let mut state = locked(shared, "run_embedded");
        let lines = chat.lines();
        let seqs: Vec<u64> = lines.iter().map(|line| line.seq).collect();
        state.lines = lines.iter().zip(stamps.times(&seqs)).map(|(line, time)| {
            let text = if line.sender.is_some() { format!("[{time}] {}", line.render()) } else { line.render() };
            (text, line.mine)
        }).collect();
        state.me = {
            let me = locked(&chat.me, "run_embedded.me");
            Some((me.name.clone(), me.port))
        };
        state.peers = chat.fresh_peers().iter().map(|peer| format!("{} ({}:{})", peer.name, peer.ip, peer.port)).collect();
        state.room_names = chat.room_names();
        state.host = chat.is_host();
        state.pending = chat.unverified_peers().into_iter().map(|(peer, code, number)| Waiting {
            seconds: peer.key.as_deref().and_then(|key| chat.join_seconds_left(key)).unwrap_or(0),
            number,
            name: peer.name,
            ip: peer.ip,
            code,
        }).collect();
        state.relay = chat.relay_status();
        state.room = chat.room_status().unwrap_or_default();
        state.mode = chat.mode_name().to_string();
        drop(state);
        thread::sleep(POLL);
    }
}

pub(crate) fn session(link: &mut Link, shared: &Arc<Mutex<Shared>>, outgoing: &Receiver<String>, reload: &AtomicBool) -> std::io::Result<()> {
    let mut stamps = Stamps::default();
    loop {
        // Settings were saved in this window: the serve node starts the relay again with them.
        if reload.swap(false, Ordering::SeqCst) {
            link.call(&json!({"cmd": "relay_reload"}))?;
        }
        if std::mem::take(&mut locked(&**shared, "session.clear").clear) {
            link.call(&json!({"cmd": "clear"}))?;
        }
        while let Ok(text) = outgoing.try_recv() {
            let answer = link.call(&json!({"cmd": "send", "text": text}))?;
            if answer["ok"] != true {
                set_status(shared, &talk_core::t("gui.err.send_failed", &[("error", &answer["error"].to_string())]));
            }
        }
        let lines = link.call(&json!({"cmd": "lines"}))?;
        let peers = link.call(&json!({"cmd": "peers"}))?;
        let mut state = locked(&**shared, "session");
        let rows: Vec<&Value> = lines["lines"].as_array().into_iter().flatten().collect();
        let seqs: Vec<u64> = rows.iter().map(|row| row["seq"].as_u64().unwrap_or(0)).collect();
        state.lines = rows.iter().zip(stamps.times(&seqs)).map(|(row, time)| {
            let text = row["text"].as_str().unwrap_or("");
            let text = if row["timed"] == true { format!("[{time}] {text}") } else { text.to_string() };
            (text, row["mine"] == true)
        }).collect();
        state.me = peers["me"]["name"].as_str().zip(peers["me"]["port"].as_u64().and_then(|port| u16::try_from(port).ok()))
            .map(|(name, port)| (name.to_string(), port));
        state.peers = peers["peers"].as_array().into_iter().flatten()
            .map(|peer| format!("{} ({}:{})", peer["name"].as_str().unwrap_or("?"), peer["ip"].as_str().unwrap_or("?"), peer["port"]))
            .collect();
        state.room_names = peers["room"].as_array().into_iter().flatten().filter_map(|name| name.as_str().map(str::to_string)).collect();
        state.host = peers["host"].as_bool().unwrap_or(false);
        state.pending = peers["pending"].as_array().into_iter().flatten().map(|peer| Waiting {
            number: peer["number"].as_u64().unwrap_or(0) as usize,
            name: peer["name"].as_str().unwrap_or("?").to_string(),
            ip: peer["ip"].as_str().unwrap_or("?").to_string(),
            code: peer["code"].as_str().unwrap_or("").to_string(),
            seconds: peer["seconds"].as_u64().unwrap_or(0),
        }).collect();
        state.relay = peers["relay"].as_str().unwrap_or("").to_string();
        state.room = peers["room_status"].as_str().unwrap_or("").to_string();
        state.mode = peers["mode"].as_str().unwrap_or("local").to_string();
        drop(state);
        thread::sleep(POLL);
    }
}

/// How a status text is shown: a short label, and its kind.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Ok,
    Pending,
    Problem,
}

/// A peer waiting for /join: the terminal's number, name and IP, the code to compare, and the seconds left to /join.
#[derive(Clone)]
pub(crate) struct Waiting {
    pub(crate) number: usize,
    pub(crate) name: String,
    pub(crate) ip: String,
    pub(crate) code: String,
    pub(crate) seconds: u64,
}

/// One waiting row in the terminal's notation: "[n] name (ip)  waiting to join · N s left to /join".
pub(crate) fn wait_row(waiting: &Waiting) -> String {
    let left = talk_core::t("nodes.seconds_left", &[("seconds", &waiting.seconds.to_string())]);
    format!("[{}] {} ({})  {} · {left}", waiting.number, waiting.name, waiting.ip, talk_core::t("nodes.waiting", &[]))
}

/// The [방장] badge: in Supabase mode the room owner (the chat's host flag), on the LAN this PC's phone-file owner.
pub(crate) fn hosting(supabase: bool, room_host: bool, lan_host: bool) -> bool {
    if supabase { room_host } else { lan_host }
}

/// The bottom line as Python shows it: "[host] peers · mode · room ID · setup code", then a problem only (see `relay_problem`).
pub(crate) fn bottom_line(hosting: bool, peers: usize, supabase: bool, room: &str, relay: &str) -> String {
    let badge = if hosting { format!("{} ", talk_core::t("host.badge", &[])) } else { String::new() };
    let mode = mode_name(supabase);
    let room = if room.is_empty() { String::new() } else { format!(" · {room}") };
    let relay = relay_problem(relay).map(|text| format!(" · {text}")).unwrap_or_default();
    let peers = talk_core::t("gui.bottom.peers", &[("peers", &peers.to_string())]);
    format!("{badge}{peers} · {mode}{room}{relay}")
}

/// The connection mode's name shown in the bottom line and the header: "CLOUD SERVICE" or "BASE STATION".
pub(crate) fn mode_name(supabase: bool) -> String {
    talk_core::t(if supabase { "gui.mode.cloud_service" } else { "gui.mode.base_station" }, &[])
}

/// The relay text when it is a problem. Success texts are never shown: the chat's only one is "relay: room joined".
pub(crate) fn relay_problem(relay: &str) -> Option<&str> {
    const SUCCESS: [&str; 1] = ["relay: room joined"];
    (!relay.is_empty() && !SUCCESS.iter().any(|success| relay.starts_with(success))).then_some(relay)
}

pub(crate) fn status_label(status: &str, supabase: bool) -> (String, Kind) {
    if status.starts_with(&status_connected()) {
        // In Supabase mode the local serve only runs the chat; the label must not call it our own server.
        (talk_core::t(if supabase { "gui.mode.supabase" } else { "gui.status.connected" }, &[]), Kind::Ok)
    } else if status.starts_with(&status_embedded()) {
        (talk_core::t("gui.mode.embedded", &[]), Kind::Ok)
    } else if status == status_starting() {
        (talk_core::t("gui.status.running", &[]), Kind::Pending)
    } else {
        (talk_core::t("gui.status.problem", &[]), Kind::Problem)
    }
}

/// Lock a mutex. A poisoned one is written to the panic log at this site first, then panics as `unwrap()` did.
pub(crate) fn locked<'a, T>(mutex: &'a Mutex<T>, site: &str) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(_) => {
            log_line(&format!("mutex poisoned at {site}"));
            panic!("mutex poisoned at {site}")
        }
    }
}

/// Where panics are written: gui-panic.log in the state folder.
pub(crate) fn panic_log_path() -> PathBuf {
    state_dir().join("gui-panic.log")
}

/// Append one entry to the panic log. A write failure is ignored: logging must never stop a start or an exit.
pub(crate) fn log_line(text: &str) {
    let _ = log_to(&panic_log_path(), text);
}

pub(crate) fn log_to(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{text}")
}

/// Keep the old panic output (stderr, invisible in this window build) and also write the panic to the log.
pub(crate) fn install_panic_log() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info.payload().downcast_ref::<&str>().map(|text| text.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(not a text message)".to_string());
        let place = info.location().map_or_else(|| "unknown".to_string(), |at| format!("{}:{}", at.file(), at.line()));
        let thread = thread::current();
        let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_secs());
        let backtrace = std::backtrace::Backtrace::force_capture();
        log_line(&format!("[unix {seconds}] thread {} panicked at {place}: {message}\n{backtrace}", thread.name().unwrap_or("unnamed")));
        previous(info);
    }));
}

