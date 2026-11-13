//! The connection-mode screens of the window. Two kinds: the first-run choice (`choose`), shown before the chat
//! when relay.json has no `mode`, and the settings window (`Form`, Ctrl+R and the top-bar button) with the mode row and the four Supabase fields.
//! The rules (validation, saving, BOM, mode, texts) live in talk-core; this file only draws and keeps the typed text.

use crate::Palette;
use eframe::egui;
use std::path::{Path, PathBuf};
use talk_core::choice_timer::ChoiceTimer;
use talk_core::relay_rest::{self, random_nonce, signed_room_body, unix_now, PutOutcome, Request, RoomAction};
use talk_core::relay_settings::{self, Mode, Settings};
use talk_core::t;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

/// The four field labels, in field order.
/// shortcut: the room ID label is written here because locales have no key for it; move it to talk-core/locales when those are allowed to change.
fn labels() -> [String; 4] {
    [
        t("gui.relay.label.url", &[]),
        t("gui.relay.label.key", &[]),
        t("gui.relay.label.name", &[]),
        "방 ID (8자리)".to_string(),
    ]
}
const KEY_FIELD: usize = 1;

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
    fields: [String; 4],
    pub message: String,
    from_env: (bool, bool),
    mine: Option<(String, String)>,
    /// This PC's short ID (what the peer types to find it); empty when it cannot be read.
    short_id: String,
    state: PathBuf,
    /// A create-room request in flight: its answer (the room code, or the reason it failed).
    creating: Option<Receiver<Result<String, String>>>,
}

/// The answer of create-room: the room code on 200, otherwise the status and body.
fn read_room_code(status: u16, text: &str) -> Result<String, String> {
    if status != 200 {
        return Err(format!("status {status}: {text}"));
    }
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| "answer is not JSON".to_string())?;
    value["room_code"].as_str().map(str::to_string).ok_or_else(|| "answer has no room_code".to_string())
}

fn require_published(outcome: PutOutcome) -> Result<(), String> {
    match outcome {
        PutOutcome::Written => Ok(()),
        PutOutcome::Taken => Err("이 PC ID가 다른 키에 사용 중입니다 (409)".to_string()),
        PutOutcome::Failed => Err("이 PC 행을 게시하지 못했습니다".to_string()),
    }
}

impl Form {
    pub fn open(state: &Path) -> Form {
        let saved = relay_settings::load_saved(state);
        let mine = talk_core::relay_chat::my_keys(state).ok();
        let short_id = talk_core::relay_key::short_id(state).unwrap_or_default();
        let message = if mine.is_none() { t("gui.relay.no_keys", &[]) } else { String::new() };
        Form {
            mode: saved.mode,
            fields: [saved.url, saved.key, saved.name, saved.room_code],
            message,
            from_env: relay_settings::from_environment(),
            mine,
            short_id,
            state: state.to_path_buf(),
            creating: None,
        }
    }

    /// Ask Supabase for a new room (create-room) with this PC's key and the display name, and put the code in the room ID field.
    /// Runs in a thread so the window keeps drawing; `show` reads the answer.
    fn start_create_room(&mut self, ctx: &egui::Context) {
        let url = self.fields[0].trim().to_string();
        let key = self.fields[1].trim().to_string();
        let name = self.fields[2].trim().to_string();
        if url.is_empty() || key.is_empty() || name.is_empty() {
            self.message = "방 만들기: 프로젝트 URL, 키, 표시 이름을 먼저 입력하세요".to_string();
            return;
        }
        let config = match relay_rest::config(Some(&url), Some(&key)) {
            Ok(Some(config)) => config,
            Ok(None) => {
                self.message = "방 만들기: 프로젝트 URL과 키가 필요합니다".to_string();
                return;
            }
            Err(error) => {
                self.message = format!("방 만들기: 릴레이 설정이 올바르지 않습니다: {error}");
                return;
            }
        };
        let seed = match talk_core::relay_key::load_or_create(&self.state) {
            Ok(seed) => seed,
            Err(error) => {
                self.message = format!("방 만들기: 서명 키를 읽을 수 없습니다: {error}");
                return;
            }
        };
        let (_, noise_pk) = match talk_core::relay_chat::my_keys(&self.state) {
            Ok(keys) => keys,
            Err(error) => {
                self.message = format!("방 만들기: Noise 키를 읽을 수 없습니다: {error}");
                return;
            }
        };
        let short_id = match talk_core::relay_key::short_id(&self.state) {
            Ok(short_id) => short_id,
            Err(error) => {
                self.message = format!("방 만들기: 이 PC ID를 읽을 수 없습니다: {error}");
                return;
            }
        };
        let nonce = match random_nonce() {
            Ok(nonce) => nonce,
            Err(error) => {
                self.message = format!("방 만들기: nonce를 만들 수 없습니다: {error}");
                return;
            }
        };
        let body = match signed_room_body(&seed, RoomAction::Create, "", &name, unix_now(), &nonce) {
            Ok(body) => body,
            Err(error) => {
                self.message = format!("방 만들기: 서명 요청을 만들 수 없습니다: {error}");
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
        self.message = "방 만드는 중...".to_string();
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
            url: self.fields[0].trim().to_string(),
            key: self.fields[1].trim().to_string(),
            name: self.fields[2].trim().to_string(),
            room_code: self.fields[3].trim().to_ascii_uppercase(),
        }
    }

    /// Save. True when the file was written. Supabase names every missing field and writes nothing.
    pub fn save(&mut self) -> bool {
        let settings = self.settings();
        if settings.mode == Mode::Supabase {
            let labels = labels();
            let missing: Vec<&str> = (0..labels.len()).filter(|&index| self.fields[index].trim().is_empty()).map(|index| labels[index].as_str()).collect();
            if !missing.is_empty() {
                self.message = t("settings.save_refused_missing", &[("fields", &missing.join(", "))]);
                return false;
            }
        }
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

    /// The text the window shows for one field (the key only as its length, by the password field).
    #[cfg(test)]
    fn shown_label(&self, index: usize) -> String {
        if index == KEY_FIELD {
            relay_settings::key_label(&self.fields[index])
        } else {
            self.fields[index].clone()
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

/// The first-run choice screen. No close button: the window's own close ends the program. Some(pick) on a click.
pub fn choose(ctx: &egui::Context, palette: &Palette, remaining: Option<u64>) -> Option<Pick> {
    let mut picked = None;
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.add_space(24.0);
        ui.colored_label(palette.accent, egui::RichText::new(t("mode.first_run_title", &[])).heading());
        ui.add_space(20.0);
        if ui.add(egui::Button::new(egui::RichText::new(t("mode.local", &[])).size(18.0)).min_size(egui::vec2(260.0, 40.0))).clicked() {
            picked = Some(Pick::Local);
        }
        ui.colored_label(palette.muted, t("mode.hint_local", &[]));
        ui.add_space(16.0);
        if ui.add(egui::Button::new(egui::RichText::new(t("mode.supabase", &[])).size(18.0)).min_size(egui::vec2(260.0, 40.0))).clicked() {
            picked = Some(Pick::Supabase);
        }
        ui.colored_label(palette.muted, t("mode.hint_supabase", &[]));
        ui.add_space(20.0);
        if let Some(seconds) = remaining {
            ui.colored_label(palette.accent, t("mode.timeout_hint", &[("n", &seconds.to_string())]));
        }
        ui.colored_label(palette.muted, t("gui.relay.close_quits", &[]));
    });
    picked
}

/// Draw the settings window: the mode row, the four fields (greyed out in Local mode), and the buttons.
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
                form.message = format!("방을 만들었습니다: {code} (저장을 눌러야 적용됩니다)");
                form.fields[3] = code;
            }
            Err(error) => form.message = format!("방 만들기 실패: {error}"),
        }
    }
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.colored_label(palette.accent, egui::RichText::new(t("settings.mode_row", &[])).heading());
        ui.horizontal(|ui| {
            ui.selectable_value(&mut form.mode, Mode::Local, t("mode.local", &[]));
            ui.selectable_value(&mut form.mode, Mode::Supabase, t("mode.supabase", &[]));
        });
        let hint = match form.mode {
            Mode::Local => t("mode.hint_local", &[]),
            Mode::Supabase => t("mode.hint_supabase", &[]),
        };
        ui.colored_label(palette.muted, hint);
        ui.add_space(8.0);
        let enabled = form.mode == Mode::Supabase;
        for (index, label) in labels().iter().enumerate() {
            ui.label(label.as_str());
            let edit = egui::TextEdit::singleline(&mut form.fields[index]).desired_width(f32::INFINITY).password(index == KEY_FIELD);
            ui.add_enabled(enabled, edit);
            let (env_url, env_key) = form.from_env;
            if (index == 0 && env_url) || (index == KEY_FIELD && env_key) {
                ui.colored_label(palette.muted, t("gui.relay.env_wins", &[]));
            }
            ui.add_space(4.0);
        }
        // Only Supabase mode can create a room, and only once this PC has its keys and no request is running.
        let can_create = enabled && form.mine.is_some() && form.creating.is_none();
        if ui.add_enabled(can_create, egui::Button::new("방 만들기")).clicked() {
            form.start_create_room(ctx);
        }
        ui.separator();
        if enabled && !form.short_id.is_empty() {
            ui.label(t("gui.relay.my_id", &[]));
            ui.add(egui::Label::new(egui::RichText::new(form.short_id.as_str()).monospace().size(22.0)));
        }
        match &form.mine {
            Some((ed, noise)) => {
                ui.label(t("gui.relay.my_ed", &[]));
                ui.add(egui::Label::new(egui::RichText::new(ed.as_str()).monospace()).wrap());
                ui.label(t("gui.relay.my_noise", &[]));
                ui.add(egui::Label::new(egui::RichText::new(noise.as_str()).monospace()).wrap());
            }
            None => {
                ui.label(t("gui.relay.no_keys_yet", &[]));
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button(t("gui.button.save", &[])).clicked() && form.save() {
                action = Action::Saved;
            }
            if ui.button(t("gui.button.close", &[])).clicked() {
                action = Action::Discard;
            }
        });
        if !form.message.is_empty() {
            ui.add_space(6.0);
            ui.colored_label(palette.muted, form.message.as_str());
        }
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
        Form { mode: Mode::Local, fields: Default::default(), message: String::new(), from_env: (false, false), mine: None, short_id: String::new(), state: state.to_path_buf(), creating: None }
    }

    #[test]
    fn the_key_is_shown_by_length_only() {
        let mut form = form_at(&folder("mask"));
        form.fields[KEY_FIELD] = "dummy-test-key".into();
        assert_eq!(form.shown_label(KEY_FIELD), "(14 chars)");
    }

    #[test]
    fn a_supabase_save_names_the_missing_fields_and_writes_nothing() {
        let dir = folder("refused");
        let mut form = form_at(&dir);
        form.mode = Mode::Supabase;
        form.fields[0] = "https://example.supabase.co".into();
        assert!(!form.save());
        assert!(form.message.contains(&t("gui.relay.label.key", &[])) && form.message.contains(&t("gui.relay.label.name", &[])), "{}", form.message);
        assert!(!dir.join(relay_settings::FILE).exists());
    }

    #[test]
    fn a_local_save_keeps_the_fields_and_writes_the_mode() {
        let dir = folder("local");
        let mut form = form_at(&dir);
        form.fields[2] = "Alice".into();
        assert!(form.save());
        let saved = relay_settings::load_saved(&dir);
        assert_eq!(saved.mode, Mode::Local);
        assert_eq!(saved.name, "Alice");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_complete_supabase_save_is_written_without_a_byte_order_mark() {
        let dir = folder("good");
        let mut form = form_at(&dir);
        form.mode = Mode::Supabase;
        form.fields = ["https://example.supabase.co".into(), "dummy-test-key".into(), "Alice".into(), "abcd2345".into()];
        assert!(form.save());
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
        let body = signed_room_body(&[7; 32], RoomAction::Create, "", "Alice", 1_700_000_000, &nonce).unwrap();
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
