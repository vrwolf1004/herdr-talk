//! Shared state: this instance, known peers, the lines shown on screen, and typed commands.

use crate::i18n::t;
use crate::net;
use crate::phone;
use crate::store::{self, Trusted};
use crate::text::{char_width, clean_text};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const PEER_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_TEXT_CHARS: usize = 2000;
pub const SEND_WORKERS: usize = 8; // upper limit of peers sent to at the same time
pub const MAX_LINES: usize = 2000;
/// Messages one sender address may deliver within MESSAGE_WINDOW. The rest are dropped.
pub const MESSAGE_LIMIT: usize = 20;
pub const MESSAGE_WINDOW: Duration = Duration::from_secs(10);
/// Leave automatically after this long without any chat activity.
pub const IDLE_LIMIT: Duration = Duration::from_secs(10 * 60);

/// Display names picked at random when no name is given: the list for this PC's language, else every list.
pub const NAMES_BY_LANGUAGE: [(&str, [&str; 10]); 11] = [
    ("ko", ["민준", "서준", "도윤", "예준", "시우", "하준", "지호", "주원", "서연", "서윤"]),
    ("en", ["Liam", "Olivia", "Noah", "Emma", "Oliver", "Ava", "Elijah", "Sophia", "James", "Isabella"]),
    ("zh", ["Haoran", "Yifan", "Ziyi", "Jiahui", "Xinyi", "Yu", "Mei", "Jie", "Lin", "Ting"]),
    ("ja", ["Haruto", "Yui", "Sota", "Hina", "Ren", "Sakura", "Yuto", "Riku", "Aoi", "Mio"]),
    ("vi", ["Anh", "Minh", "Huong", "Linh", "Tuan", "Hoa", "Duc", "Thao", "Phuc", "Lan"]),
    ("fr", ["Louis", "Gabriel", "Jules", "Lea", "Chloe", "Manon", "Lucas", "Hugo", "Camille", "Adam"]),
    ("de", ["Ben", "Mia", "Emil", "Hannah", "Paul", "Lina", "Leon", "Sofia", "Finn", "Greta"]),
    ("es", ["Lucia", "Martin", "Mateo", "Valentina", "Pablo", "Daniela", "Diego", "Paula", "Sergio", "Carmen"]),
    ("ru", ["Mikhail", "Alexander", "Maria", "Artem", "Anna", "Ivan", "Daria", "Dmitry", "Polina", "Egor"]),
    ("id", ["Budi", "Siti", "Agus", "Dewi", "Rizky", "Putri", "Adi", "Nur", "Fajar", "Ayu"]),
    ("pt", ["Miguel", "Helena", "Arthur", "Alice", "Heitor", "Laura", "Bernardo", "Julia", "Davi", "Theo"]),
];

/// Random name that is not in taken. If every name is taken, fall back to the whole list.
pub fn pick_name(names: &[&str], taken: &[String]) -> String {
    let free: Vec<&str> = names.iter().copied().filter(|name| !taken.iter().any(|used| used == name)).collect();
    let pool = if free.is_empty() { names } else { &free };
    pool[fastrand::usize(..pool.len())].to_string()
}

/// Random name from the list matching the PC's language (e.g. Korean for a Korean locale).
pub fn default_name() -> String {
    let language = sys_locale::get_locale().unwrap_or_default().to_lowercase();
    match NAMES_BY_LANGUAGE.iter().find(|(code, _)| language.starts_with(code)) {
        Some((_, names)) => pick_name(names, &[]),
        None => pick_name(&NAMES_BY_LANGUAGE.iter().flat_map(|(_, names)| *names).collect::<Vec<_>>(), &[]),
    }
}

pub fn help_box() -> Vec<String> {
    let keys = [
        "help.title_send", "help.send", "", "help.commands", "help.name", "help.lang", "help.codes",
        "help.peers", "help.trust", "help.phone", "help.help", "help.quit", "", "help.keys", "help.enter", "help.backspace",
        "help.ctrl_l", "help.ctrl_p", "help.ctrl_c",
    ];
    let lines: Vec<String> = keys.iter().map(|key| if key.is_empty() { String::new() } else { t(key, &[]) }).collect();
    let width = lines.iter().map(|line| char_width(line)).max().unwrap_or(0);
    let border = format!("+{}+", "-".repeat(width + 2));
    // Pad by display width, so wide characters (Korean, Japanese) keep the right edge straight.
    let rows = lines.iter().map(|line| format!("| {line}{} |", " ".repeat(width - char_width(line))));
    std::iter::once(border.clone()).chain(rows).chain(std::iter::once(border)).collect()
}

#[derive(Clone)]
pub struct Me {
    pub id: String,
    pub name: String,
    pub port: u16,
    /// Noise public key, announced to peers. The secret half never leaves this process.
    pub key: Vec<u8>,
    pub secret: Vec<u8>,
}

#[derive(Clone)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub seen: Instant,
    /// Announced public key. None for a peer that predates encryption.
    pub key: Option<Vec<u8>>,
}

/// How messages to a peer travel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// Peer has a key the user verified: encrypted.
    Encrypted,
    /// Peer has a key the user has not verified yet: nothing is sent.
    Unverified,
    /// Peer has no key: plain text, only when the user turned plain text on.
    Plain,
}

/// One row of the chat. The sender is kept apart from the text, so the text cannot change who it looks like it came from.
#[derive(Clone)]
pub struct Line {
    pub sender: Option<String>,
    pub text: String,
    pub mine: bool,
}

impl Line {
    /// The row as shown: "sender: text", or the text alone for a notice or a continuation row.
    pub fn render(&self) -> String {
        match &self.sender {
            Some(sender) => format!("{sender}: {}", self.text),
            None => self.text.clone(),
        }
    }
}

/// Set when the chat ends. Network threads watch it and stop.
#[derive(Default)]
pub struct Stop {
    stopped: Mutex<bool>,
    changed: Condvar,
}

impl Stop {
    pub fn set(&self) {
        *self.stopped.lock().unwrap() = true;
        self.changed.notify_all();
    }

    pub fn is_set(&self) -> bool {
        *self.stopped.lock().unwrap()
    }

    pub fn wait(&self, time: Duration) {
        let stopped = self.stopped.lock().unwrap();
        drop(self.changed.wait_timeout_while(stopped, time, |stopped| !*stopped).unwrap());
    }
}

pub struct Chat {
    pub me: Mutex<Me>,
    pub peers: Mutex<Vec<Peer>>,
    pub stop: Stop,
    lines: Mutex<Vec<Line>>,
    last_activity: Mutex<Instant>,
    dirty: AtomicBool,
    /// Arrival times of recent messages, per sender address.
    received: Mutex<HashMap<String, VecDeque<Instant>>>,
    /// Keys the user verified. Set once at start, then changed only by /trust.
    pub(crate) trusted: Mutex<Vec<Trusted>>,
    /// Plain text to peers without a key is allowed only when this is true.
    pub(crate) plaintext: bool,
    /// Where /trust saves the verified keys. None keeps them for this session only.
    pub(crate) state_dir: Option<PathBuf>,
    /// The phone host lock of this PC, while this node is the host (방장).
    host: Mutex<Option<phone::Host>>,
    /// Until when the phone address and token are on screen. Only the screen shows them: never a chat line, never a log.
    phone_until: Mutex<Option<Instant>>,
    /// When the name last changed by itself because of a clash. Names the user types do not count.
    last_auto_rename: Mutex<Option<Instant>>,
    /// The relay thread's inbox and its stop flag, while a relay runs (see relay_chat).
    relay_inbox: Mutex<Option<(Sender<String>, Arc<AtomicBool>)>>,
    /// What the status row says about the relay ("" when it is off).
    relay_status: Mutex<String>,
    /// True in Supabase mode: messages go only through the relay, never over the LAN.
    supabase: AtomicBool,
    /// One SAS nonce per peer, with its commitment for that peer: (nonce, commitment). Never shared with another peer.
    my_nonces: Mutex<HashMap<Vec<u8>, ([u8; 32], [u8; 32])>>,
    /// The commitment each peer gave us for us, pinned at first sight by its public key. A different one is refused.
    commits: Mutex<HashMap<Vec<u8>, [u8; 32]>>,
    /// (peer key, commitment) pairs already reported as refused, so the warning comes once.
    refused: Mutex<std::collections::HashSet<(Vec<u8>, [u8; 32])>>,
    /// The nonce each peer revealed, checked against its commitment, by its public key.
    reveals: Mutex<HashMap<Vec<u8>, [u8; 32]>>,
    /// Peers whose reveal is being fetched right now (one worker per peer).
    revealing: Mutex<std::collections::HashSet<Vec<u8>>>,
    /// Reveals asked for by the announce parser (key, address, port); the listener starts their workers.
    reveal_requests: Mutex<Vec<(Vec<u8>, String, u16)>>,
}


/// A fresh random nonce for the SAS commitment. Without a random source the code could be ground, so this panics instead.
fn random_nonce() -> [u8; 32] {
    let mut nonce = [0u8; 32];
    getrandom::getrandom(&mut nonce).expect("system random source for the SAS nonce");
    nonce
}

/// How long the phone address and token stay on screen when nobody hides them.
const PHONE_SHOWN: Duration = Duration::from_secs(30);

/// The shortest time between two automatic renames.
const AUTO_RENAME_GAP: Duration = Duration::from_secs(60);

impl Chat {
    pub fn new(me: Me) -> Self {
        Chat {
            me: Mutex::new(me),
            peers: Mutex::new(Vec::new()),
            stop: Stop::default(),
            lines: Mutex::new(Vec::new()),
            last_activity: Mutex::new(Instant::now()),
            dirty: AtomicBool::new(true),
            received: Mutex::new(HashMap::new()),
            trusted: Mutex::new(Vec::new()),
            plaintext: false,
            state_dir: None,
            host: Mutex::new(None),
            phone_until: Mutex::new(None),
            last_auto_rename: Mutex::new(None),
            relay_inbox: Mutex::new(None),
            relay_status: Mutex::new(String::new()),
            supabase: AtomicBool::new(false),
            my_nonces: Mutex::new(HashMap::new()),
            commits: Mutex::new(HashMap::new()),
            refused: Mutex::new(std::collections::HashSet::new()),
            reveals: Mutex::new(HashMap::new()),
            revealing: Mutex::new(std::collections::HashSet::new()),
            reveal_requests: Mutex::new(Vec::new()),
        }
    }

    /// This node's nonce for one peer, made on first use. The commitment for that peer is in the announce.
    pub fn my_nonce_for(&self, key: &[u8]) -> [u8; 32] {
        let own = self.me.lock().unwrap().key.clone();
        self.my_nonces.lock().unwrap().entry(key.to_vec()).or_insert_with(|| {
            let nonce = random_nonce();
            (nonce, store::commit(&own, &nonce))
        }).0
    }

    /// The commitment this node makes for one peer (SHA-256 over this PC's key and the nonce kept for that peer).
    pub fn my_commit_for(&self, key: &[u8]) -> [u8; 32] {
        let _ = self.my_nonce_for(key);
        self.my_nonces.lock().unwrap().get(key).map(|entry| entry.1).expect("nonce made just above")
    }

    /// One (peer key, commitment) pair per peer, hex: what the announce carries in its `commits` map.
    pub fn announced_commits(&self) -> Vec<(String, String)> {
        self.my_nonces.lock().unwrap().iter().map(|(key, entry)| (store::to_hex(key), store::to_hex(&entry.1))).collect()
    }

    /// Pin the commitment a peer gave us for us, at first sight. True when it is pinned now or was already this one;
    /// false for a different commitment (the caller refuses that peer).
    pub fn pin_commit(&self, key: &[u8], commit: [u8; 32]) -> bool {
        let mut map = self.commits.lock().unwrap();
        match map.get(key) {
            Some(pinned) => *pinned == commit,
            None => {
                map.insert(key.to_vec(), commit);
                true
            }
        }
    }

    /// True the first time a refused (key, commitment) pair is seen, so the warning is shown once.
    pub fn refuse_once(&self, key: &[u8], commit: [u8; 32]) -> bool {
        self.refused.lock().unwrap().insert((key.to_vec(), commit))
    }

    pub fn commit_of(&self, key: &[u8]) -> Option<[u8; 32]> {
        self.commits.lock().unwrap().get(key).copied()
    }

    /// The nonce a peer revealed to us, once it was checked against its commitment.
    pub fn reveal_of(&self, key: &[u8]) -> Option<[u8; 32]> {
        self.reveals.lock().unwrap().get(key).copied()
    }

    /// Keep a peer's revealed nonce once it matched the commitment. True when it was new.
    pub fn set_reveal(&self, key: &[u8], nonce: [u8; 32]) -> bool {
        self.reveals.lock().unwrap().insert(key.to_vec(), nonce).is_none()
    }

    /// The code for a peer, once both nonces are known (after the reveal exchange). None before that.
    pub fn sas_for(&self, key: &[u8]) -> Option<String> {
        let peer_nonce = *self.reveals.lock().unwrap().get(key)?;
        let own_nonce = self.my_nonces.lock().unwrap().get(key).map(|entry| entry.0)?;
        let own = self.me.lock().unwrap().key.clone();
        Some(store::sas_code(&own, &own_nonce, key, &peer_nonce))
    }

    /// Start the reveal worker for a peer. False when one is already running for it.
    pub fn start_reveal(&self, key: &[u8]) -> bool {
        self.revealing.lock().unwrap().insert(key.to_vec())
    }

    /// Ask for a peer's reveal (the announce parser cannot start a thread itself).
    pub fn request_reveal(&self, key: &[u8], ip: &str, port: u16) {
        self.reveal_requests.lock().unwrap().push((key.to_vec(), ip.to_string(), port));
    }

    pub fn take_reveal_requests(&self) -> Vec<(Vec<u8>, String, u16)> {
        std::mem::take(&mut *self.reveal_requests.lock().unwrap())
    }

    pub fn finish_reveal(&self, key: &[u8]) {
        self.revealing.lock().unwrap().remove(key);
    }

    /// True while this node holds the phone host lock.
    pub fn is_host(&self) -> bool {
        self.host.lock().unwrap().is_some()
    }

    /// Become the host if no node on this PC is. Shows the phone address once when it succeeds. Returns true on success.
    pub fn try_become_host(&self) -> bool {
        let Some(state) = &self.state_dir else { return false };
        if self.stop.is_set() || self.is_host() {
            return false;
        }
        let port = self.me.lock().unwrap().port;
        match phone::Host::try_take(state, port) {
            Ok(Some(host)) => {
                *self.host.lock().unwrap() = Some(host);
                self.show_phone();
                true
            }
            _ => false,
        }
    }

    /// Give the host lock back (the chat is ending). The next node takes over.
    pub fn release_host(&self) {
        self.host.lock().unwrap().take();
    }

    /// Keep taking the host lock while another node holds it, so a node takes over when the host ends.
    /// shortcut: whichever waiting node polls first wins, not strictly the oldest; upgrade if the order must be fixed.
    pub fn watch_host(self: &Arc<Self>) {
        let chat = self.clone();
        thread::spawn(move || {
            while !chat.stop.is_set() {
                chat.try_become_host();
                chat.stop.wait(Duration::from_secs(2));
            }
        });
    }

    pub fn show_phone(&self) {
        *self.phone_until.lock().unwrap() = Some(Instant::now() + PHONE_SHOWN);
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub fn hide_phone(&self) {
        if self.phone_until.lock().unwrap().take().is_some() {
            self.dirty.store(true, Ordering::Relaxed);
        }
    }

    pub fn toggle_phone(&self) {
        if self.phone_visible() {
            self.hide_phone();
        } else {
            self.show_phone();
        }
    }

    /// True while the phone address should be on screen. Hides it, and asks for a redraw, when its time is up.
    pub fn phone_visible(&self) -> bool {
        let mut until = self.phone_until.lock().unwrap();
        match *until {
            Some(at) if at > Instant::now() => true,
            Some(_) => {
                *until = None;
                self.dirty.store(true, Ordering::Relaxed);
                false
            }
            None => false,
        }
    }

    /// The rows to draw over the screen, read from `phone.json` now, so the token is in no chat line or copy.
    pub fn phone_overlay(&self) -> Option<Vec<String>> {
        if !self.phone_visible() {
            return None;
        }
        let Some(phone) = self.state_dir.as_deref().and_then(phone::read) else {
            return Some(vec![t("phone.none", &[])]);
        };
        let mut rows = vec![t("phone.title", &[])];
        rows.extend(phone.urls.iter().map(|url| format!("  {url}")));
        rows.push(t("phone.token", &[("token", &phone.token)]));
        rows.push(t("phone.hint", &[]));
        Some(rows)
    }

    /// True for a key the user verified, and for this PC's own key (another chat pane on this PC).
    pub fn trusts(&self, key: &[u8]) -> bool {
        let own = self.me.lock().unwrap().key == key;
        own || self.trusted.lock().unwrap().iter().any(|trusted| trusted.key == key)
    }

    /// The name the user verified this key under. None for a key that was not verified, this PC's own key included.
    pub fn trusted_name(&self, key: &[u8]) -> Option<String> {
        self.trusted.lock().unwrap().iter().find(|trusted| trusted.key == key).map(|trusted| trusted.name.clone())
    }

    /// The name and address of a fresh peer with this key (for the code line shown after the reveal).
    pub(crate) fn peer_by_key(&self, key: &[u8]) -> Option<(String, String)> {
        self.fresh_peers().into_iter().find(|peer| peer.key.as_deref() == Some(key)).map(|peer| (peer.name, peer.ip))
    }

    pub fn link_of(&self, peer: &Peer) -> Link {
        match &peer.key {
            None => Link::Plain,
            Some(key) if self.trusts(key) => Link::Encrypted,
            Some(_) => Link::Unverified,
        }
    }

    /// Accept a message connection only from a fresh peer's address, and only up to MESSAGE_LIMIT per window.
    // shortcut: a peer whose announce has not arrived yet loses its first message, the limit is per IP; upgrade if messages go missing after a join.
    pub fn admit_sender(&self, ip: &str) -> bool {
        if !self.fresh_peers().iter().any(|peer| peer.ip == ip) {
            return false;
        }
        let mut received = self.received.lock().unwrap();
        let times = received.entry(ip.to_string()).or_default();
        while times.front().is_some_and(|at| at.elapsed() >= MESSAGE_WINDOW) {
            times.pop_front();
        }
        if times.len() >= MESSAGE_LIMIT {
            return false;
        }
        times.push_back(Instant::now());
        true
    }

    /// Add rows together, so one message's rows never interleave with another's.
    pub(crate) fn add(&self, rows: Vec<Line>) {
        *self.last_activity.lock().unwrap() = Instant::now();
        let mut lines = self.lines.lock().unwrap();
        lines.extend(rows);
        let extra = lines.len().saturating_sub(MAX_LINES);
        lines.drain(..extra);
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Hand a running relay its inbox and stop flag. Replaces nothing: call stop_relay first.
    pub(crate) fn attach_relay(&self, inbox: Sender<String>, stop: Arc<AtomicBool>) {
        *self.relay_inbox.lock().unwrap() = Some((inbox, stop));
    }

    /// Stop the running relay thread (it exits at its next check) and forget its inbox.
    pub fn stop_relay(&self) {
        if let Some((_, stop)) = self.relay_inbox.lock().unwrap().take() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    pub fn set_relay_status(&self, text: &str) {
        *self.relay_status.lock().unwrap() = text.to_string();
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// The relay state for the status row ("" when the relay is off).
    pub fn relay_status(&self) -> String {
        self.relay_status.lock().unwrap().clone()
    }

    /// The connection mode for this run: true in Supabase mode.
    pub fn set_supabase_mode(&self, on: bool) {
        self.supabase.store(on, Ordering::Relaxed);
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub fn is_supabase(&self) -> bool {
        self.supabase.load(Ordering::Relaxed)
    }

    /// "supabase" or "local", as the control channel reports it.
    pub fn mode_name(&self) -> &'static str {
        if self.is_supabase() { "supabase" } else { "local" }
    }

    /// Hand one plain message to the running relay. Without a running relay the user is told it was not sent.
    pub fn relay_send(&self, text: &str) {
        match self.relay_inbox.lock().unwrap().as_ref() {
            Some((inbox, _)) => {
                let _ = inbox.send(text.to_string());
            }
            None => self.say(t("relay.not_sent", &[])),
        }
    }

    /// Show one notice line in the chat.
    pub fn say(&self, text: impl Into<String>) {
        self.add(vec![Line { sender: None, text: text.into(), mine: false }]);
    }

    pub fn lines(&self) -> Vec<Line> {
        self.lines.lock().unwrap().clone()
    }

    pub fn clear_lines(&self) {
        self.lines.lock().unwrap().clear();
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// True once after any change to the lines. The screen redraws on it.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::Relaxed)
    }

    pub fn idle_for(&self) -> Duration {
        self.last_activity.lock().unwrap().elapsed()
    }

    pub fn fresh_peers(&self) -> Vec<Peer> {
        let peers = self.peers.lock().unwrap();
        peers.iter().filter(|peer| peer.seen.elapsed() < PEER_TIMEOUT).cloned().collect()
    }

    /// Handle one line typed by the user. Return false to leave the chat.
    pub fn handle(&self, text: &str) -> bool {
        self.handle_with(text, net::send_to)
    }

    fn handle_with<F>(&self, text: &str, send: F) -> bool
    where
        F: Fn(&Peer, &Me, &str) -> io::Result<()> + Sync,
    {
        let text = text.trim();
        if text.is_empty() {
            return true;
        }
        if text.starts_with('/') {
            self.say(format!("> {text}"));
        }
        if text == "/quit" {
            return false;
        }
        if text == "/help" {
            help_box().into_iter().for_each(|line| self.say(line));
            return true;
        }
        if text == "/name" || text.starts_with("/name ") {
            let new_name = clean_text(text[5..].trim(), 32).trim().to_string();
            if new_name.is_empty() {
                self.say(t("usage.name", &[]));
            } else {
                self.rename_by_user(new_name);
            }
            return true;
        }
        if text == "/lang" || text.starts_with("/lang ") {
            let code = text[5..].trim().to_lowercase();
            let Some((_, names)) = NAMES_BY_LANGUAGE.iter().find(|(name, _)| *name == code) else {
                let codes: Vec<&str> = NAMES_BY_LANGUAGE.iter().map(|(name, _)| *name).collect();
                self.say(t("usage.lang", &[("codes", &codes.join("|"))]));
                return true;
            };
            let mut taken: Vec<String> = self.fresh_peers().into_iter().map(|peer| peer.name).collect();
            taken.push(self.me.lock().unwrap().name.clone());
            self.rename_by_user(pick_name(names, &taken));
            return true;
        }
        if text == "/peers" {
            let found = self.fresh_peers();
            if found.is_empty() {
                self.say(t("peers.none", &[]));
            }
            for peer in found {
                let state = match (self.link_of(&peer), &peer.key) {
                    (Link::Encrypted, _) => t("peers.verified", &[]),
                    (Link::Plain, _) => t("peers.plain", &[]),
                    // The code exists only after the reveal exchange; until then the line says it is pending.
                    (Link::Unverified, Some(key)) => match self.sas_for(key) {
                        Some(code) => t("peers.unverified", &[("code", &code)]),
                        None => "(code pending)".to_string(), // shortcut: English-only pending text; move it to the locales
                    },
                    (Link::Unverified, None) => String::new(),
                };
                self.say(format!("- {} ({}:{}) {state}", peer.name, peer.ip, peer.port));
            }
            return true;
        }
        if text == "/phone" || text.starts_with("/phone ") {
            match text[6..].trim() {
                "" => self.show_phone(),
                "new" => match self.state_dir.as_deref().map(phone::regenerate_token) {
                    Some(Ok(Some(()))) => {
                        self.say(t("phone.new_done", &[]));
                        self.show_phone();
                    }
                    _ => self.say(t("phone.none", &[])),
                },
                _ => self.say(t("usage.phone", &[])),
            }
            return true;
        }
        if text == "/trust" || text.starts_with("/trust ") {
            let code = store::normalize_code(&text[6..]);
            if code.is_empty() {
                self.say(t("usage.trust", &[]));
            } else {
                self.trust(&code);
            }
            return true;
        }
        let text = text.chars().take(MAX_TEXT_CHARS).collect::<String>().replace('\n', " ");
        if self.is_supabase() {
            // Supabase mode: the relay is the only path, so the LAN peers get nothing.
            let name = self.me.lock().unwrap().name.clone();
            self.add(vec![Line { sender: Some(format!("{name} {}", t("chat.you", &[]))), text: text.clone(), mine: true }]);
            self.relay_send(&text);
            return true;
        }
        let found = self.fresh_peers();
        if found.is_empty() {
            self.say(t("peers.none_hint", &[]));
            return true;
        }
        // Only peers this link allows get the message. Each skipped peer says why.
        let mut targets = Vec::new();
        for peer in found {
            match self.link_of(&peer) {
                Link::Encrypted => targets.push(peer),
                Link::Plain if self.plaintext => targets.push(peer),
                Link::Plain => self.say(t("peer.plain_off", &[("name", &peer.name)])),
                Link::Unverified => self.say(t("peer.unverified", &[("name", &peer.name)])),
            }
        }
        if targets.is_empty() {
            return true;
        }
        let me = self.me.lock().unwrap().clone();
        // A silent peer must not hold up the others, so send at the same time, at most SEND_WORKERS.
        // Failure lines still come in peer order.
        let next = AtomicUsize::new(0);
        let sent: Vec<AtomicBool> = targets.iter().map(|_| AtomicBool::new(false)).collect();
        thread::scope(|scope| {
            for _ in 0..SEND_WORKERS.min(targets.len()) {
                scope.spawn(|| loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(peer) = targets.get(index) else { return };
                    sent[index].store(send(peer, &me, &text).is_ok(), Ordering::Relaxed);
                });
            }
        });
        for (peer, sent) in targets.iter().zip(&sent) {
            if !sent.load(Ordering::Relaxed) {
                self.say(t("peer.unreachable", &[("name", &peer.name)]));
            }
        }
        self.add(vec![Line { sender: Some(format!("{} {}", me.name, t("chat.you", &[]))), text: text.to_string(), mine: true }]);
        true
    }

    /// Verify the unverified LAN peer whose code matches: remember its key, and say so.
    fn trust(&self, code: &str) {
        let lan: Vec<(String, Vec<u8>, String)> = self.fresh_peers().into_iter().filter_map(|peer| {
            let key = peer.key.clone()?;
            let matches = self.sas_for(&key).is_some_and(|sas| store::normalize_code(&sas) == code) && self.link_of(&peer) == Link::Unverified;
            matches.then_some((peer.name, key, peer.ip))
        }).collect();
        // Two different keys with the same code would mean a collision or an attacker: refuse rather than pick one.
        let distinct: std::collections::BTreeSet<&Vec<u8>> = lan.iter().map(|(_, key, _)| key).collect();
        if distinct.len() > 1 {
            self.say(t("trust.ambiguous", &[("code", code)]));
            return;
        }
        let Some((name, key, ip)) = lan.into_iter().next() else {
            self.say(t("trust.none", &[("code", code)]));
            return;
        };
        let mut trusted = self.trusted.lock().unwrap();
        trusted.push(Trusted { name: name.clone(), key, ip: ip.clone() });
        let saved = match &self.state_dir {
            Some(dir) => store::write_trusted(dir, &trusted),
            None => Ok(()),
        };
        drop(trusted);
        match saved {
            Ok(()) => self.say(t("trust.done", &[("name", &name), ("ip", &ip)])),
            Err(error) => self.say(t("trust.save_failed", &[("error", &error.to_string())])),
        }
    }

    /// Every way the name changes (/name, /lang, a clash found later) ends here, so one rule covers them all.
    fn rename(&self, new_name: String) {
        let old = std::mem::replace(&mut self.me.lock().unwrap().name, new_name.clone());
        self.say(t("name.changed", &[("old", &old), ("new", &new_name)]));
    }

    /// A name the user chose (/name, /lang): changed at once, then any clash it causes is settled.
    fn rename_by_user(&self, new_name: String) {
        self.rename(new_name);
        self.settle_name(false);
    }

    /// When a verified fresh peer has this chat's name and a smaller id, this chat gives the name up (the Python
    /// edition's rule), so of two nodes only the one with the larger id renames. Both sides compute the same answer.
    /// Two panes on one PC share an id (it comes from the key), so for equal ids the port decides the same way.
    /// The new name is the old one with a number: Name2, then Name3 if that is taken too.
    /// This changes the name by itself at most once per AUTO_RENAME_GAP, so announcements cannot keep it moving.
    pub fn resolve_name_conflict(&self) {
        self.settle_name(true);
    }

    fn settle_name(&self, limited: bool) {
        let me = self.me.lock().unwrap().clone();
        let peers = self.fresh_peers();
        // Verified: the announced key hashes to the announced id. A peer without a key proves nothing, so it cannot
        // take a name from anyone.
        let verified = |peer: &Peer| peer.key.as_deref().is_some_and(|key| store::peer_id(key) == peer.id);
        let beaten = peers.iter().any(|peer| verified(peer) && peer.name == me.name && (peer.id.as_str(), peer.port) < (me.id.as_str(), me.port));
        if !beaten {
            return;
        }
        let mut last = self.last_auto_rename.lock().unwrap();
        if limited && last.is_some_and(|at| at.elapsed() < AUTO_RENAME_GAP) {
            return;
        }
        if limited {
            *last = Some(Instant::now());
        }
        drop(last);
        let taken: Vec<String> = peers.into_iter().map(|peer| peer.name).collect();
        self.rename(numbered_name(&me.name, &taken));
    }
}

/// The first 30 characters of the name with the lowest number from 2 up that is not in taken. Same rule as
/// the Python edition's unique_name: a number already at the end stays (Room101 becomes Room1012).
pub fn numbered_name(name: &str, taken: &[String]) -> String {
    let base: String = name.chars().take(30).collect();
    (2..).map(|number| format!("{base}{number}")).find(|candidate| !taken.contains(candidate)).expect("an unused number exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(names: &[&str]) -> Chat {
        let chat = Chat::new(Me { id: "me".into(), name: "Me".into(), port: 1, key: vec![0; 32], secret: vec![1; 32] });
        *chat.peers.lock().unwrap() = names.iter().enumerate().map(|(index, name)| Peer {
            id: format!("id{index}"),
            name: name.to_string(),
            ip: "10.0.0.1".into(),
            port: 52311,
            seen: Instant::now(),
            key: None,
        }).collect();
        chat
    }

    /// Two keys whose ids sort low < high, so a test knows which node must keep the name.
    fn keys_low_high() -> ([u8; 32], [u8; 32]) {
        let (a, b) = ([1u8; 32], [2u8; 32]);
        if store::peer_id(&a) < store::peer_id(&b) { (a, b) } else { (b, a) }
    }

    /// A chat named `name` with this key and port, that sees one fresh verified peer of the same name.
    fn clashing(key: [u8; 32], port: u16, name: &str, peer_key: [u8; 32], peer_port: u16) -> Chat {
        let chat = Chat::new(Me { id: store::peer_id(&key), name: name.into(), port, key: key.to_vec(), secret: vec![1; 32] });
        chat.peers.lock().unwrap().push(Peer { name: name.into(), port: peer_port, ..keyed_peer(name, &peer_key) });
        chat
    }

    fn my_name(chat: &Chat) -> String {
        chat.me.lock().unwrap().name.clone()
    }

    #[test]
    fn numbered_name_counts_up_and_keeps_a_trailing_number() {
        let taken = |names: &[&str]| names.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(numbered_name("Jiho", &taken(&["Jiho"])), "Jiho2");
        assert_eq!(numbered_name("Jiho", &taken(&["Jiho", "Jiho2"])), "Jiho3");
        assert_eq!(numbered_name("Room101", &taken(&["Room101"])), "Room1012");
        assert_eq!(numbered_name("Room101", &taken(&["Room101", "Room1012"])), "Room1013");
        assert_eq!(numbered_name("Jiho2", &taken(&["Jiho2"])), "Jiho22");
        assert_eq!(numbered_name("12", &taken(&[])), "122");
        // Only the first 30 characters are kept, as in Python.
        assert_eq!(numbered_name(&"x".repeat(32), &taken(&[])), format!("{}2", "x".repeat(30)));
    }

    #[test]
    fn of_two_nodes_with_one_name_only_the_larger_id_renames() {
        // Like the Python edition's test: the smaller id keeps its name, the larger renames, whatever the ports.
        let (lo, hi) = keys_low_high();
        let (low, high) = (clashing(lo, 9, "Jiho", hi, 1), clashing(hi, 1, "Jiho", lo, 9));
        low.resolve_name_conflict();
        high.resolve_name_conflict();
        assert_eq!((my_name(&low), my_name(&high)), ("Jiho".to_string(), "Jiho2".to_string()));
        // Once renamed there is no clash any more.
        high.resolve_name_conflict();
        assert_eq!(my_name(&high), "Jiho2");
        // The same id (two panes on one PC) falls back to the port: the larger port renames.
        let (low, high) = (clashing(lo, 1, "Jiho", lo, 2), clashing(lo, 2, "Jiho", lo, 1));
        low.resolve_name_conflict();
        high.resolve_name_conflict();
        assert_eq!((my_name(&low), my_name(&high)), ("Jiho".to_string(), "Jiho2".to_string()));
    }

    #[test]
    fn a_taken_number_is_skipped_and_a_stale_peer_does_not_count() {
        let (lo, hi) = keys_low_high();
        let chat = clashing(hi, 1, "Jiho", lo, 1);
        chat.peers.lock().unwrap().push(keyed_peer("Jiho2", &[9; 32]));
        chat.resolve_name_conflict();
        assert_eq!(my_name(&chat), "Jiho3");
        let stale = clashing(hi, 1, "Jiho", lo, 1);
        stale.peers.lock().unwrap()[0].seen = Instant::now() - PEER_TIMEOUT;
        stale.resolve_name_conflict();
        assert_eq!(my_name(&stale), "Jiho");
    }

    #[test]
    fn a_peer_without_a_verified_key_cannot_take_my_name() {
        let (lo, hi) = keys_low_high();
        // No key at all (a plain-text peer), even with an id that sorts first.
        let chat = clashing(hi, 1, "Jiho", lo, 1);
        chat.peers.lock().unwrap()[0].key = None;
        chat.resolve_name_conflict();
        assert_eq!(my_name(&chat), "Jiho");
        // A key that does not hash to the announced id.
        let chat = clashing(hi, 1, "Jiho", lo, 1);
        chat.peers.lock().unwrap()[0].key = Some(vec![9; 32]);
        chat.resolve_name_conflict();
        assert_eq!(my_name(&chat), "Jiho");
        // The same peer with its real key does count.
        let chat = clashing(hi, 1, "Jiho", lo, 1);
        chat.resolve_name_conflict();
        assert_eq!(my_name(&chat), "Jiho2");
    }

    #[test]
    fn a_keyless_peer_cannot_start_a_clash_but_its_name_stays_taken() {
        // Jiho2 is used by a keyless peer: it starts nothing, but I still avoid showing a second Jiho2.
        let (lo, hi) = keys_low_high();
        let chat = clashing(hi, 1, "Jiho", lo, 1);
        let mut keyless = keyed_peer("Jiho2", &[9; 32]);
        keyless.key = None;
        chat.peers.lock().unwrap().push(keyless);
        chat.resolve_name_conflict();
        assert_eq!(my_name(&chat), "Jiho3");
    }

    #[test]
    fn an_automatic_rename_happens_once_per_minute() {
        let (lo, hi) = keys_low_high();
        let chat = clashing(hi, 1, "Jiho", lo, 1);
        chat.resolve_name_conflict();
        assert_eq!(my_name(&chat), "Jiho2");
        // A second verified peer takes the new name at once: the limit holds the name still.
        chat.peers.lock().unwrap().push(Peer { name: "Jiho2".into(), port: 7, ..keyed_peer("Jiho2", &lo) });
        chat.resolve_name_conflict();
        assert_eq!(my_name(&chat), "Jiho2");
        // After the gap it may change again.
        *chat.last_auto_rename.lock().unwrap() = Some(Instant::now() - AUTO_RENAME_GAP);
        chat.resolve_name_conflict();
        assert_eq!(my_name(&chat), "Jiho22"); // numbered from the current name, as in Python
    }

    #[test]
    fn name_command_follows_the_same_rule_and_ignores_the_limit() {
        let (lo, hi) = keys_low_high();
        // /name onto a name a smaller-id node already uses: this one ends up numbered.
        let chat = clashing(hi, 1, "Other", lo, 1);
        chat.peers.lock().unwrap()[0].name = "Alice".into();
        chat.handle("/name Alice");
        assert_eq!(my_name(&chat), "Alice2");
        // The user's own rename neither waits for nor starts the 60 seconds.
        assert!(chat.last_auto_rename.lock().unwrap().is_none());
        *chat.last_auto_rename.lock().unwrap() = Some(Instant::now());
        chat.peers.lock().unwrap()[0].name = "Bob".into();
        chat.handle("/name Bob");
        assert_eq!(my_name(&chat), "Bob2");
        let keeps = clashing(lo, 1, "Other", hi, 1);
        keeps.peers.lock().unwrap()[0].name = "Alice".into();
        keeps.handle("/name Alice");
        assert_eq!(my_name(&keeps), "Alice");
    }

    /// Give a peer its reveal, as the LAN exchange does (our nonce for it is made first). Returns the code both sides then see.
    fn reveal_peer(chat: &Chat, key: &[u8]) -> String {
        let nonce = [9u8; 32];
        chat.my_nonce_for(key);
        assert!(chat.set_reveal(key, nonce));
        chat.sas_for(key).expect("code after the reveal")
    }

    fn keyed_peer(name: &str, key: &[u8]) -> Peer {
        Peer { id: store::peer_id(key), name: name.into(), ip: "10.0.0.2".into(), port: 52311, seen: Instant::now(), key: Some(key.to_vec()) }
    }

    fn texts(chat: &Chat) -> Vec<String> {
        chat.lines().into_iter().map(|line| line.render()).collect()
    }

    #[test]
    fn pick_name_avoids_taken_and_falls_back() {
        let taken = ["a".to_string(), "b".to_string()];
        for _ in 0..20 {
            assert_eq!(pick_name(&["a", "b", "c"], &taken), "c");
            assert!(["a", "b"].contains(&pick_name(&["a", "b"], &taken).as_str()));
        }
    }

    #[test]
    fn default_name_comes_from_a_list() {
        let name = default_name();
        assert!(NAMES_BY_LANGUAGE.iter().any(|(_, names)| names.contains(&name.as_str())));
    }

    #[test]
    fn help_prints_the_box() {
        let chat = chat(&[]);
        assert!(chat.handle("/help"));
        assert_eq!(texts(&chat)[1..], help_box());
        let width = char_width(&help_box()[0]);
        assert!(help_box().iter().all(|row| char_width(row) == width));
    }

    #[test]
    fn name_change_and_quit() {
        let chat = chat(&[]);
        chat.handle("/name Alice");
        assert_eq!(chat.me.lock().unwrap().name, "Alice");
        assert!(!chat.handle("/quit"));
    }

    #[test]
    fn lang_avoids_names_of_fresh_peers() {
        let names = NAMES_BY_LANGUAGE[0].1;
        let chat = chat(&names[1..]);
        chat.handle("/lang ko");
        assert_eq!(chat.me.lock().unwrap().name, names[0]);
    }

    #[test]
    fn activity_restarts_the_idle_timer() {
        let chat = chat(&[]);
        *chat.last_activity.lock().unwrap() = Instant::now() - Duration::from_secs(100);
        chat.say("x");
        assert!(chat.idle_for() < Duration::from_secs(5));
    }

    #[test]
    fn old_peers_are_dropped() {
        let chat = chat(&["a"]);
        chat.peers.lock().unwrap()[0].seen = Instant::now() - PEER_TIMEOUT;
        assert!(chat.fresh_peers().is_empty());
    }

    #[test]
    fn concurrent_adds_keep_the_list_consistent() {
        let chat = chat(&[]);
        thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| (0..700).for_each(|_| chat.say("x")));
            }
        });
        assert_eq!(chat.lines().len(), MAX_LINES);
    }

    #[test]
    fn silent_peers_are_sent_to_at_once() {
        let mut chat = chat(&["a", "b", "c"]);
        chat.plaintext = true;
        let started = Instant::now();
        chat.handle_with("hello", |_, _, _| {
            thread::sleep(Duration::from_millis(300));
            Ok(())
        });
        assert!(started.elapsed() < Duration::from_millis(600));
    }

    #[test]
    fn failures_keep_peer_order_and_the_record_line_is_last() {
        let mut chat = chat(&["a", "b", "c"]);
        chat.plaintext = true;
        let delivered = Mutex::new(Vec::new());
        chat.handle_with("hello", |peer, _, _| {
            if peer.name == "b" {
                delivered.lock().unwrap().push(peer.name.clone());
                return Ok(());
            }
            Err(io::Error::other("unreachable"))
        });
        let lines = chat.lines();
        assert_eq!(*delivered.lock().unwrap(), ["b"]);
        assert_eq!(lines[0].text, t("peer.unreachable", &[("name", "a")]));
        assert_eq!(lines[1].text, t("peer.unreachable", &[("name", "c")]));
        assert!(lines[2].mine && lines[2].text.ends_with("hello"));
    }

    #[test]
    fn no_more_than_send_workers_at_once() {
        let names: Vec<String> = (0..SEND_WORKERS + 4).map(|index| format!("p{index}")).collect();
        let mut chat = chat(&names.iter().map(String::as_str).collect::<Vec<_>>());
        chat.plaintext = true;
        let (active, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
        chat.handle_with("hello", |_, _, _| {
            most.fetch_max(active.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(100));
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        });
        assert_eq!(most.load(Ordering::SeqCst), SEND_WORKERS);
    }

    #[test]
    fn no_peers_sends_nothing() {
        let chat = chat(&[]);
        chat.handle_with("hello", |_, _, _| panic!("no peer to send to"));
        assert_eq!(texts(&chat), [t("peers.none_hint", &[])]);
    }

    #[test]
    fn commands_are_echoed_before_they_run() {
        let chat = chat(&[]);
        chat.handle("  /peers ");
        assert_eq!(texts(&chat)[0], "> /peers");
    }

    #[test]
    fn name_is_cut_to_32_and_controls_removed() {
        let chat = chat(&[]);
        chat.handle(&format!("/name {}", "x".repeat(40)));
        assert_eq!(chat.me.lock().unwrap().name, "x".repeat(32));
        chat.handle("/name a\x1bb");
        assert_eq!(chat.me.lock().unwrap().name, "a b");
    }

    #[test]
    fn plain_peers_get_nothing_unless_plain_text_is_on() {
        let mut chat = chat(&["a"]);
        chat.handle_with("hello", |_, _, _| panic!("plain text is off"));
        assert_eq!(texts(&chat), [t("peer.plain_off", &[("name", "a")])]);
        chat.plaintext = true;
        let sent = Mutex::new(0);
        chat.handle_with("hello", |_, _, _| {
            *sent.lock().unwrap() += 1;
            Ok(())
        });
        assert_eq!(*sent.lock().unwrap(), 1);
    }

    #[test]
    fn unverified_keyed_peer_is_never_sent_to_even_with_plain_text_on() {
        let mut chat = chat(&[]);
        chat.plaintext = true;
        chat.peers.lock().unwrap().push(keyed_peer("Bob", &[7; 32]));
        chat.handle_with("hello", |_, _, _| panic!("unverified peer must not be sent to"));
        assert_eq!(texts(&chat)[0], t("peer.unverified", &[("name", "Bob")]));
    }

    #[test]
    fn trust_with_the_matching_code_then_send_encrypted() {
        let chat = chat(&[]);
        let key = [7u8; 32];
        chat.peers.lock().unwrap().push(keyed_peer("Bob", &key));
        chat.handle("/trust 0000");
        assert_eq!(texts(&chat).last().unwrap(), &t("trust.none", &[("code", "0000")]));
        let code = reveal_peer(&chat, &key);
        chat.handle(&format!("/trust {}", code.to_lowercase()));
        assert!(chat.trusts(&key));
        assert_eq!(chat.link_of(&chat.fresh_peers()[0]), Link::Encrypted);
        assert!(texts(&chat).contains(&t("trust.done", &[("name", "Bob"), ("ip", "10.0.0.2")])));
    }

    #[test]
    fn trust_without_a_code_shows_usage() {
        let chat = chat(&[]);
        chat.handle("/trust");
        assert_eq!(texts(&chat)[1], t("usage.trust", &[]));
    }

    #[test]
    fn only_fresh_peer_addresses_are_admitted_and_bursts_are_capped() {
        let chat = chat(&["a"]);
        assert!(!chat.admit_sender("10.0.0.9"));
        for _ in 0..MESSAGE_LIMIT {
            assert!(chat.admit_sender("10.0.0.1"));
        }
        assert!(!chat.admit_sender("10.0.0.1"));
    }

    use std::fs;

    fn phone_chat(dir: &std::path::Path, port: u16) -> Chat {
        let mut chat = Chat::new(Me { id: format!("id{port}"), name: "N".into(), port, key: vec![], secret: vec![] });
        chat.state_dir = Some(dir.to_path_buf());
        chat
    }

    fn phone_dir() -> PathBuf {
        std::env::temp_dir().join(format!("herdr-talk-chat-phone-{:016x}", fastrand::u64(..)))
    }

    #[test]
    fn the_first_node_is_host_shows_the_phone_once_and_the_token_stays_out_of_the_chat() {
        let dir = phone_dir();
        let (first, second) = (phone_chat(&dir, 1), phone_chat(&dir, 2));
        assert!(first.try_become_host() && first.is_host());
        assert!(!second.try_become_host() && !second.is_host());
        let token = phone::read(&dir).unwrap().token;
        let overlay = first.phone_overlay().expect("shown when the host starts").join("
");
        assert!(overlay.contains(&token));
        assert!(second.phone_overlay().is_none(), "a node that is not the host shows nothing by itself");
        // A node that is not the host still reads the host's phone.json when asked.
        second.handle("/phone");
        assert!(second.phone_overlay().unwrap().join("
").contains(&token));
        for chat in [&first, &second] {
            assert!(chat.lines().iter().all(|line| !line.render().contains(&token)));
        }
        first.hide_phone();
        assert!(first.phone_overlay().is_none());
        first.toggle_phone();
        assert!(first.phone_visible());
        first.release_host();
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn phone_new_changes_the_token_without_writing_it_in_the_chat() {
        let dir = phone_dir();
        let chat = phone_chat(&dir, 1);
        assert!(chat.try_become_host());
        let before = phone::read(&dir).unwrap().token;
        chat.handle("/phone new");
        let after = phone::read(&dir).unwrap().token;
        assert_ne!(before, after);
        let said: Vec<String> = chat.lines().iter().map(Line::render).collect();
        assert!(said.contains(&t("phone.new_done", &[])));
        assert!(said.iter().all(|row| !row.contains(&after) && !row.contains(&before)));
        chat.handle("/phone what");
        assert_eq!(chat.lines().last().unwrap().render(), t("usage.phone", &[]));
        chat.release_host();
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_waiting_node_takes_over_when_the_host_ends_and_keeps_the_token() {
        let dir = phone_dir();
        fs::create_dir_all(&dir).unwrap();
        // A live node that is not the host: its control port accepts connections.
        let alive = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        fs::write(dir.join("control-1.json"), format!("{{\"port\": {}, \"token\": \"x\"}}", alive.local_addr().unwrap().port())).unwrap();
        let (first, second) = (phone_chat(&dir, 1), phone_chat(&dir, 2));
        assert!(first.try_become_host());
        let token = phone::read(&dir).unwrap().token;
        first.release_host();
        assert!(phone::read(&dir).is_some(), "another node is alive, so the file stays for the successor");
        assert!(second.try_become_host());
        let phone = phone::read(&dir).unwrap();
        assert_eq!((phone.token, phone.owner_port), (token, 2));
        second.release_host();
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_phone_hides_when_its_time_is_up() {
        let dir = phone_dir();
        let chat = phone_chat(&dir, 1);
        chat.show_phone();
        *chat.phone_until.lock().unwrap() = Some(Instant::now() - Duration::from_secs(1));
        assert!(!chat.phone_visible());
        assert!(chat.take_dirty(), "the screen is asked to redraw");
    }

    #[test]
    fn supabase_mode_sends_nothing_to_lan_peers() {
        let chat = chat(&["a", "b"]);
        chat.set_supabase_mode(true);
        let sent = Mutex::new(0);
        chat.handle_with("hello", |_, _, _| {
            *sent.lock().unwrap() += 1;
            Ok(())
        });
        assert_eq!(*sent.lock().unwrap(), 0);
        assert!(chat.lines().iter().any(|line| line.text == "hello" && line.mine), "the own line is still shown");
    }

    #[test]
    fn local_mode_sends_to_lan_peers() {
        let mut chat = chat(&["a", "b"]);
        chat.plaintext = true;
        assert!(!chat.is_supabase());
        let sent = Mutex::new(0);
        chat.handle_with("hello", |_, _, _| {
            *sent.lock().unwrap() += 1;
            Ok(())
        });
        assert_eq!(*sent.lock().unwrap(), 2);
    }

    #[test]
    fn supabase_mode_without_a_running_relay_says_so() {
        let chat = chat(&["a"]);
        chat.set_supabase_mode(true);
        chat.handle_with("hello", |_, _, _| Ok(()));
        assert!(chat.lines().iter().any(|line| line.text == t("relay.not_sent", &[])));
    }
}
