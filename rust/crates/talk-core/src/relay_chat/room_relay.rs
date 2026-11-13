//! Room-based relay logic: multiple peers in a room, messages via Supabase database.
//!
//! A node joins a room and receives messages from all members.

use crate::chat::Chat;
use crate::relay_rest::{self, Config};
use crate::relay_settings::Settings;
use crate::relay_replay::NonceCache;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;

const SLICE: Duration = Duration::from_millis(200);

/// Run the room-based relay: join the room and listen for messages.
pub(crate) fn run(chat: &Chat, state: &Path, config: &Config, settings: &Settings, rx: &Receiver<String>, stop: &AtomicBool) {
    let mut cache = NonceCache::default();
    let seed = match super::seed(state) {
        Ok(seed) => seed,
        Err(error) => return chat.set_relay_status(&format!("relay: {error}")),
    };
    let room_result = join_room_and_listen(chat, config, &settings.room_code, &seed, &settings.name, rx, stop, &mut cache);
    if let Err(error) = room_result {
        chat.set_relay_status(&format!("relay: {error}"));
    }
}

/// Join the room and listen for messages from room members.
fn join_room_and_listen(
    chat: &Chat,
    config: &Config,
    room_code: &str,
    seed: &[u8; 32],
    display_name: &str,
    rx: &Receiver<String>,
    stop: &AtomicBool,
    _cache: &mut NonceCache,
) -> Result<(), String> {
    // Build request to join room
    let req = relay_rest::join_room_request(config, seed, room_code, display_name)?;
    
    // Send HTTP POST request to join the room
    let (status, response_body) = relay_rest::http_post(&req)?;
    
    // Parse the response to get room info and members
    let _room_info = relay_rest::read_join_room_response(status, &response_body)?;
    
    chat.set_relay_status("relay: room joined");
    chat.say(format!("Joined room {}. Listening for messages...", room_code));
    
    // Keep the relay thread alive
    while !stop.load(Ordering::Relaxed) {
        while let Ok(text) = rx.try_recv() {
            // TODO: Send message to room (store in room_messages table)
            chat.say(format!("Would send to room: {text}"));
        }
        thread::sleep(SLICE);
    }
    Ok(())
}
