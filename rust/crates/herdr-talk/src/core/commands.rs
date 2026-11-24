//! The herdr-talk commands: run, serve, dock start/stop, sessions, open. They parse the arguments
//! and start the chat, the control channel and the dock; the screens live in extend.

use serde_json::json;
use std::env;
use std::thread;
use std::time::Duration;
use crate::core::dock::SOURCE;
use crate::core::{control, dock, herdr_api};
use talk_core::{start, t, udp_port};

pub(crate) const DOCK_KEY: &str = "herdr-talk";
pub(crate) const AGENT_ID: &str = "talk";
pub(crate) const USAGE: &str = "usage: herdr-talk <run [--session ID] [--name NAME] [--tcp-port N] [--udp-port N] [--dock] [--pane ID] | serve [--session ID] [--name NAME] [--tcp-port N] [--udp-port N] | sessions | open | dock-start | dock-stop>";

#[derive(Default)]
pub(crate) struct RunArgs {
    pub(crate) name: Option<String>,
    pub(crate) tcp_port: Option<u16>,
    pub(crate) udp_port: Option<u16>,
    pub(crate) dock: bool,
    pub(crate) pane: Option<String>,
    /// The session folder to use (a session id, or "default"). None: a new session.
    pub(crate) session: Option<String>,
}

pub(crate) fn parse_run(args: &[String]) -> Result<RunArgs, String> {
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

pub(crate) fn this_program() -> Result<String, String> {
    env::current_exe().map(|path| path.to_string_lossy().into_owned()).map_err(|error| error.to_string())
}

/// Split the pane below the current one and run the chat there. After Ctrl+C its shell prompt stays.
pub(crate) fn open_chat() -> Result<(), String> {
    let anchor = env::var("HERDR_PANE_ID").map_err(|_| t("open.no_pane", &[]))?;
    let program = this_program()?;
    let folder = env::current_exe().ok().and_then(|path| path.parent().map(|parent| parent.to_string_lossy().into_owned())).unwrap_or_default();
    let split = herdr_api::cli(&["pane", "split", &anchor, "--direction", "down", "--cwd", &folder])?;
    let pane = split["pane"]["pane_id"].as_str().ok_or("herdr pane split gave no pane id")?;
    herdr_api::cli(&["pane", "run", pane, &program, "run"]).map(|_| ())
}

/// Remove node folders whose node has ended without cleaning up, each after its relay row (best effort). In the
/// background: the start does not wait for the relay. Quiet: a folder that cannot be removed is only named, through `report`.
pub(crate) fn tidy_sessions(report: impl Fn(String) + Send + 'static) {
    thread::spawn(move || {
        for id in talk_core::store::remove_dead_sessions(&talk_core::store::base_dir(), &talk_core::relay_chat::leave_folder) {
            report(format!("could not remove dead session {id}"));
        }
    });
}

/// Chat without a screen, for a GUI: only the control channel runs. Ends on Ctrl+C, a termination
/// signal, or a `shutdown` request, and removes the control file on the way out.
pub(crate) fn serve(args: RunArgs) -> Result<(), String> {
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
    chat.set_lan_listening(false); // no LAN peer is remembered until the mode is known
    let control = control::serve(chat.clone(), &talk_core::store::state_dir(), true).map_err(|error| error.to_string())?;
    chat.watch_host();
    // The relay runs here too, so a GUI (which talks to this serve) gets it. A bad setting only shows a line.
    if let Err(error) = talk_core::relay_chat::start(&chat, &talk_core::store::state_dir()) {
        chat.say(format!("relay: {error}"));
    }
    if talk_core::relay_settings::needs_first_run(&talk_core::store::state_dir()) {
        chat.set_lan_listening(false); // the GUI's first-run choice is still to come: its relay_reload starts the listening
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

pub(crate) fn dock_start() -> Result<(), String> {
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

pub(crate) fn dock_stop() -> Result<(), String> {
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
pub(crate) fn list_sessions() -> Result<(), String> {
    let base = talk_core::store::base_dir();
    let mut rows = vec![(talk_core::store::DEFAULT_SESSION.to_string(), base.clone())];
    rows.extend(talk_core::store::list_sessions(&base));
    for (id, dir) in rows {
        let mode = talk_core::relay_settings::load_saved(&dir).mode;
        println!("{id}  {}  mode={}", dir.display(), mode.as_str());
    }
    Ok(())
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
