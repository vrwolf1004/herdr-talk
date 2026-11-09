//! The first-run choice of the connection mode, shown by `run` before the chat when relay.json has no `mode` yet.
//! Two choices: 1 = local (same network), 2 = Supabase (register and connect). Digits, Up/Down or Tab and Enter pick;
//! Esc quits. Local is saved at once. Supabase opens the settings screen, and only a saved, complete setting starts the chat.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{Clear, ClearType};
use crossterm::{cursor, queue};
use std::io::{self, Stdout};
use std::path::Path;
use std::time::{Duration, Instant};
use talk_core::relay_settings::{self, Mode};
use talk_core::t;

/// Without a choice within this time the chat starts in Local mode, and nothing is saved.
pub const TIMEOUT: Duration = Duration::from_secs(30);

pub struct Chooser {
    pub selected: Mode,
    /// When the default starts. None once a key was pressed: then the user chooses.
    deadline: Option<Instant>,
}

impl Chooser {
    pub fn new(now: Instant) -> Chooser {
        Chooser { selected: Mode::Local, deadline: Some(now + TIMEOUT) }
    }
}

/// True when the time is up and no key was pressed.
pub fn timed_out(chooser: &Chooser, now: Instant) -> bool {
    chooser.deadline.is_some_and(|deadline| now >= deadline)
}

/// Whole seconds left (rounded up) while the timer runs; None once it has stopped.
pub fn remaining(chooser: &Chooser, now: Instant) -> Option<u64> {
    chooser.deadline.map(|deadline| deadline.saturating_duration_since(now).as_secs_f64().ceil() as u64)
}

pub enum Step {
    Stay,
    Pick(Mode),
    Quit,
}

pub fn on_key(chooser: &mut Chooser, key: KeyEvent) -> Step {
    // Any key stops the timer: the user is choosing now.
    chooser.deadline = None;
    match key.code {
        KeyCode::Esc => Step::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Step::Quit,
        KeyCode::Char('1') => Step::Pick(Mode::Local),
        KeyCode::Char('2') => Step::Pick(Mode::Supabase),
        KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
            chooser.selected = match chooser.selected {
                Mode::Local => Mode::Supabase,
                Mode::Supabase => Mode::Local,
            };
            Step::Stay
        }
        KeyCode::Enter => Step::Pick(chooser.selected),
        _ => Step::Stay,
    }
}

/// The Local choice: write `mode` = local and keep every other saved value.
pub fn save_local(state: &Path) -> Result<(), String> {
    let mut settings = relay_settings::load_saved(state);
    settings.mode = Mode::Local;
    relay_settings::save(state, &settings)
}

pub fn render(out: &mut Stdout, chooser: &Chooser, remaining: Option<u64>, width: usize, rows: usize) -> io::Result<()> {
    let option = |mode: Mode, number: u8, name: String, hint: String| {
        let marker = if chooser.selected == mode { ">" } else { " " };
        vec![format!("{marker} {number}. {name}"), format!("      {hint}")]
    };
    let mut lines = vec![t("mode.first_run_title", &[]), String::new()];
    lines.extend(option(Mode::Local, 1, t("mode.local", &[]), t("mode.hint_local", &[])));
    lines.push(String::new());
    lines.extend(option(Mode::Supabase, 2, t("mode.supabase", &[]), t("mode.hint_supabase", &[])));
    lines.push(String::new());
    if let Some(seconds) = remaining {
        lines.push(t("mode.timeout_hint", &[("n", &seconds.to_string())]));
    }
    lines.push(t("tui.first_run.keys", &[]));
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
    use crossterm::event::KeyEvent;

    fn press(chooser: &mut Chooser, code: KeyCode) -> Step {
        on_key(chooser, KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn digits_pick_and_enter_picks_the_highlighted_one() {
        let mut chooser = Chooser::new(Instant::now());
        assert!(matches!(press(&mut chooser, KeyCode::Char('1')), Step::Pick(Mode::Local)));
        assert!(matches!(press(&mut chooser, KeyCode::Char('2')), Step::Pick(Mode::Supabase)));
        assert!(matches!(press(&mut chooser, KeyCode::Enter), Step::Pick(Mode::Local)), "Local is highlighted first");
        press(&mut chooser, KeyCode::Down);
        assert!(matches!(press(&mut chooser, KeyCode::Enter), Step::Pick(Mode::Supabase)));
    }

    #[test]
    fn escape_quits_and_other_keys_do_nothing() {
        let mut chooser = Chooser::new(Instant::now());
        assert!(matches!(press(&mut chooser, KeyCode::Esc), Step::Quit));
        assert!(matches!(press(&mut chooser, KeyCode::Char('x')), Step::Stay));
    }

    #[test]
    fn the_local_choice_keeps_the_other_saved_values() {
        let dir = std::env::temp_dir().join(format!("herdr-talk-first-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(relay_settings::FILE), r#"{"url":"https://example.supabase.co","name":"Alice"}"#).unwrap();
        save_local(&dir).unwrap();
        let saved = relay_settings::load_saved(&dir);
        assert_eq!(saved.mode, Mode::Local);
        assert_eq!(saved.name, "Alice");
        assert!(!relay_settings::needs_first_run(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_timer_starts_the_default_after_thirty_seconds() {
        let start = Instant::now();
        let chooser = Chooser::new(start);
        assert!(!timed_out(&chooser, start + Duration::from_secs(29)));
        assert!(timed_out(&chooser, start + TIMEOUT));
        assert_eq!(remaining(&chooser, start + Duration::from_secs(29)), Some(1));
    }

    #[test]
    fn a_key_stops_the_timer_and_the_choice_waits() {
        let start = Instant::now();
        let mut chooser = Chooser::new(start);
        press(&mut chooser, KeyCode::Down);
        assert!(!timed_out(&chooser, start + TIMEOUT * 10), "no automatic start after a key");
        assert_eq!(remaining(&chooser, start), None);
    }
}
