//! herdr-talk: chat between PCs on the same network, as a herdr plugin.

mod control;
mod first_run;
mod settings_form;
mod dock;
mod herdr_api;
mod screen;

use serde_json::json;
use std::env;
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use talk_core::{start, t, udp_port, Chat};

const DOCK_KEY: &str = "herdr-talk";
const DOCK_REPORT: Duration = Duration::from_secs(2);
const DOCK_TOKEN_TTL_MS: u64 = 15_000;
const AGENT_ID: &str = "talk";
const SOURCE: &str = "plugin:herdr-talk";
const USAGE: &str = "usage: herdr-talk <run [--session ID] [--name NAME] [--tcp-port N] [--udp-port N] [--dock] [--pane ID] | serve [--session ID] [--name NAME] [--tcp-port N] [--udp-port N] | sessions | open | dock-start | dock-stop>";

#[derive(Default)]
struct RunArgs {
    name: Option<String>,
    tcp_port: Option<u16>,
    udp_port: Option<u16>,
    dock: bool,
    pane: Option<String>,
    /// The session folder to use (a session id, or "default"). None: a new session.
    session: Option<String>,
}

fn parse_run(args: &[String]) -> Result<RunArgs, String> {
    let mut parsed = RunArgs::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || args.next().cloned().ok_or(format!("{arg} needs a value"));
        let port = |text: String| text.parse::<u16>().map_err(|_| format!("{arg}: not a port: {text}"));
        match arg.as_str() {
            "--name" => parsed.name = Some(value()?),
            "--tcp-port" => parsed.tcp_port = Some(port(value()?)?),
            "--udp-port" => parsed.udp_port = Some(port(value()?)?),
            "--pane" => parsed.pane = Some(value()?),
            "--session" => parsed.session = Some(value()?),
            "--dock" => parsed.dock = true,
            other => return Err(format!("unknown option: {other}")),
        }
    }
    if parsed.dock && parsed.pane.is_none() {
        return Err("--dock needs --pane".to_string());
    }
    Ok(parsed)
}

fn this_program() -> Result<String, String> {
    env::current_exe().map(|path| path.to_string_lossy().into_owned()).map_err(|error| error.to_string())
}

/// Split the pane below the current one and run the chat there. After Ctrl+C its shell prompt stays.
fn open_chat() -> Result<(), String> {
    let anchor = env::var("HERDR_PANE_ID").map_err(|_| t("open.no_pane", &[]))?;
    let program = this_program()?;
    let folder = env::current_exe().ok().and_then(|path| path.parent().map(|parent| parent.to_string_lossy().into_owned())).unwrap_or_default();
    let split = herdr_api::cli(&["pane", "split", &anchor, "--direction", "down", "--cwd", &folder])?;
    let pane = split["pane"]["pane_id"].as_str().ok_or("herdr pane split gave no pane id")?;
    herdr_api::cli(&["pane", "run", pane, &program, "run"]).map(|_| ())
}

/// Keep telling the dock block how many people are connected, until the chat ends.
fn keep_dock_block(pane: String, chat: Arc<Chat>) {
    while !chat.stop.is_set() {
        let count = chat.fresh_peers().len().to_string();
        herdr_api::try_request("pane.report_metadata", json!({
            "pane_id": pane,
            "source": SOURCE,
            "display_agent": t("dock.agent_name", &[("count", &count)]),
            "tokens": {},
            "ttl_ms": DOCK_TOKEN_TTL_MS,
        }));
        chat.stop.wait(DOCK_REPORT);
    }
}

/// Remove node folders whose node has ended without cleaning up, each after its relay row (best effort). In the
/// background: the start does not wait for the relay. Quiet: a folder that cannot be removed is only named, through `report`.
fn tidy_sessions(report: impl Fn(String) + Send + 'static) {
    thread::spawn(move || {
        for id in talk_core::store::remove_dead_sessions(&talk_core::store::base_dir(), &talk_core::relay_chat::leave_folder) {
            report(format!("could not remove dead session {id}"));
        }
    });
}

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
    // The relay is optional: a bad setting shows in the status row and never stops the chat.
    if let Err(error) = talk_core::relay_chat::start(&chat, &talk_core::store::state_dir()) {
        chat.say(format!("relay: {error}"));
    }
    let state = talk_core::store::state_dir();
    let result = if talk_core::relay_settings::needs_first_run(&state) {
        match screen::choose_mode(&chat) {
            Ok(screen::Choice::Saved | screen::Choice::TimedOut) => screen::run(&chat),
            Ok(screen::Choice::Quit) => Ok(()),
            Err(error) => Err(error),
        }
    } else {
        screen::run(&chat)
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

/// Chat without a screen, for a GUI: only the control channel runs. Ends on Ctrl+C, a termination
/// signal, or a `shutdown` request, and removes the control file on the way out.
fn serve(args: RunArgs) -> Result<(), String> {
    talk_core::store::choose_session(args.session.as_deref())?;
    tidy_sessions(|line| eprintln!("{line}"));
    if args.dock || args.pane.is_some() {
        return Err("serve has no dock: --dock and --pane belong to run".to_string());
    }
    let udp = match args.udp_port {
        Some(port) => port,
        None => udp_port()?,
    };
    let chat = start(args.name, args.tcp_port.unwrap_or(0), udp).map_err(|error| error.to_string())?;
    let control = control::serve(chat.clone(), &talk_core::store::state_dir(), true).map_err(|error| error.to_string())?;
    chat.watch_host();
    // The relay runs here too, so a GUI (which talks to this serve) gets it. A bad setting only shows a line.
    if let Err(error) = talk_core::relay_chat::start(&chat, &talk_core::store::state_dir()) {
        chat.say(format!("relay: {error}"));
    }
    let signalled = chat.clone();
    ctrlc::set_handler(move || signalled.stop.set()).map_err(|error| error.to_string())?;
    let me = chat.me.lock().unwrap().clone();
    chat.say(t("start.hello", &[("name", &me.name), ("port", &me.port.to_string())]));
    while !chat.stop.is_set() {
        chat.stop.wait(Duration::from_secs(1));
    }
    chat.release_host();
    drop(control);
    Ok(())
}

fn dock_start() -> Result<(), String> {
    let folder = dock::plugins_folder();
    let stored = dock::read_dock_pane(&folder, DOCK_KEY);
    let agents = herdr_api::cli(&["agent", "list"])?;
    let running = agents["agents"].as_array().into_iter().flatten()
        .any(|agent| !stored.is_empty() && dock::pane_id(agent) == stored && agent["agent"] == AGENT_ID);
    if running {
        return Ok(());
    }
    let pane = dock::ensure_dock_pane(DOCK_KEY, &folder)?;
    herdr_api::cli(&["pane", "run", &pane, &this_program()?, "run", "--dock", "--pane", &pane]).map(|_| ())
}

fn dock_stop() -> Result<(), String> {
    let pane = dock::read_dock_pane(&dock::plugins_folder(), DOCK_KEY);
    if !pane.is_empty() {
        herdr_api::try_request("pane.release_agent", json!({"pane_id": pane, "source": SOURCE, "agent": AGENT_ID}));
        if let Some(workspace) = dock::dock_workspace_id()? {
            let panes = dock::workspace_panes(&workspace)?;
            let others = panes.iter().any(|other| dock::pane_id(other) != pane && !dock::is_sidebar(other));
            // Closing can fail when the pane is already gone, which is the state we want.
            let _ = if others {
                herdr_api::cli(&["pane", "close", &pane])
            } else {
                herdr_api::cli(&["workspace", "close", &workspace])
            };
        }
    }
    dock::release_dock_pane(DOCK_KEY)
}

/// Print the sessions under the base folder: id, folder and the connection mode saved there. The default folder first.
fn list_sessions() -> Result<(), String> {
    let base = talk_core::store::base_dir();
    let mut rows = vec![(talk_core::store::DEFAULT_SESSION.to_string(), base.clone())];
    rows.extend(talk_core::store::list_sessions(&base));
    for (id, dir) in rows {
        let mode = talk_core::relay_settings::load_saved(&dir).mode;
        println!("{id}  {}  mode={}", dir.display(), mode.as_str());
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<RunArgs, String> {
        parse_run(&args.iter().map(ToString::to_string).collect::<Vec<_>>())
    }

    #[test]
    fn parses_run_options() {
        let parsed = parse(&["--name", "a", "--tcp-port", "47900", "--dock", "--pane", "w1:p1"]).ok().unwrap();
        assert_eq!(parsed.name.as_deref(), Some("a"));
        assert_eq!(parsed.tcp_port, Some(47900));
        assert!(parsed.dock);
        assert_eq!(parsed.pane.as_deref(), Some("w1:p1"));
    }

    #[test]
    fn rejects_bad_options() {
        for args in [&["--tcp-port", "x"][..], &["--name"], &["--what"], &["--dock"]] {
            assert!(parse(args).is_err());
        }
    }
}
