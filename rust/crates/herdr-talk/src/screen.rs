//! Full-screen chat: a status row, messages, a divider, and the input line at the bottom.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute, queue};
use std::io::{self, Stdout, Write};
use std::time::Duration;
use talk_core::text::{char_width, wrap_line};
use talk_core::{t, Chat, IDLE_LIMIT};

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

fn render(out: &mut Stdout, chat: &Chat, input: &str) -> io::Result<()> {
    let (cols, rows) = terminal::size()?;
    let (cols, rows) = (cols.max(20), rows.max(5));
    let width = usize::from(cols);
    let area = rows - 3;
    let mut wrapped = Vec::new();
    for line in chat.lines() {
        wrapped.extend(wrap_line(&line.render(), width - 1).into_iter().map(|row| (row, line.mine)));
    }
    let visible = &wrapped[wrapped.len().saturating_sub(usize::from(area))..];

    let name = chat.me.lock().unwrap().name.clone();
    let peers = chat.fresh_peers().len().to_string();
    let status = fit(&t("status.line", &[("name", &name), ("peers", &peers)]), width);
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
    let shown = tail(input, width - 3);
    queue!(out, cursor::MoveTo(0, rows - 2), SetForegroundColor(Color::DarkGrey), Print("-".repeat(width)), ResetColor,
        cursor::MoveTo(0, rows - 1), Clear(ClearType::CurrentLine), Print("> "), Print(&shown), cursor::Show)?;
    out.flush()
}

/// Apply one key. Return false to leave the chat.
fn on_key(key: KeyEvent, chat: &Chat, input: &mut String) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('c' | 'd') if ctrl => return false,
        KeyCode::Char('l') if ctrl => chat.clear_lines(),
        KeyCode::Char('n') if ctrl => *input = "/name ".to_string(),
        KeyCode::Char(ch) if !ctrl => input.push(ch),
        KeyCode::Backspace => {
            input.pop();
        }
        KeyCode::Enter => return chat.handle(&std::mem::take(input)),
        _ => {}
    }
    true
}

pub fn run(chat: &Chat) -> io::Result<()> {
    let mut terminal = Terminal::enter()?;
    let mut input = String::new();
    let mut last_status = String::new();
    render(&mut terminal.0, chat, &input)?;
    loop {
        let mut redraw = chat.take_dirty();
        if event::poll(POLL)? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if !on_key(key, chat, &mut input) {
                        break;
                    }
                    redraw = true;
                }
                Event::Resize(..) => redraw = true,
                _ => {}
            }
        }
        // Peers come and go without a chat line, and the status row shows their count.
        let status = format!("{}/{}", chat.me.lock().unwrap().name, chat.fresh_peers().len());
        if redraw || status != last_status {
            last_status = status;
            render(&mut terminal.0, chat, &input)?;
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
    fn fit_and_tail_keep_to_the_width() {
        assert_eq!(fit("가나다", 5), "가나");
        assert_eq!(tail("가나다", 5), "나다");
    }
}
