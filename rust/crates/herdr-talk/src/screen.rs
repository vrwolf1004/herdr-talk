//! Full-screen chat in the classic terminal layout: header, divider, title, messages, divider, keyboard guide and the
//! status line. The first-run and settings screens use the same frame (`classic`). Rows are drawn one by one, and a frame
//! that is the same as the last one is not drawn again.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::Print;
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute, queue};
use std::cell::Cell;
use std::collections::HashMap;
use std::io::{self, Stdout, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};
use talk_core::text::{char_width, wrap_line};
use crate::first_run::{self, Chooser, Step};
use crate::settings_form::{self, Form};
use talk_core::relay_settings::Mode;
use talk_core::{t, Chat, Line, Link, IDLE_LIMIT};

const POLL: Duration = Duration::from_millis(100);

/// A colour as red, green and blue.
type Rgb = (u8, u8, u8);

// The classic terminal palette (design spec): background, primary green, body text, muted text, warning, error.
pub(crate) const BACKGROUND: Rgb = (5, 10, 8);
pub(crate) const MAIN: Rgb = (70, 255, 130);
const TEXT: Rgb = (180, 220, 190);
// #526B59 was too dark on the background; lifted so hints and secondary text stay readable.
const DIM: Rgb = (125, 154, 132);
const WARN: Rgb = (255, 184, 77);
const ALERT: Rgb = (255, 85, 85);
const BORDER: Rgb = (65, 125, 85);

/// The ANSI sequence for a 24-bit foreground or background colour. Written out by hand: the colours must reach the console
/// as 38;2 and 48;2 codes, which the crossterm colour commands did not produce here.
fn fore((r, g, b): Rgb) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}

fn back((r, g, b): Rgb) -> String {
    format!("\x1b[48;2;{r};{g};{b}m")
}

/// What a piece of text is for: it sets the colour and weight.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Tone {
    /// Primary green, bold: titles, the selected item, names.
    Title,
    /// Ordinary body text.
    Text,
    /// Hints, the keyboard guide, system notes.
    Dim,
    /// A waiting state (status tag).
    Warn,
    /// An error state (status tag).
    Alert,
    /// The phone address over the messages: dark text on the border green.
    Overlay,
    /// The divider lines: the border green.
    Border,
}

/// Print text in a tone on the background colour.
pub(crate) fn print_tone(out: &mut Stdout, text: &str, tone: Tone) -> io::Result<()> {
    let (fore_color, back_color, bold) = match tone {
        Tone::Title => (MAIN, BACKGROUND, true),
        Tone::Text => (TEXT, BACKGROUND, false),
        Tone::Dim => (DIM, BACKGROUND, false),
        Tone::Warn => (WARN, BACKGROUND, true),
        Tone::Alert => (ALERT, BACKGROUND, true),
        Tone::Overlay => (BACKGROUND, BORDER, false),
        Tone::Border => (BORDER, BACKGROUND, false),
    };
    let bold = if bold { "\x1b[1m" } else { "" };
    queue!(out, Print(format!("{}{}{bold}{text}\x1b[0m", back(back_color), fore(fore_color))))
}

/// One screen row: pieces of text, each with its tone.
pub(crate) type Row = Vec<(String, Tone)>;

/// Print a row, cut to the width.
fn print_row(out: &mut Stdout, row: &Row, width: usize) -> io::Result<()> {
    let mut used = 0;
    for (text, tone) in row {
        let text = fit(text, width.saturating_sub(used));
        used += char_width(&text);
        print_tone(out, &text, *tone)?;
    }
    Ok(())
}

/// Clear one screen row to the background colour.
pub(crate) fn clear_row(out: &mut Stdout, row: u16) -> io::Result<()> {
    queue!(out, cursor::MoveTo(0, row), Print(back(BACKGROUND)), Clear(ClearType::CurrentLine))
}

/// A row of dashes across the width.
fn dash(width: usize) -> Row {
    vec![("-".repeat(width), Tone::Border)]
}

/// A row with a left part and a right part, cut to the width so they never overlap.
fn split(left: &str, left_tone: Tone, right: &str, right_tone: Tone, width: usize) -> Row {
    let left = fit(left, width);
    let right = fit(right, width.saturating_sub(char_width(&left)).saturating_sub(1));
    let gap = width.saturating_sub(char_width(&left) + char_width(&right));
    vec![(left, left_tone), (" ".repeat(gap), Tone::Text), (right, right_tone)]
}

/// The last frame drawn by `paint`: its size and rows. An equal frame is not drawn again.
#[derive(Default, PartialEq)]
pub(crate) struct Frame {
    size: (usize, usize),
    body: Vec<Row>,
}

/// Draw a whole screen of rows. Each row is cleared on its own, not the whole screen, and nothing is written when the frame
/// is the same as the last one.
pub(crate) fn paint(out: &mut Stdout, width: usize, rows: usize, body: Vec<Row>, drawn: &mut Frame) -> io::Result<()> {
    let frame = Frame { size: (width, rows), body };
    if *drawn == frame {
        return Ok(());
    }
    queue!(out, cursor::Hide)?;
    for row in 0..rows {
        clear_row(out, row as u16)?;
        if let Some(cells) = frame.body.get(row) {
            print_row(out, cells, width)?;
        }
    }
    // The cursor stays hidden here: a screen with an input row shows it at the input (render), the others have none.
    out.flush()?;
    *drawn = frame;
    Ok(())
}

/// The chat status line text: the host badge, peers, mode, the room part when a room is saved, then a relay error
/// (a success text is never added).
pub(crate) fn status_info(badge: &str, peers: &str, mode: &str, room: Option<String>, relay: &str) -> String {
    let mut info = format!("{badge}{}", t("status.line", &[("peers", peers)]).trim());
    info.push_str(&format!(" · {mode}"));
    if let Some(room) = room {
        info.push_str(&format!(" · {room}"));
    }
    if status_tag(relay).0 == "[ERROR]" {
        info.push_str(&format!(" · {relay}"));
    }
    info
}

/// The status tag for the relay state: [SYSTEM READY] without a relay, otherwise the state the relay text names.
// shortcut: the state is read from the relay's status text; a clear state value from talk-core would replace this guess.
pub(crate) fn status_tag(relay: &str) -> (&'static str, Tone) {
    if relay.is_empty() {
        ("[SYSTEM READY]", Tone::Title)
    } else if ["failed", "could not", "no identity", "incomplete", "invalid"].iter().any(|word| relay.contains(word)) {
        ("[ERROR]", Tone::Alert)
    } else if relay.contains("room joined") {
        ("[CONNECTED]", Tone::Title)
    } else {
        ("[CONNECTING...]", Tone::Warn)
    }
}

/// The app name at the top left, with its icon (as the Python edition shows it).
const APP_NAME: &str = "▣ HERDR-TALK";

/// The maker line: alone on the bottom row, at the right end.
fn footer(width: usize) -> Row {
    split("", Tone::Dim, "made by Rust", Tone::Dim, width)
}

/// A classic screen: header, divider, the body (its first row is the title), divider, keyboard guide, the status line and
/// the maker line alone on the bottom row.
pub(crate) fn classic(body: Vec<Row>, guide: &str, relay: &str, width: usize, rows: usize) -> Vec<Row> {
    let (tag, tone) = status_tag(relay);
    let mut frame = vec![split(APP_NAME, Tone::Title, "SYS.VER 1.0", Tone::Dim, width), dash(width)];
    let room = rows.saturating_sub(frame.len() + 4);
    frame.extend(body.into_iter().take(room));
    frame.resize_with(rows.saturating_sub(4), Vec::new);
    frame.push(dash(width));
    frame.push(vec![(guide.to_string(), Tone::Dim)]);
    frame.push(vec![(tag.to_string(), tone)]);
    frame.push(footer(width));
    frame
}

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

#[cfg(windows)]
fn clock_text() -> String {
    #[repr(C)]
    #[derive(Default)]
    struct LocalTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocalTime(time: *mut LocalTime);
    }
    let mut now = LocalTime::default();
    // SAFETY: GetLocalTime writes one SYSTEMTIME (eight u16 fields, the same layout as LocalTime) into the buffer it is given.
    unsafe { GetLocalTime(&mut now) };
    format!("{:02}:{:02}:{:02}", now.hour, now.minute, now.second)
}

#[cfg(not(windows))]
fn clock_text() -> String {
    // shortcut: UTC here, because this crate has no local-time library; use the local time once one is added.
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_secs()) % 86_400;
    format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

/// The time shown in front of each chat line: the time the line was first shown, kept for as long as the line is shown.
/// Keyed by the line's `seq`, so equal texts keep their own times.
#[derive(Default)]
pub(crate) struct Stamps(HashMap<u64, String>);

impl Stamps {
    fn times(&mut self, lines: &[Line]) -> Vec<String> {
        let now = clock_text();
        let times: Vec<String> = lines.iter().map(|line| self.0.get(&line.seq).cloned().unwrap_or_else(|| now.clone())).collect();
        // Only the lines still shown are kept: a line dropped from the top takes its time with it.
        self.0 = lines.iter().zip(&times).map(|(line, time)| (line.seq, time.clone())).collect();
        times
    }
}

/// The rows of one chat line: "[time] name > text", the text wrapped under the start of the text. A notice has no name.
fn message_rows(line: &Line, time: &str, cols: usize) -> Vec<Row> {
    let Some(sender) = line.sender.as_ref() else {
        return wrap_line(&line.render(), cols).into_iter().map(|row| vec![(row, Tone::Dim)]).collect();
    };
    let text_tone = if line.mine { Tone::Title } else { Tone::Text };
    let lead = format!("[{time}] ");
    let head = format!("{lead}{sender} > ");
    let room = cols.saturating_sub(char_width(&head));
    if room < 8 {
        return wrap_line(&line.render(), cols).into_iter().map(|row| vec![(row, text_tone)]).collect();
    }
    wrap_line(&line.text, room)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            if index == 0 {
                vec![(lead.clone(), Tone::Dim), (sender.clone(), Tone::Title), (" > ".to_string(), Tone::Dim), (row, text_tone)]
            } else {
                vec![(" ".repeat(char_width(&head)), Tone::Dim), (row, text_tone)]
            }
        })
        .collect()
}

/// The input row and the column of the cursor on it.
fn input_row(input: &str, width: usize) -> (Row, usize) {
    const PROMPT: &str = "MESSAGE > ";
    let room = width.saturating_sub(char_width(PROMPT) + 1);
    let shown = tail(input, room);
    let cursor = char_width(PROMPT) + char_width(&shown);
    let typed = if input.is_empty() { (t("chat.input_hint", &[]), Tone::Dim) } else { (shown, Tone::Title) };
    (vec![(PROMPT.to_string(), Tone::Title), typed], cursor)
}

/// The CONNECTED NODES block: a title row, then one row per node: you first (HOST/YOU on the host), then each fresh peer as
/// "[n] name (ip)  state" (the number only for a peer waiting for a code check). When the rows run out, the last row says how
/// many more there are. At least three rows of messages are kept.
fn nodes_block(chat: &Chat, name: &str, rows: usize) -> Vec<Row> {
    let you = if chat.is_host() { "HOST/YOU" } else { "YOU" };
    let waiting = chat.unverified_peers();
    let mut lines: Vec<String> = vec![format!("{you} {name}")];
    for peer in chat.fresh_peers() {
        let number = waiting.iter().find(|(waiting_peer, _, _)| waiting_peer.id == peer.id).map(|(_, _, number)| format!("[{number}] ")).unwrap_or_default();
        let state = match chat.link_of(&peer) {
            Link::Encrypted => t("peers.verified", &[]),
            Link::Plain => t("peers.plain", &[]),
            Link::Unverified => t("nodes.waiting", &[]),
        };
        lines.push(format!("{number}{} ({})  {state}", peer.name, peer.ip));
    }
    let max_lines = rows.saturating_sub(11).max(1);
    if lines.len() > max_lines {
        let shown = max_lines - 1;
        let more = lines.len() - shown;
        lines.truncate(shown);
        lines.push(t("nodes.more", &[("n", &more.to_string())]));
    }
    let mut block = vec![vec![("CONNECTED NODES".to_string(), Tone::Title)]];
    block.extend(lines.into_iter().map(|line| vec![(line, Tone::Text)]));
    block
}

/// The chat screen: the messages fill the middle, the phone address (when shown) sits over their last rows.
fn chat_frame(chat: &Chat, input: &str, stamps: &mut Stamps, width: usize, rows: usize) -> (Vec<Row>, usize) {
    let lines = chat.lines();
    let times = stamps.times(&lines);
    let messages: Vec<Row> = lines.iter().zip(times).flat_map(|(line, time)| message_rows(line, &time, width - 1)).collect();
    let name = chat.me.lock().unwrap().name.clone();
    let nodes = nodes_block(chat, &name, rows);
    let area = rows - 7 - nodes.len();
    let mut body: Vec<Row> = messages[messages.len().saturating_sub(area)..].to_vec();
    // Fill the message area first: with few messages the overlay rows must still sit inside it.
    body.resize_with(area, Vec::new);
    // The help box (F1) and the phone address sit over the bottom rows of the messages, the same way.
    let help = HELP_OPEN.with(Cell::get).then(talk_core::help_box);
    for overlay in help.into_iter().chain(chat.phone_overlay()) {
        let count = overlay.len().min(area);
        for (slot, text) in body[area - count..].iter_mut().zip(&overlay) {
            let text = fit(text, width);
            let pad = " ".repeat(width - char_width(&text));
            *slot = vec![(format!("{text}{pad}"), Tone::Overlay)];
        }
    }

    let peers = chat.fresh_peers().len().to_string();
    let badge = if chat.is_host() { format!("{} ", t("host.badge", &[])) } else { String::new() };
    let mode = if chat.is_supabase() { t("mode.supabase", &[]) } else { t("mode.local", &[]) };
    let relay = chat.relay_status();
    let info = status_info(&badge, &peers, &mode, chat.room_status(), &relay);
    let (input, cursor) = input_row(input, width);

    let mut frame = vec![split(APP_NAME, Tone::Title, "SYS.VER 1.0", Tone::Dim, width), dash(width), vec![("COMMUNICATION LOG".to_string(), Tone::Title)]];
    frame.extend(body);
    frame.extend(nodes);
    frame.push(dash(width));
    frame.push(input);
    // The status line sits above the maker line: the app name, name, peers and mode, then the connection tag and the help
    // hint at the end. The left part is cut when the row is too narrow, so the hint stays visible.
    let (tag, tone) = status_tag(&relay);
    let hint = t("chat.help_hint", &[]);
    let left = fit(&info, width.saturating_sub(char_width(tag) + char_width(&hint) + 4));
    frame.push(vec![(format!("{left}  "), Tone::Dim), (tag.to_string(), tone), (format!("  {hint}"), Tone::Dim)]);
    frame.push(footer(width));
    (frame, cursor)
}

thread_local! {
    /// The help box is open (F1). Thread-local so each test thread has its own.
    static HELP_OPEN: Cell<bool> = const { Cell::new(false) };
}

/// Apply one key. Return false to leave the chat.
fn on_key(key: KeyEvent, chat: &Chat, input: &mut String) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // Any key closes the help box, and that key is not typed (Ctrl+C still quits).
    if HELP_OPEN.with(|open| open.replace(false)) && !(ctrl && key.code == KeyCode::Char('c')) {
        return true;
    }
    // The phone address hides on the first key that is not its own toggle.
    if !(ctrl && key.code == KeyCode::Char('p')) {
        chat.hide_phone();
    }
    match key.code {
        KeyCode::Char('c' | 'd') if ctrl => return false,
        KeyCode::Char('l') if ctrl => chat.clear_lines(),
        KeyCode::Char('n') if ctrl => *input = "/name ".to_string(),
        KeyCode::Char('p') if ctrl => chat.toggle_phone(),
        // Ctrl+H: on Windows it arrives as Backspace with the Control modifier, so both codes are checked.
        KeyCode::F(1) => HELP_OPEN.with(|open| open.set(true)),
        KeyCode::Char('h') | KeyCode::Backspace if ctrl => HELP_OPEN.with(|open| open.set(true)),
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
            first_run::render(&mut terminal.0, &mut chooser, remaining, usize::from(cols), usize::from(rows))?;
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
                    settings_form::render(&mut terminal.0, &mut form, usize::from(cols), usize::from(rows))?;
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

/// The chat screen. `refused`: the saved room refused this PC's trust code, so the settings screen opens first with the reason.
pub fn run(chat: &Arc<Chat>, refused: bool) -> io::Result<()> {
    let mut terminal = Terminal::enter()?;
    let mut input = String::new();
    let mut last_status = String::new();
    let mut form: Option<Form> = refused.then(|| Form::refused(&talk_core::store::state_dir()));
    let mut stamps = Stamps::default();
    let mut drawn = Frame::default();
    render(&mut terminal.0, chat, &input, &mut stamps, &mut drawn)?;
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
                        // The chat frame drawn before the settings screen is no longer on the screen.
                        drawn = Frame::default();
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
            match &mut form {
                Some(open) => {
                    let (cols, rows) = terminal::size()?;
                    settings_form::render(&mut terminal.0, open, usize::from(cols), usize::from(rows))?;
                }
                None => render(&mut terminal.0, chat, &input, &mut stamps, &mut drawn)?,
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

fn render(out: &mut Stdout, chat: &Chat, input: &str, stamps: &mut Stamps, drawn: &mut Frame) -> io::Result<()> {
    let (cols, rows) = terminal::size()?;
    let (cols, rows) = (cols.max(20), rows.max(10));
    let width = usize::from(cols);
    let (frame, cursor) = chat_frame(chat, input, stamps, width, usize::from(rows));
    paint(out, width, usize::from(rows), frame, drawn)?;
    // The cursor goes back to the end of the typed text on the input row (the row above the divider and guide).
    queue!(out, cursor::MoveTo(cursor as u16, rows - 3), cursor::Show)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::{Me, Peer};

    fn text_of(row: &Row) -> String {
        row.iter().map(|(text, _)| text.as_str()).collect()
    }

    #[test]
    fn wrapped_rows_of_a_received_message_all_carry_the_indent_and_fit() {
        let line = Line { sender: Some("Bob (plain)".into()), text: "aaaa bbbb Alice: cccc dddd".into(), mine: false, seq: 0 };
        let rows: Vec<String> = message_rows(&line, "12:30:21", 40).iter().map(text_of).collect();
        assert!(rows.len() > 1);
        assert!(rows[0].starts_with("[12:30:21] Bob (plain) > "), "{}", rows[0]);
        assert!(rows[1..].iter().all(|row| row.starts_with("  ") && char_width(row) <= 40));
    }

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
    fn f1_opens_the_help_and_the_next_key_closes_it_without_typing() {
        let (chat, mut input) = (chat(), String::new());
        on_key(key(KeyCode::F(1), KeyModifiers::NONE), &chat, &mut input);
        assert!(HELP_OPEN.with(Cell::get), "F1 opens the help");
        on_key(key(KeyCode::Char('a'), KeyModifiers::NONE), &chat, &mut input);
        assert!(!HELP_OPEN.with(Cell::get), "any key closes it");
        assert!(input.is_empty(), "the closing key is not typed");
        on_key(key(KeyCode::Char('b'), KeyModifiers::NONE), &chat, &mut input);
        assert_eq!(input, "b");
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
    fn the_status_tag_follows_the_relay_text() {
        assert_eq!(status_tag("").0, "[SYSTEM READY]");
        assert_eq!(status_tag("relay: room joined (2 members)").0, "[CONNECTED]");
        assert_eq!(status_tag("relay: room join failed: offline").0, "[ERROR]");
        assert_eq!(status_tag("relay: settings incomplete").0, "[ERROR]");
        assert_eq!(status_tag("relay: room code missing or invalid (8 uppercase letters or digits)").0, "[ERROR]");
        assert_eq!(status_tag("relay: not joined yet").0, "[CONNECTING...]");
    }

    #[test]
    fn the_stamp_of_a_line_stays_while_the_line_is_shown() {
        let line = |seq: u64, text: &str| Line { sender: None, text: text.into(), mine: false, seq };
        let mut stamps = Stamps::default();
        let first = vec![line(0, "a")];
        assert_eq!(stamps.times(&first).len(), 1);
        stamps.0.insert(0, "00:00:01".into());
        let more = vec![line(0, "a"), line(1, "b")];
        assert_eq!(stamps.times(&more)[0], "00:00:01", "the first line keeps its time");
        stamps.0.insert(1, "00:00:02".into());
        // The first line is dropped from the top: the second one keeps its time.
        assert_eq!(stamps.times(&more[1..]), vec!["00:00:02".to_string()]);
    }

    #[test]
    fn equal_texts_keep_their_own_times_when_the_first_is_dropped() {
        let same = |seq: u64| Line { sender: None, text: "same".into(), mine: false, seq };
        let mut stamps = Stamps::default();
        stamps.times(&[same(0), same(1), same(2)]);
        stamps.0.insert(0, "00:00:01".into());
        stamps.0.insert(1, "00:00:02".into());
        stamps.0.insert(2, "00:00:03".into());
        // Line 0 is cut from the top: the lines that stay keep 00:00:02 and 00:00:03, not the times of the first match.
        assert_eq!(stamps.times(&[same(1), same(2)]), vec!["00:00:02".to_string(), "00:00:03".to_string()]);
    }

    #[test]
    fn a_waiting_peer_with_a_code_shows_its_number_in_the_list() {
        let chat = chat();
        let key = [7u8; 32];
        chat.peers.lock().unwrap().push(Peer { id: "p".into(), name: "rust-supa".into(), ip: "10.0.0.7".into(), port: 1, seen: Instant::now(), key: Some(key.to_vec()) });
        chat.my_nonce_for(&key);
        assert!(chat.set_reveal(&key, [9; 32]));
        let rows: Vec<String> = nodes_block(&chat, "Ann", 30).iter().map(text_of).collect();
        assert!(rows.iter().any(|row| row.starts_with("[1] rust-supa (10.0.0.7)")), "the list shows the same number as /join: {rows:?}");
    }

    #[test]
    fn the_nodes_block_lists_one_node_per_row_and_caps_the_rest() {
        let chat = chat();
        let peer = |name: &str| Peer { id: name.into(), name: name.into(), ip: "10.0.0.5".into(), port: 1, seen: Instant::now(), key: None };
        chat.peers.lock().unwrap().extend([peer("Bob"), peer("Cy")]);
        let rows: Vec<String> = nodes_block(&chat, "Ann", 30).iter().map(text_of).collect();
        assert_eq!(rows[0], "CONNECTED NODES");
        assert_eq!(rows[1], "YOU Ann");
        assert!(rows[2].starts_with("Bob (10.0.0.5)") && rows[3].starts_with("Cy (10.0.0.5)"), "{rows:?}");
        // Few rows left: the list keeps its first rows and the last one counts the rest.
        let rows: Vec<String> = nodes_block(&chat, "Ann", 13).iter().map(text_of).collect();
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(rows[2], t("nodes.more", &[("n", "2")]), "{rows:?}");
    }

    #[test]
    fn the_status_line_has_peers_mode_room_and_only_an_error() {
        let peers = t("status.line", &[("peers", "3")]).trim().to_string();
        let room = Some("room ID: ABCD2345 · setup code: -".to_string());
        let line = status_info("[host] ", "3", "CLOUD SERVICE", room.clone(), "relay: room joined (2 members)");
        assert_eq!(line, format!("[host] {peers} · CLOUD SERVICE · room ID: ABCD2345 · setup code: -"), "a success text is not added");
        let line = status_info("", "3", "CLOUD SERVICE", room.clone(), "relay: room join failed: offline");
        assert_eq!(line, format!("{peers} · CLOUD SERVICE · room ID: ABCD2345 · setup code: - · relay: room join failed: offline"));
        let local = status_info("", "0", "BASE STATION", None, "");
        assert_eq!(local, format!("{} · {}", t("status.line", &[("peers", "0")]).trim(), t("mode.local", &[])));
    }

    #[test]
    fn the_chat_rows_follow_the_python_order_with_the_maker_alone_at_the_bottom() {
        let (frame, _) = chat_frame(&chat(), "", &mut Stamps::default(), 100, 20);
        let texts: Vec<String> = frame.iter().map(text_of).collect();
        assert_eq!(texts.len(), 20);
        assert!(texts[0].starts_with("▣ HERDR-TALK"), "{}", texts[0]);
        assert!(texts[2].starts_with("COMMUNICATION LOG"), "{}", texts[2]);
        assert!(texts[17].starts_with("MESSAGE > "), "the input: {}", texts[17]);
        assert!(texts[18].ends_with(&t("chat.help_hint", &[])), "the help hint closes the status line: {}", texts[18]);
        assert!(texts[18].contains("[SYSTEM READY]"), "the connection tag is on the status line: {}", texts[18]);
        assert!(texts[18].contains(&t("mode.local", &[])), "the mode is on the status line: {}", texts[18]);
        assert!(!texts[18].contains("herdr-talk"), "the status line has no app name: {}", texts[18]);
        assert_eq!(texts[19].trim(), "made by Rust", "the bottom row holds only the maker");
        assert!(texts[19].starts_with(' '), "the maker is at the right end");
    }

    #[test]
    fn fit_and_tail_keep_to_the_width() {
        assert_eq!(fit("가나다", 5), "가나");
        assert_eq!(tail("가나다", 5), "나다");
    }
}
