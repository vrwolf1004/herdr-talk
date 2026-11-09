//! Cross-check against the JavaScript side (mobile/test-vectors/relay.json, written by mobile/test-vectors/generate.ts).
//! This test computes every value with the Rust code and compares it with the file. Nothing is copied from Rust into the file.

#![cfg(test)]

use crate::relay;
use crate::relay_rest;
use crate::store;
use ed25519_dalek::{Signer, SigningKey, Verifier};
use serde_json::Value;

fn vector_file() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../mobile/test-vectors/relay.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap()
}

fn bytes(value: &Value, key: &str) -> Vec<u8> {
    store::from_hex(value[key].as_str().unwrap()).unwrap()
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap().to_string()
}

#[test]
fn rust_signs_the_delete_like_the_javascript_side() {
    let file = vector_file();
    let inputs = &file["inputs"];
    let outputs = &file["outputs"];
    let seed: [u8; 32] = bytes(inputs, "signingSeed").try_into().unwrap();
    let signing = SigningKey::from_bytes(&seed);
    let pk = store::to_hex(&signing.verifying_key().to_bytes());
    assert_eq!(pk, text(outputs, "rowPublic"));
    let message = relay_rest::delete_message(
        &pk,
        &text(inputs, "deleteShortId"),
        inputs["deleteTs"].as_u64().unwrap(),
        &text(inputs, "deleteNonce"),
    );
    let signature = signing.sign(message.as_bytes());
    assert_eq!(store::to_hex(&signature.to_bytes()), text(outputs, "deleteSignature"));
    assert!(signing.verifying_key().verify(message.as_bytes(), &signature).is_ok());
}

#[test]
fn rust_computes_the_same_values_as_the_javascript_side() {
    let file = vector_file();
    let inputs = &file["inputs"];
    let outputs = &file["outputs"];

    let initiator_private = bytes(inputs, "initiatorStaticPrivate");
    let ephemeral = bytes(inputs, "ephemeralPrivate");
    let builder = || snow::Builder::new("Noise_IK_25519_ChaChaPoly_BLAKE2s".parse().unwrap());

    let mut initiator = builder()
        .local_private_key(&initiator_private)
        .unwrap()
        .remote_public_key(&bytes(outputs, "responderPublic"))
        .unwrap()
        .fixed_ephemeral_key_for_testing_only(&ephemeral)
        .build_initiator()
        .unwrap();
    let payload = text(outputs, "payloadJson");
    let mut message = vec![0u8; 65535];
    let length = initiator.write_message(payload.as_bytes(), &mut message).unwrap();
    message.truncate(length);
    assert_eq!(store::to_hex(&message), text(outputs, "noiseMessage1"), "Noise message 1 differs");

    // The responder (fixed static key) reads the same message and gets the payload and the initiator's key.
    let mut responder = builder().local_private_key(&bytes(inputs, "responderStaticPrivate")).unwrap().build_responder().unwrap();
    let mut plain = vec![0u8; 65535];
    let read = responder.read_message(&message, &mut plain).unwrap();
    assert_eq!(&plain[..read], payload.as_bytes());
    assert_eq!(store::to_hex(responder.get_remote_static().unwrap()), text(outputs, "initiatorPublic"));

    // Envelope and channel name, from the two public keys.
    let initiator_public = bytes(outputs, "initiatorPublic");
    let responder_public = bytes(outputs, "responderPublic");
    assert_eq!(relay::channel_name(&initiator_public, &responder_public), text(outputs, "channelFromNoiseKeys"));
    assert_eq!(relay::channel_name(&bytes(inputs, "channelKeyA"), &bytes(inputs, "channelKeyB")), text(outputs, "channelOfKnownKeys"));
    let envelope = relay::seal(&initiator_public, &responder_public, &message).unwrap();
    assert_eq!(envelope, text(outputs, "envelope"));

    // Fingerprint code of the two Noise keys.
    assert_eq!(store::fingerprint(&initiator_public, &responder_public), text(outputs, "fingerprint"));
}

// Row signature (put-peer v3, with short_id), from the shared vector file.
#[test]
fn rust_signs_the_v3_row_like_the_javascript_side() {
    let file = vector_file();
    let inputs = &file["inputs"];
    let outputs = &file["outputs"];
    let seed: [u8; 32] = bytes(inputs, "signingSeed").try_into().unwrap();
    let signing = SigningKey::from_bytes(&seed);
    let pk = store::to_hex(&signing.verifying_key().to_bytes());
    assert_eq!(pk, text(outputs, "rowPublic"));
    let row_message = relay_rest::put_message(
        &pk,
        &text(inputs, "rowNoisePk"),
        &text(inputs, "rowShortId"),
        &text(inputs, "rowName"),
        inputs["rowTs"].as_u64().unwrap(),
        &text(inputs, "rowNonce"),
    );
    let signature = signing.sign(row_message.as_bytes());
    assert_eq!(store::to_hex(&signature.to_bytes()), text(outputs, "rowSignature"));
    assert!(signing.verifying_key().verify(row_message.as_bytes(), &signature).is_ok());
}
