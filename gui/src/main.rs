//! Windows window for herdr-talk. It connects to a running `herdr-talk run` through its local
//! control channel (127.0.0.1, token from control-<pid>.json) and shows the chat and peers.
//! It does not join the network itself: the chat, encryption and trust stay in herdr-talk.
//! It first tries every control file; with none that connects, it starts `herdr-talk serve` hidden,
//! waits for its control file, and stops it again on exit. A herdr-talk that was already running is never stopped.

#![windows_subsystem = "windows"]

mod relay_form;

use eframe::egui;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(500);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY: Duration = Duration::from_secs(2);
/// Height of the message box and the send button.
const INPUT_HEIGHT: f32 = 36.0;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_STARTS: u32 = 3;
// Status phases, in the order they show: no server of ours yet, our own serve connected, or the chat built into this window.
// The status texts are compared by their start (see status_label), so each one is read through the same function.
fn status_starting() -> String {
    talk_core::t("gui.status.starting", &[])
}

fn status_connected() -> String {
    talk_core::t("gui.status.connected", &[])
}

fn status_embedded() -> String {
    talk_core::t("gui.status.embedded", &[])
}
const START_WAIT: Duration = Duration::from_secs(10);
const TOAST_SECONDS: f32 = 3.0;
const STOP_WAIT: Duration = Duration::from_secs(3);

/// The `herdr-talk serve` this window started, if any.
type Started = Arc<Mutex<Option<Child>>>;

#[derive(Clone)]
struct Control {
    port: u16,
    token: String,
}

#[derive(Default)]
struct Shared {
    status: String,
    lines: Vec<(String, bool)>,
    peers: Vec<String>,
    /// This chat's own name and port. None until the chat says it (an older herdr-talk never does).
    me: Option<(String, u16)>,
    /// The relay state as the chat reports it ("" when the relay is off).
    relay: String,
    /// The connection mode as the chat reports it: "local" or "supabase".
    mode: String,
}

/// Which screen is shown. The connection work runs in the background on both.
#[derive(Clone, Copy, PartialEq)]
enum Screen {
    Intro,
    Chat,
}

struct App {
    screen: Screen,
    shared: Arc<Mutex<Shared>>,
    outgoing: Sender<String>,
    input: String,
    theme: Choice,
    /// Scanline overlay that imitates an old CRT monitor. On by default; the top bar checkbox turns it off.
    crt: bool,
    hwnd: isize,
    /// This PC's address on the network, shown with the own entry in the peer panel.
    local_ip: String,
    /// The theme the title bar was last colored for.
    frame_theme: Option<Choice>,
    /// Until when the phone address and token are on screen. Only this window shows them: never a chat line, never a log.
    phone_until: Option<Instant>,
    /// The auto-show at the first chat screen was decided (it happens once, and only when this window's node is the host).
    phone_checked: bool,
    /// Set when the settings were saved: the session asks the serve node to start the relay again.
    reload: Arc<AtomicBool>,
    /// The Ctrl+R settings window, while it is open.
    relay_form: Option<relay_form::Form>,
    /// No connection mode saved yet: the first-run choice comes before the chat.
    first_run: bool,
    /// The last result line (saved, or left without saving), shown in the top bar.
    notice: String,
    /// The 30-second default of the first-run choice. None until the choice is shown.
    choice_timer: Option<talk_core::choice_timer::ChoiceTimer>,
    /// The mode picked on the first screen (Local or Supabase); 입장 enters with it.
    intro_mode: talk_core::relay_settings::Mode,
    /// The toast on screen: the text last shown and when it appeared (it fades after TOAST_SECONDS).
    toast_text: String,
    toast_started: Option<Instant>,
}

const PHONE_SHOWN: Duration = Duration::from_secs(30);

/// The address this PC uses to reach the network. Connecting a UDP socket sends nothing; it only picks the address.
fn local_ip() -> String {
    std::net::UdpSocket::bind(("0.0.0.0", 0))
        .and_then(|socket| socket.connect(("192.0.2.1", 9)).and_then(|()| socket.local_addr()))
        .map_or_else(|_| "127.0.0.1".to_string(), |address| address.ip().to_string())
}

/// This window's session folder (chosen at start, see main).
fn state_dir() -> PathBuf {
    talk_core::store::state_dir()
}

fn read_control(file: &Path) -> Option<Control> {
    let value: Value = serde_json::from_str(&fs::read_to_string(file).ok()?).ok()?;
    Some(Control {
        port: u16::try_from(value["port"].as_u64()?).ok()?,
        token: value["token"].as_str()?.to_string(),
    })
}

struct Link {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
}

impl Link {
    fn open(control: &Control) -> std::io::Result<Self> {
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

    fn call(&mut self, request: &Value) -> std::io::Result<Value> {
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
fn talk_candidates() -> Vec<PathBuf> {
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
fn talk_program() -> Result<PathBuf, String> {
    let candidates = talk_candidates();
    candidates.iter().find(|path| path.is_file()).cloned().ok_or_else(|| {
        let looked: Vec<String> = candidates.iter().map(|path| path.display().to_string()).collect();
        talk_core::t("gui.err.exe_not_found", &[("paths", &looked.join(" | "))])
    })
}

/// Ask the child to end, which removes its control file, and kill it only if it does not.
fn stop_serve(started: &Started) {
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
fn ensure_serve(started: &Started, starts: &mut u32) -> Result<(), String> {
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

fn set_status(shared: &Arc<Mutex<Shared>>, text: &str) {
    locked(&**shared, "set_status").status = text.to_string();
}

/// Connect to the serve this window started, through its own control file. Another window's serve or a
/// herdr-talk pane is never used, so every window is a node of its own.
fn connect_own(dir: &Path, started: &Started) -> Option<(Link, Control)> {
    let pid = locked(&**started, "connect_own").as_ref()?.id();
    let control = read_control(&control_file(dir, pid))?;
    Link::open(&control).ok().map(|link| (link, control))
}

fn control_file(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("control-{pid}.json"))
}

/// Worker, first choice: start `herdr-talk serve` hidden and connect to it.
/// When that fails (program missing, no control file in time), the chat runs inside this window instead.
fn worker(shared: Arc<Mutex<Shared>>, outgoing: Receiver<String>, started: Started, reload: Arc<AtomicBool>) {
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
                run_embedded(&shared, &outgoing, &started, &reason);
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
fn run_embedded(shared: &Arc<Mutex<Shared>>, outgoing: &Receiver<String>, started: &Started, reason: &str) -> ! {
    stop_serve(started); // a serve that is only late would be a second chat on this PC
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
    // The relay in this window: a bad setting shows in the status bar, the chat goes on.
    if let Err(error) = talk_core::relay_chat::start(&chat, &talk_core::store::state_dir()) {
        chat.set_relay_status(&format!("relay: {error}"));
    }
    set_status(shared, &talk_core::t("gui.status.embedded_after", &[("reason", reason)]));
    loop {
        while let Ok(text) = outgoing.try_recv() {
            chat.handle(&text);
        }
        let mut state = locked(shared, "run_embedded");
        state.lines = chat.lines().into_iter().map(|line| (line.render(), line.mine)).collect();
        state.me = {
            let me = locked(&chat.me, "run_embedded.me");
            Some((me.name.clone(), me.port))
        };
        state.peers = chat.fresh_peers().iter().map(|peer| format!("{} ({}:{})", peer.name, peer.ip, peer.port)).collect();
        state.relay = chat.relay_status();
        state.mode = chat.mode_name().to_string();
        drop(state);
        thread::sleep(POLL);
    }
}

fn session(link: &mut Link, shared: &Arc<Mutex<Shared>>, outgoing: &Receiver<String>, reload: &AtomicBool) -> std::io::Result<()> {
    loop {
        // Settings were saved in this window: the serve node starts the relay again with them.
        if reload.swap(false, Ordering::SeqCst) {
            link.call(&json!({"cmd": "relay_reload"}))?;
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
        state.lines = lines["lines"].as_array().into_iter().flatten()
            .map(|row| (row["text"].as_str().unwrap_or("").to_string(), row["mine"] == true))
            .collect();
        state.me = peers["me"]["name"].as_str().zip(peers["me"]["port"].as_u64().and_then(|port| u16::try_from(port).ok()))
            .map(|(name, port)| (name.to_string(), port));
        state.peers = peers["peers"].as_array().into_iter().flatten()
            .map(|peer| format!("{} ({}:{})", peer["name"].as_str().unwrap_or("?"), peer["ip"].as_str().unwrap_or("?"), peer["port"]))
            .collect();
        state.relay = peers["relay"].as_str().unwrap_or("").to_string();
        state.mode = peers["mode"].as_str().unwrap_or("local").to_string();
        drop(state);
        thread::sleep(POLL);
    }
}

/// How a status text is shown: a short label, and its kind.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Ok,
    Pending,
    Problem,
}

fn status_label(status: &str, supabase: bool) -> (String, Kind) {
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

impl App {
    /// One frame of the settings window: Stay, Saved (reload the relay; first run goes to the chat), or Discard.
    fn settings_frame(&mut self, ctx: &egui::Context, palette: &Palette) {
        let Some(form) = self.relay_form.as_mut() else { return };
        let action = relay_form::show(ctx, palette, form);
        let message = form.message.clone();
        match action {
            relay_form::Action::Stay => {}
            relay_form::Action::Saved => {
                self.reload.store(true, Ordering::SeqCst);
                self.notice = message;
                self.relay_form = None;
                if self.first_run {
                    self.first_run = false;
                    self.screen = Screen::Chat;
                }
            }
            relay_form::Action::Discard => {
                self.relay_form = None;
                // Back to the first-run choice: its 30 seconds start again.
                self.choice_timer = None;
                self.notice = talk_core::t("settings.cancel_discard", &[]);
            }
        }
        if self.crt {
            draw_scanlines(ctx);
            draw_bezel(ctx);
        }
    }

    /// First screen: name, one line about the app, the connection state, and the button into the chat.
    fn intro(&mut self, ctx: &egui::Context, palette: &Palette, _status: &str) {
        let mode = self.intro_mode;
        let supabase = mode == talk_core::relay_settings::Mode::Supabase;
        let picked_label = if supabase { talk_core::t("gui.mode.supabase", &[]) } else { talk_core::t("gui.mode.local", &[]) };
        let mut enter = false;
        let mut select = None;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.10);
                ui.colored_label(palette.accent, egui::RichText::new("herdr-talk").size(36.0));
                ui.add_space(8.0);
                ui.label(talk_core::t("gui.intro", &[]));
                ui.add_space(14.0);
                // The selected mode, then the selector: [로컬/자체] [Supabase].
                ui.colored_label(palette.accent, egui::RichText::new(picked_label).size(18.0));
                ui.add_space(8.0);
                // Two equal buttons, centered by padding the row on the left (the row width is 2 * 160 + spacing 8).
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - 328.0) / 2.0).max(0.0));
                    let size = egui::vec2(160.0, 36.0);
                    if ui.add(egui::Button::new(egui::RichText::new(talk_core::t("gui.mode.local_button", &[])).size(16.0)).selected(!supabase).min_size(size)).clicked() {
                        select = Some(talk_core::relay_settings::Mode::Local);
                    }
                    if ui.add(egui::Button::new(egui::RichText::new("Supabase").size(16.0)).selected(supabase).min_size(size)).clicked() {
                        select = Some(talk_core::relay_settings::Mode::Supabase);
                    }
                });
                ui.add_space(14.0);
                if ui.add(egui::Button::new(egui::RichText::new(talk_core::t("gui.button.enter", &[])).size(18.0)).min_size(egui::vec2(160.0, 40.0))).clicked() {
                    enter = true;
                }
            });
        });
        if let Some(picked) = select {
            self.intro_mode = picked;
        }
        if enter {
            self.enter(self.intro_mode);
        }
    }

    /// Enter the chat in `mode`: save the mode, let the relay reload. Supabase with missing settings opens the form first.
    fn enter(&mut self, mode: talk_core::relay_settings::Mode) {
        let state = state_dir();
        if mode == talk_core::relay_settings::Mode::Local {
            match talk_core::relay_settings::set_mode(&state, mode) {
                Ok(()) => {
                    self.reload.store(true, Ordering::SeqCst);
                    self.screen = Screen::Chat;
                }
                Err(error) => self.notice = error,
            }
            return;
        }
        let saved = talk_core::relay_settings::load_saved(&state);
        let as_supabase = talk_core::relay_settings::Settings { mode, ..saved };
        if talk_core::relay_settings::validate(&as_supabase).is_ok() {
            match talk_core::relay_settings::set_mode(&state, mode) {
                Ok(()) => {
                    self.reload.store(true, Ordering::SeqCst);
                    self.screen = Screen::Chat;
                }
                Err(error) => self.notice = error,
            }
        } else {
            // Missing settings: the form opens, and a successful save goes straight into the chat.
            self.first_run = true;
            self.relay_form = Some(relay_form::Form::open_supabase(&state));
        }
    }
}

impl App {
    /// A short message at the bottom of the window. A new message restarts the timer; it fades after TOAST_SECONDS.
    fn show_toast(&mut self, ctx: &egui::Context, palette: &Palette) {
        if self.notice != self.toast_text {
            self.toast_text = self.notice.clone();
            self.toast_started = (!self.notice.is_empty()).then(Instant::now);
        }
        let Some(started) = self.toast_started else { return };
        let left = TOAST_SECONDS - started.elapsed().as_secs_f32();
        if left <= 0.0 {
            self.toast_started = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        let alpha = if left < 0.5 { left / 0.5 } else { 1.0 };
        let text = self.toast_text.clone();
        egui::Area::new(egui::Id::new("toast")).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -64.0)).interactable(false).show(ctx, |ui| {
            egui::Frame::NONE
                .fill(palette.panel.gamma_multiply(alpha))
                .stroke(egui::Stroke::new(1.0_f32, palette.accent.gamma_multiply(alpha)))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.colored_label(palette.text.gamma_multiply(alpha), text);
                });
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(POLL);
        // eframe may re-apply the OS theme's visuals, so the chosen theme's colors are set every frame.
        let palette = palette(self.theme);
        self.show_toast(ctx, &palette);
        ctx.set_visuals(visuals(&palette, ctx.theme()));
        if self.frame_theme != Some(self.theme) {
            style_frame(self.hwnd, &palette);
            self.frame_theme = Some(self.theme);
        }
        // The settings window takes the whole window while it is open (first run, or the top-bar button).
        if self.relay_form.is_some() {
            self.settings_frame(ctx, &palette);
            return;
        }
        // First run: pick the connection mode before anything else. Local is saved at once; Supabase opens the settings.
        if self.first_run {
            // Any input stops the 30-second default; when it runs out, the chat starts in Local mode and nothing is saved.
            let input_seen = ctx.input(|input| {
                input.events.iter().any(|event| {
                    matches!(event, egui::Event::Key { pressed: true, .. } | egui::Event::Text(_) | egui::Event::PointerButton { pressed: true, .. })
                })
            });
            if relay_form::tick(&mut self.choice_timer, Instant::now(), input_seen) == relay_form::Tick::Start {
                self.first_run = false;
                self.choice_timer = None;
                self.screen = Screen::Chat;
            }
        }
        if self.first_run {
            let remaining = self.choice_timer.and_then(|timer| timer.remaining(Instant::now()));
            match relay_form::choose(ctx, &palette, remaining) {
                Some(relay_form::Pick::Local) => {
                    match talk_core::relay_settings::set_mode(&state_dir(), talk_core::relay_settings::Mode::Local) {
                        Ok(()) => {
                            self.reload.store(true, Ordering::SeqCst);
                            self.first_run = false;
                            self.choice_timer = None;
                            self.screen = Screen::Chat;
                        }
                        Err(error) => self.notice = error,
                    }
                }
                Some(relay_form::Pick::Supabase) => {
                    self.choice_timer = None;
                    self.relay_form = Some(relay_form::Form::open_supabase(&state_dir()));
                }
                None => {}
            }
            if self.first_run {
                if self.crt {
                    draw_scanlines(ctx);
                    draw_bezel(ctx);
                }
                return;
            }
        }
        if self.screen == Screen::Intro {
            let status = locked(&*self.shared, "update.intro").status.clone();
            self.intro(ctx, &palette, &status);
            return;
        }
        if ctx.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::R)) {
            self.relay_form = Some(relay_form::Form::open(&state_dir()));
            return;
        }
        let state = locked(&*self.shared, "update.chat");
        // Once, at the first chat screen: show the phone address when the node this window talks to is the host (방장) of this PC.
        if !self.phone_checked {
            if let Some((_, port)) = &state.me {
                self.phone_checked = true;
                if talk_core::phone::read(&state_dir()).is_some_and(|phone| phone.owner_port == *port) {
                    self.phone_until = Some(Instant::now() + PHONE_SHOWN);
                }
            }
        }
        // Ctrl+P toggles; typing or Enter hides.
        let (toggle, typed) = ctx.input(|input| (
            input.modifiers.ctrl && input.key_pressed(egui::Key::P),
            input.events.iter().any(|event| matches!(event, egui::Event::Text(_))) || input.key_pressed(egui::Key::Enter),
        ));
        let visible = self.phone_until.is_some_and(|until| until > Instant::now());
        if toggle {
            self.phone_until = if visible { None } else { Some(Instant::now() + PHONE_SHOWN) };
        } else if typed || !visible {
            self.phone_until = None;
        }
        let mut open_settings = false;
        egui::TopBottomPanel::top("status").frame(inset(&palette, self.crt)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let supabase = talk_core::relay_settings::Mode::from_saved(&state.mode) == talk_core::relay_settings::Mode::Supabase;
                let color = match status_label(&state.status, supabase).1 {
                    Kind::Ok => palette.accent,
                    Kind::Pending => palette.muted,
                    Kind::Problem => palette.danger,
                };
                if talk_core::phone::read(&state_dir()).is_some_and(|phone| state.me.as_ref().is_some_and(|(_, port)| phone.owner_port == *port)) {
                    ui.colored_label(palette.accent, talk_core::t("host.badge", &[]));
                }
                // The top bar shows the same words as the first screen: in Supabase mode the serve line is "Supabase 모드".
                let supabase_now = talk_core::relay_settings::Mode::from_saved(&state.mode) == talk_core::relay_settings::Mode::Supabase;
                let shown = if state.status.starts_with(&status_connected()) {
                    if supabase_now { talk_core::t("gui.top.supabase_connected", &[]) } else { talk_core::t("gui.top.local_connected", &[]) }
                } else {
                    state.status.clone()
                };
                ui.colored_label(color, shown);
                if ui.button(talk_core::t("gui.button.connection_mode", &[])).clicked() {
                    open_settings = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("theme")
                        .selected_text(self.theme.name())
                        .show_ui(ui, |ui| {
                            for choice in [Choice::Dark, Choice::Light, Choice::Amber, Choice::Green] {
                                ui.selectable_value(&mut self.theme, choice, choice.name());
                            }
                        });
                    ui.checkbox(&mut self.crt, "CRT");
                });
            });
        });
        ctx.set_theme(self.theme.preference());
        if open_settings {
            self.relay_form = Some(relay_form::Form::open(&state_dir()));
            ctx.request_repaint();
        }
        egui::SidePanel::right("peers").default_width(220.0).frame(inset(&palette, self.crt)).show(ctx, |ui| {
            ui.colored_label(palette.accent, egui::RichText::new(talk_core::t("gui.peers.title", &[])).heading());
            if let Some((name, port)) = &state.me {
                ui.colored_label(palette.accent, talk_core::t("gui.peers.me", &[("name", name.as_str())]));
                ui.colored_label(palette.accent, format!("{}:{port}", self.local_ip));
                ui.separator();
            }
            if state.peers.is_empty() {
                ui.label("상대 없음");
            }
            for peer in &state.peers {
                ui.label(peer);
            }
        });
        egui::TopBottomPanel::bottom("input").frame(inset(&palette, self.crt)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let response = ui.add_sized(
                    [ui.available_width() - 76.0, INPUT_HEIGHT],
                    egui::TextEdit::singleline(&mut self.input).margin(egui::Margin::symmetric(6, 10)),
                );
                let enter = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                let clicked = ui.add_sized([68.0, INPUT_HEIGHT], egui::Button::new(talk_core::t("gui.button.send", &[]))).clicked();
                if (enter || clicked) && !self.input.trim().is_empty() {
                    let text = std::mem::take(&mut self.input);
                    match text.trim() {
                        // The window shows the phone address itself; the node only makes a new token.
                        "/phone" => self.phone_until = Some(Instant::now() + PHONE_SHOWN),
                        "/phone new" => {
                            let _ = self.outgoing.send(text);
                            self.phone_until = Some(Instant::now() + PHONE_SHOWN);
                        }
                        _ => {
                            let _ = self.outgoing.send(text);
                        }
                    }
                }
                if enter {
                    response.request_focus();
                }
            });
        });
        egui::CentralPanel::default().frame(inset(&palette, self.crt)).show(ctx, |ui| {
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                for (text, mine) in &state.lines {
                    let color = if *mine { palette.mine } else { palette.text };
                    ui.colored_label(color, text);
                }
            });
        });
        if self.phone_until.is_some() {
            let mut rows = vec![egui::RichText::new(talk_core::t("phone.title", &[])).strong()];
            match talk_core::phone::read(&state_dir()) {
                Some(phone) => {
                    rows.extend(phone.urls.iter().map(|url| egui::RichText::new(url.as_str())));
                    rows.push(egui::RichText::new(talk_core::t("phone.token", &[("token", &phone.token)])).strong());
                    rows.push(egui::RichText::new(talk_core::t("phone.hint", &[])));
                }
                None => rows.push(egui::RichText::new(talk_core::t("phone.none", &[]))),
            }
            egui::Window::new("phone").title_bar(false).anchor(egui::Align2::CENTER_BOTTOM, [0.0, -70.0]).show(ctx, |ui| {
                for row in rows {
                    ui.label(row);
                }
            });
        }
        if self.crt {
            draw_scanlines(ctx);
            draw_bezel(ctx);
        }
    }
}

/// Beige plastic bezel around the window, like an old CRT monitor's case: a dark outer edge and a light inner edge.
fn draw_bezel(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("crt_bezel")));
    let screen = ctx.screen_rect();
    let outer = egui::Color32::from_rgb(0x8a, 0x80, 0x6e);
    let light = egui::Color32::from_rgb(0xd8, 0xcf, 0xb8);
    let shadow = egui::Color32::from_rgb(0x4e, 0x48, 0x3c);
    painter.rect_stroke(screen.shrink(1.0), 0.0, egui::Stroke::new(2.0_f32, shadow), egui::StrokeKind::Inside);
    painter.rect_stroke(screen.shrink(3.0), 0.0, egui::Stroke::new(4.0_f32, outer), egui::StrokeKind::Inside);
    painter.rect_stroke(screen.shrink(6.0), 0.0, egui::Stroke::new(1.0_f32, light), egui::StrokeKind::Inside);
}

/// Dark line every 3 px over the whole window. Painted on a top layer; it does not take clicks or keys.
fn draw_scanlines(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("crt_scanlines")));
    let screen = ctx.screen_rect();
    let mut y = screen.top();
    while y < screen.bottom() {
        let line = egui::Rect::from_min_size(egui::pos2(screen.left(), y), egui::vec2(screen.width(), 1.0));
        painter.rect_filled(line, 0.0, egui::Color32::from_black_alpha(90));
        y += 3.0;
    }
}

/// Lock a mutex. A poisoned one is written to the panic log at this site first, then panics as `unwrap()` did.
fn locked<'a, T>(mutex: &'a Mutex<T>, site: &str) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(_) => {
            log_line(&format!("mutex poisoned at {site}"));
            panic!("mutex poisoned at {site}")
        }
    }
}

/// Where panics are written: gui-panic.log in the state folder.
fn panic_log_path() -> PathBuf {
    state_dir().join("gui-panic.log")
}

/// Append one entry to the panic log. A write failure is ignored: logging must never stop a start or an exit.
fn log_line(text: &str) {
    let _ = log_to(&panic_log_path(), text);
}

fn log_to(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{text}")
}

/// Keep the old panic output (stderr, invisible in this window build) and also write the panic to the log.
fn install_panic_log() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_log_write_does_not_panic() {
        let dir = std::env::temp_dir().join(format!("herdr-talk-gui-log-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        // The log path is a folder, so opening it for append fails.
        assert!(log_to(&dir, "x").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn log_entries_are_appended() {
        let dir = std::env::temp_dir().join(format!("herdr-talk-gui-append-test-{}", std::process::id()));
        let file = dir.join("gui-panic.log");
        log_to(&file, "first").unwrap();
        log_to(&file, "second").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "first\nsecond\n");
        fs::remove_dir_all(&dir).unwrap();
    }
}

fn main() -> eframe::Result<()> {
    install_panic_log();
    // A new session per window unless --session ID names an existing one (or "default").
    let args: Vec<String> = std::env::args().collect();
    let requested = args.windows(2).find(|pair| pair[0] == "--session").map(|pair| pair[1].clone());
    if let Err(error) = talk_core::store::choose_session(requested.as_deref()) {
        eprintln!("{error}");
        return Ok(());
    }
    for id in talk_core::store::remove_dead_sessions(&talk_core::store::base_dir()) {
        eprintln!("could not remove dead session {id}");
    }
    let shared = Arc::new(Mutex::new(Shared { status: status_starting(), ..Default::default() }));
    let (outgoing, queue) = mpsc::channel();
    let started: Started = Arc::default();
    let reload = Arc::new(AtomicBool::new(false));
    {
        let (shared, started, reload) = (shared.clone(), started.clone(), reload.clone());
        thread::Builder::new().name("worker".into()).spawn(move || worker(shared, queue, started, reload)).expect("worker thread");
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([760.0, 520.0]).with_title("herdr-talk"),
        ..Default::default()
    };
    let result = eframe::run_native(
        "herdr-talk",
        options,
        Box::new(|cc| {
            install_korean_font(&cc.egui_ctx);
            let hwnd = window_hwnd(cc);
            cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
            Ok(Box::new(App { screen: Screen::Intro, shared, outgoing, input: String::new(), theme: Choice::Green, crt: true, hwnd, local_ip: local_ip(), frame_theme: None, phone_until: None, phone_checked: false, reload: reload.clone(), relay_form: None, first_run: talk_core::relay_settings::needs_first_run(&state_dir()), notice: String::new(), choice_timer: None, intro_mode: talk_core::relay_settings::Mode::from_saved(talk_core::relay_settings::load_saved(&state_dir()).mode.as_str()), toast_text: String::new(), toast_started: None }))
        }),
    );
    stop_serve(&started);
    // A normal stop deletes this node's own relay row first (best effort), then its folder.
    if result.is_ok() && talk_core::store::session_id().is_some() {
        talk_core::relay_chat::leave(&talk_core::store::state_dir());
    }
    if let Some(id) = talk_core::store::remove_own_session() {
        eprintln!("could not remove session {id}");
    }
    result
}

/// This window's HWND, or 0 when it cannot be read.
fn window_hwnd(cc: &eframe::CreationContext<'_>) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(win32)) => win32.hwnd.get(),
        _ => 0,
    }
}

/// One DWM window attribute. The result is ignored: Windows before 11 (22H2 for the colors) refuses
/// attributes it does not know, and the window then keeps its default look.
fn set_dwm(hwnd: isize, attribute: i32, value: u32) {
    if hwnd == 0 {
        return;
    }
    // SAFETY: hwnd is this window's, and the value is a u32 of the size passed.
    unsafe {
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd as windows_sys::Win32::Foundation::HWND,
            attribute as u32,
            std::ptr::from_ref(&value).cast(),
            std::mem::size_of::<u32>() as u32,
        );
    }
}

/// COLORREF is 0x00BBGGRR.
fn colorref(color: egui::Color32) -> u32 {
    u32::from(color.r()) | u32::from(color.g()) << 8 | u32::from(color.b()) << 16
}

/// Square outer corners (Windows 11 rounds them), and the title bar in the theme's colors.
fn style_frame(hwnd: isize, palette: &Palette) {
    const CORNER_PREFERENCE: i32 = 33;
    const BORDER_COLOR: i32 = 34;
    const CAPTION_COLOR: i32 = 35;
    const TEXT_COLOR: i32 = 36;
    const DO_NOT_ROUND: u32 = 1;
    set_dwm(hwnd, CORNER_PREFERENCE, DO_NOT_ROUND);
    set_dwm(hwnd, CAPTION_COLOR, colorref(palette.background));
    set_dwm(hwnd, TEXT_COLOR, colorref(palette.text));
    set_dwm(hwnd, BORDER_COLOR, colorref(palette.border));
}

struct Palette {
    background: egui::Color32,
    panel: egui::Color32,
    border: egui::Color32,
    text: egui::Color32,
    muted: egui::Color32,
    accent: egui::Color32,
    accent_dark: egui::Color32,
    accent_light: egui::Color32,
    mine: egui::Color32,
    danger: egui::Color32,
}

/// The color themes the select offers. The look (square corners, 1px borders, flat) is the same for all.
#[derive(Clone, Copy, PartialEq)]
enum Choice {
    Dark,
    Light,
    Amber,
    Green,
}

impl Choice {
    fn name(self) -> &'static str {
        match self {
            Choice::Dark => "Dark",
            Choice::Light => "Light",
            Choice::Amber => "Amber",
            Choice::Green => "Green",
        }
    }

    /// The egui base the palette is laid over: Light on light neutrals, the rest on the dark base.
    fn preference(self) -> egui::ThemePreference {
        match self {
            Choice::Light => egui::ThemePreference::Light,
            Choice::Dark | Choice::Amber | Choice::Green => egui::ThemePreference::Dark,
        }
    }
}

const fn rgb(hex: u32) -> egui::Color32 {
    egui::Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Dark: the colors of chromeExt-PicHunter (src/popup.css). Light: PicHunter has none, so this is the
/// same blue accent on light neutrals. Amber and Green: phosphor text on black.
fn palette(choice: Choice) -> Palette {
    match choice {
        Choice::Dark => Palette {
            background: rgb(0x1a1a2e), panel: rgb(0x16213e), border: rgb(0x2a3a5c), text: rgb(0xe0e0e0), muted: rgb(0xb0bec5),
            accent: rgb(0x4fc3f7), accent_dark: rgb(0x0288d1), accent_light: rgb(0x81d4fa), mine: rgb(0x81d4fa), danger: rgb(0xe57373),
        },
        Choice::Light => Palette {
            background: rgb(0xf5f7fa), panel: rgb(0xffffff), border: rgb(0xcfd8dc), text: rgb(0x1a1a2e), muted: rgb(0x546e7a),
            accent: rgb(0x0277bd), accent_dark: rgb(0x0288d1), accent_light: rgb(0x4fc3f7), mine: rgb(0x01579b), danger: rgb(0xc62828),
        },
        Choice::Amber => Palette {
            background: rgb(0x000000), panel: rgb(0x0d0900), border: rgb(0x7a5400), text: rgb(0xffb000), muted: rgb(0xb37a00),
            accent: rgb(0xffd24d), accent_dark: rgb(0x8a5f00), accent_light: rgb(0xffe08a), mine: rgb(0xffd24d), danger: rgb(0xff5533),
        },
        Choice::Green => Palette {
            background: rgb(0x000000), panel: rgb(0x00100a), border: rgb(0x1a7a36), text: rgb(0x33ff66), muted: rgb(0x1fa040),
            accent: rgb(0x99ffb3), accent_dark: rgb(0x14802f), accent_light: rgb(0xccffd9), mine: rgb(0x99ffb3), danger: rgb(0xff5555),
        },
    }
}

/// The palette's colors on egui's dark or light base: square corners, 1px borders, no shadows, no hover growth.
fn visuals(palette: &Palette, theme: egui::Theme) -> egui::Visuals {
    let mut visuals = match theme {
        egui::Theme::Dark => egui::Visuals::dark(),
        egui::Theme::Light => egui::Visuals::light(),
    };
    let white = egui::Color32::WHITE;
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.background;
    visuals.extreme_bg_color = palette.panel;
    visuals.override_text_color = Some(palette.text);
    visuals.hyperlink_color = palette.accent;
    visuals.selection.bg_fill = palette.accent_dark;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, white);
    let rounding = egui::CornerRadius::ZERO;
    visuals.window_corner_radius = rounding;
    visuals.menu_corner_radius = rounding;
    visuals.window_shadow = egui::Shadow::NONE;
    visuals.popup_shadow = egui::Shadow::NONE;
    for widget in [&mut visuals.widgets.noninteractive, &mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active, &mut visuals.widgets.open] {
        widget.corner_radius = rounding;
        widget.expansion = 0.0;
    }
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, palette.border);
    visuals.widgets.inactive.weak_bg_fill = palette.panel;
    visuals.widgets.inactive.bg_fill = palette.panel;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, palette.border);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, palette.muted);
    visuals.widgets.hovered.weak_bg_fill = palette.border;
    visuals.widgets.hovered.bg_fill = palette.border;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, palette.accent);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, palette.text);
    visuals.widgets.active.weak_bg_fill = palette.accent_dark;
    visuals.widgets.active.bg_fill = palette.accent_dark;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, palette.accent_light);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, white);
    visuals
}

/// Galmuri11 (pixel font with Hangul, OFL, bundled in the exe) is the main font for all text.
/// Windows system fonts stay as fallbacks for any character Galmuri does not have.
fn install_korean_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("galmuri".to_owned(), egui::FontData::from_static(GALMURI).into());
    let fallbacks = ["C:\\Windows\\Fonts\\malgun.ttf", "C:\\Windows\\Fonts\\gulim.ttc"];
    if let Some(bytes) = fallbacks.iter().find_map(|path| fs::read(path).ok()) {
        fonts.font_data.insert("system".to_owned(), egui::FontData::from_owned(bytes).into());
    }
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        list.insert(0, "galmuri".to_owned());
        if fonts.font_data.contains_key("system") {
            list.push("system".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

static GALMURI: &[u8] = include_bytes!("../assets/Galmuri11.ttf");

/// Panel frame that keeps text inside the bezel when the CRT look is on.
fn inset(palette: &Palette, crt: bool) -> egui::Frame {
    egui::Frame::new().fill(palette.background).inner_margin(egui::Margin::same(if crt { 12 } else { 6 }))
}
