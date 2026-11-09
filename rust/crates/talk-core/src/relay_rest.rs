//! Supabase REST side of the relay: settings, the requests for the `peers` table, and checks of the answers.
//! This module builds requests and reads answers; it does not send anything. (Sending needs TLS, which is not in
//! talk-core yet; see the reply that added this module.) Same rules as python/src/relay_rest.py.

use ed25519_dalek::{Signer, SigningKey};
use serde_json::Value;
use std::time::Duration;

pub const URL_VARIABLE: &str = "HERDR_TALK_RELAY_URL";
pub const KEY_VARIABLE: &str = "HERDR_TALK_RELAY_KEY";
pub const PUT_TAG: &str = "herdr-talk-put-v3";
const TIMEOUT: Duration = Duration::from_secs(5);

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
    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    /// The publishable key. Crate-internal, used only to build the websocket URL.
    pub(crate) fn key(&self) -> &str {
        &self.key
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

/// A peer as the relay returns it, after checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub pk: String,
    pub name: String,
    /// The X25519 key of the peer's Noise identity, as written by put-peer.
    pub noise_pk: String,
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

pub fn config_from_env() -> Result<Option<Config>, String> {
    config(std::env::var(URL_VARIABLE).ok().as_deref(), std::env::var(KEY_VARIABLE).ok().as_deref())
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

/// 8 lowercase characters, letters and digits: the short ID that others type to find this PC.
pub fn valid_short_id(text: &str) -> bool {
    text.len() == 8 && text.bytes().all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
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

/// Lookup of one peer by pk.
pub fn get_request(config: &Config, pk: &str) -> Result<Request, String> {
    if !valid_pk(pk) {
        return Err("pk must be 64 lowercase hex characters".to_string());
    }
    let mut headers = common_headers(config);
    headers.push(("Accept".to_string(), "application/json".to_string()));
    Ok(Request {
        method: "GET",
        url: format!("{}/rest/v1/peers?pk=eq.{pk}&select=pk,name,noise_pk", config.url),
        headers,
        body: None,
    })
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

/// The answer to a lookup. Ok(None): no such peer. Anything malformed is an error, and the caller drops it.
pub fn read_get_response(status: u16, body: &str, pk: &str) -> Result<Option<Peer>, String> {
    if status != 200 {
        return Err(format!("status {status}"));
    }
    let rows: Value = serde_json::from_str(body).map_err(|_| "answer is not JSON".to_string())?;
    let rows = rows.as_array().ok_or("answer is not a list")?;
    let row = match rows.as_slice() {
        [] => return Ok(None),
        [row] => row,
        _ => return Err("more than one row for one pk".to_string()),
    };
    let answered_pk = row["pk"].as_str().ok_or("row has no pk")?;
    let name = row["name"].as_str().ok_or("row has no name")?;
    let noise_pk = row["noise_pk"].as_str().ok_or("row has no noise_pk")?;
    if !valid_hex(noise_pk, 64) {
        return Err("row noise_pk is not valid".to_string());
    }
    if answered_pk != pk || !valid_pk(answered_pk) {
        return Err("row pk does not match the request".to_string());
    }
    if !valid_name(name) {
        return Err("row name is not valid".to_string());
    }
    Ok(Some(Peer { pk: answered_pk.to_string(), name: name.to_string(), noise_pk: noise_pk.to_string() }))
}

/// Lookup by short ID: `short_id = eq.<id>`, at most one row (the column is unique).
pub fn short_id_request(config: &Config, short_id: &str) -> Result<Request, String> {
    if !valid_short_id(short_id) {
        return Err("short_id must be 8 lowercase letters or digits".to_string());
    }
    let mut headers = common_headers(config);
    headers.push(("Accept".to_string(), "application/json".to_string()));
    Ok(Request {
        method: "GET",
        url: format!("{}/rest/v1/peers?short_id=eq.{short_id}&select=pk,name,noise_pk,short_id", config.url),
        headers,
        body: None,
    })
}

/// The answer to a short ID lookup: exactly one checked row. Zero rows (or anything else) is "not found" for the user.
pub fn read_short_response(status: u16, body: &str, short_id: &str) -> Result<Peer, String> {
    if status != 200 {
        return Err("the relay did not answer".to_string());
    }
    let rows: Value = serde_json::from_str(body).map_err(|_| "the relay answer is not JSON".to_string())?;
    let rows = rows.as_array().ok_or("the relay answer is not a list")?;
    let row = match rows.as_slice() {
        [row] => row,
        _ => return Err("상대를 찾지 못함".to_string()),
    };
    let pk = row["pk"].as_str().ok_or("row has no pk")?;
    let name = row["name"].as_str().ok_or("row has no name")?;
    let noise_pk = row["noise_pk"].as_str().ok_or("row has no noise_pk")?;
    if row["short_id"].as_str() != Some(short_id) || !valid_pk(pk) {
        return Err("row does not match the request".to_string());
    }
    if !valid_name(name) || !valid_hex(noise_pk, 64) {
        return Err("row is not valid".to_string());
    }
    Ok(Peer { pk: pk.to_string(), name: name.to_string(), noise_pk: noise_pk.to_string() })
}

/// Find the peer for the short ID the user typed. The full key of the row found is what the caller uses from then on.
/// Every error is a message for the user.
pub fn find_peer(config: &Config, short_id: &str) -> Result<Peer, String> {
    let request = short_id_request(config, short_id).map_err(|_| "상대 ID는 8자리 (소문자 영문·숫자)".to_string())?;
    let mut builder = agent(config).get(&request.url);
    for (key, value) in &request.headers {
        builder = builder.header(key.as_str(), value.as_str());
    }
    let mut response = builder.call().map_err(|_| "the relay did not answer".to_string())?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().map_err(|_| "the relay did not answer".to_string())?;
    read_short_response(status, &body, short_id)
}

/// Agent for one relay: 5 seconds for the whole request, no proxy for loopback (a test server), the environment's proxy otherwise.
fn agent(config: &Config) -> ureq::Agent {
    let proxy = if is_loopback(&config.url) { None } else { ureq::Proxy::try_from_env() };
    let settings = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
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

/// Look up one peer on the relay. None when there is no such peer, the call failed, or the answer does not check out.
pub fn get_peer(config: &Config, pk: &str) -> Option<Peer> {
    let request = get_request(config, pk).ok()?;
    let mut builder = agent(config).get(&request.url);
    for (key, value) in &request.headers {
        builder = builder.header(key.as_str(), value.as_str());
    }
    let mut response = builder.call().ok()?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().ok()?;
    read_get_response(status, &body, pk).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Dummy values only. No test here sends anything.
    const PK: &str = "0101010101010101010101010101010101010101010101010101010101010101";

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
    const SHORT: &str = "abcd2345";

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
    fn get_request_selects_pk_and_name() {
        let request = get_request(&dummy(), PK).unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, format!("http://127.0.0.1:9/rest/v1/peers?pk=eq.{PK}&select=pk,name,noise_pk"));
        assert!(request.body.is_none());
    }

    #[test]
    fn bad_input_is_refused_before_a_request_exists() {
        assert!(put_request(&dummy(), &SEED, &"AB".repeat(32), SHORT, "A", 1, VECTOR_NONCE).is_err());
        assert!(put_request(&dummy(), &SEED, NOISE, SHORT, "", 1, VECTOR_NONCE).is_err());
        assert!(put_request(&dummy(), &SEED, NOISE, SHORT, &"x".repeat(33), 1, VECTOR_NONCE).is_err());
        assert!(put_request(&dummy(), &SEED, NOISE, SHORT, "a\u{7}b", 1, VECTOR_NONCE).is_err());
        assert!(put_request(&dummy(), &SEED, NOISE, SHORT, "A", 1, "ABC").is_err());
        assert!(put_request(&dummy(), &SEED, NOISE, "ABCD2345", "A", 1, VECTOR_NONCE).is_err());
        assert!(put_request(&dummy(), &SEED, NOISE, "abc", "A", 1, VECTOR_NONCE).is_err());
        assert!(get_request(&dummy(), "abc").is_err());
    }

    #[test]
    fn name_length_counts_characters() {
        assert!(valid_name(&"가".repeat(32)));
        assert!(!valid_name(&"가".repeat(33)));
    }

    #[test]
    fn answers_are_checked() {
        let good = format!(r#"[{{"pk":"{PK}","name":"Alice","noise_pk":"abababababababababababababababababababababababababababababababab"}}]"#);
        assert_eq!(read_get_response(200, &good, PK), Ok(Some(Peer { pk: PK.into(), name: "Alice".into(), noise_pk: NOISE.into() })));
        assert_eq!(read_get_response(200, "[]", PK), Ok(None));
        assert!(read_get_response(401, &good, PK).is_err());
        assert!(read_get_response(200, "{}", PK).is_err());
        assert!(read_get_response(200, "not json", PK).is_err());
        assert!(read_get_response(200, &good, &"2".repeat(64)).is_err());
        assert!(read_get_response(200, &format!(r#"[{{"pk":"{PK}","name":"","noise_pk":"abababababababababababababababababababababababababababababababab"}}]"#), PK).is_err());
        assert!(read_get_response(200, &format!("[{good_row},{good_row}]", good_row = r#"{"pk":"x"}"#), PK).is_err());
    }

    #[test]
    fn write_status_decides_the_outcome() {
        assert_eq!(put_outcome(201), PutOutcome::Written);
        assert_eq!(put_outcome(204), PutOutcome::Written);
        assert_eq!(put_outcome(409), PutOutcome::Taken);
        assert_eq!(put_outcome(401), PutOutcome::Failed);
    }

    /// One-shot local HTTP server on 127.0.0.1. Hands back the request it read, then answers with `response`.
    fn serve_once(response: &'static str) -> (Config, std::sync::mpsc::Receiver<String>) {
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
    fn get_peer_reads_one_checked_row() {
        let body = format!(r#"[{{"pk":"{PK}","name":"Alice","noise_pk":"abababababababababababababababababababababababababababababababab"}}]"#);
        let response: &'static str = Box::leak(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_boxed_str());
        let (config, received) = serve_once(response);
        assert_eq!(get_peer(&config, PK), Some(Peer { pk: PK.into(), name: "Alice".into(), noise_pk: NOISE.into() }));
        let request = received.recv().unwrap().to_ascii_lowercase();
        assert!(request.starts_with(&format!("get /rest/v1/peers?pk=eq.{PK}&select=pk,name,noise_pk http/1.1")), "{request}");
    }

    #[test]
    fn short_id_is_eight_lowercase_letters_or_digits() {
        assert!(valid_short_id("abcd2345"));
        for bad in ["abcd234", "abcd23456", "ABCD2345", "abcd-234", "ab cd234", ""] {
            assert!(!valid_short_id(bad), "{bad}");
        }
    }

    #[test]
    fn short_lookup_needs_exactly_one_row_with_the_same_short_id() {
        let full = "ab".repeat(32);
        let row = format!(r#"{{"pk":"{full}","name":"Bob","noise_pk":"{}","short_id":"abcd2345"}}"#, "cd".repeat(32));
        assert_eq!(read_short_response(200, &format!("[{row}]"), "abcd2345").unwrap().pk, full);
        assert!(read_short_response(200, "[]", "abcd2345").unwrap_err().contains("찾지"));
        assert!(read_short_response(200, &format!("[{row},{row}]"), "abcd2345").unwrap_err().contains("찾지"));
        assert!(read_short_response(500, &format!("[{row}]"), "abcd2345").is_err());
        assert!(read_short_response(200, &format!("[{row}]"), "zzzz9999").is_err(), "row must carry the asked short_id");
    }

    #[test]
    fn short_lookup_request_uses_eq() {
        let request = short_id_request(&dummy(), "abcd2345").unwrap();
        assert!(request.url.ends_with("/rest/v1/peers?short_id=eq.abcd2345&select=pk,name,noise_pk,short_id"), "{}", request.url);
        assert!(short_id_request(&dummy(), "abc").is_err());
        assert!(short_id_request(&dummy(), "ABCD2345").is_err());
    }

    #[test]
    fn find_peer_refuses_other_forms_without_a_request() {
        assert!(find_peer(&dummy(), &"ab".repeat(32)).unwrap_err().contains("8"));
        assert!(find_peer(&dummy(), "abc").unwrap_err().contains("8"));
    }
}
