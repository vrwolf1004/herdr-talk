//! Windows window for herdr-talk. It connects to a running `herdr-talk run` through its local
//! control channel (127.0.0.1, token from control-<pid>.json) and shows the chat and peers.
//! It does not join the network itself: the chat, encryption and trust stay in herdr-talk.
//! It first tries every control file; with none that connects, it starts `herdr-talk serve` hidden,
//! waits for its control file, and stops it again on exit. A herdr-talk that was already running is never stopped.

#![windows_subsystem = "windows"]

mod core;
mod extend;

use eframe::egui;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::core::*;
use crate::extend::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn the_bottom_line_shows_the_room_and_setup_code_and_only_a_problem() {
        let host = talk_core::t("host.badge", &[]);
        let peers = |n: &str| talk_core::t("gui.bottom.peers", &[("peers", n)]);
        let cloud = talk_core::t("gui.mode.cloud_service", &[]);
        let base = talk_core::t("gui.mode.base_station", &[]);
        let room = "room ID: ABCD2345 · setup code: -";
        assert_eq!(bottom_line(false, 2, true, room, ""), format!("{} · {cloud} · {room}", peers("2")));
        assert_eq!(bottom_line(true, 0, false, "", ""), format!("{host} {} · {base}", peers("0")));
        assert_eq!(bottom_line(false, 1, true, room, "relay: room joined (3 members)"), format!("{} · {cloud} · {room}", peers("1")));
        assert_eq!(bottom_line(false, 1, true, room, "relay: trust code missing or invalid (8 digits)"), format!("{} · {cloud} · {room} · relay: trust code missing or invalid (8 digits)", peers("1")));
    }

    #[test]
    fn the_help_lists_rejoin_and_the_rejoin_text_is_translated() {
        assert!(HELP_KEYS.contains(&"help.rejoin"));
        let done = talk_core::t("rejoin.done", &[("seconds", "60")]);
        assert!(done.contains("60") && done != "rejoin.done", "{done}");
    }

    #[test]
    fn a_waiting_row_is_in_the_terminal_notation_with_the_seconds_left() {
        let waiting = |seconds| Waiting { number: 2, name: "Bob".into(), ip: "10.0.0.2".into(), code: "1234 5678".into(), seconds };
        let row = wait_row(&waiting(42));
        assert!(row.starts_with("[2] Bob (10.0.0.2)  "), "{row}");
        assert!(row.contains(&talk_core::t("nodes.waiting", &[])), "{row}");
        assert!(row.contains(&talk_core::t("nodes.seconds_left", &[("seconds", "42")])), "{row}");
    }

    #[test]
    fn the_host_badge_follows_the_room_owner_in_supabase_and_the_lan_owner_otherwise() {
        // Supabase: only the room owner (the chat's host flag), never the PC's LAN owner.
        assert!(hosting(true, true, false));
        assert!(!hosting(true, false, true));
        // LAN: unchanged, the phone-file owner only.
        assert!(hosting(false, false, true));
        assert!(!hosting(false, true, false));
    }

    #[test]
    fn only_failures_are_shown_from_the_relay_text() {
        assert_eq!(relay_problem(""), None);
        assert_eq!(relay_problem("relay: room joined (2 members)"), None);
        assert_eq!(relay_problem("relay: settings incomplete"), Some("relay: settings incomplete"));
    }

    #[test]
    fn f3_switches_the_language_only_when_it_is_pressed() {
        // No config folder here, so the switch is for this run only and the notice says so; the language is put back at the end.
        std::env::remove_var("HERDR_PLUGIN_CONFIG_DIR");
        let before = talk_core::i18n::language();
        let press = |key: egui::Key| {
            let ctx = egui::Context::default();
            let event = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
            ctx.begin_pass(egui::RawInput { events: vec![event], ..Default::default() });
            let answer = language_key(&ctx);
            let _ = ctx.end_pass();
            answer
        };
        assert_eq!(press(egui::Key::F8), None);
        assert_eq!(talk_core::i18n::language(), before);
        assert_eq!(press(egui::Key::F3), Some(talk_core::t("tui.lang.not_saved", &[])), "no config folder: the notice says so");
        assert_ne!(talk_core::i18n::language(), before);
        for _ in 0..8 {
            if talk_core::i18n::language() == before {
                break;
            }
            talk_core::i18n::switch_language_notice();
        }
        assert_eq!(talk_core::i18n::language(), before);

        // With a config folder the switch is saved and nothing is said.
        let dir = std::env::temp_dir().join(format!("herdr-talk-gui-f3-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("HERDR_PLUGIN_CONFIG_DIR", &dir);
        assert_eq!(press(egui::Key::F3), Some(String::new()), "saved: nothing to say");
        assert!(dir.join("config.json").exists(), "the language is saved");
        for _ in 0..8 {
            if talk_core::i18n::language() == before {
                break;
            }
            talk_core::i18n::switch_language_notice();
        }
        assert_eq!(talk_core::i18n::language(), before);
        std::env::remove_var("HERDR_PLUGIN_CONFIG_DIR");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_log_write_does_not_panic() {
        let dir = std::env::temp_dir().join(format!("herdr-talk-gui-log-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        // The log path is a folder, so opening it for append fails.
        assert!(log_to(&dir, "x").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn log_entries_are_appended() {
        let dir = std::env::temp_dir().join(format!("herdr-talk-gui-append-test-{}", std::process::id()));
        let file = dir.join("gui-panic.log");
        log_to(&file, "first").unwrap();
        log_to(&file, "second").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "first\nsecond\n");
        fs::remove_dir_all(&dir).unwrap();
    }
}

fn main() -> eframe::Result<()> {
    install_panic_log();
    // A new session per window unless --session ID names an existing one (or "default").
    let args: Vec<String> = std::env::args().collect();
    let requested = args.windows(2).find(|pair| pair[0] == "--session").map(|pair| pair[1].clone());
    if let Err(error) = talk_core::store::choose_session(requested.as_deref()) {
        eprintln!("{error}");
        return Ok(());
    }
    // In the background: the window does not wait for the relay.
    thread::spawn(|| {
        for id in talk_core::store::remove_dead_sessions(&talk_core::store::base_dir(), &talk_core::relay_chat::leave_folder) {
            eprintln!("could not remove dead session {id}");
        }
    });
    let shared = Arc::new(Mutex::new(Shared { status: status_starting(), ..Default::default() }));
    let (outgoing, queue) = mpsc::channel();
    let started: Started = Arc::default();
    let reload = Arc::new(AtomicBool::new(false));
    {
        let (shared, started, reload) = (shared.clone(), started.clone(), reload.clone());
        thread::Builder::new().name("worker".into()).spawn(move || worker(shared, queue, started, reload)).expect("worker thread");
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([400.0, 620.0]).with_title("herdr-talk"),
        ..Default::default()
    };
    let result = eframe::run_native(
        "herdr-talk",
        options,
        Box::new(|cc| {
            install_korean_font(&cc.egui_ctx);
            let hwnd = window_hwnd(cc);
            cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
            Ok(Box::new(App { screen: Screen::Intro, shared, outgoing, input: String::new(), theme: Choice::Green, crt: true, hwnd, local_ip: local_ip(), frame_theme: None, phone_until: None, phone_checked: false, reload: reload.clone(), relay_form: None, first_run: talk_core::relay_settings::needs_first_run(&state_dir()), notice: String::new(), choice_timer: None, intro_mode: talk_core::relay_settings::Mode::from_saved(talk_core::relay_settings::load_saved(&state_dir()).mode.as_str()), toast_text: String::new(), toast_started: None, help_open: false, verify: None }))
        }),
    );
    stop_serve(&started);
    // A stop, also one with an error, deletes this node's own relay row first (best effort), then its folder: the keys are in it.
    if talk_core::store::session_id().is_some() {
        talk_core::relay_chat::leave(&talk_core::store::state_dir());
    }
    if let Some(id) = talk_core::store::remove_own_session() {
        eprintln!("could not remove session {id}");
    }
    result
}

