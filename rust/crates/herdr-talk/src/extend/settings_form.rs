//! The relay settings screen (Ctrl+R). The connection mode is not on this screen (the first-run choice sets it).
//! Four fields: setup code (redeemed for the project URL and key on save), display name (a read-only code), the room ID
//! (8 characters) and the trust code (8 digits). Tab or Up/Down moves between rows; Enter goes on, and saves on the last field;
//! Esc leaves without saving. Ctrl+N asks the relay for a new room (Supabase mode) and puts its code in the room ID field.
//! The URL and key are never drawn: only the key's length. Environment variables win over the saved values, and the screen says so.
//! Saving in Local mode keeps the typed fields; saving in Supabase mode checks them all (talk-core validate).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread;
use talk_core::relay_key;
use talk_core::relay_rest::{self, Request};
use talk_core::relay_settings::{self, Mode, Settings};
use talk_core::{relay_chat, t, Chat};
use crate::extend::frame::{classic, paint, Frame, Row, Tone};
use talk_core::text::char_width;

/// The four field labels, in field order.
fn labels() -> [String; 4] {
    [t("tui.settings.label.setup", &[]), t("tui.settings.label.trust", &[]), t("tui.settings.label.name", &[]), t("tui.settings.label.room", &[])]
}
const SETUP_FIELD: usize = 0;
/// The display name is typed by the user (empty at first).
const TRUST_FIELD: usize = 1;
const NAME_FIELD: usize = 2;
const ROOM_FIELD: usize = 3;
/// The fields, numbered 1 to 4. The mode has no row: the first-run choice sets it, and focus never rests on a row that is not drawn.
const FIELDS: usize = 4;

/// The Unique ID row (the name) is not drawn: focus moving onto it goes on to the next row (down) or the one before (up).
fn skip_name(focus: usize, down: bool) -> usize {
    if focus != NAME_FIELD + 1 {
        focus
    } else if down {
        focus + 1
    } else {
        focus - 1
    }
}

pub struct Form {
    mode: Mode,
    fields: [String; 4],
    /// The saved project URL and key (never drawn; the setup code row replaces them).
    url: String,
    key: String,
    /// The setup code last redeemed, saved with the URL and key (shown on the chat status line).
    setup_code: String,
    focus: usize,
    message: String,
    from_env: (bool, bool),
    mine: Option<(String, String)>,
    state: PathBuf,
    /// True once a setting was written (the first-run choice then goes on to the chat).
    saved: bool,
    /// A create-room request running in the background: its answer (the code, or the reason it failed).
    creating: Option<mpsc::Receiver<Result<String, String>>>,
    /// The operator passphrase being typed (F9 opens the line): never drawn, only its length.
    passphrase: Option<String>,
    /// An issue-setup-code request running in the background: its answer (the new code, or the reason it failed).
    issuing: Option<mpsc::Receiver<Result<String, String>>>,
    /// The last frame drawn, so an unchanged screen is not drawn again.
    drawn: Frame,
}

pub enum Action {
    Stay,
    Leave,
}

impl Form {
    pub fn open(state: &Path) -> Form {
        let saved = relay_settings::load_saved(state);
        let mine = relay_chat::my_keys(state).ok();
        let message = if mine.is_none() { t("tui.settings.no_keys", &[]) } else { String::new() };
        // The Unique ID row is read-only: this PC's short ID (a name saved by an older version is kept).
        let name = if saved.name.is_empty() { relay_key::short_id(state).unwrap_or_default() } else { saved.name };
        let fields = [String::new(), saved.trust, name, saved.room_code];
        Form {
            mode: saved.mode,
            fields,
            url: saved.url,
            key: saved.key,
            setup_code: saved.setup_code,
            focus: 1,
            message,
            from_env: relay_settings::from_environment(),
            mine,
            state: state.to_path_buf(),
            saved: false,
            creating: None,
            passphrase: None,
            issuing: None,
            drawn: Frame::default(),
        }
    }

    /// The settings screen opened because the saved room refused this PC's trust code: the reason is on the message line.
    pub fn refused(state: &Path) -> Form {
        let mut form = Form::open(state);
        form.message = t("room.trust_wrong", &[]);
        form
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
            url: self.url.trim().to_string(),
            key: self.key.trim().to_string(),
            setup_code: self.setup_code.clone(),
            name: self.fields[NAME_FIELD].trim().to_string(),
            room_code: self.fields[ROOM_FIELD].trim().to_ascii_uppercase(),
            trust: self.fields[TRUST_FIELD].trim().to_string(),
        }
    }

    /// Save, then start the relay with the new settings. A typed setup code is redeemed first; the message never shows the key.
    fn save(&mut self, chat: &Arc<Chat>) {
        let settings = self.settings();
        if let Err(refusal) = relay_settings::check_save(&settings, &self.fields[SETUP_FIELD]) {
            self.message = save_message(refusal);
            return;
        }
        match relay_settings::save(&self.state, &settings) {
            Err(error) => self.message = t("tui.settings.not_saved", &[("error", &error.to_string())]),
            Ok(()) => {
                self.saved = true;
                let started = relay_chat::start(chat, &self.state);
                let joined = settings.mode == Mode::Supabase && started.is_ok(); // the chat needs no "relay started" line
                let text = match (settings.mode, started) {
                    (Mode::Local, _) => t("settings.save_ok_local", &[]),
                    (Mode::Supabase, Ok(_)) => t("settings.save_ok_supabase", &[]),
                    (Mode::Supabase, Err(error)) => format!("relay: {error}"),
                };
                self.message = text.clone();
                // The chat screen shows the result line, except the plain success of a relay start.
                if !joined {
                    chat.say(text);
                }
            }
        }
    }

    /// F2 in the setup-code row: redeems the typed code and saves only the URL and key to relay.json. Neither is drawn.
    fn save_setup_code(&mut self) {
        if self.focus != SETUP_FIELD + 1 {
            self.message = t("tui.settings.f2_row_only", &[]);
            return;
        }
        let typed = self.fields[SETUP_FIELD].trim().to_string();
        if typed.is_empty() {
            self.message = t("setup.invalid", &[]);
            return;
        }
        match redeem_and_save(&self.state, &relay_settings::setup_url(), &typed) {
            Err(message) => self.message = message,
            Ok((url, key, setup)) => {
                self.url = url;
                self.key = key;
                self.setup_code = setup;
                self.fields[SETUP_FIELD].clear();
                self.message = t("tui.settings.setup_saved", &[]);
            }
        }
    }

    /// F3: switches the language and saves it to config.json, as the chat does. A save failure is shown on the message line.
    fn switch_language(&mut self) {
        self.message = talk_core::i18n::switch_language_notice();
    }

    /// F9: opens the operator passphrase line (hidden: not named on the screen). Nothing is sent until Enter.
    fn open_passphrase(&mut self) {
        if self.passphrase.is_none() {
            self.passphrase = Some(String::new());
            self.message = t("tui.settings.operator_hint", &[]);
        }
    }

    /// Enter on the passphrase line: asks the relay for a new setup code in the background. The passphrase is not kept after that.
    fn issue_setup_code(&mut self) {
        let Some(passphrase) = self.passphrase.take() else { return };
        if self.issuing.is_some() {
            return;
        }
        if passphrase.is_empty() {
            self.message = t("tui.settings.issue_failed", &[("error", &t("tui.settings.passphrase_empty", &[]))]);
            return;
        }
        let url = relay_settings::setup_url();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(relay_rest::issue_setup_code(&url, &passphrase));
        });
        self.issuing = Some(receiver);
        self.message = t("tui.settings.issuing", &[]);
    }

    /// Applies an issue-setup-code answer that has come back. True when the screen must be drawn again.
    fn poll_issue(&mut self) -> bool {
        let Some(receiver) = self.issuing.as_ref() else { return false };
        let answer = match receiver.try_recv() {
            Ok(answer) => answer,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => Err(t("tui.settings.request_stopped", &[])),
        };
        self.issuing = None;
        self.message = match answer {
            Ok(code) => {
                self.fields[SETUP_FIELD] = code.clone(); // the new code goes into the setup code input
                t("tui.settings.new_code", &[("code", &code)])
            }
            Err(error) => t("tui.settings.issue_failed", &[("error", &error)]),
        };
        true
    }

    /// Ctrl+N: ask the relay for a new room with this PC's key and put the code in the room ID field (saved with Enter).
    fn create_room(&mut self) {
        if self.creating.is_some() {
            return;
        }
        // Only one room: a new room is refused while another one is saved (the empty code is never a saved room).
        if relay_settings::has_other_room(&self.state, "") {
            self.message = t("settings.one_room_only", &[]);
            return;
        }
        if self.mine.is_none() {
            self.message = t("tui.settings.no_keys", &[]);
            return;
        }
        let settings = self.settings();
        match relay_settings::check_create(&settings) {
            Ok(()) => {}
            Err(relay_settings::CreateRefusal::NoConnection) => {
                self.message = t("tui.settings.create_need_connection", &[]);
                return;
            }
            Err(relay_settings::CreateRefusal::BadTrust) => {
                self.message = t("tui.settings.create_bad_trust", &[]);
                return;
            }
        }
        let url = settings.url.trim_end_matches('/').to_string();
        let key = settings.key.clone();
        let name = settings.name.clone();
        let trust = settings.trust.clone();
        let config = match relay_rest::config(Some(&url), Some(&key)) {
            Ok(Some(config)) => config,
            Ok(None) => {
                self.message = t("tui.settings.create_need_url_key", &[]);
                return;
            }
            Err(error) => {
                self.message = t("tui.settings.create_failed", &[("error", &error)]);
                return;
            }
        };
        // The room's owner row (peers) must exist before the room is made (rooms.owner_pk references it).
        let short_id = match relay_key::short_id(&self.state) {
            Ok(short_id) => short_id,
            Err(error) => {
                self.message = t("tui.settings.create_failed", &[("error", &error)]);
                return;
            }
        };
        let noise_pk = self.mine.as_ref().map(|(_, noise)| noise.clone()).unwrap_or_default();
        let seed = match relay_key::load_or_create(&self.state) {
            Ok(seed) => seed,
            Err(error) => {
                self.message = t("tui.settings.create_failed", &[("error", &error)]);
                return;
            }
        };
        let nonce = match relay_rest::random_nonce() {
            Ok(nonce) => nonce,
            Err(error) => {
                self.message = t("tui.settings.create_failed", &[("error", &error)]);
                return;
            }
        };
        let body = match relay_rest::signed_room_body(&seed, relay_rest::RoomAction::Create, "", &name, &trust, relay_rest::unix_now(), &nonce) {
            Ok(body) => body,
            Err(error) => {
                self.message = t("tui.settings.create_failed", &[("error", &error)]);
                return;
            }
        };
        let request = Request {
            method: "POST",
            url: format!("{url}/functions/v1/create-room"),
            headers: vec![
                ("apikey".to_string(), key.clone()),
                ("Authorization".to_string(), format!("Bearer {key}")),
                ("Content-Type".to_string(), "application/json".to_string()),
            ],
            body: Some(body),
        };
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let answer = publish_message(relay_rest::put_peer(&config, &seed, &noise_pk, &short_id, &name)).and_then(|()| {
                relay_rest::http_post(&request).and_then(|(status, text)| read_room_code(status, &text))
            });
            let _ = sender.send(answer);
        });
        self.creating = Some(receiver);
        self.message = t("tui.settings.create_busy", &[]);
    }

    /// Applies a create-room answer that has come back. True when the screen must be drawn again.
    pub fn poll(&mut self) -> bool {
        if self.poll_issue() {
            return true;
        }
        let Some(receiver) = self.creating.as_ref() else { return false };
        let answer = match receiver.try_recv() {
            Ok(answer) => answer,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => Err(t("tui.settings.create_stopped", &[])),
        };
        self.creating = None;
        match answer {
            Ok(code) => {
                self.message = t("tui.settings.room_made", &[("code", &code)]);
                self.fields[ROOM_FIELD] = code;
            }
            Err(error) => self.message = t("tui.settings.create_failed", &[("error", &error)]),
        }
        true
    }
}

/// F2's work: redeem the typed code at `url`, then save the URL, key and code. Ok gives them; Err is the message to show.
/// Nothing is saved when the redeem is refused.
fn redeem_and_save(state: &Path, url: &str, typed: &str) -> Result<(String, String, String), String> {
    let setup = relay_rest::normalize_setup_code(typed).unwrap_or_default();
    let (project_url, key) = relay_rest::redeem_setup_code(typed, url)?;
    relay_settings::save_connection(state, &project_url, &key, &setup).map_err(|error| t("tui.settings.not_saved", &[("error", &error)]))?;
    Ok((project_url, key, setup))
}

/// The message for a refused final save (Enter), as the screen shows it.
fn save_message(refusal: relay_settings::SaveRefusal) -> String {
    use relay_settings::{Missing, SaveRefusal};
    match refusal {
        SaveRefusal::UnredeemedCode => t("tui.settings.press_f2", &[]),
        SaveRefusal::MissingFields(fields) => {
            let labels = labels();
            let names: Vec<&str> = fields
                .iter()
                .map(|field| match field {
                    Missing::Name => labels[NAME_FIELD].as_str(),
                    Missing::Room => labels[ROOM_FIELD].as_str(),
                    Missing::Trust => labels[TRUST_FIELD].as_str(),
                })
                .collect();
            t("settings.save_refused_missing", &[("fields", &names.join(", "))])
        }
        SaveRefusal::NoConnection => t("setup.needed", &[]),
    }
}

/// What the row write (put-peer) answer means for the create: Ok to go on, or the message to show (the create is not sent).
fn publish_message(outcome: relay_rest::PutOutcome) -> Result<(), String> {
    match outcome {
        relay_rest::PutOutcome::Written => Ok(()),
        relay_rest::PutOutcome::Taken => Err(t("tui.settings.publish_taken", &[])),
        relay_rest::PutOutcome::Failed => Err(t("tui.settings.publish_failed", &[])),
    }
}

/// The create-room answer: the room code on 200, otherwise the status and body.
fn read_room_code(status: u16, text: &str) -> Result<String, String> {
    if status != 200 {
        return Err(t("tui.settings.answer_status", &[("status", &status.to_string()), ("text", text)]));
    }
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| t("tui.settings.answer_not_json", &[]))?;
    value["room_code"]
        .as_str()
        .filter(|code| relay_rest::valid_room_code(code))
        .map(str::to_string)
        .ok_or_else(|| t("tui.settings.answer_no_room", &[]))
}

/// One key for the form.
pub fn on_key(form: &mut Form, key: KeyEvent, chat: &Arc<Chat>) -> Action {
    match key.code {
        KeyCode::Esc if form.passphrase.is_some() => {
            form.passphrase = None;
            form.message.clear();
        }
        KeyCode::Esc => return Action::Leave,
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => form.create_room(),
        KeyCode::F(9) => form.open_passphrase(),
        KeyCode::Enter if form.passphrase.is_some() => form.issue_setup_code(),
        KeyCode::Backspace if form.passphrase.is_some() => {
            if let Some(passphrase) = form.passphrase.as_mut() {
                passphrase.pop();
            }
        }
        KeyCode::Char(ch) if form.passphrase.is_some() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(passphrase) = form.passphrase.as_mut() {
                passphrase.push(ch);
            }
        }
        KeyCode::F(2) => form.save_setup_code(),
        KeyCode::F(3) => form.switch_language(),
        // Focus cycles over the drawn fields only (1 to FIELDS).
        KeyCode::Tab | KeyCode::Down => form.focus = skip_name(form.focus % FIELDS + 1, true),
        KeyCode::Up => form.focus = skip_name((form.focus + FIELDS - 2) % FIELDS + 1, false),
        KeyCode::Enter if form.focus < FIELDS => form.focus = skip_name(form.focus + 1, true),
        KeyCode::Enter => form.save(chat),
        KeyCode::Backspace => {
            form.fields[form.focus - 1].pop();
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => form.fields[form.focus - 1].push(ch),
        _ => {}
    }
    Action::Stay
}

fn shown(form: &Form, index: usize) -> String {
    // The saved setup code is shown as it is (as the Python edition shows it). The key itself is never drawn:
    // only its length, when an older settings file has no setup code.
    if index == SETUP_FIELD && form.fields[index].is_empty() {
        if form.from_env.0 || form.from_env.1 {
            return t("settings.code_environment", &[]);
        }
        if !form.setup_code.is_empty() {
            return form.setup_code.clone();
        }
        return if form.key.is_empty() { "-".to_string() } else { t("settings.code_saved", &[("n", &form.key.chars().count().to_string())]) };
    }
    form.fields[index].clone()
}

/// The lines of the settings screen: the text and its tone. Three sections: the Supabase connection, the room settings,
/// and this PC's identity. The connection mode is not named on this screen (it is chosen at the start).
fn lines(form: &Form) -> Vec<(String, Tone)> {
    // In Local mode the fields are kept but not used: they are drawn dimmed.
    let field_tone = if form.mode == Mode::Local { Tone::Dim } else { Tone::Text };
    // "LABEL : [value____]": the value is filled with underscores to the input width; the focused field is the bright one.
    let labels = labels();
    let column = label_column(&labels);
    let field = |index: usize| {
        let text = field_text(form.focus == index + 1, &labels[index], &shown(form, index), column);
        (text, if form.focus == index + 1 { Tone::Title } else { field_tone })
    };
    // The hints under the setup-code and room rows, as the Python edition shows them.
    let mut lines: Vec<(String, Tone)> = vec![
        (t("tui.settings.title", &[]), Tone::Title),
        (String::new(), Tone::Text),
        field(SETUP_FIELD),
        (t("tui.settings.hint.setup", &[]), Tone::Dim),
        field(TRUST_FIELD),
        field(ROOM_FIELD),
        (t("tui.settings.hint.room", &[]), Tone::Dim),
    ];
    // Opened by F9 (not named on the screen): only the passphrase's length is drawn.
    if let Some(passphrase) = &form.passphrase {
        lines.push((t("tui.settings.passphrase", &[("stars", &"*".repeat(passphrase.chars().count()))]), Tone::Title));
    }
    lines.extend([(String::new(), Tone::Text), (t("tui.settings.identity", &[]), Tone::Title)]);
    match &form.mine {
        Some((ed, noise)) => {
            lines.push((t("tui.settings.ed25519", &[("key", ed)]), Tone::Dim));
            lines.push((t("tui.settings.noise", &[("key", noise)]), Tone::Dim));
        }
        None => lines.push((t("tui.settings.no_identity", &[]), Tone::Dim)),
    }
    lines.push((String::new(), Tone::Text));
    lines.push((form.message.clone(), Tone::Warn));
    lines
}

/// The width of the input line in the field rows.
const FIELD_WIDTH: usize = 24;

/// The label column: the widest label of the current language, and at least 22 columns (as the Python edition does),
/// so the ' : [' column is the same on every row.
fn label_column(labels: &[String]) -> usize {
    labels.iter().map(|label| char_width(label)).max().unwrap_or(0).max(22)
}

/// One field row: the marker, the label padded to the column, then the value in brackets with its underscore fill.
fn field_text(selected: bool, label: &str, value: &str, column: usize) -> String {
    let blanks = FIELD_WIDTH.saturating_sub(char_width(value));
    format!("{} {} : [{value}{}]", if selected { "▶" } else { " " }, pad_to(label, column), "_".repeat(blanks))
}

/// `text` padded with spaces to `cols` columns.
fn pad_to(text: &str, cols: usize) -> String {
    format!("{text}{}", " ".repeat(cols.saturating_sub(char_width(text))))
}

pub fn render(out: &mut Stdout, form: &mut Form, width: usize, rows: usize) -> io::Result<()> {
    let body: Vec<Row> = lines(form).into_iter().map(|(text, tone)| vec![(text, tone)]).collect();
    let frame = classic(body, &t("tui.settings.guide", &[]), "", width, rows);
    paint(out, width, rows, frame, &mut form.drawn)
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
            url: String::new(),
            key: String::new(),
            setup_code: String::new(),
            focus: 1,
            message: String::new(),
            from_env: (false, false),
            mine: None,
            state: state.to_path_buf(),
            saved: false,
            creating: None,
            passphrase: None,
            issuing: None,
            drawn: Frame::default(),
        }
    }

    /// A local relay that answers one request with `response` and sends the request it read back. Returns its URL.
    fn mock_relay(response: String) -> (String, mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let (sent, received) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            // Headers up to the blank line, then the body by its Content-Length.
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let head = String::from_utf8_lossy(&request).to_ascii_lowercase();
            let length: usize = head.lines().find_map(|line| line.strip_prefix("content-length:")).map_or(0, |value| value.trim().parse().unwrap());
            let mut body = vec![0u8; length];
            stream.read_exact(&mut body).unwrap();
            request.extend(body);
            sent.send(String::from_utf8_lossy(&request).to_string()).unwrap();
            stream.write_all(response.as_bytes()).unwrap();
        });
        (url, received)
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
    fn focus_starts_on_a_drawn_field_and_cycles_over_the_fields_only() {
        let (chat, mut form) = (chat(), form_at(&folder("cycle")));
        assert_eq!(form.focus, 1, "the first key goes to a field that is drawn");
        let visited: Vec<usize> = (0..FIELDS + 1)
            .map(|_| {
                press(&mut form, &chat, KeyCode::Tab);
                form.focus
            })
            .collect();
        assert_eq!(visited, vec![2, 4, 1, 2, 4], "Tab skips the Unique ID row and wraps from the last field to the first, never to 0");
        press(&mut form, &chat, KeyCode::Up);
        assert_eq!(form.focus, 2, "Up skips the Unique ID row too");
        press(&mut form, &chat, KeyCode::Up);
        assert_eq!(form.focus, 1);
        press(&mut form, &chat, KeyCode::Up);
        assert_eq!(form.focus, 4, "Up wraps from the first field to the last");
    }

    #[test]
    fn the_unique_id_is_this_pcs_short_id_and_cannot_be_typed() {
        let dir = folder("name-typed");
        let form = Form::open(&dir);
        let id = relay_key::short_id(&dir).unwrap();
        assert_eq!(form.fields[NAME_FIELD], id, "the row shows this PC's short ID");
        let (chat, mut form) = (chat(), form);
        form.focus = NAME_FIELD + 1;
        press(&mut form, &chat, KeyCode::Char('A'));
        press(&mut form, &chat, KeyCode::Backspace);
        assert_eq!(form.fields[NAME_FIELD], id, "typing and backspace change nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn letters_typed_on_the_first_screen_reach_a_field() {
        let (chat, mut form) = (chat(), form_at(&folder("first-key")));
        press(&mut form, &chat, KeyCode::Char('h'));
        assert_eq!(form.fields[0], "h");
    }

    #[test]
    fn typing_goes_to_the_focused_field_and_enter_moves_on() {
        let (chat, mut form) = (chat(), form_at(&folder("typing")));
        for ch in "ab-cd".chars() {
            press(&mut form, &chat, KeyCode::Char(ch));
        }
        press(&mut form, &chat, KeyCode::Enter);
        press(&mut form, &chat, KeyCode::Enter);
        press(&mut form, &chat, KeyCode::Enter);
        press(&mut form, &chat, KeyCode::Char('k'));
        assert_eq!(form.fields[SETUP_FIELD], "ab-cd");
        assert_eq!(form.fields[ROOM_FIELD], "k");
        assert_eq!(form.focus, 4);
    }

    #[test]
    fn the_saved_key_is_shown_as_its_length_only() {
        let mut form = form_at(&folder("mask"));
        form.key = "dummy-test-key".into();
        assert_eq!(shown(&form, SETUP_FIELD), t("settings.code_saved", &[("n", "14")]));
        form.fields[SETUP_FIELD] = "ABCD2345".into();
        assert_eq!(shown(&form, SETUP_FIELD), "ABCD2345", "a typed code is shown until it is redeemed");
    }

    #[test]
    fn environment_values_are_marked() {
        let mut form = form_at(&folder("env"));
        form.from_env = (true, false);
        assert_eq!(shown(&form, SETUP_FIELD), t("settings.code_environment", &[]), "an env value is said, never shown");
        assert!(!shown(&form, NAME_FIELD).contains("environment"));
    }

    #[test]
    fn local_save_keeps_the_fields_and_saves_the_mode() {
        let dir = folder("local");
        let (chat, mut form) = (chat(), form_at(&dir));
        form.fields[NAME_FIELD] = "Alice".into();
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
        form.url = "https://x.example".into();
        form.key = "dummy-test-key".into();
        form.save(&chat);
        assert!(form.message.contains(&t("tui.settings.label.name", &[])), "{}", form.message);
        assert!(!dir.join(relay_settings::FILE).exists());
    }

    #[test]
    fn supabase_save_without_a_setup_code_or_saved_connection_asks_for_the_code() {
        let dir = folder("no-setup");
        let (chat, mut form) = (chat(), form_at(&dir));
        form.mode = Mode::Supabase;
        form.fields = [String::new(), "12345678".into(), "Alice".into(), "ABCD2345".into()];
        form.save(&chat);
        assert_eq!(form.message, t("setup.needed", &[]));
        assert!(!dir.join(relay_settings::FILE).exists());
    }

    #[test]
    fn enter_does_not_redeem_a_typed_setup_code() {
        let dir = folder("enter-setup");
        let (chat, mut form) = (chat(), form_at(&dir));
        form.mode = Mode::Supabase;
        form.fields = ["ABCD2345".into(), "12345678".into(), "Alice".into(), "ABCD2345".into()];
        form.focus = FIELDS;
        press(&mut form, &chat, KeyCode::Enter);
        assert_eq!(form.message, t("tui.settings.press_f2", &[]));
        assert_eq!(form.fields[SETUP_FIELD], "ABCD2345", "the typed code stays for F2");
        assert!(!dir.join(relay_settings::FILE).exists());
    }

    #[test]
    fn f2_outside_the_setup_row_says_so() {
        let (chat, mut form) = (chat(), form_at(&folder("f2-elsewhere")));
        form.fields[SETUP_FIELD] = "ABCD2345".into();
        form.focus = NAME_FIELD + 1;
        press(&mut form, &chat, KeyCode::F(2));
        assert_eq!(form.message, t("tui.settings.f2_row_only", &[]));
        assert_eq!(form.fields[SETUP_FIELD], "ABCD2345");
    }

    #[test]
    fn f2_with_a_malformed_or_empty_code_is_refused_before_any_request() {
        let dir = folder("f2-bad");
        let (chat, mut form) = (chat(), form_at(&dir));
        press(&mut form, &chat, KeyCode::F(2));
        assert_eq!(form.message, t("setup.invalid", &[]), "empty");
        form.fields[SETUP_FIELD] = "ABCD".into();
        press(&mut form, &chat, KeyCode::F(2));
        assert_eq!(form.message, t("setup.invalid", &[]));
        assert_eq!(form.fields[SETUP_FIELD], "ABCD", "the typed code stays for a fix");
        assert!(form.url.is_empty() && form.key.is_empty());
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
        form.fields[ROOM_FIELD] = "not a key".into();
        form.focus = FIELDS;
        press(&mut form, &chat, KeyCode::Enter);
        assert!(form.message.starts_with(&t("tui.settings.not_saved", &[("error", "")])), "{}", form.message);
    }

    #[test]
    fn the_screen_has_the_python_sections_and_rows() {
        let form = form_at(&folder("sections"));
        let texts: Vec<String> = lines(&form).into_iter().map(|(text, _)| text).collect();
        assert_eq!(texts[0], t("tui.settings.title", &[]));
        assert!(texts.iter().any(|text| text == &t("tui.settings.identity", &[])));
        assert!(!texts.iter().any(|text| text.contains("[0")), "no numbered sections");
        assert!(!texts.iter().any(|text| text.contains("Publishable key") || text.contains("Ctrl+N: create room (Supabase")), "no key row or old note");
        let rows: Vec<&String> = texts.iter().filter(|text| text.contains(" : [")).collect();
        let labels: Vec<&str> = rows.iter().map(|text| text.split(" : [").next().unwrap().trim_start_matches(['▶', ' '])).map(str::trim_end).collect();
        assert_eq!(labels, [t("tui.settings.label.setup", &[]), t("tui.settings.label.trust", &[]), t("tui.settings.label.room", &[])], "the Unique ID row is not drawn");
        assert!(texts.iter().any(|text| text == &t("tui.settings.hint.setup", &[])));
        assert!(texts.iter().any(|text| text == &t("tui.settings.hint.room", &[])));
    }

    #[test]
    fn a_background_create_room_answer_is_applied_by_poll() {
        let mut form = form_at(&folder("poll"));
        let (sender, receiver) = mpsc::channel();
        form.creating = Some(receiver);
        assert!(!form.poll(), "no answer yet: nothing to draw");
        assert!(form.creating.is_some());
        sender.send(Ok("ABCD1234".to_string())).unwrap();
        assert!(form.poll());
        assert_eq!(form.fields[ROOM_FIELD], "ABCD1234");
        assert!(form.creating.is_none());
        assert!(form.message.contains("ABCD1234"), "{}", form.message);
    }

    #[test]
    fn a_failed_background_create_room_says_why() {
        let mut form = form_at(&folder("poll-error"));
        let (sender, receiver) = mpsc::channel();
        form.creating = Some(receiver);
        sender.send(Err("status 500".to_string())).unwrap();
        assert!(form.poll());
        assert!(form.fields[ROOM_FIELD].is_empty());
        assert_eq!(form.message, t("tui.settings.create_failed", &[("error", "status 500")]));
    }

    #[test]
    fn a_failed_row_write_stops_the_create_with_a_message() {
        assert_eq!(publish_message(relay_rest::PutOutcome::Written), Ok(()));
        assert!(publish_message(relay_rest::PutOutcome::Taken).unwrap_err().contains("409"));
        assert!(publish_message(relay_rest::PutOutcome::Failed).unwrap_err().contains("게시하지 못했습니다"));
    }

    #[test]
    fn a_refused_room_opens_the_settings_with_the_trust_message() {
        let form = Form::refused(&folder("refused-room"));
        assert_eq!(form.message, t("room.trust_wrong", &[]));
    }

    #[test]
    fn the_input_column_is_the_same_in_every_language() {
        let files = [
            ("en", include_str!("../../../talk-core/locales/en.json")),
            ("ko", include_str!("../../../talk-core/locales/ko.json")),
            ("ru", include_str!("../../../talk-core/locales/ru.json")),
            ("fr", include_str!("../../../talk-core/locales/fr.json")),
            ("es", include_str!("../../../talk-core/locales/es.json")),
            ("de", include_str!("../../../talk-core/locales/de.json")),
            ("ja", include_str!("../../../talk-core/locales/ja.json")),
        ];
        let keys = ["tui.settings.label.setup", "tui.settings.label.trust", "tui.settings.label.name", "tui.settings.label.room"];
        for (lang, text) in files {
            let messages: serde_json::Value = serde_json::from_str(text).unwrap();
            let labels: Vec<String> = keys.iter().map(|key| messages[*key].as_str().unwrap().to_string()).collect();
            let column = label_column(&labels);
            assert!(column >= 22 && labels.iter().all(|label| char_width(label) <= column), "{lang}");
            let bracket_columns: Vec<usize> = labels
                .iter()
                .map(|label| char_width(field_text(false, label, "value", column).split(" : [").next().unwrap()))
                .collect();
            assert!(bracket_columns.iter().all(|at| *at == bracket_columns[0]), "{lang}: {bracket_columns:?}");
        }
    }

    #[test]
    fn a_redeemed_setup_code_is_saved_with_url_key_and_code() {
        // F2's path with a mock relay: a 200 answer saves URL, key and setup code.
        let dir = folder("f2-redeem");
        let _ = std::fs::remove_dir_all(&dir);
        let body = r#"{"url":"https://p.supabase.co","key":"k1"}"#;
        let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let (relay_url, received) = mock_relay(response);
        let (url, key, setup) = redeem_and_save(&dir, &relay_url, "abcd-2345").unwrap();
        assert!(received.recv().unwrap().contains(r#""code":"ABCD2345""#));
        assert_eq!((url.as_str(), key.as_str(), setup.as_str()), ("https://p.supabase.co", "k1", "ABCD2345"));
        let saved = relay_settings::load_saved(&dir);
        assert_eq!((saved.mode, saved.url.as_str(), saved.key.as_str(), saved.setup_code.as_str()), (Mode::Supabase, "https://p.supabase.co", "k1", "ABCD2345"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refused_setup_code_saves_nothing_and_says_why() {
        // F2's path with a mock relay: a 404 answer saves nothing and the message says the code is wrong.
        let dir = folder("f2-refused");
        let _ = std::fs::remove_dir_all(&dir);
        let (relay_url, _received) = mock_relay("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string());
        assert_eq!(redeem_and_save(&dir, &relay_url, "ABCD2345"), Err(t("setup.wrong", &[])));
        assert!(!dir.join(relay_settings::FILE).exists(), "nothing is written");
    }

    #[test]
    fn f3_switches_the_language_and_saves_it() {
        let dir = folder("f3-language");
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("HERDR_PLUGIN_CONFIG_DIR", &dir);
        let (chat, mut form) = (chat(), form_at(&dir));
        let before = talk_core::i18n::language();
        assert!(matches!(press(&mut form, &chat, KeyCode::F(3)), Action::Stay));
        let after = talk_core::i18n::language();
        assert_ne!(before, after, "the language moved to the next one");
        assert!(form.message.is_empty(), "{}", form.message);
        let saved = std::fs::read_to_string(dir.join("config.json")).unwrap();
        assert!(saved.contains(&format!("\"lang\": \"{after}\"")) || saved.contains(&format!("\"lang\":\"{after}\"")), "{saved}");
        // Without a config folder the switch still happens, and the not-saved line says so.
        std::env::remove_var("HERDR_PLUGIN_CONFIG_DIR");
        press(&mut form, &chat, KeyCode::F(3));
        assert_eq!(form.message, t("tui.lang.not_saved", &[]));
        // Put the language back: it is process-wide and other tests read it.
        while talk_core::i18n::language() != before {
            let _ = talk_core::i18n::switch_language();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn f9_opens_a_masked_passphrase_line_and_keys_go_there() {
        let (chat, mut form) = (chat(), form_at(&folder("f9")));
        press(&mut form, &chat, KeyCode::F(9));
        for ch in "pw1".chars() {
            press(&mut form, &chat, KeyCode::Char(ch));
        }
        assert_eq!(form.passphrase.as_deref(), Some("pw1"));
        assert_eq!(form.focus, 1, "the keys do not move the field");
        assert!(lines(&form).iter().any(|(text, _)| text == &t("tui.settings.passphrase", &[("stars", "***")])), "only the length is drawn");
        press(&mut form, &chat, KeyCode::Backspace);
        assert_eq!(form.passphrase.as_deref(), Some("pw"));
        assert!(!lines(&form).iter().any(|(text, _)| text.contains("F9")), "F9 is not named on the screen");
    }

    #[test]
    fn esc_closes_the_passphrase_line_without_leaving() {
        let (chat, mut form) = (chat(), form_at(&folder("f9-esc")));
        press(&mut form, &chat, KeyCode::F(9));
        assert!(matches!(press(&mut form, &chat, KeyCode::Esc), Action::Stay));
        assert!(form.passphrase.is_none());
        assert!(matches!(press(&mut form, &chat, KeyCode::Esc), Action::Leave));
    }

    #[test]
    fn an_empty_passphrase_is_refused_without_a_request() {
        let (chat, mut form) = (chat(), form_at(&folder("f9-empty")));
        press(&mut form, &chat, KeyCode::F(9));
        press(&mut form, &chat, KeyCode::Enter);
        assert_eq!(form.message, t("tui.settings.issue_failed", &[("error", &t("tui.settings.passphrase_empty", &[]))]));
        assert!(form.issuing.is_none());
    }

    #[test]
    fn an_issue_answer_shows_the_code_or_the_reason() {
        let mut form = form_at(&folder("issue-poll"));
        let (sender, receiver) = mpsc::channel();
        form.issuing = Some(receiver);
        sender.send(Ok("ABCD2345".to_string())).unwrap();
        assert!(form.poll());
        assert_eq!(form.message, t("tui.settings.new_code", &[("code", "ABCD2345")]));
        assert!(form.issuing.is_none());
        let (sender, receiver) = mpsc::channel();
        form.issuing = Some(receiver);
        sender.send(Err("the operator passphrase is wrong".to_string())).unwrap();
        assert!(form.poll());
        assert_eq!(form.message, t("tui.settings.issue_failed", &[("error", "the operator passphrase is wrong")]));
    }

    #[test]
    fn create_room_answer_needs_200_and_a_valid_code() {
        assert_eq!(read_room_code(200, r#"{"room_code":"ABCD1234"}"#), Ok("ABCD1234".to_string()));
        assert!(read_room_code(500, r#"{"error":"x"}"#).is_err());
        assert!(read_room_code(200, "not json").is_err());
        assert!(read_room_code(200, r#"{"room_code":"abcd1234"}"#).is_err());
        assert!(read_room_code(200, "{}").is_err());
    }

    #[test]
    fn ctrl_n_without_keys_or_outside_supabase_sends_nothing() {
        let chat = chat();
        let mut form = form_at(&folder("ctrl-n"));
        let ctrl_n = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL);
        on_key(&mut form, ctrl_n, &chat);
        assert_eq!(form.message, t("tui.settings.no_keys", &[]));
        form.mine = Some(("11".repeat(32), "22".repeat(32)));
        on_key(&mut form, ctrl_n, &chat);
        assert_eq!(form.message, t("tui.settings.create_need_connection", &[]), "{}", form.message);
        assert!(form.fields[ROOM_FIELD].is_empty());
    }

    #[test]
    fn a_second_room_cannot_be_created_while_one_is_saved() {
        let dir = folder("second-room");
        let saved = relay_settings::Settings { mode: Mode::Local, room_code: "ABCD2345".into(), ..Default::default() };
        relay_settings::save(&dir, &saved).unwrap();
        let (chat, mut form) = (chat(), form_at(&dir));
        form.mine = Some(("11".repeat(32), "22".repeat(32)));
        on_key(&mut form, KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL), &chat);
        assert_eq!(form.message, t("settings.one_room_only", &[]));
        assert!(form.creating.is_none(), "no room is asked for");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_complete_supabase_setting_is_saved_and_the_first_run_goes_on() {
        let dir = folder("first-run-supabase");
        let chat = chat();
        let mut form = Form::open_supabase(&dir);
        form.url = "http://127.0.0.1:9".into();
        form.key = "dummy-test-key".into();
        form.fields = [String::new(), "12345678".into(), "Alice".into(), "ABCD2345".into()];
        press(&mut form, &chat, KeyCode::Enter);
        assert!(!form.is_saved(), "Enter on a field row only moves on");
        form.focus = FIELDS;
        press(&mut form, &chat, KeyCode::Enter);
        assert!(form.is_saved(), "{}", form.message);
        let saved = relay_settings::load_saved(&dir);
        assert_eq!(saved.mode, Mode::Supabase);
        assert_eq!((saved.url.as_str(), saved.key.as_str()), ("http://127.0.0.1:9", "dummy-test-key"), "no code typed: the saved URL and key are kept");
        assert_eq!(saved.trust, "12345678");
        chat.stop_relay();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A one-shot local relay: reads one request, answers with `response`. Returns its URL.
    fn serve_once(response: String) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_millis(300))).unwrap();
            let mut buf = [0u8; 4096];
            while matches!(stream.read(&mut buf), Ok(n) if n > 0) {}
            let _ = stream.write_all(response.as_bytes());
        });
        url
    }

    /// Runs F2 on the form with the given typed code, the relay answering `response`. The URL env var is only set here.
    fn f2_with_answer(form: &mut Form, chat: &Arc<Chat>, typed: &str, response: String) {
        // The F2 tests share the env var, so they run one at a time.
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::set_var(relay_rest::URL_VARIABLE, serve_once(response));
        form.fields[SETUP_FIELD] = typed.into();
        form.focus = SETUP_FIELD + 1;
        press(form, chat, KeyCode::F(2));
        std::env::remove_var(relay_rest::URL_VARIABLE);
    }

    fn ok_answer() -> String {
        let body = r#"{"url":"http://127.0.0.1:9","key":"dummy-test-key"}"#;
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }

    #[test]
    fn f2_redeems_the_code_and_saves_only_the_connection() {
        let dir = folder("f2-full");
        let chat = chat();
        let mut form = form_at(&dir);
        form.fields[NAME_FIELD] = "Alice".into();
        f2_with_answer(&mut form, &chat, "abcd-2345", ok_answer());
        assert_eq!(form.message, t("tui.settings.setup_saved", &[]));
        assert_eq!((form.url.as_str(), form.key.as_str(), form.setup_code.as_str()), ("http://127.0.0.1:9", "dummy-test-key", "ABCD2345"));
        assert!(form.fields[SETUP_FIELD].is_empty(), "the typed code is cleared once redeemed");
        let saved = relay_settings::load_saved(&dir);
        assert_eq!((saved.mode, saved.url.as_str(), saved.key.as_str(), saved.setup_code.as_str()), (Mode::Supabase, "http://127.0.0.1:9", "dummy-test-key", "ABCD2345"));
        assert!(saved.name.is_empty(), "F2 does not save the other fields (the name typed here waits for Enter)");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_refused_f2_keeps_the_typed_code_and_writes_nothing() {
        let dir = folder("f2-refused");
        let chat = chat();
        let mut form = form_at(&dir);
        f2_with_answer(&mut form, &chat, "abcd-2345", "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into());
        assert_eq!(form.message, t("setup.wrong", &[]));
        assert_eq!(form.fields[SETUP_FIELD], "abcd-2345", "the typed code stays for a fix");
        assert!(form.url.is_empty() && form.key.is_empty());
        assert!(!dir.join(relay_settings::FILE).exists());
    }

    #[test]
    fn enter_after_f2_saves_the_rest_only_when_every_field_is_filled() {
        let dir = folder("f2-then-enter");
        let chat = chat();
        let mut form = form_at(&dir);
        form.mode = Mode::Supabase; // the checks of a Supabase save apply (Local saves keep the fields)
        f2_with_answer(&mut form, &chat, "ABCD2345", ok_answer());
        // Trust and name are still empty: the final save refuses and keeps the connection F2 saved.
        form.fields = [String::new(), String::new(), String::new(), "ABCD2345".into()];
        form.focus = FIELDS;
        press(&mut form, &chat, KeyCode::Enter);
        assert!(!form.is_saved());
        assert!(form.message.contains(&t("tui.settings.label.trust", &[])), "{}", form.message);
        let saved = relay_settings::load_saved(&dir);
        assert_eq!((saved.url.as_str(), saved.key.as_str()), ("http://127.0.0.1:9", "dummy-test-key"));
        assert!(saved.trust.is_empty() && saved.name.is_empty());
        // All fields filled: the final save writes them with the redeemed connection, and the setup code stays.
        form.fields = [String::new(), "12345678".into(), "Alice".into(), "ABCD2345".into()];
        press(&mut form, &chat, KeyCode::Enter);
        assert!(form.is_saved(), "{}", form.message);
        let saved = relay_settings::load_saved(&dir);
        assert_eq!((saved.trust.as_str(), saved.name.as_str(), saved.room_code.as_str()), ("12345678", "Alice", "ABCD2345"));
        assert_eq!((saved.url.as_str(), saved.key.as_str(), saved.setup_code.as_str()), ("http://127.0.0.1:9", "dummy-test-key", "ABCD2345"));
        chat.stop_relay();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
