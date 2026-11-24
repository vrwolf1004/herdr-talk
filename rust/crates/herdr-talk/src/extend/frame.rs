//! The chat frame as rows of coloured text: the header, the messages, the nodes, the input row and the status row.
//! Pure row building plus the paint into the terminal; the keys and the loop are in screen.rs.

use crossterm::style::Print;
use crossterm::terminal::{Clear, ClearType};
use crossterm::{cursor, queue};
use std::cell::Cell;
use std::io::{self, Stdout, Write};
use talk_core::clock::Stamps;
use talk_core::text::{char_width, wrap_line};
use talk_core::{t, Chat, Line, Link};

/// A colour as red, green and blue.
type Rgb = (u8, u8, u8);

// The classic terminal palette (design spec): background, primary green, body text, muted text, warning, error.
pub(crate) const BACKGROUND: Rgb = (5, 10, 8);
pub(crate) const MAIN: Rgb = (70, 255, 130);
pub(crate) const TEXT: Rgb = (180, 220, 190);
// #526B59 was too dark on the background; lifted so hints and secondary text stay readable.
pub(crate) const DIM: Rgb = (125, 154, 132);
pub(crate) const WARN: Rgb = (255, 184, 77);
pub(crate) const ALERT: Rgb = (255, 85, 85);
pub(crate) const BORDER: Rgb = (65, 125, 85);

/// The ANSI sequence for a 24-bit foreground or background colour. Written out by hand: the colours must reach the console
/// as 38;2 and 48;2 codes, which the crossterm colour commands did not produce here.
pub(crate) fn fore((r, g, b): Rgb) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}

pub(crate) fn back((r, g, b): Rgb) -> String {
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
pub(crate) fn print_row(out: &mut Stdout, row: &Row, width: usize) -> io::Result<()> {
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
pub(crate) fn dash(width: usize) -> Row {
    vec![("-".repeat(width), Tone::Border)]
}

/// A row with a left part and a right part, cut to the width so they never overlap.
pub(crate) fn split(left: &str, left_tone: Tone, right: &str, right_tone: Tone, width: usize) -> Row {
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
    if status_tag(relay).1 == Tone::Alert {
        info.push_str(&format!(" · {relay}"));
    }
    info
}

/// The status tag for the relay state: [SYSTEM READY] without a relay, otherwise the state the relay text names.
// shortcut: the state is read from the relay's status text; a clear state value from talk-core would replace this guess.
pub(crate) fn status_tag(relay: &str) -> (String, Tone) {
    if relay.is_empty() {
        (t("tui.tag.ready", &[]), Tone::Title)
    } else if ["failed", "could not", "no identity", "incomplete", "invalid"].iter().any(|word| relay.contains(word)) {
        (t("tui.tag.error", &[]), Tone::Alert)
    } else if relay.contains("room joined") {
        (t("tui.tag.connected", &[]), Tone::Title)
    } else {
        (t("tui.tag.connecting", &[]), Tone::Warn)
    }
}

/// The app name at the top left, with its icon (as the Python edition shows it).
pub(crate) const APP_NAME: &str = "▣ HERDR-TALK";

/// The maker line: alone on the bottom row, at the right end.
pub(crate) fn footer(width: usize) -> Row {
    split("", Tone::Dim, &t("tui.footer.made", &[]), Tone::Dim, width)
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
    frame.push(vec![(tag, tone)]);
    frame.push(footer(width));
    frame
}
pub(crate) fn fits(chars: impl Iterator<Item = char>, cols: usize) -> Vec<char> {
    let mut width = 0;
    chars.take_while(|ch| {
        width += char_width(ch.encode_utf8(&mut [0; 4]));
        width <= cols
    }).collect()
}

pub(crate) fn fit(text: &str, cols: usize) -> String {
    fits(text.chars(), cols).into_iter().collect()
}

/// The end of the typed text that fits, so the cursor stays visible.
pub(crate) fn tail(text: &str, cols: usize) -> String {
    fits(text.chars().rev(), cols).into_iter().rev().collect()
}

/// The rows of one chat line: "[time] name > text", the text wrapped under the start of the text. A notice has no name.
pub(crate) fn message_rows(line: &Line, time: &str, cols: usize) -> Vec<Row> {
    let Some(sender) = line.sender.as_ref() else {
        return wrap_line(&line.render(), cols).into_iter().map(|row| vec![(row, Tone::Dim)]).collect();
    };
    let text_tone = if line.mine { Tone::Title } else { Tone::Text };
    let lead = format!("[{time}] ");
    let head = format!("{lead}{sender} : ");
    let room = cols.saturating_sub(char_width(&head));
    if room < 8 {
        return wrap_line(&line.render(), cols).into_iter().map(|row| vec![(row, text_tone)]).collect();
    }
    wrap_line(&line.text, room)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            if index == 0 {
                vec![(lead.clone(), Tone::Dim), (sender.clone(), Tone::Title), (" : ".to_string(), Tone::Dim), (row, text_tone)]
            } else {
                vec![(" ".repeat(char_width(&head)), Tone::Dim), (row, text_tone)]
            }
        })
        .collect()
}

/// The input row and the column of the cursor on it.
pub(crate) fn input_row(input: &str, width: usize) -> (Row, usize) {
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
pub(crate) fn nodes_block(chat: &Chat, name: &str, rows: usize) -> Vec<Row> {
    let you = if chat.is_host() { "HOST/YOU" } else { "YOU" };
    let waiting = chat.unverified_peers();
    // CLOUD SERVICE: the people in the room follow on the same row ("HOST/YOU 도윤, 서연, 예준"); none in the LAN mode.
    let together: Vec<String> = std::iter::once(format!("{you} {name}")).chain(chat.room_names()).collect();
    let mut lines: Vec<String> = vec![together.join(", ")];
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
pub(crate) fn chat_frame(chat: &Chat, input: &str, stamps: &mut Stamps, width: usize, rows: usize) -> (Vec<Row>, usize) {
    let lines = chat.lines();
    let seqs: Vec<u64> = lines.iter().map(|line| line.seq).collect();
    let times = stamps.times(&seqs);
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

    // "Peers" are the people in the room in Supabase mode, the peers found on the network in the LAN mode.
    let peers = if chat.is_supabase() { chat.room_names().len() } else { chat.fresh_peers().len() }.to_string();
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
    let left = fit(&info, width.saturating_sub(char_width(&tag) + char_width(&hint) + 4));
    // The tag and the hint end at the right edge: the gap takes what the left part leaves (two columns at least).
    let gap = width.saturating_sub(char_width(&left) + char_width(&tag) + char_width(&hint) + 2).max(2);
    frame.push(vec![(format!("{left}{}", " ".repeat(gap)), Tone::Dim), (tag, tone), (format!("  {hint}"), Tone::Dim)]);
    frame.push(footer(width));
    (frame, cursor)
}

thread_local! {
    /// The help box is open (F1). Thread-local so each test thread has its own.
    pub(crate) static HELP_OPEN: Cell<bool> = const { Cell::new(false) };
}

