//! Shared state: this instance, known peers, the lines shown on screen, and typed commands.

use crate::i18n::t;
use crate::net;
use crate::store::{self, Trusted};
use crate::text::{char_width, clean_text};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
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
        "help.peers", "help.trust", "help.help", "help.quit", "", "help.keys", "help.enter", "help.backspace",
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
}

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
        }
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
                self.rename(new_name);
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
            self.rename(pick_name(names, &taken));
            return true;
        }
        if text == "/peers" {
            let found = self.fresh_peers();
            if found.is_empty() {
                self.say(t("peers.none", &[]));
            }
            let own = self.me.lock().unwrap().key.clone();
            for peer in found {
                let state = match (self.link_of(&peer), &peer.key) {
                    (Link::Encrypted, _) => t("peers.verified", &[]),
                    (Link::Plain, _) => t("peers.plain", &[]),
                    (Link::Unverified, Some(key)) => t("peers.unverified", &[("code", &store::fingerprint(&own, key))]),
                    (Link::Unverified, None) => String::new(),
                };
                self.say(format!("- {} ({}:{}) {state}", peer.name, peer.ip, peer.port));
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
        self.add(vec![Line { sender: Some(format!("{} {}", me.name, t("chat.you", &[]))), text, mine: true }]);
        true
    }

    /// Verify the unverified peer whose code matches: remember its key, and say so.
    fn trust(&self, code: &str) {
        let own = self.me.lock().unwrap().key.clone();
        let matching: Vec<(String, Vec<u8>, String)> = self.fresh_peers().into_iter().filter_map(|peer| {
            let key = peer.key.clone()?;
            let matches = store::normalize_code(&store::fingerprint(&own, &key)) == code && self.link_of(&peer) == Link::Unverified;
            matches.then_some((peer.name, key, peer.ip))
        }).collect();
        // Two different keys with the same code would mean a collision or an attacker: refuse rather than pick one.
        let distinct: std::collections::BTreeSet<&Vec<u8>> = matching.iter().map(|(_, key, _)| key).collect();
        if distinct.len() > 1 {
            self.say(t("trust.ambiguous", &[("code", code)]));
            return;
        }
        let Some((name, key, ip)) = matching.into_iter().next() else {
            self.say(t("trust.none", &[("code", code)]));
            return;
        };
        let mut trusted = self.trusted.lock().unwrap();
        trusted.push(Trusted { name: name.clone(), key, ip });
        let saved = match &self.state_dir {
            Some(dir) => store::write_trusted(dir, &trusted),
            None => Ok(()),
        };
        drop(trusted);
        match saved {
            Ok(()) => self.say(t("trust.done", &[("name", &name)])),
            Err(error) => self.say(t("trust.save_failed", &[("error", &error.to_string())])),
        }
    }

    /// Every way the name changes (/name, /lang, a clash found later) ends here, so one rule covers them all.
    fn rename(&self, new_name: String) {
        let old = std::mem::replace(&mut self.me.lock().unwrap().name, new_name.clone());
        self.say(t("name.changed", &[("old", &old), ("new", &new_name)]));
        self.resolve_name_conflict();
    }

    /// When a fresh peer has this chat's name and a smaller id, this chat gives the name up (the Python edition's
    /// rule), so of two nodes only the one with the larger id renames. Both sides compute the same answer.
    /// Two panes on one PC share an id (it comes from the key), so for equal ids the port decides the same way.
    /// The new name is the old one with a number: Name2, then Name3 if that is taken too.
    pub fn resolve_name_conflict(&self) {
        let me = self.me.lock().unwrap().clone();
        let peers = self.fresh_peers();
        let beaten = peers.iter().any(|peer| peer.name == me.name && (peer.id.as_str(), peer.port) < (me.id.as_str(), me.port));
        if beaten {
            let taken: Vec<String> = peers.into_iter().map(|peer| peer.name).collect();
            self.rename(numbered_name(&me.name, &taken));
        }
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

    /// A chat named `name` with the given id and port, that sees one fresh peer of the same name.
    fn clashing(id: &str, port: u16, name: &str, peer_id: &str, peer_port: u16) -> Chat {
        let chat = Chat::new(Me { id: id.into(), name: name.into(), port, key: vec![0; 32], secret: vec![1; 32] });
        chat.peers.lock().unwrap().push(Peer { id: peer_id.into(), name: name.into(), ip: "10.0.0.1".into(), port: peer_port, seen: Instant::now(), key: None });
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
        // Same cases as the Python edition's test: "aaa" keeps its name, "bbb" renames, whatever the ports.
        let (low, high) = (clashing("aaa", 9, "Jiho", "bbb", 1), clashing("bbb", 1, "Jiho", "aaa", 9));
        low.resolve_name_conflict();
        high.resolve_name_conflict();
        assert_eq!((my_name(&low), my_name(&high)), ("Jiho".to_string(), "Jiho2".to_string()));
        // Once renamed there is no clash any more.
        high.resolve_name_conflict();
        assert_eq!(my_name(&high), "Jiho2");
        // The same id (two panes on one PC) falls back to the port: the larger port renames.
        let (low, high) = (clashing("k", 1, "Jiho", "k", 2), clashing("k", 2, "Jiho", "k", 1));
        low.resolve_name_conflict();
        high.resolve_name_conflict();
        assert_eq!((my_name(&low), my_name(&high)), ("Jiho".to_string(), "Jiho2".to_string()));
    }

    #[test]
    fn a_taken_number_is_skipped_and_a_stale_peer_does_not_count() {
        let chat = clashing("b", 1, "Jiho", "a", 1);
        chat.peers.lock().unwrap().push(Peer { id: "c".into(), name: "Jiho2".into(), ip: "10.0.0.3".into(), port: 1, seen: Instant::now(), key: None });
        chat.resolve_name_conflict();
        assert_eq!(my_name(&chat), "Jiho3");
        let stale = clashing("b", 1, "Jiho", "a", 1);
        stale.peers.lock().unwrap()[0].seen = Instant::now() - PEER_TIMEOUT;
        stale.resolve_name_conflict();
        assert_eq!(my_name(&stale), "Jiho");
    }

    #[test]
    fn name_command_follows_the_same_rule() {
        // /name onto a name a smaller-id node already uses: this one ends up numbered.
        let chat = clashing("b", 1, "Other", "a", 1);
        chat.peers.lock().unwrap()[0].name = "Alice".into();
        chat.handle("/name Alice");
        assert_eq!(my_name(&chat), "Alice2");
        let keeps = clashing("a", 1, "Other", "b", 1);
        keeps.peers.lock().unwrap()[0].name = "Alice".into();
        keeps.handle("/name Alice");
        assert_eq!(my_name(&keeps), "Alice");
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
        chat.handle(&format!("/trust {}", store::fingerprint(&[0; 32], &key).to_uppercase()));
        assert!(chat.trusts(&key));
        assert_eq!(chat.link_of(&chat.fresh_peers()[0]), Link::Encrypted);
        assert!(texts(&chat).contains(&t("trust.done", &[("name", "Bob")])));
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
}
