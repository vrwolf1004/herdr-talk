//! The relay settings screen (Ctrl+R). Four fields, Tab or Up/Down to move, Enter to go on or save, Esc to leave.
//! The key is never drawn: only its length. The environment variables win over the saved values, and the screen says so.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{Clear, ClearType};
use crossterm::{cursor, queue};
use std::io::{self, Stdout};
use talk_core::relay_settings::{self, Settings};
use talk_core::{relay_chat, Chat};

const LABELS: [&str; 4] = ["Project URL", "Publishable key", "Display name", "Peer key (Ed25519)"];
const KEY_FIELD: usize = 1;

pub struct Form {
    fields: [String; 4],
    focus: usize,
    message: String,
    from_env: (bool, bool),
    mine: Option<(String, String)>,
    state: std::path::PathBuf,
}

pub enum Action {
    Stay,
    Leave,
}

impl Form {
    pub fn open(state: &std::path::Path) -> Form {
        let saved = relay_settings::load_saved(state);
        let mine = relay_chat::my_keys(state).ok();
        let message = if mine.is_none() { "start a chat once first: this PC has no keys yet".to_string() } else { String::new() };
        Form {
            fields: [saved.url, saved.key, saved.name, saved.peer],
            focus: 0,
            message,
            from_env: relay_settings::from_environment(),
            mine,
            state: state.to_path_buf(),
        }
    }

    fn settings(&self) -> Settings {
        Settings {
            url: self.fields[0].trim().to_string(),
            key: self.fields[1].trim().to_string(),
            name: self.fields[2].trim().to_string(),
            peer: self.fields[3].trim().to_string(),
        }
    }

    /// Save, then start the relay with the new settings. The message says what happened, never a key value.
    fn save(&mut self, chat: &std::sync::Arc<Chat>) {
        match relay_settings::save(&self.state, &self.settings()) {
            Err(error) => self.message = format!("not saved: {error}"),
            Ok(()) => match relay_chat::start(chat, &self.state) {
                Ok(true) => self.message = "saved; relay started".to_string(),
                Ok(false) => self.message = "saved; relay is off".to_string(),
                Err(error) => self.message = format!("saved, but the relay did not start: {error}"),
            },
        }
    }
}

/// One key for the form. Enter on the last field saves; Esc leaves without saving.
pub fn on_key(form: &mut Form, key: KeyEvent, chat: &std::sync::Arc<Chat>) -> Action {
    match key.code {
        KeyCode::Esc => return Action::Leave,
        KeyCode::Tab | KeyCode::Down => form.focus = (form.focus + 1) % LABELS.len(),
        KeyCode::Up => form.focus = (form.focus + LABELS.len() - 1) % LABELS.len(),
        KeyCode::Enter if form.focus + 1 < LABELS.len() => form.focus += 1,
        KeyCode::Enter => form.save(chat),
        KeyCode::Backspace => {
            form.fields[form.focus].pop();
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => form.fields[form.focus].push(ch),
        _ => {}
    }
    Action::Stay
}

fn shown(form: &Form, index: usize) -> String {
    let env = match index {
        0 if form.from_env.0 => " (environment)",
        KEY_FIELD if form.from_env.1 => " (environment)",
        _ => "",
    };
    if index == KEY_FIELD {
        format!("{}{env}", relay_settings::key_label(&form.fields[index]))
    } else {
        format!("{}{env}", form.fields[index])
    }
}

pub fn render(out: &mut Stdout, form: &Form, width: usize, rows: usize) -> io::Result<()> {
    let mut lines: Vec<String> = vec!["relay settings  (Tab/Up/Down move · Enter next or save · Esc leave)".to_string(), String::new()];
    for (index, label) in LABELS.iter().enumerate() {
        let marker = if index == form.focus { ">" } else { " " };
        lines.push(format!("{marker} {label}: {}", shown(form, index)));
    }
    lines.push(String::new());
    match &form.mine {
        Some((ed, noise)) => {
            lines.push(format!("this PC's Ed25519 key (give it to the peer): {ed}"));
            lines.push(format!("this PC's Noise key (verify it with /trust): {noise}"));
        }
        None => lines.push("this PC's keys: not yet (start a chat once)".to_string()),
    }
    lines.push(String::new());
    lines.push(form.message.clone());
    queue!(out, cursor::Hide, Clear(ClearType::All))?;
    for (row, text) in lines.iter().take(rows).enumerate() {
        let text: String = text.chars().take(width).collect();
        queue!(out, cursor::MoveTo(0, row as u16))?;
        if row == 0 {
            queue!(out, SetAttribute(Attribute::Bold), SetBackgroundColor(Color::DarkGrey), SetForegroundColor(Color::White), Print(text), ResetColor, SetAttribute(Attribute::Reset))?;
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

    fn chat() -> std::sync::Arc<Chat> {
        std::sync::Arc::new(Chat::new(Me { id: "me".into(), name: "Me".into(), port: 1, key: vec![], secret: vec![] }))
    }

    fn form() -> Form {
        Form {
            fields: [String::new(), String::new(), String::new(), String::new()],
            focus: 0,
            message: String::new(),
            from_env: (false, false),
            mine: None,
            state: std::env::temp_dir().join(format!("herdr-talk-form-test-{}", std::process::id())),
        }
    }

    fn press(form: &mut Form, chat: &std::sync::Arc<Chat>, code: KeyCode) -> Action {
        on_key(form, KeyEvent::new(code, KeyModifiers::NONE), chat)
    }

    #[test]
    fn typing_goes_to_the_focused_field_and_enter_moves_on() {
        let (chat, mut form) = (chat(), form());
        for ch in "https://x.example".chars() {
            press(&mut form, &chat, KeyCode::Char(ch));
        }
        press(&mut form, &chat, KeyCode::Enter);
        press(&mut form, &chat, KeyCode::Char('k'));
        assert_eq!(form.fields[0], "https://x.example");
        assert_eq!(form.fields[1], "k");
        assert_eq!(form.focus, 1);
    }

    #[test]
    fn the_key_field_shows_its_length_only() {
        let mut form = form();
        form.fields[KEY_FIELD] = "dummy-test-key".into();
        let shown_text = shown(&form, KEY_FIELD);
        assert_eq!(shown_text, "(14 chars)");
        assert!(!shown_text.contains("dummy"));
    }

    #[test]
    fn environment_values_are_marked() {
        let mut form = form();
        form.from_env = (true, false);
        assert!(shown(&form, 0).ends_with("(environment)"));
        assert!(!shown(&form, KEY_FIELD).contains("environment"));
    }

    #[test]
    fn escape_leaves_without_saving() {
        let (chat, mut form) = (chat(), form());
        press(&mut form, &chat, KeyCode::Char('a'));
        assert!(matches!(press(&mut form, &chat, KeyCode::Esc), Action::Leave));
        assert!(!form.state.join(relay_settings::FILE).exists());
    }

    #[test]
    fn a_bad_peer_key_is_refused_with_a_message() {
        let (chat, mut form) = (chat(), form());
        form.fields[3] = "not a key".into();
        form.focus = 3;
        press(&mut form, &chat, KeyCode::Enter);
        assert!(form.message.starts_with("not saved"), "{}", form.message);
    }
}
