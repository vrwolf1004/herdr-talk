//! Supabase REST side of the relay: settings, the requests for the `peers` table, and checks of the answers.
//! This module builds requests and reads answers; it does not send anything. (Sending needs TLS, which is not in
//! talk-core yet; see the reply that added this module.) Same rules as python/src/relay_rest.py.

use ed25519_dalek::{Signer, SigningKey};

/// The tag of the room requests (create and join), in front of the signed bytes.
pub const ROOM_TAG: &str = "herdr-talk-room-v1";
use serde_json::Value;
use std::time::Duration;

pub const URL_VARIABLE: &str = "HERDR_TALK_RELAY_URL";
pub const KEY_VARIABLE: &str = "HERDR_TALK_RELAY_KEY";
pub const PUT_TAG: &str = "herdr-talk-put-v3";
pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);

/// Where the relay is and the publishable key. Never the secret key.
#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    url: String,
    key: String,
}

// The key is not printed, so logs and debug output never show it.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config").field("url", &self.url).field("key", &"<hidden>").finish()
    }
}

impl Config {
    /// The relay URL (no trailing slash). Crate-internal: the key must not leave this crate through here.
    #[cfg(test)]
    pub(crate) fn url(&self) -> &str {
        &self.url
    }

}

/// One HTTP request, ready to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// Settings from two values. Either one missing (or empty) means the relay is off: Ok(None).
/// A URL that is not https (except loopback, for tests) is an error, so the key never goes over plain HTTP.
pub fn config(url: Option<&str>, key: Option<&str>) -> Result<Option<Config>, String> {
    let (Some(url), Some(key)) = (non_empty(url), non_empty(key)) else { return Ok(None) };
    let url = url.trim_end_matches('/').to_string();
    if !url.starts_with("https://") && !is_loopback(&url) {
        return Err(format!("{URL_VARIABLE} must start with https://"));
    }
    Ok(Some(Config { url, key: key.to_string() }))
}

fn is_loopback(url: &str) -> bool {
    url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost")
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

/// 64 lowercase hex characters.
pub fn valid_pk(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 1 to 32 characters, no control characters (the same rule the table enforces).
pub fn valid_name(text: &str) -> bool {
    let count = text.chars().count();
    (1..=32).contains(&count) && !text.chars().any(char::is_control)
}

fn common_headers(config: &Config) -> Vec<(String, String)> {
    vec![
        ("apikey".to_string(), config.key.clone()),
        ("Authorization".to_string(), format!("Bearer {}", config.key)),
    ]
}

/// 64 lowercase hex characters for a key, or 32 for a nonce.
pub fn valid_hex(text: &str, length: usize) -> bool {
    text.len() == length && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 8 uppercase characters, letters and digits: the short ID that others type to find this PC. Input in lowercase is
/// made uppercase before this check (the callers do it).
pub fn valid_short_id(text: &str) -> bool {
    text.len() == 8 && text.bytes().all(|byte| byte.is_ascii_digit() || byte.is_ascii_uppercase())
}

/// 8 uppercase characters, letters and digits: the room code shared by all peers in a room.
pub fn valid_room_code(text: &str) -> bool {
    text.len() == 8 && text.bytes().all(|byte| byte.is_ascii_digit() || byte.is_ascii_uppercase())
}

/// The bytes that are signed (v3): PUT_TAG, pk, noise_pk, short_id, name, ts, nonce, one per line, no newline at the end.
pub fn put_message(pk: &str, noise_pk: &str, short_id: &str, name: &str, ts: u64, nonce: &str) -> String {
    format!("{PUT_TAG}\n{pk}\n{noise_pk}\n{short_id}\n{name}\n{ts}\n{nonce}")
}

/// Signed write of this PC's row through the put-peer function. `seed` is the Ed25519 private key (32 bytes);
/// pk is the matching public key. The body keeps the fixed order pk, noise_pk, short_id, name, ts, nonce, sig.
pub fn put_request(config: &Config, seed: &[u8; 32], noise_pk: &str, short_id: &str, name: &str, ts: u64, nonce: &str) -> Result<Request, String> {
    if !valid_hex(noise_pk, 64) {
        return Err("noise_pk must be 64 lowercase hex characters".to_string());
    }
    if !valid_short_id(short_id) {
        return Err("short_id must be 8 lowercase letters or digits".to_string());
    }
    if !valid_name(name) {
        return Err("name must be 1 to 32 characters without control characters".to_string());
    }
    if !valid_hex(nonce, 32) {
        return Err("nonce must be 32 lowercase hex characters".to_string());
    }
    let signing = SigningKey::from_bytes(seed);
    let pk = to_hex(&signing.verifying_key().to_bytes());
    let signature = to_hex(&signing.sign(put_message(&pk, noise_pk, short_id, name, ts, nonce).as_bytes()).to_bytes());
    let name_json = serde_json::to_string(name).map_err(|error| error.to_string())?;
    let mut headers = common_headers(config);
    headers.push(("Content-Type".to_string(), "application/json".to_string()));
    Ok(Request {
        method: "POST",
        url: format!("{}/functions/v1/put-peer", config.url),
        headers,
        body: Some(format!(
            r#"{{"pk":"{pk}","noise_pk":"{noise_pk}","short_id":"{short_id}","name":{name_json},"ts":{ts},"nonce":"{nonce}","sig":"{signature}"}}"#
        )),
    })
}

/// Tag of the signed string for deleting a row (its own format; a put signature never verifies as a delete).
pub const DELETE_TAG: &str = "herdr-talk-del-v1";

/// The bytes that are signed for a delete: DELETE_TAG, pk, short_id, ts, nonce, one per line, no newline at the end.
pub fn delete_message(pk: &str, short_id: &str, ts: u64, nonce: &str) -> String {
    format!("{DELETE_TAG}\n{pk}\n{short_id}\n{ts}\n{nonce}")
}

/// Signed request to delete this PC's own row through delete-peer. The relay removes the row only when pk and short_id both match.
pub fn delete_request(config: &Config, seed: &[u8; 32], short_id: &str, ts: u64, nonce: &str) -> Result<Request, String> {
    if !valid_short_id(short_id) {
        return Err("short_id must be 8 lowercase letters or digits".to_string());
    }
    if !valid_hex(nonce, 32) {
        return Err("nonce must be 32 lowercase hex characters".to_string());
    }
    let signing = SigningKey::from_bytes(seed);
    let pk = to_hex(&signing.verifying_key().to_bytes());
    let signature = to_hex(&signing.sign(delete_message(&pk, short_id, ts, nonce).as_bytes()).to_bytes());
    let mut headers = common_headers(config);
    headers.push(("Content-Type".to_string(), "application/json".to_string()));
    Ok(Request {
        method: "POST",
        url: format!("{}/functions/v1/delete-peer", config.url),
        headers,
        body: Some(format!(r#"{{"pk":"{pk}","short_id":"{short_id}","ts":{ts},"nonce":"{nonce}","sig":"{signature}"}}"#)),
    })
}

/// Ask the relay to delete this PC's row. True only for a 2xx answer. Every failure is false; no error text is kept.
pub fn delete_peer(config: &Config, seed: &[u8; 32], short_id: &str) -> bool {
    delete_peer_within(config, seed, short_id, TIMEOUT)
}

/// The same delete with its own limit for the whole request.
pub(crate) fn delete_peer_within(config: &Config, seed: &[u8; 32], short_id: &str, timeout: Duration) -> bool {
    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else { return false };
    let mut random = [0u8; 16];
    if getrandom::getrandom(&mut random).is_err() {
        return false;
    }
    let Ok(request) = delete_request(config, seed, short_id, now.as_secs(), &to_hex(&random)) else { return false };
    let mut builder = agent_within(config, timeout).post(&request.url);
    for (key, value) in &request.headers {
        builder = builder.header(key.as_str(), value.as_str());
    }
    let body = request.body.unwrap_or_default();
    match builder.send(body.as_str()) {
        Ok(response) => (200..300).contains(&response.status().as_u16()),
        Err(_) => false,
    }
}

/// What the put-peer answer means: written, the short ID belongs to another key (409), or any other failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutOutcome {
    Written,
    Taken,
    Failed,
}

pub fn put_outcome(status: u16) -> PutOutcome {
    match status {
        200..=299 => PutOutcome::Written,
        409 => PutOutcome::Taken,
        _ => PutOutcome::Failed,
    }
}

/// Which room request is signed: a create (the new room's owner) or a join.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomAction {
    Create,
    Join,
}

impl RoomAction {
    fn word(self) -> &'static str {
        match self {
            RoomAction::Create => "create",
            RoomAction::Join => "join",
        }
    }
}

/// The bytes signed for a room request (v1): ROOM_TAG, action, room code (an empty line for create), peer pk, display
/// name, ts, nonce, one per line, no newline at the end. The Python relay_rest.room_message and supabase checks.rs match it.
pub fn room_message(action: RoomAction, room_code: &str, peer_pk: &str, display_name: &str, ts: u64, nonce: &str) -> String {
    let code = if action == RoomAction::Create { "" } else { room_code };
    let ts = ts.to_string();
    let newline = char::from(10).to_string();
    [ROOM_TAG, action.word(), code, peer_pk, display_name, ts.as_str(), nonce].join(newline.as_str())
}

/// The JSON body of a room request, signed with the peer's Ed25519 key (the seed from relay_key): peer_pk, display_name,
/// ts, nonce and sig, plus room_code for a join. Refuses bad input before any signature is made.
pub fn signed_room_body(seed: &[u8; 32], action: RoomAction, room_code: &str, display_name: &str, ts: u64, nonce: &str) -> Result<String, String> {
    if action == RoomAction::Join && !valid_room_code(room_code) {
        return Err("room_code must be 8 uppercase letters or digits".to_string());
    }
    if !valid_name(display_name) {
        return Err("display_name must be 1 to 32 characters without control characters".to_string());
    }
    if !valid_hex(nonce, 32) {
        return Err("nonce must be 32 lowercase hex characters".to_string());
    }
    let signing = SigningKey::from_bytes(seed);
    let peer_pk = to_hex(&signing.verifying_key().to_bytes());
    let sig = to_hex(&signing.sign(room_message(action, room_code, &peer_pk, display_name, ts, nonce).as_bytes()).to_bytes());
    let mut body = serde_json::Map::new();
    if action == RoomAction::Join {
        body.insert("room_code".to_string(), Value::String(room_code.to_string()));
    }
    body.insert("peer_pk".to_string(), Value::String(peer_pk));
    body.insert("display_name".to_string(), Value::String(display_name.to_string()));
    body.insert("ts".to_string(), Value::from(ts));
    body.insert("nonce".to_string(), Value::String(nonce.to_string()));
    body.insert("sig".to_string(), Value::String(sig));
    Ok(Value::Object(body).to_string())
}

/// 16 random bytes as 32 lowercase hex characters: the nonce of one request (the server accepts each nonce once).
pub fn random_nonce() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|error| error.to_string())?;
    Ok(to_hex(&bytes))
}

/// Seconds since the Unix epoch (the ts of a signed request; the server accepts 300 seconds either way).
pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_secs())
}

/// Request to join a room by code, signed with this PC's key (ts and a fresh nonce are made here).
pub fn join_room_request(config: &Config, seed: &[u8; 32], room_code: &str, display_name: &str) -> Result<Request, String> {
    signed_join_request(config, seed, room_code, display_name, unix_now(), &random_nonce()?)
}

/// The join request with ts and nonce given (tests use this; join_room_request makes them).
pub(crate) fn signed_join_request(config: &Config, seed: &[u8; 32], room_code: &str, display_name: &str, ts: u64, nonce: &str) -> Result<Request, String> {
    let body = signed_room_body(seed, RoomAction::Join, room_code, display_name, ts, nonce)?;
    let mut headers = common_headers(config);
    headers.push(("Content-Type".to_string(), "application/json".to_string()));
    Ok(Request { method: "POST", url: format!("{}/functions/v1/join-room", config.url), headers, body: Some(body) })
}

/// Response from join-room: room code, owner pk, and list of member pks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomInfo {
    pub room_code: String,
    pub owner_pk: String,
    pub members: Vec<String>,
}

/// Parse the response from join-room Edge Function.
pub fn read_join_room_response(status: u16, body: &str) -> Result<RoomInfo, String> {
    if status == 410 {
        return Err("방 기한이 지났습니다".to_string()); // join-room: the room is past its 24 hours
    }
    if status != 200 {
        return Err(format!("status {status}: {body}"));
    }
    let value: Value = serde_json::from_str(body).map_err(|_| "answer is not JSON".to_string())?;
    let room_code = value["room_code"].as_str().ok_or("response has no room_code")?;
    let owner_pk = value["owner_pk"].as_str().ok_or("response has no owner_pk")?;
    let members = value["members"].as_array().ok_or("response has no members array")?;
    
    if !valid_room_code(room_code) {
        return Err("room_code is not valid".to_string());
    }
    if !valid_pk(owner_pk) {
        return Err("owner_pk is not valid".to_string());
    }
    
    let members: Result<Vec<String>, String> = members
        .iter()
        .map(|m| {
            let pk = m.as_str().ok_or("member is not a string".to_string())?;
            if !valid_pk(pk) {
                return Err("member pk is not valid".to_string());
            }
            Ok(pk.to_string())
        })
        .collect();
    
    Ok(RoomInfo {
        room_code: room_code.to_string(),
        owner_pk: owner_pk.to_string(),
        members: members?,
    })
}

/// Agent for one relay: 5 seconds for the whole request, no proxy for loopback (a test server), the environment's proxy otherwise.
fn agent(config: &Config) -> ureq::Agent {
    agent_within(config, TIMEOUT)
}

fn agent_within(config: &Config, timeout: Duration) -> ureq::Agent {
    let proxy = if is_loopback(&config.url) { None } else { ureq::Proxy::try_from_env() };
    let settings = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .proxy(proxy)
        .build();
    ureq::Agent::new_with_config(settings)
}

/// Signed write of this PC's row. Every failure is Failed; no error text is kept, so a key never reaches a log.
pub fn put_peer(config: &Config, seed: &[u8; 32], noise_pk: &str, short_id: &str, name: &str) -> PutOutcome {
    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else { return PutOutcome::Failed };
    let mut random = [0u8; 16];
    if getrandom::getrandom(&mut random).is_err() {
        return PutOutcome::Failed;
    }
    let Ok(request) = put_request(config, seed, noise_pk, short_id, name, now.as_secs(), &to_hex(&random)) else { return PutOutcome::Failed };
    let mut builder = agent(config).post(&request.url);
    for (key, value) in &request.headers {
        builder = builder.header(key.as_str(), value.as_str());
    }
    let body = request.body.unwrap_or_default();
    match builder.send(body.as_str()) {
        Ok(response) => put_outcome(response.status().as_u16()),
        Err(_) => PutOutcome::Failed,
    }
}

/// Execute an HTTP POST request and return (status, body).
pub fn http_post(request: &Request) -> Result<(u16, String), String> {
    // Create a dummy config for the agent (we only need a timeout, URL will be from request)
    let dummy_config = Config { url: "http://127.0.0.1".to_string(), key: String::new() };
    let mut builder = agent_within(&dummy_config, TIMEOUT).post(&request.url);
    for (key, value) in &request.headers {
        builder = builder.header(key.as_str(), value.as_str());
    }
    let body = request.body.clone().unwrap_or_default();
    let mut response = builder.send(body.as_str()).map_err(|e| format!("HTTP error: {e}"))?;
    let status = response.status().as_u16();
    let response_body = response.body_mut().read_to_string().map_err(|e| format!("Failed to read response: {e}"))?;
    Ok((status, response_body))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // Dummy values only. No test here sends anything.
    fn dummy() -> Config {
        config(Some("http://127.0.0.1:9/"), Some("dummy-test-key")).unwrap().unwrap()
    }

    fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
        request.headers.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }

    #[test]
    fn settings_need_both_values_and_https_unless_loopback() {
        assert_eq!(config(None, Some("k")), Ok(None));
        assert_eq!(config(Some("https://x.example"), Some("  ")), Ok(None));
        assert!(config(Some("https://x.example.co/"), Some("k")).unwrap().is_some());
        assert!(config(Some("http://example.com"), Some("k")).is_err());
        assert!(config(Some("http://localhost:9"), Some("k")).unwrap().is_some());
    }

    #[test]
    fn config_debug_hides_the_key() {
        assert!(!format!("{:?}", dummy()).contains("dummy-test-key"));
    }

    const SEED: [u8; 32] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
    ];
    const NOISE: &str = "abababababababababababababababababababababababababababababababab";
    const VECTOR_NONCE: &str = "00112233445566778899aabbccddeeff";
    const SHORT: &str = "ABCD2345";

    #[test]
    fn v3_message_and_body_are_signed_over_the_fixed_order() {
        let signing = SigningKey::from_bytes(&SEED);
        let pk = to_hex(&signing.verifying_key().to_bytes());
        let message = put_message(&pk, NOISE, SHORT, "Alice", 1700000000, VECTOR_NONCE);
        assert_eq!(message, format!("herdr-talk-put-v3\n{pk}\n{NOISE}\n{SHORT}\nAlice\n1700000000\n{VECTOR_NONCE}"));
        let request = put_request(&dummy(), &SEED, NOISE, SHORT, "Alice", 1700000000, VECTOR_NONCE).unwrap();
        let body = request.body.unwrap();
        let sig = to_hex(&signing.sign(message.as_bytes()).to_bytes());
        assert_eq!(body, format!(r#"{{"pk":"{pk}","noise_pk":"{NOISE}","short_id":"{SHORT}","name":"Alice","ts":1700000000,"nonce":"{VECTOR_NONCE}","sig":"{sig}"}}"#));
        let public: [u8; 32] = std::array::from_fn(|index| u8::from_str_radix(&pk[index * 2..index * 2 + 2], 16).unwrap());
        let sig_bytes: [u8; 64] = std::array::from_fn(|index| u8::from_str_radix(&sig[index * 2..index * 2 + 2], 16).unwrap());
        let key = ed25519_dalek::VerifyingKey::from_bytes(&public).unwrap();
        assert!(key.verify_strict(message.as_bytes(), &ed25519_dalek::Signature::from_bytes(&sig_bytes)).is_ok());
    }

    #[test]
    fn put_request_is_a_signed_put_peer_call() {
        let request = put_request(&dummy(), &SEED, NOISE, SHORT, "Alice", 1700000000, VECTOR_NONCE).unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.url, "http://127.0.0.1:9/functions/v1/put-peer");
        assert_eq!(header(&request, "apikey"), Some("dummy-test-key"));
        assert_eq!(header(&request, "Authorization"), Some("Bearer dummy-test-key"));
        assert_eq!(header(&request, "x-herdr-pk"), None);
    }

    #[test]
    fn delete_message_and_body_are_signed_over_the_fixed_order() {
        let signing = SigningKey::from_bytes(&SEED);
        let pk = to_hex(&signing.verifying_key().to_bytes());
        let message = delete_message(&pk, SHORT, 1700000000, VECTOR_NONCE);
        assert_eq!(message, format!("herdr-talk-del-v1\n{pk}\n{SHORT}\n1700000000\n{VECTOR_NONCE}"));
        let request = delete_request(&dummy(), &SEED, SHORT, 1700000000, VECTOR_NONCE).unwrap();
        assert_eq!(request.url, "http://127.0.0.1:9/functions/v1/delete-peer");
        let sig = to_hex(&signing.sign(message.as_bytes()).to_bytes());
        assert_eq!(request.body.unwrap(), format!(r#"{{"pk":"{pk}","short_id":"{SHORT}","ts":1700000000,"nonce":"{VECTOR_NONCE}","sig":"{sig}"}}"#));
        assert!(delete_request(&dummy(), &SEED, "abcd2345", 1, VECTOR_NONCE).is_err());
    }

    #[test]
    fn delete_peer_sends_the_signed_body_and_accepts_204() {
        let (config, received) = serve_once("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        assert!(delete_peer(&config, &SEED, SHORT));
        let raw = received.recv().unwrap();
        assert!(raw.starts_with("POST /functions/v1/delete-peer HTTP/1.1\r\n"), "{raw}");
        assert!(raw.contains(&format!(r#""short_id":"{SHORT}","ts":"#)), "{raw}");
    }

    #[test]
    fn delete_peer_is_false_for_a_refusal() {
        let (config, _received) = serve_once("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        assert!(!delete_peer(&config, &SEED, SHORT));
    }

    #[test]
    fn bad_input_is_refused_before_a_request_exists() {
        assert!(put_request(&dummy(), &SEED, &"AB".repeat(32), SHORT, "A", 1, VECTOR_NONCE).is_err());
        assert!(put_request(&dummy(), &SEED, NOISE, SHORT, "", 1, VECTOR_NONCE).is_err());
        assert!(put_request(&dummy(), &SEED, NOISE, SHORT, &"x".repeat(33), 1, VECTOR_NONCE).is_err());
        assert!(put_request(&dummy(), &SEED, NOISE, SHORT, "a\u{7}b", 1, VECTOR_NONCE).is_err());
        assert!(put_request(&dummy(), &SEED, NOISE, SHORT, "A", 1, "ABC").is_err());
        assert!(put_request(&dummy(), &SEED, NOISE, "abcd2345", "A", 1, VECTOR_NONCE).is_err());
        assert!(put_request(&dummy(), &SEED, NOISE, "abc", "A", 1, VECTOR_NONCE).is_err());
    }

    // Fixed vectors shared with python/src/relay_rest.py and supabase/functions/*/checks.test.ts: seed bytes 0..31.
    fn vector_seed() -> [u8; 32] {
        std::array::from_fn(|index| index as u8)
    }

    #[test]
    fn room_signatures_match_the_shared_vectors() {
        let seed = vector_seed();
        let nonce = "00112233445566778899aabbccddeeff";
        let create: Value = serde_json::from_str(&signed_room_body(&seed, RoomAction::Create, "", "Alice", 1_700_000_000, nonce).unwrap()).unwrap();
        assert_eq!(create["peer_pk"], "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8");
        assert_eq!(create["sig"], "f26ec1d02def0df18204cf6507e74e121432cf29ff999b89f30b3137f6084fd2e10b3b6a5d1982a13a531547bbb23370fe821a91a27af0b616b3a05c9b718b0c");
        assert!(create.get("room_code").is_none(), "create has no room code");
        let join: Value = serde_json::from_str(&signed_room_body(&seed, RoomAction::Join, "ABCD1234", "Bob", 1_700_000_000, nonce).unwrap()).unwrap();
        assert_eq!(join["room_code"], "ABCD1234");
        assert_eq!(join["sig"], "4128444185a03e137deca1e066f73d7bf2121162a1f620c01b75f0e217511189388e4da58a7774bb5b18f0e2263a77ff65136eeab8b6a765ac53ca8a15f33c08");
    }

    #[test]
    fn the_signed_message_is_the_seven_line_string() {
        let text = room_message(RoomAction::Create, "", "pk", "Alice", 1, "n");
        assert_eq!(text.split(char::from(10)).collect::<Vec<_>>(), ["herdr-talk-room-v1", "create", "", "pk", "Alice", "1", "n"]);
    }

    #[test]
    fn a_join_request_is_signed_and_refuses_bad_input() {
        let seed = vector_seed();
        let request = signed_join_request(&dummy(), &seed, "ABCD1234", "Bob", 1_700_000_000, "00112233445566778899aabbccddeeff").unwrap();
        assert_eq!(request.url, "http://127.0.0.1:9/functions/v1/join-room");
        let body: Value = serde_json::from_str(request.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["sig"], "4128444185a03e137deca1e066f73d7bf2121162a1f620c01b75f0e217511189388e4da58a7774bb5b18f0e2263a77ff65136eeab8b6a765ac53ca8a15f33c08");
        assert!(signed_join_request(&dummy(), &seed, "abcd1234", "Bob", 1, "00112233445566778899aabbccddeeff").is_err());
        assert!(signed_join_request(&dummy(), &seed, "ABCD1234", "Bob", 1, "short").is_err());
        assert!(join_room_request(&dummy(), &seed, "ABCD1234", "Bob").is_ok());
    }

    #[test]
    fn an_expired_room_says_so() {
        let error = read_join_room_response(410, r#"{"error":"방 기한이 지났습니다"}"#).unwrap_err();
        assert_eq!(error, "방 기한이 지났습니다");
    }

    #[test]
    fn name_length_counts_characters() {
        assert!(valid_name(&"가".repeat(32)));
        assert!(!valid_name(&"가".repeat(33)));
    }

    #[test]
    fn write_status_decides_the_outcome() {
        assert_eq!(put_outcome(201), PutOutcome::Written);
        assert_eq!(put_outcome(204), PutOutcome::Written);
        assert_eq!(put_outcome(409), PutOutcome::Taken);
        assert_eq!(put_outcome(401), PutOutcome::Failed);
    }

    /// One-shot local HTTP server on 127.0.0.1. Hands back the request it read, then answers with `response`.
    pub(crate) fn serve_once(response: &'static str) -> (Config, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                request.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut payload = vec![0u8; length];
            reader.read_exact(&mut payload).unwrap();
            request.push_str(&String::from_utf8(payload).unwrap());
            sent.send(request).unwrap();
            stream.write_all(response.as_bytes()).unwrap();
        });
        let config = config(Some(&format!("http://127.0.0.1:{port}")), Some("dummy-test-key")).unwrap().unwrap();
        (config, received)
    }

    #[test]
    fn put_peer_sends_the_signed_body_and_accepts_201() {
        let (config, received) = serve_once("HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        assert_eq!(put_peer(&config, &SEED, NOISE, SHORT, "Alice"), PutOutcome::Written);
        let raw = received.recv().unwrap();
        let request = raw.to_ascii_lowercase();
        assert!(request.starts_with("post /functions/v1/put-peer http/1.1\r\n"), "{request}");
        assert!(request.contains("apikey: dummy-test-key\r\n"));
        assert!(request.contains("authorization: bearer dummy-test-key\r\n"));
        assert!(!request.contains("x-herdr-pk"));
        assert!(raw.contains(&format!(r#""short_id":"{SHORT}","name":"Alice""#)), "{raw}");
    }

    #[test]
    fn put_peer_reports_a_taken_short_id_and_a_refusal() {
        let (config, _received) = serve_once("HTTP/1.1 409 Conflict\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        assert_eq!(put_peer(&config, &SEED, NOISE, SHORT, "Alice"), PutOutcome::Taken);
        let (config, _received) = serve_once("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        assert_eq!(put_peer(&config, &SEED, NOISE, SHORT, "Alice"), PutOutcome::Failed);
    }

    #[test]
    fn short_id_is_eight_uppercase_letters_or_digits() {
        assert!(valid_short_id("ABCD2345"));
        for bad in ["ABCD234", "ABCD23456", "abcd2345", "ABCD-234", "AB CD234", ""] {
            assert!(!valid_short_id(bad), "{bad}");
        }
    }

}
