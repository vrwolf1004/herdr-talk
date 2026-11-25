//! The command answers of the control channel, shared by every edition: a request (one JSON object) and the answer
//! (one JSON object). The socket and the token live in the terminal edition (herdr-talk/src/control.rs).

use crate::{Chat, Line};
use serde_json::{json, Value};
use std::sync::Arc;

/// How many chat lines a `lines` answer carries (the newest).
pub const LINES_SHOWN: usize = 300;

/// Answer one request. `allow_shutdown` is false for the terminal chat (only the GUI's serve may stop the node).
pub fn answer(chat: &Arc<Chat>, request: &Value, allow_shutdown: bool) -> Value {
    match request.get("cmd").and_then(Value::as_str) {
        Some("lines") => {
            let lines: Vec<Line> = chat.lines();
            let start = lines.len().saturating_sub(LINES_SHOWN);
            let rows: Vec<Value> = lines[start..]
                .iter()
                // "timed": a line with a sender gets the [HH:MM] stamp; a notice does not, as in the TUI.
                .map(|line| json!({"text": line.render(), "mine": line.mine, "seq": line.seq, "timed": line.sender.is_some()}))
                .collect();
            json!({"ok": true, "lines": rows})
        }
        Some("peers") => {
            let peers: Vec<Value> = chat
                .fresh_peers()
                .iter()
                .map(|peer| json!({"name": peer.name, "ip": peer.ip, "port": peer.port}))
                .collect();
            // This chat's own name and port, so a GUI can list it above the others.
            let me = chat.me.lock().unwrap().clone();
            // Peers waiting for a /join: their name and the code the GUI shows for the comparison.
            // The same numbers as the terminal's list ([n]), so /join n means the same peer in both.
            let pending: Vec<Value> = chat.unverified_peers().iter().filter_map(|(peer, code, number)| {
                let key = peer.key.as_deref()?;
                Some(json!({"name": peer.name, "ip": peer.ip, "code": code, "number": number, "seconds": chat.join_seconds_left(key).unwrap_or(0)}))
            }).collect();
            json!({"ok": true, "peers": peers, "pending": pending, "me": {"name": me.name, "port": me.port}, "room": chat.room_names(), "room_status": chat.room_status().unwrap_or_default(), "relay": chat.relay_status(), "mode": chat.mode_name(), "host": chat.is_host()})
        }
        Some("send") => match request.get("text").and_then(Value::as_str) {
            // Same path as typing in the terminal: commands and plain messages both work.
            Some(text) => {
                chat.handle(text);
                json!({"ok": true})
            }
            None => json!({"ok": false, "error": "text missing"}),
        },
        // The settings were saved (by the GUI or the terminal screen): start the relay again with them.
        Some("relay_reload") => match crate::relay_chat::start(chat, &crate::store::state_dir()) {
            Ok(on) => json!({"ok": true, "on": on}),
            Err(error) => json!({"ok": false, "error": error}),
        },
        // Ctrl+L in the GUI: the same clear as the terminal's Ctrl+L.
        Some("clear") => {
            chat.clear_lines();
            json!({"ok": true})
        }
        Some("shutdown") if allow_shutdown => {
            chat.stop.set();
            json!({"ok": true})
        }
        _ => json!({"ok": false, "error": "unknown cmd"}),
    }
}
