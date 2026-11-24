//! The terminal chat: enter and leave the full-screen mode, the key loop and the first-run choice.
//! The rows it draws are built in frame.rs.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute, queue};
use std::io::{self, Stdout, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};
use talk_core::clock::Stamps;
use crate::extend::first_run::{self, Chooser, Step};
use crate::extend::frame::{chat_frame, paint, Frame, HELP_OPEN};
use crate::extend::settings_form::{self, Form};
use talk_core::relay_settings::Mode;
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
        KeyCode::F(1) => HELP_OPEN.with(|open| open.set(true)),
        // F3 switches the language; a save failure is said in the chat (the switch still happens).
        KeyCode::F(3) => {
            let notice = talk_core::i18n::switch_language_notice();
            if !notice.is_empty() {
                chat.say(notice);
            }
        }
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
            Step::Language => {
                // Same as the chat's F3: a save failure is said in the chat (the switch still happens).
                let notice = talk_core::i18n::switch_language_notice();
                if !notice.is_empty() {
                    chat.say(notice);
                }
            }
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
    use crate::extend::frame::*;
    use std::cell::Cell;
    use talk_core::Line;
    use talk_core::text::char_width;
    use talk_core::{Me, Peer};

    fn text_of(row: &Row) -> String {
        row.iter().map(|(text, _)| text.as_str()).collect()
    }

    #[test]
    fn wrapped_rows_of_a_received_message_all_carry_the_indent_and_fit() {
        let line = Line { sender: Some("Bob (plain)".into()), text: "aaaa bbbb Alice: cccc dddd".into(), mine: false, seq: 0 };
        let rows: Vec<String> = message_rows(&line, "12:30:21", 40).iter().map(text_of).collect();
        assert!(rows.len() > 1);
        assert!(rows[0].starts_with("[12:30:21] Bob (plain) : "), "{}", rows[0]);
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
        assert_eq!(status_tag("").0, t("tui.tag.ready", &[]));
        assert_eq!(status_tag("relay: room joined (2 members)").0, t("tui.tag.connected", &[]));
        assert_eq!(status_tag("relay: room join failed: offline").0, t("tui.tag.error", &[]));
        assert_eq!(status_tag("relay: settings incomplete").0, t("tui.tag.error", &[]));
        assert_eq!(status_tag("relay: room code missing or invalid (8 uppercase letters or digits)").0, t("tui.tag.error", &[]));
        assert_eq!(status_tag("relay: not joined yet").0, t("tui.tag.connecting", &[]));
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
    fn the_nodes_block_lists_the_people_in_the_room_after_you() {
        let chat = chat();
        chat.set_room_names(vec!["서연".to_string(), "예준".to_string()]);
        let rows: Vec<String> = nodes_block(&chat, "Ann", 30).iter().map(text_of).collect();
        assert_eq!(rows[1], "YOU Ann, 서연, 예준", "one row: you, then the people in the room");
        assert_eq!(rows.len(), 2);
        chat.stop_relay();
        let rows: Vec<String> = nodes_block(&chat, "Ann", 30).iter().map(text_of).collect();
        assert_eq!(rows[1], "YOU Ann", "the names are dropped when the relay stops");
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
        assert!(texts[18].contains(&t("tui.tag.ready", &[])), "the connection tag is on the status line: {}", texts[18]);
        assert!(texts[18].contains(&t("mode.local", &[])), "the mode is on the status line: {}", texts[18]);
        assert!(!texts[18].contains("herdr-talk"), "the status line has no app name: {}", texts[18]);
        assert_eq!(texts[19].trim(), t("tui.footer.made", &[]), "the bottom row holds only the maker");
        assert!(texts[19].starts_with(' '), "the maker is at the right end");
    }

    #[test]
    fn fit_and_tail_keep_to_the_width() {
        assert_eq!(fit("가나다", 5), "가나");
        assert_eq!(tail("가나다", 5), "나다");
    }
}
