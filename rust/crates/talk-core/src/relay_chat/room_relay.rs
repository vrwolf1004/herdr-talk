//! Room messages (Supabase mode). A node joins the room, sends each typed text to every trusted member (one Noise
//! message per member, then one post), and shows the messages addressed to it after checking the sender.
//! Design: docs/room-messages-design.md. Unverified members get no text: the send says how many were left out.

use crate::chat::{Chat, Line, MAX_TEXT_CHARS};
use crate::i18n::t;
use crate::net::noise_builder;
use crate::relay_key;
use crate::relay_rest::{self, Config, Delivery, Message, PutOutcome};
use crate::relay_replay::NonceCache;
use crate::relay_settings::Settings;
use crate::store::{self, Identity};
use crate::text::{clean_text, name_key};
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SLICE: Duration = Duration::from_millis(200);
const POLL: Duration = Duration::from_secs(2);
const MEMBERS_EVERY: Duration = Duration::from_secs(10);
const JOIN_RETRY_AFTER: Duration = Duration::from_secs(10);

/// The one retry of a failed room join. A successful join gives the retry back.
struct JoinRetry {
    left: bool,
}

impl JoinRetry {
    fn new() -> Self {
        JoinRetry { left: true }
    }

    fn succeeded(&mut self) {
        self.left = true;
    }

    /// The delay before the retry, or None when no retry is left.
    fn failed(&mut self) -> Option<Duration> {
        std::mem::replace(&mut self.left, false).then_some(JOIN_RETRY_AFTER)
    }
}
const NOISE_MAX: usize = 65535;

/// One room member: its Ed25519 key (hex) and its Noise key (bytes, the one /trust checks).
struct Member {
    pk: String,
    noise: Vec<u8>,
}

/// Run the room relay: join the room, then send what the user types and show what the members send.
pub(crate) fn run(chat: &Chat, state: &Path, config: &Config, settings: &Settings, rx: &Receiver<String>, stop: &AtomicBool) {
    let seed = match super::seed(state) {
        Ok(seed) => seed,
        Err(error) => return chat.set_relay_status(&format!("relay: {error}")),
    };
    let identity = match store::read_identity(state) {
        Ok(Some(identity)) => identity,
        _ => return chat.set_relay_status("relay: no identity yet: start a chat first"),
    };
    let me = store::to_hex(&SigningKey::from_bytes(&seed).verifying_key().to_bytes());
    if let Err(error) = listen(chat, state, config, settings, &seed, &identity, &me, rx, stop) {
        chat.set_relay_status(&format!("relay: {error}"));
    }
}

fn listen(chat: &Chat, state: &Path, config: &Config, settings: &Settings, seed: &[u8; 32], identity: &Identity, me: &str, rx: &Receiver<String>, stop: &AtomicBool) -> Result<(), String> {
    let mut cache = NonceCache::default();
    // Rows sent before this relay started are not shown (kept in memory only, one time per start).
    let started = unix_now();
    let mut members: Vec<Member> = Vec::new();
    let mut since: Option<String> = None;
    // This PC's row is published once, before the first join (the room's member rows point at it: a foreign key).
    let mut published = false;
    // A join error stays on the status line: later fetch errors do not replace it while the room is not joined.
    let mut joined = false;
    let mut join = JoinRetry::new();
    let mut joining = true;
    // Sender key -> 대화명 seen in messages (and this PC's own). Two keys with one name get a key suffix on the screen.
    let mut names: HashMap<String, String> = HashMap::new();
    let (mut next_members, mut next_fetch) = (Instant::now(), Instant::now());
    while !stop.load(Ordering::Relaxed) {
        if joining && Instant::now() >= next_members {
            next_members = Instant::now() + MEMBERS_EVERY;
            if !published {
                match publish_row(state, config, seed, identity, &settings.name) {
                    Ok(()) => published = true,
                    Err(error) => chat.set_relay_status(&format!("relay: {error}")),
                }
            }
            if published {
                match load_members(config, seed, &settings.room_code, &settings.name) {
                    Ok(list) => {
                        chat.set_relay_status(&format!("relay: room joined ({} members)", list.len()));
                        members = list;
                        joined = true;
                        join.succeeded();
                    }
                    Err(error) => {
                        joined = false;
                        chat.set_relay_status(&format!("relay: {error}"));
                        // One retry 10 seconds later; after that the join is not tried again.
                        match join.failed() {
                            Some(delay) => next_members = Instant::now() + delay,
                            None => joining = false,
                        }
                    }
                }
            }
        }
        while let Ok(text) = rx.try_recv() {
            send_text(chat, config, seed, &settings.room_code, identity, me, &members, &mut names, &text);
        }
        if Instant::now() >= next_fetch {
            next_fetch = Instant::now() + POLL;
            if let Err(error) = fetch_round(chat, config, seed, &settings.room_code, identity, &members, &mut names, &mut since, &mut cache, started) {
                if joined {
                    chat.set_relay_status(&format!("relay: {error}"));
                }
            }
        }
        thread::sleep(SLICE);
    }
    Ok(())
}

/// Publish this PC's row (name, Noise key, short id), as the create path does. The answer is shown in words; no key.
fn publish_row(state: &Path, config: &Config, seed: &[u8; 32], identity: &Identity, name: &str) -> Result<(), String> {
    let short_id = relay_key::short_id(state)?;
    publish_message(relay_rest::put_peer(config, seed, &store::to_hex(&identity.public), &short_id, name))
}

/// What a row write means for the join: Ok to go on, or the message to show (the join is not tried).
fn publish_message(outcome: PutOutcome) -> Result<(), String> {
    match outcome {
        PutOutcome::Written => Ok(()),
        PutOutcome::Taken => Err(t("room.put_taken", &[])),
        PutOutcome::Failed => Err(t("room.put_failed", &[])),
    }
}

/// Join the room (idempotent) and return its members with their Noise keys.
fn load_members(config: &Config, seed: &[u8; 32], room_code: &str, name: &str) -> Result<Vec<Member>, String> {
    let request = relay_rest::join_room_request(config, seed, room_code, name)?;
    let (status, body) = relay_rest::http_post(&request)?;
    let info = relay_rest::read_join_room_response(status, &body)?;
    relay_rest::members_noise_keys(config, &info.members)?
        .into_iter()
        .map(|(pk, noise)| Ok(Member { pk, noise: store::from_hex(&noise).ok_or("noise key is not hex")? }))
        .collect()
}

/// Send one text to every other member of the room, one Noise message each, in one post (no /trust step in rooms).
#[allow(clippy::too_many_arguments)]
fn send_text(chat: &Chat, config: &Config, seed: &[u8; 32], room_code: &str, identity: &Identity, me: &str, members: &[Member], names: &mut HashMap<String, String>, text: &str) {
    let own = name_key(&chat.me.lock().unwrap().name);
    names.insert(me.to_string(), own.clone());
    let others: Vec<&Member> = members.iter().filter(|member| member.pk != me).collect();
    let mut recipients = others;
    if recipients.is_empty() {
        chat.say("방 메시지 실패: 방에 다른 멤버가 없습니다".to_string());
        return;
    }
    recipients.sort_by(|a, b| a.pk.cmp(&b.pk));
    match post_to(config, seed, room_code, identity, &recipients, &own, text) {
        Ok(()) => {
            chat.add(vec![Line { sender: Some(format!("{} {}", label(names, me, &own), t("chat.you", &[]))), text: text.to_string(), mine: true, seq: 0 }]);
        }
        Err(error) => chat.say(format!("방 메시지 실패: {error}")),
    }
}

fn post_to(config: &Config, seed: &[u8; 32], room_code: &str, identity: &Identity, recipients: &[&Member], name: &str, text: &str) -> Result<(), String> {
    let ts = unix_now();
    let nonce = random_hex(16)?;
    let message_id = random_hex(16)?;
    let payload = json!({"v": 1, "ts": ts, "nonce": nonce, "name": name, "text": text}).to_string();
    let mut deliveries = Vec::new();
    for member in recipients {
        let ciphertext = seal(&identity.secret, &member.noise, payload.as_bytes())?;
        deliveries.push(Delivery { recipient_pk: member.pk.clone(), ciphertext: store::to_hex(&ciphertext) });
    }
    let body = relay_rest::post_message_body(seed, room_code, &message_id, ts, &nonce, &deliveries)?;
    let (status, answer) = relay_rest::http_post(&relay_rest::post_message_request(config, body))?;
    if status == 200 {
        Ok(())
    } else {
        Err(format!("status {status}: {answer}"))
    }
}

/// Ask for this PC's inbox since the last row seen, and show each message that checks out.
#[allow(clippy::too_many_arguments)]
fn fetch_round(chat: &Chat, config: &Config, seed: &[u8; 32], room_code: &str, identity: &Identity, members: &[Member], names: &mut HashMap<String, String>, since: &mut Option<String>, cache: &mut NonceCache, started: u64) -> Result<(), String> {
    let nonce = random_hex(16)?;
    let body = relay_rest::fetch_message_body(seed, room_code, since.as_deref(), unix_now(), &nonce)?;
    let (status, answer) = relay_rest::http_post(&relay_rest::fetch_messages_request(config, body))?;
    for row in relay_rest::read_fetch_response(status, &answer)? {
        *since = Some(row.created_at.clone());
        if after_start(&row, started) {
            show_message(chat, identity, members, names, cache, &row);
        }
    }
    Ok(())
}

/// A row sent before this relay started is not shown: the join shows only what comes after it.
fn after_start(row: &Message, started: u64) -> bool {
    row.ts >= started
}

/// The label for a sender: the 대화명, plus the first 4 characters of the key when another known key has the same 대화명.
fn label(names: &HashMap<String, String>, pk: &str, name: &str) -> String {
    let clash = names.iter().any(|(other, other_name)| other != pk && other_name == name);
    if clash {
        format!("{name} ({})", pk.get(..4).unwrap_or(pk))
    } else {
        name.to_string()
    }
}

/// One row: shown only when the sender is a member, the Noise key is the member's, the payload matches the row (ts, nonce),
/// and the nonce is new. Every member of the room is accepted (no /trust step in rooms). The name shown is the one the
/// sender wrote into the payload, or the first 4 characters of its key when that name is not valid. Every failure is silent.
fn show_message(chat: &Chat, identity: &Identity, members: &[Member], names: &mut HashMap<String, String>, cache: &mut NonceCache, row: &Message) {
    let Some(sender) = members.iter().find(|member| member.pk == row.sender_pk) else { return };
    let Some(ciphertext) = store::from_hex(&row.ciphertext) else { return };
    let Ok((payload, remote)) = open(&identity.secret, &ciphertext) else { return };
    if remote != sender.noise {
        return;
    }
    let Ok(value) = serde_json::from_slice::<Value>(&payload) else { return };
    let text = value["text"].as_str().unwrap_or("");
    if value["v"].as_u64() != Some(1) || value["ts"].as_u64() != Some(row.ts) || value["nonce"].as_str() != Some(row.nonce.as_str()) {
        return;
    }
    if !cache.accept(&row.nonce, row.ts, unix_now()) {
        return;
    }
    let text = clean_text(text, MAX_TEXT_CHARS);
    if text.trim().is_empty() {
        return;
    }
    let claimed = name_key(value["name"].as_str().unwrap_or(""));
    let name = if relay_rest::valid_name(&claimed) { claimed } else { row.sender_pk.chars().take(4).collect() };
    names.insert(row.sender_pk.clone(), name.clone());
    chat.add(vec![Line { sender: Some(label(names, &row.sender_pk, &name)), text, mine: false, seq: 0 }]);
}

/// The initiator's Noise message (IK, the payload in message 1): the sender key is proven to the receiver.
fn seal(secret: &[u8], peer_noise: &[u8], payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut handshake = noise_builder()
        .local_private_key(secret)
        .and_then(|builder| builder.remote_public_key(peer_noise))
        .and_then(|builder| builder.build_initiator())
        .map_err(|error| error.to_string())?;
    let mut buffer = vec![0u8; NOISE_MAX];
    let n = handshake.write_message(payload, &mut buffer).map_err(|error| error.to_string())?;
    buffer.truncate(n);
    Ok(buffer)
}

/// The receiver side: the payload and the sender's Noise key (proven by the message).
fn open(secret: &[u8], ciphertext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let mut handshake = noise_builder().local_private_key(secret).and_then(|builder| builder.build_responder()).map_err(|error| error.to_string())?;
    let mut buffer = vec![0u8; NOISE_MAX];
    let n = handshake.read_message(ciphertext, &mut buffer).map_err(|error| error.to_string())?;
    let remote = handshake.get_remote_static().ok_or("no sender key")?.to_vec();
    Ok((buffer[..n].to_vec(), remote))
}

fn random_hex(bytes: usize) -> Result<String, String> {
    let mut random = vec![0u8; bytes];
    getrandom::getrandom(&mut random).map_err(|error| error.to_string())?;
    Ok(store::to_hex(&random))
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::Me;
    use crate::store::Trusted;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    const NONCE: &str = "00112233445566778899aabbccddeeff";
    const NL: char = '\n';

    fn noise_keys() -> (Vec<u8>, Vec<u8>) {
        let keys = noise_builder().generate_keypair().unwrap();
        (keys.public, keys.private)
    }

    fn ed_key(seed: u8) -> (SigningKey, String) {
        let signing = SigningKey::from_bytes(&[seed; 32]);
        let pk = store::to_hex(&signing.verifying_key().to_bytes());
        (signing, pk)
    }

    fn verifies(pk_hex: &str, message: &str, sig_hex: &str) -> bool {
        let key = VerifyingKey::from_bytes(&store::from_hex(pk_hex).unwrap().try_into().unwrap()).unwrap();
        let sig = Signature::from_bytes(&store::from_hex(sig_hex).unwrap().try_into().unwrap());
        key.verify(message.as_bytes(), &sig).is_ok()
    }

    fn two_deliveries() -> Vec<Delivery> {
        vec![
            Delivery { recipient_pk: "a".repeat(64), ciphertext: "0102".into() },
            Delivery { recipient_pk: "b".repeat(64), ciphertext: "0304".into() },
        ]
    }

    #[test]
    fn the_deliveries_digest_is_the_literal_the_node_test_checks() {
        assert_eq!(relay_rest::deliveries_digest(&two_deliveries()), "3db42da64fc2575ca5eaf89fe0573df1fe77200fe1873e89fa61fbbf6c367de0");
    }

    #[test]
    fn a_failed_row_write_stops_the_join_with_a_message() {
        assert_eq!(publish_message(PutOutcome::Written), Ok(()));
        assert!(publish_message(PutOutcome::Taken).unwrap_err().contains("409"));
        assert!(publish_message(PutOutcome::Failed).unwrap_err().contains("게시하지 못했습니다"));
    }

    #[test]
    fn a_post_is_signed_over_the_fixed_message_and_refused_when_malformed() {
        let (_signing, pk) = ed_key(1);
        let seed = [1u8; 32];
        let message_id = "ff".repeat(16);
        let body: Value = serde_json::from_str(&relay_rest::post_message_body(&seed, "ABCD1234", &message_id, 1_700_000_000, NONCE, &two_deliveries()).unwrap()).unwrap();
        let digest = relay_rest::deliveries_digest(&two_deliveries());
        let message = [relay_rest::POST_TAG, "post", "ABCD1234", &pk, &message_id, "1700000000", NONCE, &digest].join(&NL.to_string());
        assert!(verifies(&pk, &message, body["sig"].as_str().unwrap()));
        assert_eq!(body["sender_pk"], pk);
        // Refused before any signature: unsorted recipients, too many recipients, a bad room code.
        let mut unsorted = two_deliveries();
        unsorted.reverse();
        assert!(relay_rest::post_message_body(&seed, "ABCD1234", &message_id, 1, NONCE, &unsorted).is_err());
        let nine: Vec<Delivery> = (0..9).map(|index| Delivery { recipient_pk: format!("{:02x}", index).repeat(32), ciphertext: "01".into() }).collect();
        assert!(relay_rest::post_message_body(&seed, "ABCD1234", &message_id, 1, NONCE, &nine).is_err());
        assert!(relay_rest::post_message_body(&seed, "abcd1234", &message_id, 1, NONCE, &two_deliveries()).is_err());
    }

    #[test]
    fn a_fetch_is_signed_by_the_recipient_key() {
        let (_, pk) = ed_key(2);
        let seed = [2u8; 32];
        let body: Value = serde_json::from_str(&relay_rest::fetch_message_body(&seed, "ABCD1234", None, 1_700_000_000, NONCE).unwrap()).unwrap();
        assert_eq!(body["recipient_pk"], pk);
        let message = [relay_rest::FETCH_TAG, "fetch", "ABCD1234", &pk, "1700000000", NONCE].join(&NL.to_string());
        assert!(verifies(&pk, &message, body["sig"].as_str().unwrap()));
        assert!(body.get("since").is_none());
        let with_since: Value = serde_json::from_str(&relay_rest::fetch_message_body(&seed, "ABCD1234", Some("2026-11-16T10:00:00Z"), 1_700_000_000, NONCE).unwrap()).unwrap();
        assert_eq!(with_since["since"], "2026-11-16T10:00:00Z");
    }

    #[test]
    fn the_fetch_answer_is_read_or_refused_as_a_whole() {
        let (_, sender) = ed_key(3);
        let good = format!(r#"{{"messages":[{{"message_id":"{}","sender_pk":"{}","ts":5,"nonce":"{}","ciphertext":"0102","created_at":"2026-11-16T10:00:00Z"}}]}}"#, "ff".repeat(16), sender, NONCE);
        let rows = relay_rest::read_fetch_response(200, &good).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ts, 5);
        assert!(relay_rest::read_fetch_response(500, &good).is_err(), "a failed status is refused");
        let bad_sender = good.replace(&sender, "zz");
        assert!(relay_rest::read_fetch_response(200, &bad_sender).is_err(), "a malformed row drops the whole answer");
    }

    #[test]
    fn a_noise_message_opens_for_its_receiver_and_proves_the_sender() {
        let (alice_public, alice_secret) = noise_keys();
        let (bob_public, bob_secret) = noise_keys();
        let ciphertext = seal(&alice_secret, &bob_public, b"hello").unwrap();
        let (payload, remote) = open(&bob_secret, &ciphertext).unwrap();
        assert_eq!(payload, b"hello");
        assert_eq!(remote, alice_public, "the receiver learns the sender's key from the message");
        let (carol_public, _) = noise_keys();
        assert!(open(&noise_keys().1, &ciphertext).is_err(), "a third key cannot open it");
        assert_ne!(remote, carol_public);
    }

    #[test]
    fn a_message_is_shown_once_from_a_member_and_nothing_else() {
        let (alice_public, alice_secret) = noise_keys();
        let (bob_public, bob_secret) = noise_keys();
        let bob = Chat::new(Me { id: "b".into(), name: "Bob".into(), port: 1, key: bob_public.clone(), secret: bob_secret.clone() });
        let identity = Identity { public: bob_public.clone(), secret: bob_secret.clone() };
        let (_, alice_pk) = ed_key(4);
        let members = vec![Member { pk: alice_pk.clone(), noise: alice_public.clone() }];
        let ts = unix_now(); // the replay window is measured from now
        let payload = json!({"v": 1, "ts": ts, "nonce": NONCE, "name": "Alice", "text": "hello room"}).to_string();
        let ciphertext = store::to_hex(&seal(&alice_secret, &bob_public, payload.as_bytes()).unwrap());
        let row = Message { message_id: "ff".repeat(16), sender_pk: alice_pk.clone(), ts, nonce: NONCE.into(), ciphertext: ciphertext.clone(), created_at: "2026".into() };
        let mut cache = NonceCache::default();
        let mut names = HashMap::new();
        show_message(&bob, &identity, &members, &mut names, &mut cache, &row);
        show_message(&bob, &identity, &members, &mut names, &mut cache, &row);
        let shown: Vec<String> = bob.lines().into_iter().map(|line| line.render()).collect();
        assert_eq!(shown, vec!["Alice: hello room".to_string()], "shown once; a replay is dropped");
        // A sender that is not a member, and a sender whose key differs from the member's, show nothing.
        let (_, stranger) = ed_key(5);
        let stranger_row = Message { sender_pk: stranger, ..row.clone() };
        show_message(&bob, &identity, &members, &mut names, &mut NonceCache::default(), &stranger_row);
        let other_key_members = vec![Member { pk: alice_pk.clone(), noise: noise_keys().0 }];
        show_message(&bob, &identity, &other_key_members, &mut names, &mut NonceCache::default(), &row);
        assert_eq!(bob.lines().len(), 1, "nothing else is shown");
    }

    #[test]
    fn a_failed_join_is_retried_once_after_ten_seconds_and_a_success_gives_the_retry_back() {
        let mut join = JoinRetry::new();
        assert_eq!(join.failed(), Some(Duration::from_secs(10)));
        assert_eq!(join.failed(), None, "no second retry");
        join.succeeded();
        assert_eq!(join.failed(), Some(Duration::from_secs(10)), "a successful join gives the retry back");
    }

    #[test]
    fn rows_sent_before_the_relay_started_are_refused_and_later_ones_pass() {
        let row = |ts| Message { message_id: "ff".repeat(16), sender_pk: String::new(), ts, nonce: NONCE.into(), ciphertext: String::new(), created_at: "2026".into() };
        let started = 1_000;
        assert!(!after_start(&row(999), started), "a row from before the start is refused");
        assert!(after_start(&row(1_000), started), "a row from the start second passes");
        assert!(after_start(&row(1_001), started), "a row after the start passes");
    }

    #[test]
    fn the_name_the_sender_wrote_is_shown() {
        let (alice_public, alice_secret) = noise_keys();
        let (bob_public, bob_secret) = noise_keys();
        let bob = Chat::new(Me { id: "b".into(), name: "Bob".into(), port: 1, key: bob_public.clone(), secret: bob_secret.clone() });
        let identity = Identity { public: bob_public.clone(), secret: bob_secret.clone() };
        let (_, alice_pk) = ed_key(4);
        let members = vec![Member { pk: alice_pk.clone(), noise: alice_public.clone() }];
        let ts = unix_now();
        // The screen shows the name Alice wrote.
        let payload = json!({"v": 1, "ts": ts, "nonce": NONCE, "name": "Alice", "text": "hi"}).to_string();
        let ciphertext = store::to_hex(&seal(&alice_secret, &bob_public, payload.as_bytes()).unwrap());
        let row = Message { message_id: "ff".repeat(16), sender_pk: alice_pk.clone(), ts, nonce: NONCE.into(), ciphertext, created_at: "2026".into() };
        show_message(&bob, &identity, &members, &mut HashMap::new(), &mut NonceCache::default(), &row);
        let shown: Vec<String> = bob.lines().into_iter().map(|line| line.render()).collect();
        assert_eq!(shown, vec!["Alice: hi".to_string()]);
        // Shown like a LAN message: the name is the sender, the text is separate, and it is not this PC's own line.
        let line = &bob.lines()[0];
        assert_eq!((line.sender.as_deref(), line.text.as_str(), line.mine), (Some("Alice"), "hi", false));
    }

    #[test]
    fn a_shared_name_gets_a_key_suffix() {
        let (_, pk) = ed_key(6);
        let (_, other) = ed_key(7);
        let mut names = HashMap::new();
        names.insert(pk.clone(), "방장".to_string());
        assert_eq!(label(&names, &pk, "방장"), "방장", "alone under its name: no suffix");
        names.insert(other.clone(), "방장".to_string());
        assert_eq!(label(&names, &pk, "방장"), format!("방장 ({})", &pk[..4]));
        assert!(!relay_rest::valid_name(""), "an empty 대화명 is refused");
    }
}
