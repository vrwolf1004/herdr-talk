//! Cross-check against the JavaScript side (mobile/test-vectors/relay.json, written by mobile/test-vectors/generate.ts).
//! This test computes every value with the Rust code and compares it with the file. Nothing is copied from Rust into the file.

#![cfg(test)]

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

    // Fingerprint code of the two Noise keys.
    let initiator_public = bytes(outputs, "initiatorPublic");
    let responder_public = bytes(outputs, "responderPublic");
    // The vector file (shared with mobile) still writes the code with a dash; the code itself is the same.
    assert_eq!(store::fingerprint(&initiator_public, &responder_public), text(outputs, "fingerprint").replace('-', ""));
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

// Room messages (relay_chat/room_relay.rs, relay_rest.rs). The values come from the Python side (core/room.py,
// core/relay_rest.py); Rust computes the signatures and the digest again and reads the Noise ciphertext itself.
#[test]
fn rust_computes_the_room_message_values_like_the_python_side() {
    let file = vector_file();
    let inputs = &file["inputs"];
    let outputs = &file["outputs"];
    let room_code = text(inputs, "roomCode");
    let ts = inputs["roomTs"].as_u64().unwrap();
    let nonce = text(inputs, "roomNonce");
    let message_id = text(inputs, "roomMessageId");

    let sender_seed: [u8; 32] = bytes(inputs, "roomSenderSeed").try_into().unwrap();
    let sender = SigningKey::from_bytes(&sender_seed);
    let sender_pk = store::to_hex(&sender.verifying_key().to_bytes());
    assert_eq!(sender_pk, text(outputs, "roomSenderPublic"));

    let recipient_pk = text(outputs, "roomRecipientPublic");
    let ciphertext = text(outputs, "roomCiphertext");
    let digest = relay_rest::deliveries_digest(&[relay_rest::Delivery { recipient_pk: recipient_pk.clone(), ciphertext: ciphertext.clone() }]);
    assert_eq!(digest, text(outputs, "roomDeliveriesDigest"));

    let post_lines = [relay_rest::POST_TAG, "post", &room_code, &sender_pk, &message_id, &ts.to_string(), &nonce, &digest].join("\n");
    assert_eq!(post_lines, text(outputs, "roomPostSignedText"));
    assert_eq!(store::to_hex(&sender.sign(post_lines.as_bytes()).to_bytes()), text(outputs, "roomPostSig"));

    let recipient_seed: [u8; 32] = bytes(inputs, "roomRecipientSeed").try_into().unwrap();
    let recipient = SigningKey::from_bytes(&recipient_seed);
    assert_eq!(store::to_hex(&recipient.verifying_key().to_bytes()), recipient_pk);
    let fetch_lines = [relay_rest::FETCH_TAG, "fetch", &room_code, &recipient_pk, &ts.to_string(), &nonce].join("\n");
    assert_eq!(fetch_lines, text(outputs, "roomFetchSignedText"));
    assert_eq!(store::to_hex(&recipient.sign(fetch_lines.as_bytes()).to_bytes()), text(outputs, "roomFetchSig"));

    // The Python ciphertext opens with the recipient's Noise key, to the payload and the sender's Noise key.
    let mut responder = crate::net::noise_builder()
        .local_private_key(&bytes(inputs, "roomRecipientNoisePrivate"))
        .and_then(|builder| builder.build_responder())
        .unwrap();
    let mut buffer = vec![0u8; 65535];
    let n = responder.read_message(&store::from_hex(&ciphertext).unwrap(), &mut buffer).unwrap();
    assert_eq!(&buffer[..n], text(outputs, "roomPayloadJson").as_bytes());
    assert_eq!(responder.get_remote_static().unwrap(), bytes(outputs, "roomSenderNoisePublic").as_slice());

    // The Python fetch answer reads into the same row.
    let rows = relay_rest::read_fetch_response(200, &text(outputs, "roomFetchAnswer")).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].message_id, message_id);
    assert_eq!(rows[0].ciphertext, ciphertext);
    assert_eq!(rows[0].ts, ts);

    // The replay cache: the first accept is true, the same nonce again is refused.
    let mut cache = crate::relay_replay::NonceCache::default();
    assert_eq!(cache.accept(&nonce, ts, ts), text(outputs, "roomNonceFirstAccept") == "true");
    assert_eq!(cache.accept(&nonce, ts, ts), text(outputs, "roomNonceSecondAccept") == "true");
}
