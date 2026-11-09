//! Wire format v1 for the relay (docs/supabase-relay.md): the channel name and the one-line envelope.
//! Only the bytes are handled here. `ct` is whatever Noise produced; this module never encrypts.

use crate::store::{from_hex, to_hex};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;

const CHANNEL_PREFIX: &[u8] = b"herdr-talk-relay-v1";
const KEY_LEN: usize = 32;
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Why an envelope is dropped. Each reason is a separate case, so a new one shows up as a compile error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    Format,
    Version,
    Party,
    Channel,
}

impl fmt::Display for Reject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Reject::Format => "envelope is not a v1 JSON line",
            Reject::Version => "unsupported envelope version",
            Reject::Party => "sender is not one of the two keys",
            Reject::Channel => "channel does not match the two keys",
        };
        f.write_str(text)
    }
}

/// The channel name for two public keys: hex(SHA-256(prefix || min || max)). The order of the keys does not matter.
pub fn channel_name(a: &[u8], b: &[u8]) -> String {
    let (low, high) = if a <= b { (a, b) } else { (b, a) };
    let mut hasher = Sha256::new();
    hasher.update(CHANNEL_PREFIX);
    hasher.update(low);
    hasher.update(high);
    to_hex(&hasher.finalize())
}

/// One envelope as a JSON line. Keys come out in the order v, channel, from, to, ct (built by hand, not by a map).
/// Both keys must be 32 bytes; hex and base64 never need JSON escaping.
pub fn seal(from: &[u8], to: &[u8], ct: &[u8]) -> Option<String> {
    if from.len() != KEY_LEN || to.len() != KEY_LEN {
        return None;
    }
    Some(format!(
        r#"{{"v":1,"channel":"{}","from":"{}","to":"{}","ct":"{}"}}"#,
        channel_name(from, to),
        to_hex(from),
        to_hex(to),
        base64_encode(ct),
    ))
}

/// A checked envelope: the two keys of the channel, the sender, and the ciphertext bytes.
#[derive(Debug, PartialEq, Eq)]
pub struct Envelope {
    pub from: Vec<u8>,
    pub to: Vec<u8>,
    pub ct: Vec<u8>,
}

/// Check one line against the two keys this side expects on the channel (`pair`). Nothing is trusted before this passes.
pub fn open(line: &str, pair: (&[u8], &[u8])) -> Result<Envelope, Reject> {
    let value: Value = serde_json::from_str(line).map_err(|_| Reject::Format)?;
    match value["v"].as_u64() {
        Some(1) => {}
        Some(_) => return Err(Reject::Version),
        None => return Err(Reject::Format),
    }
    let from = key_field(&value["from"])?;
    let to = key_field(&value["to"])?;
    let channel = value["channel"].as_str().ok_or(Reject::Format)?;
    let ct = base64_decode(value["ct"].as_str().ok_or(Reject::Format)?).ok_or(Reject::Format)?;
    if channel != channel_name(pair.0, pair.1) {
        return Err(Reject::Channel);
    }
    let in_pair = |key: &[u8]| key == pair.0 || key == pair.1;
    if !in_pair(&from) || !in_pair(&to) {
        return Err(Reject::Party);
    }
    Ok(Envelope { from, to, ct })
}

/// A key field: 64 lowercase hex characters, 32 bytes.
fn key_field(value: &Value) -> Result<Vec<u8>, Reject> {
    let text = value.as_str().ok_or(Reject::Format)?;
    let key = from_hex(text).filter(|key| key.len() == KEY_LEN).ok_or(Reject::Format)?;
    if to_hex(&key) != text {
        return Err(Reject::Format); // uppercase hex is not the v1 form
    }
    Ok(key)
}

/// Standard base64 with padding (RFC 4648).
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(BASE64[(n >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Inverse of `base64_encode`. None for anything that is not padded standard base64.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(4) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let quads = bytes.len() / 4;
    for (index, quad) in bytes.chunks(4).enumerate() {
        let pad = quad.iter().rev().take_while(|byte| **byte == b'=').count();
        if pad > 2 || (pad > 0 && index + 1 != quads) {
            return None;
        }
        let mut n = 0u32;
        for (position, byte) in quad.iter().enumerate() {
            let value = if position >= 4 - pad {
                0
            } else {
                BASE64.iter().position(|symbol| symbol == byte)? as u32
            };
            n = n << 6 | value;
        }
        out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8][..3 - pad]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test vector: keys 0x01×32 and 0x02×32. The channel was computed outside this code with
    // `sha256sum` over the bytes "herdr-talk-relay-v1" || 0x01×32 || 0x02×32.
    const KEY_A: [u8; 32] = [1; 32];
    const KEY_B: [u8; 32] = [2; 32];
    const CHANNEL: &str = "c3492c89862240d31414b19d37da93d42c204f15687632dd436238f5701eb83e";
    const CT: [u8; 4] = [0, 1, 2, 255];
    const ENVELOPE: &str = r#"{"v":1,"channel":"c3492c89862240d31414b19d37da93d42c204f15687632dd436238f5701eb83e","from":"0101010101010101010101010101010101010101010101010101010101010101","to":"0202020202020202020202020202020202020202020202020202020202020202","ct":"AAEC/w=="}"#;

    #[test]
    fn channel_name_matches_the_external_vector_in_either_order() {
        assert_eq!(channel_name(&KEY_A, &KEY_B), CHANNEL);
        assert_eq!(channel_name(&KEY_B, &KEY_A), CHANNEL);
    }

    #[test]
    fn seal_gives_the_vector_line() {
        assert_eq!(seal(&KEY_A, &KEY_B, &CT).unwrap(), ENVELOPE);
    }

    #[test]
    fn open_reads_the_vector_line_from_either_side() {
        let expected = Envelope { from: KEY_A.to_vec(), to: KEY_B.to_vec(), ct: CT.to_vec() };
        assert_eq!(open(ENVELOPE, (&KEY_A, &KEY_B)).unwrap(), expected);
        assert_eq!(open(ENVELOPE, (&KEY_B, &KEY_A)).unwrap(), expected);
    }

    #[test]
    fn open_rejects_version_two() {
        let line = ENVELOPE.replacen(r#""v":1"#, r#""v":2"#, 1);
        assert_eq!(open(&line, (&KEY_A, &KEY_B)), Err(Reject::Version));
    }

    #[test]
    fn open_rejects_a_sender_outside_the_pair() {
        let line = ENVELOPE.replacen(&to_hex(&KEY_A), &to_hex(&[3; 32]), 1);
        assert_eq!(open(&line, (&KEY_A, &KEY_B)), Err(Reject::Party));
    }

    #[test]
    fn open_rejects_a_channel_that_does_not_match() {
        let line = ENVELOPE.replacen(CHANNEL, &"0".repeat(64), 1);
        assert_eq!(open(&line, (&KEY_A, &KEY_B)), Err(Reject::Channel));
    }

    #[test]
    fn open_rejects_bad_shapes() {
        assert_eq!(open("not json", (&KEY_A, &KEY_B)), Err(Reject::Format));
        let key = [0xab; 32];
        let line = seal(&key, &KEY_B, &CT).unwrap();
        let upper = line.replacen(&to_hex(&key), &to_hex(&key).to_uppercase(), 1);
        assert_eq!(open(&upper, (&key, &KEY_B)), Err(Reject::Format));
    }

    #[test]
    fn base64_matches_rfc_4648() {
        for (plain, coded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64_encode(plain.as_bytes()), coded);
            assert_eq!(base64_decode(coded).unwrap(), plain.as_bytes());
        }
        assert_eq!(base64_decode("Zg"), None);
        assert_eq!(base64_decode("Z==="), None);
    }
}
