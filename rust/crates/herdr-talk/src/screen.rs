//! Full-screen chat in the classic terminal layout: header, divider, title, messages, divider, keyboard guide and the
//! status line. The first-run and settings screens use the same frame (`classic`). Rows are drawn one by one, and a frame
//! that is the same as the last one is not drawn again.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::Print;
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute, queue};
use std::collections::HashMap;
use std::io::{self, Stdout, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};
use talk_core::text::{char_width, wrap_line};
use crate::first_run::{self, Chooser, Step};
use crate::settings_form::{self, Form};
use talk_core::relay_settings::Mode;
use talk_core::{t, Chat, Line, Peer, IDLE_LIMIT};

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
    queue!(out, cursor::Show)?;
    out.flush()?;
    *drawn = frame;
    Ok(())
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

/// The CONNECTED NODES row text: you first (HOST/YOU on the host), then each fresh peer.
fn nodes_text(me: &str, host: bool, peers: &[Peer]) -> String {
    let you = if host { "HOST/YOU" } else { "YOU" };
    let mut nodes = vec![format!("{you} {me}")];
    nodes.extend(peers.iter().map(|peer| format!("PEER {}", peer.name)));
    nodes.join(" · ")
}

/// The chat screen: the messages fill the middle, the phone address (when shown) sits over their last rows.
fn chat_frame(chat: &Chat, input: &str, stamps: &mut Stamps, width: usize, rows: usize) -> (Vec<Row>, usize) {
    let lines = chat.lines();
    let times = stamps.times(&lines);
    let messages: Vec<Row> = lines.iter().zip(times).flat_map(|(line, time)| message_rows(line, &time, width - 1)).collect();
    let name = chat.me.lock().unwrap().name.clone();
    let fresh = chat.fresh_peers();
    let area = rows - 9;
    let mut body: Vec<Row> = messages[messages.len().saturating_sub(area)..].to_vec();
    // Fill the message area first: with few messages the overlay rows must still sit inside it.
    body.resize_with(area, Vec::new);
    if let Some(overlay) = chat.phone_overlay() {
        let count = overlay.len().min(area);
        for (slot, text) in body[area - count..].iter_mut().zip(&overlay) {
            let text = fit(text, width);
            let pad = " ".repeat(width - char_width(&text));
            *slot = vec![(format!("{text}{pad}"), Tone::Overlay)];
        }
    }

    let peers = fresh.len().to_string();
    let badge = if chat.is_host() { format!("{} ", t("host.badge", &[])) } else { String::new() };
    let info = format!("{badge}{}", t("status.line", &[("name", &name), ("peers", &peers)]));
    let relay = chat.relay_status();
    let (input, cursor) = input_row(input, width);
    let nodes = vec![("CONNECTED NODES  ".to_string(), Tone::Title), (nodes_text(&name, chat.is_host(), &fresh), Tone::Text)];

    let mut frame = vec![split(APP_NAME, Tone::Title, "SYS.VER 1.0", Tone::Dim, width), dash(width), vec![("COMMUNICATION LOG".to_string(), Tone::Title)]];
    frame.extend(body);
    frame.push(nodes);
    frame.push(dash(width));
    // shortcut: the keyboard guide is English-only like the other Ctrl key notes; move it to the locales with them.
    frame.push(vec![("Enter: SEND · /help: COMMANDS · Ctrl+N: NAME · Ctrl+P: PHONE · Ctrl+R: SETTINGS · Ctrl+C: QUIT".to_string(), Tone::Dim)]);
    frame.push(input);
    // The status line sits above the maker line, as in the Python edition: the connection tag, then the app name, name and peers.
    let (tag, tone) = status_tag(&relay);
    frame.push(vec![(format!("{tag}  "), tone), (info, Tone::Dim)]);
    frame.push(footer(width));
    (frame, cursor)
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

pub fn run(chat: &Arc<Chat>) -> io::Result<()> {
    let mut terminal = Terminal::enter()?;
    let mut input = String::new();
    let mut last_status = String::new();
    let mut form: Option<Form> = None;
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
    use talk_core::Me;

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
    fn the_nodes_row_lists_you_then_the_peers() {
        let peer = |name: &str| Peer { id: name.into(), name: name.into(), ip: String::new(), port: 1, seen: Instant::now(), key: None };
        assert_eq!(nodes_text("Ann", false, &[]), "YOU Ann");
        assert_eq!(nodes_text("Ann", true, &[peer("Bob"), peer("Cy")]), "HOST/YOU Ann · PEER Bob · PEER Cy");
    }

    #[test]
    fn the_chat_rows_follow_the_python_order_with_the_maker_alone_at_the_bottom() {
        let (frame, _) = chat_frame(&chat(), "", &mut Stamps::default(), 60, 20);
        let texts: Vec<String> = frame.iter().map(text_of).collect();
        assert_eq!(texts.len(), 20);
        assert!(texts[0].starts_with("▣ HERDR-TALK"), "{}", texts[0]);
        assert!(texts[2].starts_with("COMMUNICATION LOG"), "{}", texts[2]);
        assert!(texts[16].starts_with("Enter: SEND"), "the guide: {}", texts[16]);
        assert!(texts[17].starts_with("MESSAGE > "), "the input: {}", texts[17]);
        assert!(texts[18].starts_with("[SYSTEM READY]  "), "the status line: {}", texts[18]);
        assert_eq!(texts[19].trim(), "made by Rust", "the bottom row holds only the maker");
        assert!(texts[19].starts_with(' '), "the maker is at the right end");
    }

    #[test]
    fn fit_and_tail_keep_to_the_width() {
        assert_eq!(fit("가나다", 5), "가나");
        assert_eq!(tail("가나다", 5), "나다");
    }
}
