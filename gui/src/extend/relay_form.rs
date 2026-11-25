//! The connection-mode screens of the window. Two kinds: the first-run choice (`choose`), shown before the chat
//! when relay.json has no `mode`, and the settings window (`Form`, Ctrl+R and the top-bar button) with the mode row, the setup-code row and the room rows.
//! The rules (validation, saving, BOM, mode, texts) live in talk-core; this file only draws and keeps the typed text.

use crate::extend::Palette;
use eframe::egui;
use std::path::{Path, PathBuf};
use talk_core::choice_timer::ChoiceTimer;
use talk_core::relay_rest::{self, random_nonce, signed_room_body, unix_now, PutOutcome, Request, RoomAction};
use talk_core::relay_settings::{self, Missing, Mode, SaveRefusal, Settings};
use talk_core::t;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

/// The four field labels in field order: setup code, trust code, display name, room ID.
fn labels() -> [String; 4] {
    [
        t("gui.relay.label.code", &[]),
        t("gui.relay.label.trust", &[]),
        t("gui.relay.label.name", &[]),
        t("gui.relay.label.room", &[]),
    ]
}
/// The trust check's answer: the refusal shows only while the message is still the one from opening the window,
/// so a late answer never replaces a newer message.
fn apply_trust(message: &mut String, opened: &str, refused: bool) {
    if refused && message == opened {
        *message = t("room.trust_wrong", &[]);
    }
}

/// Takes the trust check's answer if it has come back, and applies it with the message as it is now.
fn poll_trust(form: &mut Form) {
    let answer = form.trust_check.as_ref().and_then(|(receiver, opened)| receiver.try_recv().ok().map(|refused| (refused, opened.clone())));
    if let Some((refused, opened)) = answer {
        form.trust_check = None;
        apply_trust(&mut form.message, &opened, refused);
    }
}

/// The bar's text for an issue answer: the new code (shown once, never stored), or the reason it failed.
fn issue_message(answer: Result<String, String>) -> String {
    match answer {
        Ok(code) => t("gui.relay.issued", &[("code", &code)]),
        Err(error) => t("gui.relay.issue_failed", &[("error", &error)]),
    }
}

/// Whether F9 was pressed this frame (opens the operator passphrase row).
fn operator_key(ctx: &egui::Context) -> bool {
    ctx.input(|input| input.key_pressed(egui::Key::F9))
}

/// The Supabase URL for the setup-code calls: the environment URL wins over the default, as everywhere else.
fn setup_url() -> String {
    std::env::var(relay_rest::URL_VARIABLE)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| relay_rest::DEFAULT_URL.to_string())
}

/// The field indexes, in the order of `labels`.
const CODE_FIELD: usize = 0;
/// The trust code (8 digits): the owner's code at create, the code each participant types to join.
const TRUST_FIELD: usize = 1;
const NAME_FIELD: usize = 2;
const ROOM_FIELD: usize = 3;

/// The first-run pick.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Local,
    Supabase,
}

/// What the settings window did this frame.
pub enum Action {
    Stay,
    /// Saved (the file and the relay reload are the caller's next step).
    Saved,
    /// Left without saving (Esc, or the close button).
    Discard,
}

pub struct Form {
    mode: Mode,
    /// The saved URL and key: never typed here, a setup code replaces them.
    url: String,
    key: String,
    /// The saved setup code (the one redeemed last, or the one already saved); kept on every save.
    setup_code: String,
    /// The room trust check started by `open`: true when the saved room refuses the trust code.
    trust_check: Option<(Receiver<bool>, String)>,
    fields: [String; 4],
    pub message: String,
    from_env: (bool, bool),
    mine: Option<(String, String)>,
    /// This PC's short ID (what the peer types to find it); empty when it cannot be read.
    short_id: String,
    state: PathBuf,
    /// A create-room request in flight: its answer (the room code, or the reason it failed).
    creating: Option<Receiver<Result<String, String>>>,
    /// A setup-code redeem in flight: its answer (url and key, or the reason it failed).
    redeeming: Option<Receiver<Result<(String, String, String), String>>>,
    /// The operator passphrase row, opened with F9. The passphrase is taken out when the request starts and never saved.
    operator: bool,
    passphrase: String,
    /// An issue-setup-code request in flight: its answer (the new code, or the reason it failed).
    issuing: Option<Receiver<Result<String, String>>>,
}

/// What Save does next, decided from the form alone (no network, no file).
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Nothing is written; the message says what is missing or wrong.
    Refuse(String),
    /// Write relay.json now from the form.
    Write,
}

/// The answer of create-room: the room code on 200, otherwise the status and body.
fn read_room_code(status: u16, text: &str) -> Result<String, String> {
    if status != 200 {
        return Err(t("gui.relay.answer_status", &[("status", &status.to_string()), ("text", text)]));
    }
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| t("gui.relay.answer_not_json", &[]))?;
    value["room_code"].as_str().map(str::to_string).ok_or_else(|| t("gui.relay.answer_no_room", &[]))
}

fn require_published(outcome: PutOutcome) -> Result<(), String> {
    match outcome {
        PutOutcome::Written => Ok(()),
        PutOutcome::Taken => Err(t("room.put_taken", &[])),
        PutOutcome::Failed => Err(t("room.put_failed", &[])),
    }
}

impl Form {
    pub fn open(state: &Path) -> Form {
        let saved = relay_settings::load_saved(state);
        let mine = talk_core::relay_chat::my_keys(state).ok();
        let short_id = talk_core::relay_key::short_id(state).unwrap_or_default();
        // The Unique ID row is read-only: this PC's short ID (a name saved by an older version is kept).
        let name = if saved.name.is_empty() { talk_core::relay_key::short_id(state).unwrap_or_default() } else { saved.name };
        let fields = [String::new(), saved.trust, name, saved.room_code];
        let message = if mine.is_none() { t("gui.relay.no_keys", &[]) } else { String::new() };
        // The join check makes a network call, so it runs in a thread; `show` shows the trust message when it says no.
        let opened = message.clone();
        let (sender, trust_check) = mpsc::channel();
        let state_dir = state.to_path_buf();
        std::thread::spawn(move || {
            let _ = sender.send(talk_core::relay_chat::room_refuses(&state_dir));
        });
        Form {
            mode: saved.mode,
            url: saved.url,
            setup_code: saved.setup_code,
            trust_check: Some((trust_check, opened)),
            key: saved.key,
            fields,
            message,
            from_env: relay_settings::from_environment(),
            mine,
            short_id,
            state: state.to_path_buf(),
            creating: None,
            redeeming: None,
            operator: false,
            passphrase: String::new(),
            issuing: None,
        }
    }

    /// Ask Supabase for a new room (create-room) with this PC's key and the display name, and put the code in the room ID field.
    /// Runs in a thread so the window keeps drawing; `show` reads the answer.
    fn start_create_room(&mut self, ctx: &egui::Context) {
        // Only one room: a new room is refused while another one is saved (the empty code is never a saved room).
        if relay_settings::has_other_room(&self.state, "") {
            self.message = t("settings.one_room_only", &[]);
            return;
        }
        let url = self.url.trim().to_string();
        let key = self.key.trim().to_string();
        let name = self.fields[NAME_FIELD].trim().to_string();
        let trust = self.fields[TRUST_FIELD].trim().to_string();
        if !relay_rest::valid_trust(&trust) {
            self.message = t("gui.relay.create.bad_trust", &[]);
            return;
        }
        if url.is_empty() || key.is_empty() || name.is_empty() {
            self.message = t("gui.relay.create.need_save", &[]);
            return;
        }
        let config = match relay_rest::config(Some(&url), Some(&key)) {
            Ok(Some(config)) => config,
            Ok(None) => {
                self.message = t("gui.relay.create.need_url", &[]);
                return;
            }
            Err(error) => {
                self.message = t("gui.relay.create.bad_settings", &[("error", &error.to_string())]);
                return;
            }
        };
        let seed = match talk_core::relay_key::load_or_create(&self.state) {
            Ok(seed) => seed,
            Err(error) => {
                self.message = t("gui.relay.create.keys_unreadable", &[("error", &error.to_string())]);
                return;
            }
        };
        let (_, noise_pk) = match talk_core::relay_chat::my_keys(&self.state) {
            Ok(keys) => keys,
            Err(error) => {
                self.message = t("gui.relay.create.keys_unreadable", &[("error", &error.to_string())]);
                return;
            }
        };
        let short_id = match talk_core::relay_key::short_id(&self.state) {
            Ok(short_id) => short_id,
            Err(error) => {
                self.message = t("gui.relay.create.keys_unreadable", &[("error", &error.to_string())]);
                return;
            }
        };
        let nonce = match random_nonce() {
            Ok(nonce) => nonce,
            Err(error) => {
                self.message = t("gui.relay.create.request_unbuilt", &[("error", &error.to_string())]);
                return;
            }
        };
        let body = match signed_room_body(&seed, RoomAction::Create, "", &name, &trust, unix_now(), &nonce) {
            Ok(body) => body,
            Err(error) => {
                self.message = t("gui.relay.create.request_unbuilt", &[("error", &error.to_string())]);
                return;
            }
        };
        let request = Request {
            method: "POST",
            url: format!("{url}/functions/v1/create-room"),
            headers: vec![
                ("apikey".to_string(), key.clone()),
                ("Authorization".to_string(), format!("Bearer {key}")),
                ("Content-Type".to_string(), "application/json".to_string()),
            ],
            body: Some(body),
        };
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let answer = require_published(relay_rest::put_peer(&config, &seed, &noise_pk, &short_id, &name))
                .and_then(|()| relay_rest::http_post(&request))
                .and_then(|(status, text)| read_room_code(status, &text));
            let _ = sender.send(answer);
            ctx.request_repaint();
        });
        self.creating = Some(receiver);
        self.message = t("gui.relay.creating", &[]);
    }

    /// Redeem the setup code in a thread so the window keeps drawing; `show` applies the answer.
    fn start_redeem(&mut self, ctx: &egui::Context, code: String) {
        let url = setup_url();
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(relay_rest::redeem_setup_code(&code, &url).map(|(url, key)| (url, key, code)));
            ctx.request_repaint();
        });
        self.redeeming = Some(receiver);
        self.message = t("gui.relay.redeeming", &[]);
    }

    /// The operator button or Enter: issue a setup code in a thread. The passphrase is taken out of the form first.
    fn start_issue(&mut self, ctx: &egui::Context) {
        let passphrase = std::mem::take(&mut self.passphrase);
        if passphrase.is_empty() {
            self.message = t("gui.relay.operator_empty", &[]);
            return;
        }
        let url = setup_url();
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(relay_rest::issue_setup_code(&url, &passphrase));
            ctx.request_repaint();
        });
        self.issuing = Some(receiver);
        self.message = t("gui.relay.issuing", &[]);
    }

    /// The Save setup code button (Python's F2): redeem the typed code now and store its URL and key.
    fn save_setup_code(&mut self, ctx: &egui::Context) {
        match relay_rest::normalize_setup_code(self.fields[CODE_FIELD].trim()) {
            Some(code) if code == self.setup_code && !self.url.is_empty() => {
                self.message = match relay_settings::save_connection(&self.state, &self.url, &self.key, &self.setup_code) {
                    Ok(()) => t("gui.relay.setup_saved", &[]),
                    Err(error) => t("gui.relay.not_saved", &[("error", &error)]),
                };
            }
            Some(code) => self.start_redeem(ctx, code),
            None => self.message = t("setup.invalid", &[]),
        }
    }

    /// The settings window opened for Supabase (the first-run pick).
    pub fn open_supabase(state: &Path) -> Form {
        let mut form = Form::open(state);
        form.mode = Mode::Supabase;
        form
    }

    fn settings(&self) -> Settings {
        Settings {
            mode: self.mode,
            url: self.url.trim().to_string(),
            key: self.key.trim().to_string(),
            setup_code: self.setup_code.clone(),
            name: self.fields[NAME_FIELD].trim().to_string(),
            room_code: self.fields[ROOM_FIELD].trim().to_ascii_uppercase(),
            trust: self.fields[TRUST_FIELD].trim().to_string(),
        }
    }

    /// The final save's rules, from talk-core's check_save (the same as Python's). Local writes at once.
    fn next_step(&self) -> Step {
        if self.mode == Mode::Local {
            return Step::Write;
        }
        let labels = labels();
        // The input keeps the saved code after a save: that code is already redeemed, so it is not a pending one.
        let typed = relay_rest::normalize_setup_code(self.fields[CODE_FIELD].trim());
        let pending = if typed.as_deref() == Some(self.setup_code.as_str()) { "" } else { self.fields[CODE_FIELD].as_str() };
        match relay_settings::check_save(&self.settings(), pending) {
            Ok(()) => Step::Write,
            Err(SaveRefusal::UnredeemedCode) => Step::Refuse(t("gui.relay.save_code_first", &[])),
            Err(SaveRefusal::MissingFields(missing)) => {
                let names: Vec<&str> = missing
                    .iter()
                    .map(|field| match field {
                        Missing::Name => labels[NAME_FIELD].as_str(),
                        Missing::Room => labels[ROOM_FIELD].as_str(),
                        Missing::Trust => labels[TRUST_FIELD].as_str(),
                    })
                    .collect();
                Step::Refuse(t("settings.save_refused_missing", &[("fields", &names.join(", "))]))
            }
            Err(SaveRefusal::NoConnection) => Step::Refuse(t("gui.relay.code_needed", &[("label", &labels[CODE_FIELD])])),
        }
    }

    /// The final Save button: refuse, or write now. Returns the window action.
    fn save(&mut self) -> Action {
        match self.next_step() {
            Step::Refuse(message) => {
                self.message = message;
                Action::Stay
            }
            Step::Write => {
                if self.save_now() {
                    Action::Saved
                } else {
                    Action::Stay
                }
            }
        }
    }

    /// Write relay.json from the form. True when the file was written.
    fn save_now(&mut self) -> bool {
        let settings = self.settings();
        match relay_settings::save(&self.state, &settings) {
            Ok(()) => {
                self.message = t(if settings.mode == Mode::Local { "settings.save_ok_local" } else { "settings.save_ok_supabase" }, &[]);
                true
            }
            Err(error) => {
                self.message = t("gui.relay.not_saved", &[("error", &error.to_string())]);
                false
            }
        }
    }
}

/// What the first-run window does in one frame: keep waiting (with the seconds left, when the timer runs), or start Local.
#[derive(Debug, PartialEq, Eq)]
pub enum Tick {
    Wait(Option<u64>),
    Start,
}

/// The 30-second default, from talk-core's ChoiceTimer. The timer starts when there is none; any input stops it.
/// Start means the time is up: the chat starts in Local mode and nothing is saved.
pub fn tick(timer: &mut Option<ChoiceTimer>, now: Instant, input_seen: bool) -> Tick {
    let running = timer.get_or_insert_with(|| ChoiceTimer::start(now));
    if input_seen {
        running.stop();
    }
    if running.timed_out(now) {
        return Tick::Start;
    }
    Tick::Wait(running.remaining(now))
}

fn connection_card(
    ui: &mut egui::Ui,
    palette: &Palette,
    number: &str,
    title: &str,
    button: &str,
    description: &str,
) -> egui::Response {
    let response = egui::Frame::new()
        .fill(palette.panel)
        .stroke(egui::Stroke::new(1.0_f32, palette.border))
        .inner_margin(egui::Margin::symmetric(22, 20))
        .show(ui, |ui| {
            let content_width = ui.available_width().min(436.0);
            ui.set_min_width(content_width);
            ui.set_max_width(content_width);
            ui.colored_label(palette.accent, egui::RichText::new(format!("[{number}] {title}")).monospace().size(18.0));
            ui.add_space(14.0);
            let button = ui.add_sized(
                [ui.available_width(), 48.0],
                egui::Button::new(egui::RichText::new(button).monospace().size(18.0)),
            );
            ui.add_space(10.0);
            ui.colored_label(palette.muted, egui::RichText::new(description).monospace());
            button
        })
        .inner;
    if response.hovered() {
        ui.painter().rect_stroke(
            response.rect.expand(1.0),
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.0_f32, palette.accent),
            egui::StrokeKind::Outside,
        );
    }
    response
}

/// The first-run choice screen. No close button: the window's own close ends the program. Some(pick) on a click.
pub fn choose(ctx: &egui::Context, palette: &Palette, remaining: Option<u64>) -> Option<Pick> {
    let mut picked = None;
    egui::TopBottomPanel::top("connection_terminal_header")
        .exact_height(50.0)
        .frame(egui::Frame::new().fill(palette.background).stroke(egui::Stroke::new(1.0_f32, palette.border)).inner_margin(egui::Margin::symmetric(18, 12)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(palette.accent, egui::RichText::new("▣ HERDR-TALK").monospace().strong().size(18.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.colored_label(palette.muted, egui::RichText::new(format!("SYS.VER 1.0  ·  {}", t("gui.first.ready", &[]))).monospace());
                });
            });
        });
    egui::TopBottomPanel::bottom("connection_terminal_status")
        .exact_height(40.0)
        .frame(egui::Frame::new().fill(palette.background).stroke(egui::Stroke::new(1.0_f32, palette.border)).inner_margin(egui::Margin::symmetric(18, 10)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(palette.accent, egui::RichText::new(t("gui.first.ready", &[])).monospace());
                ui.colored_label(palette.muted, egui::RichText::new(t("gui.first.waiting", &[])).monospace());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.colored_label(palette.muted, egui::RichText::new(t("gui.lang_hint", &[])).monospace());
                });
            });
        });
    egui::CentralPanel::default().frame(egui::Frame::new().fill(palette.background)).show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.12).max(16.0));
            ui.colored_label(palette.accent, egui::RichText::new(t("mode.local", &[])).monospace().size(15.0));
            ui.add_space(6.0);
            ui.colored_label(palette.text, egui::RichText::new(t("gui.first.tagline", &[])).monospace().size(16.0));
            ui.add_space(26.0);
            if connection_card(
                ui,
                palette,
                &t("gui.first.primary", &[]),
                &t("mode.local", &[]),
                &t("gui.first.connect", &[]),
                &t("mode.hint_local", &[]),
            )
            .clicked()
            {
                picked = Some(Pick::Local);
            }
            ui.add_space(18.0);
            ui.colored_label(palette.muted, egui::RichText::new(t("gui.first.alternative", &[])).monospace().size(13.0));
            if ui.link(egui::RichText::new(t("mode.supabase", &[])).monospace().size(14.0).color(palette.muted).underline()).clicked() {
                picked = Some(Pick::Supabase);
            }
            ui.add_space(20.0);
            if let Some(seconds) = remaining {
                ui.colored_label(palette.muted, egui::RichText::new(t("mode.timeout_hint", &[("n", &seconds.to_string())])).monospace());
            }
            ui.colored_label(palette.muted, egui::RichText::new(t("gui.relay.close_quits", &[])).monospace());
        });
    });
    picked
}

/// Draw `add` in a column of at most 680px, centred in the width of `ui` with 24px gutters.
fn centered_column<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let area = ui.available_rect_before_wrap();
    let width = (area.width() - 48.0).clamp(0.0, 680.0);
    let left = area.left() + (area.width() - width) / 2.0;
    let rect = egui::Rect::from_min_size(egui::pos2(left, area.top()), egui::vec2(width, area.height()));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), add).inner
}

/// The redeem answer (the URL, key and typed code from the relay): on success the URL, key and setup code are saved at once,
/// as the TUI does; a failure saves nothing. The code stays in the input, so a second save with it only re-saves (no second redeem).
fn apply_redeem(form: &mut Form, answer: Result<(String, String, String), String>) {
    match answer {
        Ok((url, key, code)) => {
            form.fields[CODE_FIELD] = code.clone();
            form.url = url;
            form.key = key;
            form.setup_code = code;
            form.message = match relay_settings::save_connection(&form.state, &form.url, &form.key, &form.setup_code) {
                Ok(()) => t("gui.relay.setup_saved", &[]),
                Err(error) => t("gui.relay.not_saved", &[("error", &error)]),
            };
        }
        Err(error) => form.message = t("gui.relay.redeem_failed", &[("error", &error)]),
    }
}

/// Draw the settings window: the mode row, the setup-code row (greyed out in Local mode), the room rows, and the buttons.
pub fn show(ctx: &egui::Context, palette: &Palette, form: &mut Form) -> Action {
    let mut action = Action::Stay;
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        return Action::Discard;
    }
    // The create-room answer, when it has come back.
    if let Some(answer) = form.creating.as_ref().and_then(|receiver| receiver.try_recv().ok()) {
        form.creating = None;
        match answer {
            Ok(code) => {
                form.message = t("gui.relay.created", &[("code", &code)]);
                form.fields[ROOM_FIELD] = code;
            }
            Err(error) => form.message = t("gui.relay.create_failed", &[("error", &error)]),
        }
    }
    // The trust check from `open`: show the refusal in the message line; keep polling until it answers.
    poll_trust(form);
    if form.trust_check.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    // The operator passphrase row opens with F9 (not named in any text).
    if operator_key(ctx) {
        form.operator = true;
    }
    // The issue answer: the code is shown once in the bar, never written to a file or log.
    if let Some(answer) = form.issuing.as_ref().and_then(|receiver| receiver.try_recv().ok()) {
        form.issuing = None;
        if let Ok(code) = &answer {
            form.fields[CODE_FIELD] = code.clone(); // the new code goes into the setup code input
        }
        form.message = issue_message(answer);
    }
    // The Save setup code answer: the URL, key and setup code are written at once (the middle save); nothing is shown.
    if let Some(answer) = form.redeeming.as_ref().and_then(|receiver| receiver.try_recv().ok()) {
        form.redeeming = None;
        apply_redeem(form, answer);
    }
    // F2 is the same as the Save setup code button (the TUI's F2).
    if form.mode == Mode::Supabase && form.redeeming.is_none() && ctx.input(|input| input.key_pressed(egui::Key::F2)) {
        form.save_setup_code(ctx);
    }
    egui::TopBottomPanel::bottom("relay_settings_actions")
        .exact_height(60.0)
        .frame(egui::Frame::new().fill(palette.background).stroke(egui::Stroke::new(1.0_f32, palette.border)).inner_margin(egui::Margin::symmetric(20, 10)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if !form.message.is_empty() {
                    ui.colored_label(palette.muted, egui::RichText::new(form.message.as_str()).monospace());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let save = egui::Button::new(egui::RichText::new(t("gui.relay.button.save", &[])).monospace().color(palette.accent)).min_size(egui::vec2(150.0, 30.0));
                    if ui.add_enabled(form.redeeming.is_none(), save).clicked() {
                        action = form.save();
                    }
                    if ui.add_sized([90.0, 30.0], egui::Button::new(egui::RichText::new(t("gui.relay.button.cancel", &[])).monospace())).clicked() {
                        action = Action::Discard;
                    }
                });
            });
        });
    egui::TopBottomPanel::top("relay_settings_title")
        .exact_height(78.0)
        .frame(egui::Frame::new().fill(palette.background).inner_margin(egui::Margin::symmetric(24, 24)))
        .show(ctx, |ui| {
            centered_column(ui, |ui| {
                ui.colored_label(palette.accent, egui::RichText::new(t("gui.relay.title", &[])).monospace().heading());
            });
        });
    egui::CentralPanel::default().frame(egui::Frame::new().fill(palette.background)).show(ctx, |ui| {
        centered_column(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("relay_settings_scroll").show(ui, |ui| {
                    ui.add_space(24.0);
                    let enabled = form.mode == Mode::Supabase;
                    let labels = labels();
                    egui::Frame::new()
                        .fill(palette.panel)
                        .stroke(egui::Stroke::new(1.0_f32, palette.border))
                        .inner_margin(egui::Margin::same(20))
                        .show(ui, |ui| {
                            ui.colored_label(palette.accent, egui::RichText::new(t("gui.relay.section1", &[])).monospace());
                            ui.add_space(14.0);
                            ui.colored_label(palette.text, egui::RichText::new(labels[CODE_FIELD].as_str()).monospace());
                            let redeeming = form.redeeming.is_some();
                            let mut save_setup = false;
                            ui.add_enabled_ui(enabled, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_sized([(ui.available_width() - 170.0).max(120.0), 30.0], egui::TextEdit::singleline(&mut form.fields[CODE_FIELD]));
                                    let button = egui::Button::new(egui::RichText::new(t("gui.relay.button.save_code", &[])).monospace()).min_size(egui::vec2(160.0, 30.0));
                                    save_setup = ui.add_enabled(!redeeming, button).clicked();
                                });
                            });
                            ui.colored_label(palette.muted, egui::RichText::new(t("gui.relay.hint.code", &[])).monospace());
                            if save_setup {
                                form.save_setup_code(ctx);
                            }
                            if form.operator {
                                ui.add_space(14.0);
                                ui.colored_label(palette.text, egui::RichText::new(t("gui.relay.label.passphrase", &[])).monospace());
                                let issuing = form.issuing.is_some() || form.redeeming.is_some();
                                let mut issue = false;
                                ui.horizontal(|ui| {
                                    let edit = egui::TextEdit::singleline(&mut form.passphrase).password(true);
                                    let response = ui.add_sized([(ui.available_width() - 170.0).max(120.0), 30.0], edit);
                                    let button = egui::Button::new(egui::RichText::new(t("gui.relay.button.issue", &[])).monospace()).min_size(egui::vec2(160.0, 30.0));
                                    issue = ui.add_enabled(!issuing, button).clicked() || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                                });
                                if issue && !issuing {
                                    form.start_issue(ctx);
                                }
                            }
                            let (env_url, env_key) = form.from_env;
                            if env_url || env_key {
                                ui.colored_label(palette.muted, egui::RichText::new(t("gui.relay.env_wins", &[])).monospace());
                            }
                            // The key is shown by its length only.
                            if !form.key.is_empty() {
                                ui.colored_label(palette.muted, egui::RichText::new(format!("{} {}", t("gui.relay.label.key", &[]), relay_settings::key_label(&form.key))).monospace());
                            }
                        });
                    ui.add_space(24.0);
                    egui::Frame::new()
                        .fill(palette.panel)
                        .stroke(egui::Stroke::new(1.0_f32, palette.border))
                        .inner_margin(egui::Margin::same(20))
                        .show(ui, |ui| {
                            ui.colored_label(palette.accent, egui::RichText::new(t("gui.relay.section2", &[])).monospace());
                            ui.add_space(14.0);
                            // The rows in Python's order: trust code, room ID (the Unique ID is not drawn).
                            for index in [TRUST_FIELD, ROOM_FIELD] {
                                ui.colored_label(palette.text, egui::RichText::new(labels[index].as_str()).monospace());
                                let edit = egui::TextEdit::singleline(&mut form.fields[index]);
                                ui.add_enabled_ui(enabled, |ui| {
                                    ui.add_sized([ui.available_width(), 30.0], edit);
                                });
                                ui.add_space(14.0);
                            }
                            let can_create = enabled && form.mine.is_some() && form.creating.is_none();
                            if ui.add_enabled(can_create, egui::Button::new(egui::RichText::new(t("gui.relay.button.create_room", &[])).monospace()).min_size(egui::vec2(160.0, 30.0))).clicked() {
                                form.start_create_room(ctx);
                            }
                        });
                    ui.add_space(24.0);
                    egui::Frame::new()
                        .fill(palette.panel)
                        .stroke(egui::Stroke::new(1.0_f32, palette.border))
                        .inner_margin(egui::Margin::same(20))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.colored_label(palette.accent, egui::RichText::new(t("gui.relay.section3", &[])).monospace());
                            ui.add_space(14.0);
                            if enabled && !form.short_id.is_empty() {
                                ui.colored_label(palette.text, egui::RichText::new(t("gui.relay.my_id", &[])).monospace());
                                ui.add(egui::Label::new(egui::RichText::new(form.short_id.as_str()).monospace().size(22.0)));
                                ui.add_space(14.0);
                            }
                            match &form.mine {
                                Some((ed, noise)) => {
                                    ui.colored_label(palette.text, egui::RichText::new(t("gui.relay.my_ed", &[])).monospace());
                                    ui.add(egui::Label::new(egui::RichText::new(ed.as_str()).monospace().size(11.5).color(palette.muted)).wrap());
                                    ui.add_space(14.0);
                                    ui.colored_label(palette.text, egui::RichText::new(t("gui.relay.my_noise", &[])).monospace());
                                    ui.add(egui::Label::new(egui::RichText::new(noise.as_str()).monospace().size(11.5).color(palette.muted)).wrap());
                                }
                                None => {
                                    ui.colored_label(palette.muted, egui::RichText::new(t("gui.relay.no_keys_yet", &[])).monospace());
                                }
                            }
                        });
                    ui.add_space(20.0);
                });
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("herdr-talk-gui-form-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn form_at(state: &Path) -> Form {
        Form {
            mode: Mode::Local,
            url: String::new(),
            key: String::new(),
            setup_code: String::new(),
            trust_check: None,
            fields: Default::default(),
            message: String::new(),
            from_env: (false, false),
            mine: None,
            short_id: String::new(),
            state: state.to_path_buf(),
            creating: None,
            redeeming: None,
            operator: false,
            passphrase: String::new(),
            issuing: None,
        }
    }

    /// A Supabase form with a trust code, a display name and a room ID, and no setup code and no saved URL and key.
    fn supabase_form(state: &Path) -> Form {
        let mut form = form_at(state);
        form.mode = Mode::Supabase;
        form.fields = [String::new(), "12345678".into(), "Alice".into(), "abcd2345".into()];
        form
    }

    #[test]
    fn a_redeemed_setup_code_saves_url_key_and_code_and_a_refusal_saves_nothing() {
        // The server answer is injected (no network): the redeem answer goes straight to apply_redeem, as show() does.
        let ok_dir = folder("redeem_ok");
        let mut form = supabase_form(&ok_dir);
        form.fields[CODE_FIELD] = "abcd-2345".into();
        apply_redeem(&mut form, Ok(("https://example.supabase.co".into(), "dummy-test-key".into(), "ABCD2345".into())));
        let saved = relay_settings::load_saved(&ok_dir);
        assert_eq!((saved.url.as_str(), saved.key.as_str(), saved.setup_code.as_str()), ("https://example.supabase.co", "dummy-test-key", "ABCD2345"));
        assert_eq!(form.message, t("gui.relay.setup_saved", &[]));
        let _ = std::fs::remove_dir_all(&ok_dir);

        let refused_dir = folder("redeem_refused");
        let mut form = supabase_form(&refused_dir);
        form.fields[CODE_FIELD] = "abcd-2345".into();
        apply_redeem(&mut form, Err("wrong".into()));
        let saved = relay_settings::load_saved(&refused_dir);
        assert!(saved.url.is_empty() && saved.key.is_empty() && saved.setup_code.is_empty(), "nothing is saved");
        assert_eq!(form.message, t("gui.relay.redeem_failed", &[("error", "wrong")]));
        let _ = std::fs::remove_dir_all(&refused_dir);
    }

    #[test]
    fn a_saved_code_left_in_the_input_does_not_block_the_final_save() {
        let mut form = supabase_form(&folder("saved-code-input"));
        form.url = "https://example.supabase.co".into();
        form.key = "k".into();
        form.setup_code = "ABCD2345".into();
        form.fields[CODE_FIELD] = "ABCD-2345".into();
        assert!(matches!(form.next_step(), Step::Write));
        form.fields[CODE_FIELD] = "WXYZ6789".into();
        assert!(matches!(form.next_step(), Step::Refuse(_)));
    }

    #[test]
    fn a_typed_setup_code_is_not_redeemed_by_the_final_save() {
        let mut form = supabase_form(&folder("typed"));
        form.url = "https://example.supabase.co".into();
        form.key = "dummy-test-key".into();
        form.fields[CODE_FIELD] = "abcd-2345".into();
        assert!(matches!(form.next_step(), Step::Refuse(message) if message == t("gui.relay.save_code_first", &[])));
    }

    #[test]
    fn the_save_setup_code_button_refuses_a_malformed_code_without_a_request() {
        let mut form = supabase_form(&folder("button"));
        form.fields[CODE_FIELD] = "ABC".into();
        form.save_setup_code(&egui::Context::default());
        assert!(form.redeeming.is_none());
        assert_eq!(form.message, t("setup.invalid", &[]));
    }

    #[test]
    fn the_final_save_keeps_the_saved_setup_code() {
        let dir = folder("keep_code");
        let mut form = supabase_form(&dir);
        form.url = "https://example.supabase.co".into();
        form.key = "dummy-test-key".into();
        form.setup_code = "ABCD2345".into();
        form.fields[ROOM_FIELD] = "ABCD2345".into();
        assert!(form.save_now());
        assert_eq!(relay_settings::load_saved(&dir).setup_code, "ABCD2345");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_trust_refusal_shows_only_while_the_opening_message_is_still_shown() {
        let mut message = String::new();
        apply_trust(&mut message, "", true);
        assert_eq!(message, t("room.trust_wrong", &[]));
        let mut newer = "saved".to_string();
        apply_trust(&mut newer, "", true);
        assert_eq!(newer, "saved", "a late refusal does not replace a newer message");
        let mut accepted = String::new();
        apply_trust(&mut accepted, "", false);
        assert_eq!(accepted, "", "an accepted trust code shows nothing");
    }

    #[test]
    fn a_late_trust_refusal_does_not_overwrite_a_message_shown_after_the_window_opened() {
        // Repro: the window opens with no message, the user saves (a new message), then the slow trust check says no.
        let dir = folder("late_trust");
        let mut form = form_at(&dir);
        let (sender, receiver) = mpsc::channel();
        form.trust_check = Some((receiver, String::new()));
        form.message = t("settings.save_ok_local", &[]);
        sender.send(true).unwrap();
        poll_trust(&mut form);
        assert_eq!(form.message, t("settings.save_ok_local", &[]), "the newer message stays");
        assert!(form.trust_check.is_none(), "the answer is taken once");
        // And the answer still shows when nothing newer was written in the meantime.
        let (sender, receiver) = mpsc::channel();
        form.trust_check = Some((receiver, String::new()));
        form.message.clear();
        sender.send(true).unwrap();
        poll_trust(&mut form);
        assert_eq!(form.message, t("room.trust_wrong", &[]));
    }

    #[test]
    fn a_join_refused_with_403_gives_the_trust_message() {
        match relay_rest::read_join_room_response(403, "") {
            Err(error) => assert_eq!(error, t("room.trust_wrong", &[])),
            Ok(_) => panic!("403 must not join"),
        }
    }

    #[test]
    fn an_issue_answer_shows_the_code_or_the_reason() {
        assert_eq!(issue_message(Ok("ABCD2345".into())), t("gui.relay.issued", &[("code", "ABCD2345")]));
        assert_eq!(issue_message(Err("wrong passphrase".into())), t("gui.relay.issue_failed", &[("error", "wrong passphrase")]));
    }

    #[test]
    fn f9_opens_the_operator_row_and_other_keys_do_not() {
        let ctx = egui::Context::default();
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        ctx.begin_pass(egui::RawInput { events: vec![key(egui::Key::F9)], ..Default::default() });
        assert!(operator_key(&ctx));
        let _ = ctx.end_pass();
        ctx.begin_pass(egui::RawInput { events: vec![key(egui::Key::F8)], ..Default::default() });
        assert!(!operator_key(&ctx));
        let _ = ctx.end_pass();
    }

    #[test]
    fn issuing_without_a_passphrase_sends_no_request() {
        let mut form = supabase_form(&folder("issue"));
        form.start_issue(&egui::Context::default());
        assert!(form.issuing.is_none());
        assert_eq!(form.message, t("gui.relay.operator_empty", &[]));
    }

    #[test]
    fn saved_url_and_key_are_kept_when_no_code_is_typed() {
        let dir = folder("kept");
        let mut form = supabase_form(&dir);
        form.url = "https://example.supabase.co".into();
        form.key = "dummy-test-key".into();
        assert_eq!(form.next_step(), Step::Write);
        assert!(form.save_now());
        let saved = relay_settings::load_saved(&dir);
        assert_eq!(saved.url, "https://example.supabase.co");
        assert_eq!(saved.key, "dummy-test-key");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn with_no_code_and_no_saved_url_and_key_the_save_refuses_and_asks_for_the_code() {
        let form = supabase_form(&folder("neither"));
        match form.next_step() {
            Step::Refuse(message) => assert!(message.contains(&labels()[CODE_FIELD]), "{message}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_unique_id_starts_as_this_pcs_short_id_and_is_what_is_saved() {
        let dir = folder("name");
        let form = Form::open(&dir);
        let id = talk_core::relay_key::short_id(&dir).unwrap();
        assert_eq!(form.fields[NAME_FIELD], id, "the row shows this PC's short ID");
        assert_eq!(form.settings().name, id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_supabase_save_names_the_missing_fields_and_writes_nothing() {
        let mut form = supabase_form(&folder("refused"));
        form.url = "https://example.supabase.co".into();
        form.key = "dummy-test-key".into();
        form.fields[NAME_FIELD].clear();
        match form.next_step() {
            Step::Refuse(message) => assert!(message.contains(&labels()[NAME_FIELD]), "{message}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_local_save_keeps_the_fields_and_writes_the_mode() {
        let dir = folder("local");
        let mut form = form_at(&dir);
        form.fields[NAME_FIELD] = "Alice".into();
        assert_eq!(form.next_step(), Step::Write);
        assert!(form.save_now());
        let saved = relay_settings::load_saved(&dir);
        assert_eq!(saved.mode, Mode::Local);
        assert_eq!(saved.name, "Alice");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_complete_supabase_save_is_written_without_a_byte_order_mark() {
        let dir = folder("good");
        let mut form = supabase_form(&dir);
        form.url = "https://example.supabase.co".into();
        form.key = "dummy-test-key".into();
        assert!(form.save_now());
        let bytes = std::fs::read(dir.join(relay_settings::FILE)).unwrap();
        assert!(!bytes.starts_with(&[0xEF, 0xBB, 0xBF]), "written without BOM");
        let saved = relay_settings::load_saved(&dir);
        assert_eq!(saved.mode, Mode::Supabase);
        assert_eq!(saved.room_code, "ABCD2345");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_signed_create_body_has_the_required_fields() {
        let nonce = "00".repeat(16);
        let body = signed_room_body(&[7; 32], RoomAction::Create, "", "Alice", "12345678", 1_700_000_000, &nonce).unwrap();
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        for field in ["peer_pk", "display_name", "ts", "nonce", "sig"] {
            assert!(value.get(field).is_some(), "{field} missing from {body}");
        }
        assert_eq!(value["display_name"], "Alice");
        assert_eq!(value["ts"], 1_700_000_000);
        assert_eq!(value["nonce"], nonce);
    }

    #[test]
    fn a_failed_peer_publication_skips_create_request() {
        let mut create_called = false;
        let result = require_published(PutOutcome::Failed).and_then(|()| {
            create_called = true;
            Ok::<(), String>(())
        });
        assert!(result.is_err());
        assert!(!create_called);
    }

    #[test]
    fn a_file_with_a_byte_order_mark_is_read() {
        let dir = folder("bom");
        std::fs::create_dir_all(&dir).unwrap();
        let text = format!("{}{}", '\u{feff}', r#"{"url":"https://example.supabase.co","key":"k","name":"Bob","room_code":""}"#);
        std::fs::write(dir.join(relay_settings::FILE), text).unwrap();
        assert_eq!(relay_settings::load_saved(&dir).name, "Bob");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_timer_starts_waits_and_then_chooses_local() {
        let start = Instant::now();
        let mut timer = None;
        assert_eq!(tick(&mut timer, start, false), Tick::Wait(Some(30)));
        assert_eq!(tick(&mut timer, start + std::time::Duration::from_secs(29), false), Tick::Wait(Some(1)));
        assert_eq!(tick(&mut timer, start + std::time::Duration::from_secs(30), false), Tick::Start);
    }

    #[test]
    fn input_stops_the_timer_and_nothing_starts_later() {
        let start = Instant::now();
        let mut timer = None;
        tick(&mut timer, start, false);
        assert_eq!(tick(&mut timer, start + std::time::Duration::from_secs(5), true), Tick::Wait(None));
        assert_eq!(tick(&mut timer, start + std::time::Duration::from_secs(120), false), Tick::Wait(None));
    }

    #[test]
    fn a_fresh_timer_starts_again_after_the_settings_window() {
        let start = Instant::now();
        let mut timer = None;
        tick(&mut timer, start, false);
        timer = None; // what the window does when the settings window is cancelled
        assert_eq!(tick(&mut timer, start + std::time::Duration::from_secs(60), false), Tick::Wait(Some(30)));
    }

}
