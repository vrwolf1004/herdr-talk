//! The relay settings screen (Ctrl+R). First row: the connection mode (Local or Supabase, one of the two, never both).
//! Then four fields: project URL, publishable key, display name and the peer's short ID (8 characters). Tab or Up/Down moves between
//! rows; Left/Right or Space toggles the mode; Enter goes on, and saves on the last field; Esc leaves without saving.
//! The key is never drawn: only its length. Environment variables win over the saved values, and the screen says so.
//! Saving in Local mode keeps the typed fields; saving in Supabase mode checks them all (talk-core validate).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{Clear, ClearType};
use crossterm::{cursor, queue};
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use talk_core::relay_key;
use talk_core::relay_settings::{self, Mode, Settings};
use talk_core::{relay_chat, t, Chat};

/// The four field labels, in field order.
fn labels() -> [String; 4] {
    [
        t("tui.settings.label.url", &[]),
        t("tui.settings.label.key", &[]),
        t("tui.settings.label.name", &[]),
        t("tui.settings.label.peer", &[]),
    ]
}
const KEY_FIELD: usize = 1;
/// Rows: 0 is the mode, 1 to 4 are the four fields.
const ROWS: usize = 5;

pub struct Form {
    mode: Mode,
    fields: [String; 4],
    focus: usize,
    message: String,
    from_env: (bool, bool),
    mine: Option<(String, String)>,
    /// This PC's short ID (what the peer types to find it); empty when it cannot be read.
    short_id: String,
    state: PathBuf,
    /// True once a setting was written (the first-run choice then goes on to the chat).
    saved: bool,
}

pub enum Action {
    Stay,
    Leave,
}

impl Form {
    pub fn open(state: &Path) -> Form {
        let saved = relay_settings::load_saved(state);
        let mine = relay_chat::my_keys(state).ok();
        let short_id = relay_key::short_id(state).unwrap_or_default();
        let message = if mine.is_none() { t("tui.settings.no_keys", &[]) } else { String::new() };
        Form {
            mode: saved.mode,
            fields: [saved.url, saved.key, saved.name, saved.room_code],
            focus: 0,
            message,
            from_env: relay_settings::from_environment(),
            mine,
            short_id,
            state: state.to_path_buf(),
            saved: false,
        }
    }

    /// The settings screen opened for Supabase (the first-run choice): the mode row is on Supabase, the cursor on the URL.
    pub fn open_supabase(state: &Path) -> Form {
        let mut form = Form::open(state);
        form.mode = Mode::Supabase;
        form.focus = 1;
        form
    }

    pub fn is_saved(&self) -> bool {
        self.saved
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

    /// Save, then start the relay with the new settings. The message says what happened, never a key value.
    fn save(&mut self, chat: &Arc<Chat>) {
        let settings = self.settings();
        // Supabase needs every field: name the missing ones instead of a generic refusal.
        if settings.mode == Mode::Supabase {
            let labels = labels();
            let missing: Vec<&str> = (0..labels.len()).filter(|&index| self.fields[index].trim().is_empty()).map(|index| labels[index].as_str()).collect();
            if !missing.is_empty() {
                self.message = t("settings.save_refused_missing", &[("fields", &missing.join(", "))]);
                return;
            }
        }
        match relay_settings::save(&self.state, &settings) {
            Err(error) => self.message = t("tui.settings.not_saved", &[("error", &error.to_string())]),
            Ok(()) => {
                self.saved = true;
                let started = relay_chat::start(chat, &self.state);
                let text = match (settings.mode, started) {
                    (Mode::Local, _) => t("settings.save_ok_local", &[]),
                    (Mode::Supabase, Ok(_)) => t("settings.save_ok_supabase", &[]),
                    (Mode::Supabase, Err(error)) => format!("relay: {error}"),
                };
                self.message = text.clone();
                // The chat screen shows the result line.
                chat.say(text);
            }
        }
    }

    fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            Mode::Local => Mode::Supabase,
            Mode::Supabase => Mode::Local,
        };
    }
}

/// One key for the form.
pub fn on_key(form: &mut Form, key: KeyEvent, chat: &Arc<Chat>) -> Action {
    let on_mode_row = form.focus == 0;
    match key.code {
        KeyCode::Esc => return Action::Leave,
        KeyCode::Tab | KeyCode::Down => form.focus = (form.focus + 1) % ROWS,
        KeyCode::Up => form.focus = (form.focus + ROWS - 1) % ROWS,
        KeyCode::Left | KeyCode::Right if on_mode_row => form.toggle_mode(),
        KeyCode::Char(' ') if on_mode_row => form.toggle_mode(),
        KeyCode::Enter if form.focus + 1 < ROWS => form.focus += 1,
        KeyCode::Enter => form.save(chat),
        KeyCode::Backspace if !on_mode_row => {
            form.fields[form.focus - 1].pop();
        }
        KeyCode::Char(ch) if !on_mode_row && !key.modifiers.contains(KeyModifiers::CONTROL) => form.fields[form.focus - 1].push(ch),
        _ => {}
    }
    Action::Stay
}

fn shown(form: &Form, index: usize) -> String {
    let env = match index {
        0 if form.from_env.0 => t("tui.settings.from_env", &[]),
        KEY_FIELD if form.from_env.1 => t("tui.settings.from_env", &[]),
        _ => String::new(),
    };
    if index == KEY_FIELD {
        format!("{}{env}", relay_settings::key_label(&form.fields[index]))
    } else {
        format!("{}{env}", form.fields[index])
    }
}

fn mode_row(mode: Mode) -> String {
    let local = t("mode.local", &[]);
    let supabase = t("mode.supabase", &[]);
    let label = t("settings.mode_row", &[]);
    match mode {
        Mode::Local => format!("{label}: [{local}]  {supabase}"),
        Mode::Supabase => format!("{label}:  {local}  [{supabase}]"),
    }
}

pub fn render(out: &mut Stdout, form: &Form, width: usize, rows: usize) -> io::Result<()> {
    let hint = match form.mode {
        Mode::Local => t("mode.hint_local", &[]),
        Mode::Supabase => t("mode.hint_supabase", &[]),
    };
    let marker = |row: usize| if form.focus == row { ">" } else { " " };
    let mut lines: Vec<(String, bool)> = vec![
        (t("tui.settings.help", &[]), false),
        (String::new(), false),
        (format!("{} {}", marker(0), mode_row(form.mode)), false),
        (format!("  {hint}"), false),
        (String::new(), false),
    ];
    // In Local mode the fields are kept but not used: they are drawn dimmed.
    let dimmed = form.mode == Mode::Local;
    for (index, label) in labels().iter().enumerate() {
        lines.push((format!("{} {label}: {}", marker(index + 1), shown(form, index)), dimmed));
    }
    lines.push((String::new(), false));
    if !form.short_id.is_empty() {
        lines.push((t("tui.settings.my_id", &[("id", &form.short_id)]), false));
    }
    match &form.mine {
        Some((ed, noise)) => {
            lines.push((t("tui.settings.my_ed", &[("key", ed)]), false));
            lines.push((t("tui.settings.my_noise", &[("key", noise)]), false));
        }
        None => lines.push((t("tui.settings.no_keys_yet", &[]), false)),
    }
    lines.push((String::new(), false));
    lines.push((form.message.clone(), false));
    queue!(out, cursor::Hide, Clear(ClearType::All))?;
    for (row, (text, dim)) in lines.iter().take(rows).enumerate() {
        let text: String = text.chars().take(width).collect();
        queue!(out, cursor::MoveTo(0, row as u16))?;
        if row == 0 {
            queue!(out, SetAttribute(Attribute::Bold), SetBackgroundColor(Color::DarkGrey), SetForegroundColor(Color::White), Print(text), ResetColor, SetAttribute(Attribute::Reset))?;
        } else if *dim {
            queue!(out, SetForegroundColor(Color::DarkGrey), Print(text), ResetColor)?;
        } else {
            queue!(out, Print(text))?;
        }
    }
    queue!(out, cursor::Show)
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::Me;

    fn chat() -> Arc<Chat> {
        Arc::new(Chat::new(Me { id: "me".into(), name: "Me".into(), port: 1, key: vec![], secret: vec![] }))
    }

    fn form_at(state: &Path) -> Form {
        Form {
            mode: Mode::Local,
            fields: Default::default(),
            focus: 0,
            message: String::new(),
            from_env: (false, false),
            mine: None,
            short_id: String::new(),
            state: state.to_path_buf(),
            saved: false,
        }
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("herdr-talk-form-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn press(form: &mut Form, chat: &Arc<Chat>, code: KeyCode) -> Action {
        on_key(form, KeyEvent::new(code, KeyModifiers::NONE), chat)
    }

    #[test]
    fn left_and_right_switch_the_mode_on_the_mode_row_only() {
        let (chat, mut form) = (chat(), form_at(&folder("switch")));
        press(&mut form, &chat, KeyCode::Right);
        assert_eq!(form.mode, Mode::Supabase);
        press(&mut form, &chat, KeyCode::Left);
        assert_eq!(form.mode, Mode::Local);
        press(&mut form, &chat, KeyCode::Tab);
        press(&mut form, &chat, KeyCode::Right);
        assert_eq!(form.mode, Mode::Local, "on a field row Right does not switch the mode");
    }

    #[test]
    fn typing_goes_to_the_focused_field_and_enter_moves_on() {
        let (chat, mut form) = (chat(), form_at(&folder("typing")));
        press(&mut form, &chat, KeyCode::Tab);
        for ch in "https://x.example".chars() {
            press(&mut form, &chat, KeyCode::Char(ch));
        }
        press(&mut form, &chat, KeyCode::Enter);
        press(&mut form, &chat, KeyCode::Char('k'));
        assert_eq!(form.fields[0], "https://x.example");
        assert_eq!(form.fields[1], "k");
        assert_eq!(form.focus, 2);
    }

    #[test]
    fn the_key_field_shows_its_length_only() {
        let mut form = form_at(&folder("mask"));
        form.fields[KEY_FIELD] = "dummy-test-key".into();
        assert_eq!(shown(&form, KEY_FIELD), "(14 chars)");
    }

    #[test]
    fn environment_values_are_marked() {
        let mut form = form_at(&folder("env"));
        form.from_env = (true, false);
        assert!(shown(&form, 0).ends_with(&t("tui.settings.from_env", &[])));
        assert!(!shown(&form, KEY_FIELD).contains(&t("tui.settings.from_env", &[])));
    }

    #[test]
    fn local_save_keeps_the_fields_and_saves_the_mode() {
        let dir = folder("local");
        let (chat, mut form) = (chat(), form_at(&dir));
        form.fields[2] = "Alice".into();
        form.save(&chat);
        assert_eq!(form.message, t("settings.save_ok_local", &[]), "{}", form.message);
        let saved = relay_settings::load_saved(&dir);
        assert_eq!(saved.mode, Mode::Local);
        assert_eq!(saved.name, "Alice");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn supabase_save_is_refused_when_a_field_is_missing() {
        let dir = folder("refused");
        let (chat, mut form) = (chat(), form_at(&dir));
        form.mode = Mode::Supabase;
        form.fields[0] = "https://x.example".into();
        form.save(&chat);
        assert!(form.message.contains(&t("tui.settings.label.name", &[])) && form.message.contains(&t("tui.settings.label.key", &[])), "{}", form.message);
        assert!(!dir.join(relay_settings::FILE).exists());
    }

    #[test]
    fn escape_leaves_without_saving() {
        let dir = folder("escape");
        let (chat, mut form) = (chat(), form_at(&dir));
        press(&mut form, &chat, KeyCode::Right);
        assert!(matches!(press(&mut form, &chat, KeyCode::Esc), Action::Leave));
        assert!(!dir.join(relay_settings::FILE).exists());
    }

    #[test]
    fn a_bad_peer_key_is_refused_with_a_message() {
        let dir = folder("badpeer");
        let (chat, mut form) = (chat(), form_at(&dir));
        form.fields[3] = "not a key".into();
        form.focus = 4;
        press(&mut form, &chat, KeyCode::Enter);
        assert!(form.message.starts_with(&t("tui.settings.not_saved", &[("error", "")])), "{}", form.message);
    }

    #[test]
    fn a_complete_supabase_setting_is_saved_and_the_first_run_goes_on() {
        let dir = folder("first-run-supabase");
        let chat = chat();
        let mut form = Form::open_supabase(&dir);
        form.fields = ["http://127.0.0.1:9".into(), "dummy-test-key".into(), "Alice".into(), "abcd2345".into()];
        press(&mut form, &chat, KeyCode::Enter);
        assert!(!form.is_saved(), "Enter on a field row only moves on");
        form.focus = 4;
        press(&mut form, &chat, KeyCode::Enter);
        assert!(form.is_saved(), "{}", form.message);
        assert_eq!(relay_settings::load_saved(&dir).mode, Mode::Supabase);
        chat.stop_relay();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
