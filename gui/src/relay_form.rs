//! The connection-mode screens of the window. Two kinds: the first-run choice (`choose`), shown before the chat
//! when relay.json has no `mode`, and the settings window (`Form`, Ctrl+R and the top-bar button) with the mode row and the four Supabase fields.
//! The rules (validation, saving, BOM, mode, texts) live in talk-core; this file only draws and keeps the typed text.

use crate::Palette;
use eframe::egui;
use std::path::{Path, PathBuf};
use talk_core::choice_timer::ChoiceTimer;
use talk_core::relay_settings::{self, Mode, Settings};
use talk_core::t;
use std::time::Instant;

/// The four field labels, in field order.
fn labels() -> [String; 4] {
    [
        t("gui.relay.label.url", &[]),
        t("gui.relay.label.key", &[]),
        t("gui.relay.label.name", &[]),
        t("gui.relay.label.room_code", &[]),
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
            url: self.fields[0].trim().to_string(),
            key: self.fields[1].trim().to_string(),
            name: self.fields[2].trim().to_string(),
            room_code: self.fields[3].trim().to_string(),
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
        ui.separator();
        if !form.short_id.is_empty() {
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
        Form { mode: Mode::Local, fields: Default::default(), message: String::new(), from_env: (false, false), mine: None, short_id: String::new(), state: state.to_path_buf() }
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
        assert_eq!(relay_settings::load_saved(&dir).mode, Mode::Supabase);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_with_a_byte_order_mark_is_read() {
        let dir = folder("bom");
        std::fs::create_dir_all(&dir).unwrap();
        let text = format!("{}{}", '\u{feff}', r#"{"url":"https://example.supabase.co","key":"k","name":"Bob","peer":""}"#);
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
