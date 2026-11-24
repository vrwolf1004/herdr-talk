//! herdr-talk: chat between PCs on the same network, as a herdr plugin.
//! This file only starts the command and assembles the chat with the screen (see `run`).

mod core;
mod extend;

use crate::core::commands::{USAGE, parse_run, serve, dock_start, dock_stop, list_sessions, open_chat};
use crate::core::commands::{tidy_sessions, RunArgs, AGENT_ID, DOCK_KEY};
use crate::core::control;
use crate::core::dock::{self, keep_dock_block, SOURCE};
use crate::core::herdr_api;
use crate::extend::screen;
use serde_json::json;
use std::env;
use std::process::ExitCode;
use std::thread;
use talk_core::{start, t, udp_port};

fn run(args: RunArgs) -> Result<(), String> {
    talk_core::store::choose_session(args.session.as_deref())?;
    let udp = match args.udp_port {
        Some(port) => port,
        None => udp_port()?,
    };
    let chat = start(args.name, args.tcp_port.unwrap_or(0), udp).map_err(|error| error.to_string())?;
    let tidy_chat = chat.clone();
    tidy_sessions(move |line| tidy_chat.say(line)); // into the chat: stderr would break the full screen
    let dock_pane = args.pane.filter(|_| args.dock);
    if let Some(pane) = &dock_pane {
        herdr_api::try_request("pane.report_agent", json!({"pane_id": pane, "source": SOURCE, "agent": AGENT_ID, "state": "idle"}));
        let (pane, chat) = (pane.clone(), chat.clone());
        thread::spawn(move || keep_dock_block(pane, chat));
    }
    let _control = control::serve(chat.clone(), &talk_core::store::state_dir(), false).map_err(|error| error.to_string())?;
    chat.watch_host(); // the first node on this PC owns the phone address; another takes over when it ends
    let me = chat.me.lock().unwrap().clone();
    chat.say(t("start.hello", &[("name", &me.name), ("port", &me.port.to_string())]));
    let state = talk_core::store::state_dir();
    chat.set_lan_listening(false); // no LAN peer is remembered until the mode is known (a saved one, or the first-run choice)
    // A saved room that refuses this PC's trust code is not joined: the settings screen opens with the reason instead.
    let refused = talk_core::relay_chat::room_refuses(&state);
    // The relay is optional: a bad setting shows in the status row and never stops the chat.
    if !refused {
        if let Err(error) = talk_core::relay_chat::start(&chat, &state) {
            chat.say(format!("relay: {error}"));
        }
    }
    let result = if talk_core::relay_settings::needs_first_run(&state) {
        chat.set_lan_listening(false);
        let chosen = screen::choose_mode(&chat);
        chat.set_lan_listening(true);
        match chosen {
            Ok(screen::Choice::Saved | screen::Choice::TimedOut) => screen::run(&chat, refused),
            Ok(screen::Choice::Quit) => Ok(()),
            Err(error) => Err(error),
        }
    } else {
        chat.set_lan_listening(true);
        screen::run(&chat, refused)
    }
    .map_err(|error| error.to_string());
    chat.stop_relay();
    chat.stop.set();
    chat.release_host();
    if let Some(pane) = dock_pane {
        herdr_api::try_request("pane.release_agent", json!({"pane_id": pane, "source": SOURCE, "agent": AGENT_ID}));
        dock::release_dock_pane(DOCK_KEY)?;
    }
    result
}


fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => parse_run(&args[1..]).and_then(run),
        Some("serve") => parse_run(&args[1..]).and_then(serve),
        Some("sessions") => list_sessions(),
        Some("open") => open_chat(),
        Some("dock-start") => dock_start(),
        Some("dock-stop") => dock_stop(),
        _ => Err(USAGE.to_string()),
    };
    // A stop, also one with an error, deletes this node's own relay row first (best effort), then its folder: the keys are in it.
    if talk_core::store::session_id().is_some() {
        talk_core::relay_chat::leave(&talk_core::store::state_dir());
    }
    if let Some(id) = talk_core::store::remove_own_session() {
        eprintln!("could not remove session {id}");
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
