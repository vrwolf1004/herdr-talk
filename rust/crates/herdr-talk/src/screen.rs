//! Full-screen chat: a status row, messages, a divider, and the input line at the bottom.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute, queue};
use std::io::{self, Stdout, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};
use talk_core::text::{char_width, wrap_line};
use crate::first_run::{self, Chooser, Step};
use crate::settings_form::{self, Form};
use talk_core::relay_settings::Mode;
use talk_core::{t, Chat, Line, IDLE_LIMIT};

const POLL: Duration = Duration::from_millis(100);

/// Puts the terminal back on every way out, panics included.
struct Terminal(Stdout);

impl Terminal {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut terminal = Terminal(io::stdout());
        execute!(terminal.0, EnterAlternateScreen)?;
        Ok(terminal)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = execute!(self.0, LeaveAlternateScreen, cursor::Show);
        let _ = terminal::disable_raw_mode();
    }
}

fn fits(chars: impl Iterator<Item = char>, cols: usize) -> Vec<char> {
    let mut width = 0;
    chars.take_while(|ch| {
        width += char_width(ch.encode_utf8(&mut [0; 4]));
        width <= cols
    }).collect()
}

fn fit(text: &str, cols: usize) -> String {
    fits(text.chars(), cols).into_iter().collect()
}

/// The end of the typed text that fits, so the cursor stays visible.
fn tail(text: &str, cols: usize) -> String {
    fits(text.chars().rev(), cols).into_iter().rev().collect()
}

/// Screen rows of one chat line. A received message repeats its sender on every wrapped row,
/// so a continuation row cannot pass for another sender. A sender too long to leave room for text is not repeated.
fn rows_of(line: &Line, cols: usize) -> Vec<String> {
    let Some(sender) = line.sender.as_ref().filter(|_| !line.mine) else {
        return wrap_line(&line.render(), cols);
    };
    let prefix = format!("{sender}: ");
    let room = cols.saturating_sub(char_width(&prefix));
    if room < 8 {
        return wrap_line(&line.render(), cols);
    }
    wrap_line(&line.text, room).into_iter().map(|row| format!("{prefix}{row}")).collect()
}

fn render(out: &mut Stdout, chat: &Chat, input: &str) -> io::Result<()> {
    let (cols, rows) = terminal::size()?;
    let (cols, rows) = (cols.max(20), rows.max(5));
    let width = usize::from(cols);
    let area = rows - 3;
    let mut wrapped = Vec::new();
    for line in chat.lines() {
        wrapped.extend(rows_of(&line, width - 1).into_iter().map(|row| (row, line.mine)));
    }
    let visible = &wrapped[wrapped.len().saturating_sub(usize::from(area))..];

    let name = chat.me.lock().unwrap().name.clone();
    let peers = chat.fresh_peers().len().to_string();
    let badge = if chat.is_host() { format!("{} ", t("host.badge", &[])) } else { String::new() };
    let mode = t(if chat.is_supabase() { "mode.supabase" } else { "mode.local" }, &[]);
    let relay = chat.relay_status();
    let relay = if relay.is_empty() { String::new() } else { format!(" · {relay}") };
    let relay = format!(" · {}{relay}", t("mode.status", &[("mode", &mode)]));
    let status = fit(&format!("{badge}{}{relay}", t("status.line", &[("name", &name), ("peers", &peers)])), width);
    let padding = " ".repeat(width - char_width(&status));
    queue!(out, cursor::Hide, cursor::MoveTo(0, 0), SetBackgroundColor(Color::DarkGrey), SetForegroundColor(Color::White),
        Print(status), Print(padding), ResetColor)?;
    for row in 0..area {
        queue!(out, cursor::MoveTo(0, row + 1), Clear(ClearType::CurrentLine))?;
        if let Some((text, mine)) = visible.get(usize::from(row)) {
            if *mine {
                queue!(out, SetAttribute(Attribute::Bold), SetForegroundColor(Color::Blue), Print(text),
                    SetAttribute(Attribute::Reset), ResetColor)?;
            } else {
                queue!(out, Print(text))?;
            }
        }
    }
    // The phone address and token, on this screen only, over the bottom rows of the messages. They are not chat lines.
    if let Some(overlay) = chat.phone_overlay() {
        let count = overlay.len().min(usize::from(area));
        for (index, text) in overlay.iter().take(count).enumerate() {
            let text = fit(text, width);
            queue!(out, cursor::MoveTo(0, area + 1 - count as u16 + index as u16), SetBackgroundColor(Color::DarkGreen), SetForegroundColor(Color::White),
                Print(&text), Print(" ".repeat(width - char_width(&text))), ResetColor)?;
        }
    }
    let shown = tail(input, width - 3);
    queue!(out, cursor::MoveTo(0, rows - 2), SetForegroundColor(Color::DarkGrey), Print("-".repeat(width)), ResetColor,
        cursor::MoveTo(0, rows - 1), Clear(ClearType::CurrentLine), Print("> "), Print(&shown), cursor::Show)?;
    out.flush()
}

/// Apply one key. Return false to leave the chat.
fn on_key(key: KeyEvent, chat: &Chat, input: &mut String) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // The phone address hides on the first key that is not its own toggle.
    if !(ctrl && key.code == KeyCode::Char('p')) {
        chat.hide_phone();
    }
    match key.code {
        KeyCode::Char('c' | 'd') if ctrl => return false,
        KeyCode::Char('l') if ctrl => chat.clear_lines(),
        KeyCode::Char('n') if ctrl => *input = "/name ".to_string(),
        KeyCode::Char('p') if ctrl => chat.toggle_phone(),
        KeyCode::Char(ch) if !ctrl => input.push(ch),
        KeyCode::Backspace => {
            input.pop();
        }
        KeyCode::Enter => return chat.handle(&std::mem::take(input)),
        _ => {}
    }
    true
}

/// How the first-run choice ended.
pub enum Choice {
    /// The user picked a mode, and it was saved (Local or a complete Supabase setting).
    Saved,
    /// No choice within the time limit: the chat starts in Local mode, and nothing is saved.
    TimedOut,
    /// The user pressed Esc on the choice.
    Quit,
}

/// First run: ask for the connection mode before the chat. Supabase goes to the settings screen; Esc there comes
/// back to the choice (the timer starts again), saving nothing.
pub fn choose_mode(chat: &Arc<Chat>) -> io::Result<Choice> {
    let state = talk_core::store::state_dir();
    let mut terminal = Terminal::enter()?;
    let mut chooser = Chooser::new(Instant::now());
    let mut shown: Option<Option<u64>> = None;
    loop {
        let now = Instant::now();
        if first_run::timed_out(&chooser, now) {
            return Ok(Choice::TimedOut);
        }
        let remaining = first_run::remaining(&chooser, now);
        if shown != Some(remaining) {
            let (cols, rows) = terminal::size()?;
            first_run::render(&mut terminal.0, &chooser, remaining, usize::from(cols), usize::from(rows))?;
            shown = Some(remaining);
        }
        if !event::poll(POLL)? {
            continue;
        }
        let Event::Key(key) = event::read()? else { continue };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        shown = None;
        match first_run::on_key(&mut chooser, key) {
            Step::Stay => {}
            Step::Quit => return Ok(Choice::Quit),
            Step::Pick(Mode::Local) => {
                first_run::save_local(&state).map_err(io::Error::other)?;
                return Ok(Choice::Saved);
            }
            Step::Pick(Mode::Supabase) => {
                let mut form = Form::open_supabase(&state);
                loop {
                    form.poll(); // a create-room answer that has come back
                    let (cols, rows) = terminal::size()?;
                    settings_form::render(&mut terminal.0, &form, usize::from(cols), usize::from(rows))?;
                    if !event::poll(POLL)? {
                        continue;
                    }
                    let Event::Key(key) = event::read()? else { continue };
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    if let settings_form::Action::Leave = settings_form::on_key(&mut form, key, chat) {
                        break;
                    }
                    if form.is_saved() {
                        return Ok(Choice::Saved);
                    }
                }
                // Esc in the settings: back to the choice, with a fresh countdown.
                chooser = Chooser::new(Instant::now());
            }
        }
    }
}

pub fn run(chat: &Arc<Chat>) -> io::Result<()> {
    let mut terminal = Terminal::enter()?;
    let mut input = String::new();
    let mut last_status = String::new();
    let mut form: Option<Form> = None;
    render(&mut terminal.0, chat, &input)?;
    loop {
        chat.phone_visible(); // hides the phone address when its time is up, and asks for a redraw
        let mut redraw = chat.take_dirty();
        if event::poll(POLL)? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if let Some(open) = form.as_mut() {
                        let action = settings_form::on_key(open, key, chat);
                        if open.is_saved() {
                            form = None;
                        } else if let settings_form::Action::Leave = action {
                            form = None;
                            chat.say(t("settings.cancel_discard", &[]));
                        }
                    } else if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('r') {
                        form = Some(Form::open(&talk_core::store::state_dir()));
                    } else if !on_key(key, chat, &mut input) {
                        break;
                    }
                    redraw = true;
                }
                Event::Resize(..) => redraw = true,
                _ => {}
            }
        }
        if let Some(open) = form.as_mut() {
            redraw |= open.poll(); // a create-room answer that has come back
        }
        // Peers come and go without a chat line, and the status row shows their count.
        let status = format!("{}/{}/{}/{}", chat.me.lock().unwrap().name, chat.fresh_peers().len(), chat.is_host(), chat.relay_status());
        if redraw || status != last_status {
            last_status = status;
            match &form {
                Some(open) => {
                    let (cols, rows) = terminal::size()?;
                    settings_form::render(&mut terminal.0, open, usize::from(cols), usize::from(rows))?;
                }
                None => render(&mut terminal.0, chat, &input)?,
            }
        }
        if chat.idle_for() >= IDLE_LIMIT {
            drop(terminal);
            println!("{}", t("idle.left", &[("minutes", &(IDLE_LIMIT.as_secs() / 60).to_string())]));
            return Ok(());
        }
    }
    drop(terminal);
    println!("{}", t("bye", &[]));
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn wrapped_rows_of_a_received_message_all_carry_the_sender() {
        let line = Line { sender: Some("Bob (plain)".into()), text: "aaaa bbbb Alice: cccc dddd".into(), mine: false };
        let rows = rows_of(&line, 24);
        assert!(rows.len() > 1);
        assert!(rows.iter().all(|row| row.starts_with("Bob (plain): ") && char_width(row) <= 24));
    }

    use super::*;
    use talk_core::Me;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn chat() -> Chat {
        Chat::new(Me { id: "me".into(), name: "Me".into(), port: 1, key: vec![], secret: vec![] })
    }

    #[test]
    fn typing_and_control_keys() {
        let (chat, mut input) = (chat(), String::new());
        on_key(key(KeyCode::Char('가'), KeyModifiers::NONE), &chat, &mut input);
        on_key(key(KeyCode::Char('b'), KeyModifiers::NONE), &chat, &mut input);
        on_key(key(KeyCode::Backspace, KeyModifiers::NONE), &chat, &mut input);
        assert_eq!(input, "가");
        on_key(key(KeyCode::Char('n'), KeyModifiers::CONTROL), &chat, &mut input);
        assert_eq!(input, "/name ");
        assert!(!on_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL), &chat, &mut input));
        assert!(!on_key(key(KeyCode::Char('d'), KeyModifiers::CONTROL), &chat, &mut input));
    }

    #[test]
    fn commands_are_echoed_and_ctrl_l_clears() {
        let (chat, mut input) = (chat(), "/name Bob".to_string());
        assert!(on_key(key(KeyCode::Enter, KeyModifiers::NONE), &chat, &mut input));
        assert_eq!(chat.lines()[0].text, "> /name Bob");
        on_key(key(KeyCode::Char('l'), KeyModifiers::CONTROL), &chat, &mut input);
        assert!(chat.lines().is_empty());
    }

    #[test]
    fn ctrl_p_toggles_the_phone_and_any_other_key_hides_it() {
        let (chat, mut input) = (chat(), String::new());
        on_key(key(KeyCode::Char('p'), KeyModifiers::CONTROL), &chat, &mut input);
        assert!(chat.phone_visible());
        assert!(input.is_empty(), "Ctrl+P types nothing");
        on_key(key(KeyCode::Char('p'), KeyModifiers::CONTROL), &chat, &mut input);
        assert!(!chat.phone_visible());
        on_key(key(KeyCode::Char('p'), KeyModifiers::CONTROL), &chat, &mut input);
        on_key(key(KeyCode::Char('a'), KeyModifiers::NONE), &chat, &mut input);
        assert!(!chat.phone_visible());
        assert_eq!(input, "a");
    }

    #[test]
    fn fit_and_tail_keep_to_the_width() {
        assert_eq!(fit("가나다", 5), "가나");
        assert_eq!(tail("가나다", 5), "나다");
    }
}
