//! Shared state: this instance, known peers, the lines shown on screen, and typed commands.

use crate::i18n::t;
use crate::net;
use crate::phone;
use crate::store::{self, Trusted};
use crate::text::{char_width, clean_text};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const PEER_TIMEOUT: Duration = Duration::from_secs(10);
/// A LAN peer must be joined (/join) within this time of its first announce, or it is kicked for this chat.
pub const JOIN_LIMIT: Duration = Duration::from_secs(30);
pub const MAX_TEXT_CHARS: usize = 2000;
/// Every peer in the list (verified or not, this PC not counted): a new one is ignored at this count. Change it here.
pub const MAX_TOTAL_PEERS: usize = 10;
/// Verified peers in one room (LAN or relay), this PC included: a new verified peer is ignored when MAX_PEERS - 1 are here.
pub const MAX_PEERS: usize = 10;
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
        "help.peers", "help.trust", "help.rejoin", "help.help", "help.quit", "", "help.keys", "help.enter", "help.backspace",
        "help.ctrl_l", "help.ctrl_c",
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
    /// Order of arrival, set by `Chat::add`: two lines with the same text are still two lines.
    pub seq: u64,
}

impl Line {
    /// The row as shown: "sender: text", or the text alone for a notice or a continuation row.
    pub fn render(&self) -> String {
        match &self.sender {
            Some(sender) => format!("{sender} : {}", self.text),
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
    line_seq: AtomicU64,
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
    /// When the name last changed by itself because of a clash. Names the user types do not count.
    last_auto_rename: Mutex<Option<Instant>>,
    /// The relay thread's inbox and its stop flag, while a relay runs (see relay_chat).
    relay_inbox: Mutex<Option<(Sender<String>, Arc<AtomicBool>)>>,
    /// What the status row says about the relay ("" when it is off).
    relay_status: Mutex<String>,
    /// The numbers of the peers waiting for a code check (see waiting_number).
    wait_numbers: Mutex<Vec<(Vec<u8>, usize)>>,
    /// True in Supabase mode: messages go only through the relay, never over the LAN.
    supabase: AtomicBool,
    /// Supabase: this node created the room (its join answer's owner_pk is this node's key).
    room_owner: AtomicBool,
    /// The other people in the room (CLOUD SERVICE), for the CONNECTED NODES list.
    room_names: Mutex<Vec<String>>,
    /// False while the first-run screen is open: the mode is not chosen yet, so no LAN peer is remembered.
    lan_listening: AtomicBool,
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
    /// When each key was first announced and not joined yet (/join). Starts the JOIN_LIMIT clock.
    waiting_since: Mutex<HashMap<Vec<u8>, Instant>>,
    /// Keys kicked for not joining in time. Their announcements are ignored for the rest of this chat.
    kicked: Mutex<std::collections::HashSet<Vec<u8>>>,
}


/// A fresh random nonce for the SAS commitment. Without a random source the code could be ground, so this panics instead.
fn random_nonce() -> [u8; 32] {
    let mut nonce = [0u8; 32];
    getrandom::getrandom(&mut nonce).expect("system random source for the SAS nonce");
    nonce
}


/// The shortest time between two automatic renames.
const AUTO_RENAME_GAP: Duration = Duration::from_secs(60);

impl Chat {
    pub fn new(me: Me) -> Self {
        Chat {
            me: Mutex::new(me),
            peers: Mutex::new(Vec::new()),
            stop: Stop::default(),
            lines: Mutex::new(Vec::new()),
            line_seq: AtomicU64::new(0),
            last_activity: Mutex::new(Instant::now()),
            dirty: AtomicBool::new(true),
            received: Mutex::new(HashMap::new()),
            trusted: Mutex::new(Vec::new()),
            plaintext: false,
            state_dir: None,
            host: Mutex::new(None),
            last_auto_rename: Mutex::new(None),
            relay_inbox: Mutex::new(None),
            relay_status: Mutex::new(String::new()),
            wait_numbers: Mutex::new(Vec::new()),
            supabase: AtomicBool::new(false),
            room_owner: AtomicBool::new(false),
            room_names: Mutex::new(Vec::new()),
            lan_listening: AtomicBool::new(true),
            my_nonces: Mutex::new(HashMap::new()),
            commits: Mutex::new(HashMap::new()),
            refused: Mutex::new(std::collections::HashSet::new()),
            reveals: Mutex::new(HashMap::new()),
            revealing: Mutex::new(std::collections::HashSet::new()),
            reveal_requests: Mutex::new(Vec::new()),
            waiting_since: Mutex::new(HashMap::new()),
            kicked: Mutex::new(std::collections::HashSet::new()),
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
    /// The host badge: in Supabase mode the room owner only; on the LAN the first node on this PC.
    pub fn is_host(&self) -> bool {
        if self.is_supabase() {
            return self.room_owner.load(Ordering::Relaxed);
        }
        self.host.lock().unwrap().is_some()
    }

    /// Set from the room join answer: true when this node's key is the room's owner_pk.
    pub fn set_room_owner(&self, owner: bool) {
        self.room_owner.store(owner, Ordering::Relaxed);
    }

    /// Become the host if no node on this PC is. Returns true on success.
    pub fn try_become_host(&self) -> bool {
        let Some(state) = &self.state_dir else { return false };
        if self.stop.is_set() || self.host.lock().unwrap().is_some() {
            return false;
        }
        let port = self.me.lock().unwrap().port;
        match phone::Host::try_take(state, port) {
            Ok(Some(host)) => {
                *self.host.lock().unwrap() = Some(host);
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

    /// The number of a peer waiting for a code check. It is given once, the first time the peer's code is shown, and kept
    /// (other peers' numbers do not change when one is verified).
    pub fn waiting_number(&self, key: &[u8]) -> usize {
        let mut numbers = self.wait_numbers.lock().unwrap();
        if let Some((_, number)) = numbers.iter().find(|(known, _)| known == key) {
            return *number;
        }
        let number = numbers.len() + 1;
        numbers.push((key.to_vec(), number));
        number
    }

    /// The peers waiting for a code check, with their codes and numbers, in number order. Only a peer whose code is known
    /// is listed (the code exists after the reveal exchange).
    pub fn unverified_peers(&self) -> Vec<(Peer, String, usize)> {
        let mut waiting: Vec<(Peer, String, usize)> = self.fresh_peers().into_iter().filter_map(|peer| {
            let key = peer.key.clone()?;
            if self.link_of(&peer) != Link::Unverified {
                return None;
            }
            let code = self.sas_for(&key)?;
            let number = self.waiting_number(&key);
            Some((peer, code, number))
        }).collect();
        waiting.sort_by_key(|(_, _, number)| *number);
        waiting
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
        lines.extend(rows.into_iter().map(|mut row| {
            row.seq = self.line_seq.fetch_add(1, Ordering::Relaxed);
            row
        }));
        let extra = lines.len().saturating_sub(MAX_LINES);
        lines.drain(..extra);
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Hand a running relay its inbox and stop flag. Replaces nothing: call stop_relay first.
    pub(crate) fn attach_relay(&self, inbox: Sender<String>, stop: Arc<AtomicBool>) {
        *self.relay_inbox.lock().unwrap() = Some((inbox, stop));
    }

    /// Forget the inbox when the relay thread that owns `stop` has ended. A newer relay's inbox is left alone.
    pub(crate) fn detach_relay(&self, stop: &Arc<AtomicBool>) {
        let mut inbox = self.relay_inbox.lock().unwrap();
        if inbox.as_ref().is_some_and(|(_, current)| Arc::ptr_eq(current, stop)) {
            *inbox = None;
        }
    }

    /// Stop the running relay thread (it exits at its next check) and forget its inbox.
    pub fn set_room_names(&self, names: Vec<String>) {
        *self.room_names.lock().unwrap() = names;
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub fn room_names(&self) -> Vec<String> {
        self.room_names.lock().unwrap().clone()
    }

    pub fn stop_relay(&self) {
        self.room_names.lock().unwrap().clear();
        self.set_room_owner(false);
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

    /// The room part of the chat status line: "room ID: X · setup code: Y" (Supabase mode with a saved room). None otherwise.
    pub fn room_status(&self) -> Option<String> {
        if !self.is_supabase() {
            return None;
        }
        let saved = crate::relay_settings::load_saved(self.state_dir.as_deref()?);
        (!saved.room_code.is_empty()).then(|| {
            let setup = if saved.setup_code.is_empty() { "-" } else { saved.setup_code.as_str() };
            format!("room ID: {} · setup code: {setup}", saved.room_code)
        })
    }

    /// The connection mode for this run: true in Supabase mode.
    pub fn set_supabase_mode(&self, on: bool) {
        self.supabase.store(on, Ordering::Relaxed);
        if on {
            // LAN peers heard while the settings were open (before the mode was known) are dropped.
            self.peers.lock().unwrap().clear();
            self.waiting_since.lock().unwrap().clear();
            self.kicked.lock().unwrap().clear();
        }
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub fn set_lan_listening(&self, on: bool) {
        self.lan_listening.store(on, Ordering::Relaxed);
    }

    pub fn is_lan_listening(&self) -> bool {
        self.lan_listening.load(Ordering::Relaxed)
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
        let sent = match self.relay_inbox.lock().unwrap().as_ref() {
            Some((inbox, _)) => inbox.send(text.to_string()).is_ok(),
            None => false,
        };
        // A relay whose thread has ended has no receiver left: the user is told instead of losing the line.
        if !sent {
            self.say(t("relay.not_sent", &[]));
        }
    }

    /// Show one notice line in the chat.
    pub fn say(&self, text: impl Into<String>) {
        self.add(vec![Line { sender: None, text: text.into(), mine: false, seq: 0 }]);
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
        let fresh: Vec<Peer> = self.peers.lock().unwrap().iter().filter(|peer| peer.seen.elapsed() < PEER_TIMEOUT).cloned().collect();
        fresh.into_iter().filter(|peer| !peer.key.as_deref().is_some_and(|key| self.is_kicked(key, &peer.name, &peer.ip))).collect()
    }

    /// Start the JOIN_LIMIT clock for a key that is not joined yet. Only its first announce sets it.
    pub fn note_waiting(&self, key: &[u8]) {
        self.waiting_since.lock().unwrap().entry(key.to_vec()).or_insert_with(Instant::now);
    }

    /// /rejoin: forget the kicked peers, so their next announce starts a new /join clock. Verified peers are not touched.
    pub fn rejoin_kicked(&self) {
        let kicked: Vec<Vec<u8>> = self.kicked.lock().unwrap().drain().collect();
        let mut waiting = self.waiting_since.lock().unwrap();
        for key in &kicked {
            waiting.remove(key);
        }
        drop(waiting);
        self.say(t("rejoin.done", &[("seconds", &JOIN_LIMIT.as_secs().to_string())]));
    }

    /// Seconds left for a peer to /join (from its first announce, rounded up), or None when it is joined or not waiting.
    pub fn join_seconds_left(&self, key: &[u8]) -> Option<u64> {
        if self.trusts(key) {
            return None;
        }
        let since = *self.waiting_since.lock().unwrap().get(key)?;
        let left = JOIN_LIMIT.saturating_sub(since.elapsed());
        Some(left.as_secs() + u64::from(left.subsec_nanos() > 0))
    }

    /// True for a kicked key: not joined within JOIN_LIMIT, and kicked for the rest of this chat. A joined key is never kicked.
    /// The first kick says so once in the chat.
    pub fn is_kicked(&self, key: &[u8], name: &str, ip: &str) -> bool {
        // CLOUD SERVICE has no LAN chat: nobody is kicked there.
        if self.is_supabase() || self.trusts(key) {
            return false;
        }
        let mut kicked = self.kicked.lock().unwrap();
        if kicked.contains(key) {
            return true;
        }
        let late = self.waiting_since.lock().unwrap().get(key).is_some_and(|since| since.elapsed() >= JOIN_LIMIT);
        if late {
            kicked.insert(key.to_vec());
            drop(kicked);
            self.say(t("peer.kicked", &[("name", name), ("ip", ip), ("seconds", &JOIN_LIMIT.as_secs().to_string())]));
        }
        late
    }

    /// /rejoin: forget the kicked keys that are not trusted, so the next announce waits for /join again with a new 60 s clock.
    /// Trusted (already confirmed) keys are left as they are.
    pub fn rejoin(&self) {
        let released: Vec<Vec<u8>> = self.kicked.lock().unwrap().iter().filter(|key| !self.trusts(key)).cloned().collect();
        for key in &released {
            self.kicked.lock().unwrap().remove(key);
            self.waiting_since.lock().unwrap().remove(key);
        }
        self.say(t("rejoin.done", &[("seconds", &JOIN_LIMIT.as_secs().to_string())]));
    }

    /// True for a key that was kicked (its announcements are ignored).
    pub fn was_kicked(&self, key: &[u8]) -> bool {
        self.kicked.lock().unwrap().contains(key)
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
                        None => t("peers.code_pending", &[]),
                    },
                    (Link::Unverified, None) => String::new(),
                };
                let number = self.unverified_peers().iter().find(|(waiting, _, _)| waiting.id == peer.id).map(|(_, _, number)| format!("[{number}] ")).unwrap_or_default();
                self.say(format!("- {number}{} ({}:{}) {state}", peer.name, peer.ip, peer.port));
            }
            return true;
        }
        if text == "/rejoin" {
            self.rejoin_kicked();
            return true;
        }
        if text == "/rejoin" {
            self.rejoin();
            return true;
        }
        if text == "/join" || text.starts_with("/join ") {
            let arg = text[5..].trim();
            // A number picks a waiting peer from the list ([1], [2] ...); a code is compared as before.
            if !arg.is_empty() && arg.len() <= 2 && arg.bytes().all(|byte| byte.is_ascii_digit()) {
                let picked = arg.parse::<usize>().ok().and_then(|number| self.unverified_peers().into_iter().find(|(_, _, waiting)| *waiting == number));
                match picked {
                    Some((_, code, _)) => self.trust(&code),
                    None => self.say(t("trust.no_number", &[("n", arg)])),
                }
                return true;
            }
            let code = store::normalize_code(arg);
            if code.is_empty() {
                self.say(t("usage.trust", &[]));
            } else {
                self.trust(&code);
            }
            return true;
        }
        let text = text.chars().take(MAX_TEXT_CHARS).collect::<String>().replace('\n', " ");
        if self.is_supabase() {
            // Supabase mode: the relay is the only path, so the LAN peers get nothing. A running relay shows the sent line
            // itself (once, after the post succeeds); without one, the line is shown here so the user sees what was typed.
            if self.relay_inbox.lock().unwrap().is_none() {
                let name = self.me.lock().unwrap().name.clone();
                self.add(vec![Line { sender: Some(format!("{name} {}", t("chat.you", &[]))), text: text.clone(), mine: true, seq: 0 }]);
            }
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
        self.add(vec![Line { sender: Some(format!("{} {}", me.name, t("chat.you", &[]))), text: text.to_string(), mine: true, seq: 0 }]);
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

    #[test]
    fn the_room_status_shows_the_room_and_setup_code_in_supabase_mode() {
        let dir = std::env::temp_dir().join(format!("herdr-talk-room-status-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut chat = chat(&[]);
        chat.state_dir = Some(dir.clone());
        assert_eq!(chat.room_status(), None, "Local mode shows nothing");
        chat.set_supabase_mode(true);
        assert_eq!(chat.room_status(), None, "no saved room yet");
        let saved = crate::relay_settings::Settings { mode: crate::relay_settings::Mode::Supabase, url: "https://example.supabase.co".into(), key: "k".into(), name: "Alice".into(), room_code: "ABCD2345".into(), trust: "12345678".into(), setup_code: "WXYZ6789".into() };
        crate::relay_settings::save(&dir, &saved).unwrap();
        assert_eq!(chat.room_status().as_deref(), Some("room ID: ABCD2345 · setup code: WXYZ6789"));
        crate::relay_settings::save(&dir, &crate::relay_settings::Settings { setup_code: String::new(), ..saved }).unwrap();
        assert_eq!(chat.room_status().as_deref(), Some("room ID: ABCD2345 · setup code: -"));
        let _ = std::fs::remove_dir_all(&dir);
    }

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
        chat.handle("/join 0000");
        assert_eq!(texts(&chat).last().unwrap(), &t("trust.none", &[("code", "0000")]));
        let code = reveal_peer(&chat, &key);
        chat.handle(&format!("/join {}", code.to_lowercase()));
        assert!(chat.trusts(&key));
        assert_eq!(chat.link_of(&chat.fresh_peers()[0]), Link::Encrypted);
        assert!(texts(&chat).contains(&t("trust.done", &[("name", "Bob"), ("ip", "10.0.0.2")])));
    }

    #[test]
    fn trust_by_number_picks_the_waiting_peer_and_names_it() {
        let chat = chat(&[]);
        let (bob, cy) = ([7u8; 32], [8u8; 32]);
        chat.peers.lock().unwrap().push(keyed_peer("Bob", &bob));
        chat.peers.lock().unwrap().push(keyed_peer("Cy", &cy));
        reveal_peer(&chat, &bob);
        reveal_peer(&chat, &cy);
        chat.handle("/join 0");
        assert_eq!(texts(&chat).last().unwrap(), &t("trust.no_number", &[("n", "0")]));
        chat.handle("/join 2");
        assert!(chat.trusts(&cy), "2 is Cy");
        assert!(!chat.trusts(&bob));
        assert!(texts(&chat).contains(&t("trust.done", &[("name", "Cy"), ("ip", "10.0.0.2")])), "the confirmation names the peer");
        chat.handle("/join 2");
        assert_eq!(texts(&chat).last().unwrap(), &t("trust.no_number", &[("n", "2")]), "a verified peer is not waiting any more");
        chat.handle("/join 1");
        assert!(chat.trusts(&bob));
    }

    #[test]
    fn waiting_numbers_are_unique_and_stay_when_one_is_verified() {
        let chat = chat(&[]);
        let keys = [[1u8; 32], [2u8; 32], [3u8; 32]];
        for (index, key) in keys.iter().enumerate() {
            chat.peers.lock().unwrap().push(keyed_peer(&format!("P{index}"), key));
        }
        // The codes come in a different order than the peers are listed: each number is kept from its first display.
        reveal_peer(&chat, &keys[2]);
        assert_eq!(chat.waiting_number(&keys[2]), 1);
        reveal_peer(&chat, &keys[0]);
        reveal_peer(&chat, &keys[1]);
        assert_eq!(chat.waiting_number(&keys[0]), 2);
        assert_eq!(chat.waiting_number(&keys[1]), 3);
        let numbers: Vec<usize> = chat.unverified_peers().iter().map(|(_, _, number)| *number).collect();
        assert_eq!(numbers, vec![1, 2, 3], "three waiting peers, numbers 1 to 3");
        chat.handle("/join 2");
        assert!(chat.trusts(&keys[0]), "2 stays with its peer");
        let left: Vec<usize> = chat.unverified_peers().iter().map(|(_, _, number)| *number).collect();
        assert_eq!(left, vec![1, 3], "the others keep their numbers after one is verified");
        chat.handle("/join 3");
        assert!(chat.trusts(&keys[1]), "3 is still the same peer");
    }

    #[test]
    fn trust_without_a_code_shows_usage() {
        let chat = chat(&[]);
        chat.handle("/join");
        assert_eq!(texts(&chat)[1], t("usage.trust", &[]));
    }

    #[test]
    fn rejoin_forgets_a_kicked_peer_so_it_waits_again_with_a_new_clock() {
        let chat = chat(&[]);
        let key = [7u8; 32];
        chat.peers.lock().unwrap().push(keyed_peer("Dan", &key));
        chat.note_waiting(&key);
        chat.waiting_since.lock().unwrap().insert(key.to_vec(), Instant::now() - JOIN_LIMIT);
        assert!(chat.fresh_peers().is_empty() && chat.was_kicked(&key));
        assert!(chat.handle("/rejoin"));
        assert!(!chat.was_kicked(&key));
        assert_eq!(chat.join_seconds_left(&key), None, "the clock starts again at the next announce");
        chat.note_waiting(&key);
        assert_eq!(chat.fresh_peers().len(), 1, "listed again");
        assert!(texts(&chat).contains(&t("rejoin.done", &[("seconds", &JOIN_LIMIT.as_secs().to_string())])));
    }

    #[test]
    fn the_join_clock_counts_down_and_is_none_for_a_key_not_waiting() {
        let chat = chat(&[]);
        let key = [8u8; 32];
        assert_eq!(chat.join_seconds_left(&key), None);
        chat.note_waiting(&key);
        assert_eq!(chat.join_seconds_left(&key), Some(JOIN_LIMIT.as_secs()));
        chat.waiting_since.lock().unwrap().insert(key.to_vec(), Instant::now() - Duration::from_secs(10));
        let left = chat.join_seconds_left(&key).unwrap();
        assert!(left == JOIN_LIMIT.as_secs() - 10 || left == JOIN_LIMIT.as_secs() - 9, "{left}");
        chat.waiting_since.lock().unwrap().insert(key.to_vec(), Instant::now() - JOIN_LIMIT);
        assert_eq!(chat.join_seconds_left(&key), Some(0));
    }

    #[test]
    fn a_peer_not_joined_within_the_limit_is_kicked_for_the_chat() {
        let chat = chat(&[]);
        let key = [9u8; 32];
        chat.peers.lock().unwrap().push(keyed_peer("Dan", &key));
        chat.note_waiting(&key);
        chat.waiting_since.lock().unwrap().insert(key.to_vec(), Instant::now() - JOIN_LIMIT);
        assert!(chat.fresh_peers().is_empty(), "a kicked peer is not listed");
        assert!(chat.was_kicked(&key));
        assert!(texts(&chat).contains(&t("peer.kicked", &[("name", "Dan"), ("ip", "10.0.0.2"), ("seconds", &JOIN_LIMIT.as_secs().to_string())])));
    }

    #[test]
    fn a_joined_peer_is_never_kicked() {
        let chat = chat(&[]);
        let key = [9u8; 32];
        chat.peers.lock().unwrap().push(keyed_peer("Dan", &key));
        chat.waiting_since.lock().unwrap().insert(key.to_vec(), Instant::now() - JOIN_LIMIT);
        chat.trusted.lock().unwrap().push(Trusted { name: "Dan".into(), key: key.to_vec(), ip: "10.0.0.2".into() });
        assert_eq!(chat.fresh_peers().len(), 1);
        assert!(!chat.was_kicked(&key));
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
    fn the_first_node_is_the_host_and_the_second_is_not() {
        let dir = phone_dir();
        let (first, second) = (phone_chat(&dir, 1), phone_chat(&dir, 2));
        assert!(first.try_become_host() && first.is_host());
        assert!(!second.try_become_host() && !second.is_host());
        first.release_host();
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn in_supabase_only_the_room_owner_is_the_host_even_if_this_pc_already_has_one() {
        let dir = phone_dir();
        let chat = phone_chat(&dir, 1);
        chat.set_supabase_mode(true);
        // This node holds the PC-level lock first, but it did not create the room: no host badge.
        assert!(chat.try_become_host());
        assert!(!chat.is_host(), "the PC lock alone is not the room host in Supabase");
        chat.set_room_owner(true);
        assert!(chat.is_host(), "the owner of the room is the host");
        chat.set_room_owner(false);
        assert!(!chat.is_host());
        chat.release_host();
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rejoin_clears_the_kicked_record_and_starts_a_new_clock_but_keeps_verified_peers() {
        let chat = chat(&[]);
        let kicked = [5u8; 32];
        let verified = [6u8; 32];
        chat.note_waiting(&kicked);
        chat.waiting_since.lock().unwrap().insert(kicked.to_vec(), Instant::now() - JOIN_LIMIT);
        assert!(chat.is_kicked(&kicked, "Dan", "10.0.0.2"));
        chat.trusted.lock().unwrap().push(Trusted { name: "Eve".into(), key: verified.to_vec(), ip: "10.0.0.3".into() });
        assert!(chat.handle("/rejoin"));
        assert!(!chat.was_kicked(&kicked), "the kicked record is gone");
        assert_eq!(chat.join_seconds_left(&kicked), None, "no clock until the next announce");
        chat.note_waiting(&kicked);
        assert!((29..=30).contains(&chat.join_seconds_left(&kicked).unwrap()), "a new clock");
        assert!(chat.trusts(&verified), "a verified peer is kept");
        assert!(texts(&chat).contains(&t("rejoin.done", &[("seconds", &JOIN_LIMIT.as_secs().to_string())])));
    }

    #[test]
    fn the_join_countdown_runs_from_the_first_announce_and_stops_when_joined() {
        let chat = chat(&[]);
        let key = [9u8; 32];
        assert_eq!(chat.join_seconds_left(&key), None, "not waiting yet");
        chat.note_waiting(&key);
        let left = chat.join_seconds_left(&key).unwrap();
        assert!((29..=30).contains(&left), "{left}");
        chat.waiting_since.lock().unwrap().insert(key.to_vec(), Instant::now() - Duration::from_secs(20));
        assert!((9..=10).contains(&chat.join_seconds_left(&key).unwrap()));
        chat.trusted.lock().unwrap().push(Trusted { name: "Dan".into(), key: key.to_vec(), ip: "10.0.0.2".into() });
        assert_eq!(chat.join_seconds_left(&key), None, "a joined peer has no countdown");
    }

    #[test]
    fn a_waiting_node_takes_over_when_the_host_ends() {
        let dir = phone_dir();
        fs::create_dir_all(&dir).unwrap();
        // A live node that is not the host: its control port accepts connections.
        let alive = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        fs::write(dir.join("control-1.json"), format!("{{\"port\": {}, \"token\": \"x\"}}", alive.local_addr().unwrap().port())).unwrap();
        let (first, second) = (phone_chat(&dir, 1), phone_chat(&dir, 2));
        assert!(first.try_become_host());
        first.release_host();
        assert!(phone::read(&dir).is_some(), "another node is alive, so the file stays for the successor");
        assert!(second.try_become_host());
        assert_eq!(phone::read(&dir).unwrap().owner_port, 2);
        second.release_host();
        fs::remove_dir_all(dir).ok();
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

    #[test]
    fn in_room_mode_a_typed_message_is_not_echoed_here_and_goes_to_the_relay_once() {
        let chat = chat(&["a"]);
        chat.set_supabase_mode(true);
        let (inbox, received) = std::sync::mpsc::channel();
        chat.attach_relay(inbox, Arc::new(AtomicBool::new(false)));
        chat.handle_with("hi", |_, _, _| Ok(()));
        assert_eq!(received.try_recv().unwrap(), "hi");
        assert!(!chat.lines().iter().any(|line| line.mine && line.text == "hi"), "the relay's own echo is the only one");
    }

    #[test]
    fn a_line_to_a_relay_that_has_ended_is_reported_not_lost() {
        let chat = chat(&["a"]);
        let (inbox, received) = std::sync::mpsc::channel::<String>();
        chat.attach_relay(inbox, Arc::new(AtomicBool::new(false)));
        drop(received); // the relay thread is gone
        chat.relay_send("hi");
        assert!(chat.lines().iter().any(|line| line.text == t("relay.not_sent", &[])));
    }

    #[test]
    fn a_finished_relay_thread_detaches_only_its_own_inbox() {
        let chat = chat(&["a"]);
        let (old_stop, new_stop) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (old_inbox, _old_rx) = std::sync::mpsc::channel::<String>();
        chat.attach_relay(old_inbox, old_stop.clone());
        let (new_inbox, new_rx) = std::sync::mpsc::channel::<String>();
        chat.attach_relay(new_inbox, new_stop.clone()); // a newer relay replaced the old one
        chat.detach_relay(&old_stop);
        chat.relay_send("still here");
        assert_eq!(new_rx.try_recv().unwrap(), "still here", "the newer relay keeps its inbox");
        chat.detach_relay(&new_stop);
        assert!(chat.relay_inbox.lock().unwrap().is_none());
    }
}
