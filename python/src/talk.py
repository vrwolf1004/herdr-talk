"""herdr-talk: simple chat between PCs on the same network (phase 1, no server).

How it works
  - Every instance broadcasts its name and TCP port over UDP every few seconds.
  - Other instances on the same network hear the broadcast and remember the peer.
  - A message is sent straight to each peer over TCP. Nothing is stored.

Screen
  - One screen on every console, the same layout as the Rust edition: a status row, messages, a divider, and the input row at the bottom.

Commands (typed in the input line)
  <text>     send to every peer
  /peers     list peers seen recently
  /help      show commands
  /quit      leave the chat
"""

import argparse
import os
import socket
import sys
import threading

from core.names import (  # re-exported: the name rules live in core/names.py
    NAMES_BY_LANGUAGE, clean_text, default_name, peer_name, pick_name, plain_sender, sender_name, squeeze_spaces, unique_name)
import herdr_api
from core import relay_chat, relay_rest, relay_settings
from core import sessions
from core import store
from core.events import Line, Notice
from core.chat import (  # the chat state and the LAN receive side: core/chat.py, re-exported for the screen and tests
    Chat, RelayHooks, remember_peer, reveal_message, read_reveal, exchange_reveal, show_code, reveal_later, receive_messages, receive_one, listen_for_peers, _listen_loop, _read_plain, _receive_encrypted, resolve_name_clash, MAX_TOTAL_PEERS, PEER_TIMEOUT_SECONDS, MAX_PEERS, JOIN_LIMIT_SECONDS, AUTO_RENAME_GAP_SECONDS, SEND_WORKERS, MAX_LINES, MESSAGE_LIMIT, MESSAGE_WINDOW_SECONDS, PHONE_SHOWN_SECONDS, REVEAL_RETRY_SECONDS, LOOP_POLL_SECONDS, MAX_CONNECTIONS, MAX_PER_IP, CONNECTION_SECONDS, MAX_MESSAGE_BYTES)  # noqa: F401
from core.net import (  # the announce, the send and the discovery port: core/net.py
    APP, ANNOUNCE_SECONDS, SEND_TIMEOUT_SECONDS, MAX_FRAME, MAX_TEXT_CHARS, STOP, _PEER_CLOSED_ERRNOS, _PEER_CLOSED_WINERRORS, announce, is_local_address, _Deadline, _peer_closed, send_to, _send_encrypted, deliver, DEFAULT_UDP_PORT, UDP_PORT_VARIABLE, parse_udp_port, udp_port)  # noqa: F401
from core.control import (  # the control channel: core/control.py, re-exported for the GUI tests
    CONTROL_MAX_REQUEST_BYTES, CONTROL_MAX_CLIENTS, CONTROL_IDLE_SECONDS, CONTROL_LINES_SHOWN, Control, control_answer, remove_stale_control_files, start_control, _read_request, _control_client)  # noqa: F401
from extend.dock_block import AGENT_ID, DOCK_KEY, SOURCE, keep_dock_block
from extend.tui import first_run_screen, run_ansi, settings_screen  # the full-screen chat: extend/tui.py
from extend import settings_form as relay_form  # noqa: F401  (re-exported for the tests)
from extend.screen import (  # the screen layout and keys: extend/screen.py
    KEY_F1, char_width, enable_vt, read_char, read_char_timeout, wrap_line, help_box, fit, tail, ACCENT, BORDER, MUTED, TEXT, BOLD,
    message_rows, MAX_NODE_ROWS, ERROR_WORDS, status_tag, screen_frame, on_key, event_rows, event_texts, print_event, add_event,
    phone_overlay)  # noqa: F401

Chat.on_event = staticmethod(print_event)  # no screen yet: the events go to stdout


def load_dock_layout():
    """설치된 herdr-dock-grid 가 있으면 그 원본을, 없으면 이 plugin 의 복사본(src/dock_layout.py)을 쓴다."""
    grid = herdr_api.installed_plugin_path("herdr-dock-grid")
    if grid:
        sys.path.insert(0, os.path.join(grid, "src"))
    import dock_layout
    return dock_layout


dock = load_dock_layout()

from i18n import t



# Display names picked at random when --name is not given: the list for this PC's language, else every list.
# Each entry: language code -> (Windows language name, names).




def state_dir():
    """This process's node folder (see sessions.py): the session chosen at start, else the base folder."""
    return sessions.state_dir()


def herdr_cli(*args):
    return herdr_api.cli(*args)  # raises herdr_api.HerdrError on failure or after the CLI time limit


def open_chat(_args):
    """현재 pane 아래에 pane 을 나눠 chat 을 실행한다. 대화가 끝나면(Ctrl+C) 그 셸 프롬프트가 남는다."""
    anchor = os.environ.get("HERDR_PANE_ID")
    if not anchor:
        print(t("open.no_pane"), file=sys.stderr)
        return 1
    src_dir = os.path.dirname(os.path.abspath(__file__))
    try:
        pane_id = herdr_cli("pane", "split", anchor, "--direction", "down", "--cwd", src_dir)["pane"]["pane_id"]
        herdr_cli("pane", "run", pane_id, sys.executable, "-X", "utf8", os.path.abspath(__file__), "run")
    except (herdr_api.HerdrError, ValueError, KeyError) as error:
        print(error, file=sys.stderr)
        return 1
    return 0


def start_node(args, state=None, plaintext=None):
    """Bind the sockets and start the network threads. Returns (server, chat).
    Keys and trusted peers live in the state folder (shared with the Rust edition); plain messages follow config.json."""
    state = state or state_dir()
    public, secret = store.load_or_create_identity(state)
    trusted = store.read_trusted(state)
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("", args.tcp_port))
    server.listen()
    name = args.name or default_name()
    me = {"id": store.peer_id(public), "name": name, "port": server.getsockname()[1], "key": public, "secret": secret}

    chat = Chat(me, plaintext=store.plaintext_allowed() if plaintext is None else plaintext)
    chat.trusted = trusted
    chat.state_dir = state
    # The first node on this PC is the host (방장): the badge shows it; the name stays a country name, as on every node.
    chat.try_become_host()
    chat.on_event = lambda event: add_event(chat, event)
    threading.Thread(target=receive_messages, args=(server, chat), daemon=True).start()
    threading.Thread(target=announce, args=(args.udp_port, me), daemon=True).start()
    threading.Thread(target=listen_for_peers, args=(args.udp_port, chat), daemon=True).start()
    return server, chat


def serve(args):
    """Chat without a screen, for a GUI: only the control channel runs. Ends on Ctrl+C or a `shutdown` request."""
    server, chat = start_node(args)
    _tidy_sessions(chat)
    control = start_control(chat, state_dir(), allow_shutdown=True)
    chat.watch_host()
    chat.start_relay()
    STOP.wait(ANNOUNCE_SECONDS + 0.5)
    renamed = resolve_name_clash(chat)
    if renamed:
        chat.emit(Notice("name.changed", dict(old=renamed[0], new=renamed[1])))
    chat.emit(Notice("start.hello", dict(name=chat.me["name"], port=chat.me["port"])))
    try:
        while not STOP.wait(1):
            pass
    except KeyboardInterrupt:
        pass
    finally:
        STOP.set()
        chat.stop_relay()
        chat.release_host()
        control.close()
        server.close()
    return 0


def run(args):
    server, chat = start_node(args)
    chat.lan_listening = False  # no LAN peer is remembered until the mode is known (a saved one, or the first-run choice)
    _tidy_sessions(chat)
    me = chat.me
    control = start_control(chat, state_dir())
    chat.watch_host()  # the first node on this PC owns the phone address; another takes over when it ends
    try:
        quit_first_run = bool(chat.state_dir and relay_settings.needs_first_run(chat.state_dir) and first_run_screen(chat.state_dir) == "quit")
    except KeyboardInterrupt:  # Ctrl+C on the first-run screen is a quit, not a crash
        quit_first_run = True
    if quit_first_run:
        STOP.set()
        chat.release_host()
        control.close()
        server.close()
        return 0
    # A room that refuses this trust code is not entered: back to the settings screen with the reason, until saved or cancelled.
    while chat.state_dir and relay_chat.room_refuses(chat.state_dir):
        outcome, _ = settings_screen(chat.state_dir, open_supabase=True, notice=relay_rest.TRUST_WRONG)
        if outcome != "saved":
            STOP.set()
            chat.release_host()
            control.close()
            server.close()
            return 0
    chat.start_relay()

    if args.dock:
        herdr_api.try_request("pane.report_agent", {"pane_id": args.pane, "source": SOURCE, "agent": AGENT_ID, "state": "idle"})
        threading.Thread(target=keep_dock_block, args=(args.pane, chat), daemon=True).start()
    # Wait one announce cycle so peers that are already here are known before the name is final.
    STOP.wait(ANNOUNCE_SECONDS + 0.5)
    renamed = resolve_name_clash(chat)
    if renamed:
        chat.emit(Notice("name.changed", dict(old=renamed[0], new=renamed[1])))
    chat.emit(Notice("start.hello", dict(name=me["name"], port=me["port"])))
    try:
        return run_ansi(chat)
    finally:
        STOP.set()
        chat.stop_relay()
        chat.release_host()
        control.close()
        server.close()
        if args.dock:
            herdr_api.try_request("pane.release_agent", {"pane_id": args.pane, "source": SOURCE, "agent": AGENT_ID})
            dock.release_dock_pane(DOCK_KEY)


def dock_start(_args):
    folder = dock.plugins_folder()
    stored = dock.read_dock_pane(folder, DOCK_KEY)
    agents = herdr_api.cli("agent", "list").get("agents", [])
    if stored and any(a["pane_id"] == stored and a.get("agent") == AGENT_ID for a in agents):
        return 0  # 이미 떠 있다
    _, pane_id = dock.ensure_dock_pane(DOCK_KEY, folder)
    herdr_api.cli("pane", "run", pane_id, sys.executable, "-X", "utf8", os.path.abspath(__file__),
                  "run", "--dock", "--pane", pane_id)
    return 0


def dock_stop(_args):
    folder = dock.plugins_folder()
    pane_id = dock.read_dock_pane(folder, DOCK_KEY)
    if pane_id:
        herdr_api.try_request("pane.release_agent", {"pane_id": pane_id, "source": SOURCE, "agent": AGENT_ID})
        workspace_id = dock.dock_workspace_id()
        others = [p for p in dock.workspace_panes(workspace_id) if p["pane_id"] != pane_id and not dock.is_sidebar(p)] if workspace_id else []
        if others:
            herdr_api.cli("pane", "close", pane_id, check=False)
        elif workspace_id:
            herdr_api.cli("workspace", "close", workspace_id, check=False)
    dock.release_dock_pane(DOCK_KEY)
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(prog="herdr-talk")
    commands = parser.add_subparsers(dest="command", required=True)
    chat = commands.add_parser("run", help="open the chat in this pane")
    chat.add_argument("--name", default=None, help="name shown to others (random if omitted)")
    chat.add_argument("--tcp-port", type=int, default=0, help="TCP port for messages (0 = pick one)")
    chat.add_argument("--udp-port", type=int, default=None, help="discovery port (keep the same on every PC; default HERDR_TALK_UDP_PORT or 47850)")
    chat.add_argument("--dock", action="store_true", help="show this pane as the herdr-dock block")
    chat.add_argument("--pane", default=None, help="herdr pane id of the dock block (with --dock)")
    chat.add_argument("--session", default=None, help="use this node folder (an id, or default); default: a new node")
    serve_parser = commands.add_parser("serve", help="run without a screen; a GUI talks to it over the control channel")
    serve_parser.add_argument("--name", default=None, help="name shown to others (random if omitted)")
    serve_parser.add_argument("--tcp-port", type=int, default=0, help="TCP port for messages (0 = pick one)")
    serve_parser.add_argument("--udp-port", type=int, default=None, help="discovery port (keep the same on every PC; default HERDR_TALK_UDP_PORT or 47850)")
    serve_parser.add_argument("--session", default=None, help="use this node folder (an id, or default); default: a new node")
    commands.add_parser("sessions", help="list the node folders with their connection mode")
    commands.add_parser("open", help="split the current pane and run the chat in it")
    commands.add_parser("dock-start", help="show the chat as a block in the herdr-dock workspace")
    commands.add_parser("dock-stop", help="remove the dock block")
    args = parser.parse_args(argv)
    try:
        return _dispatch(args)
    except (OSError, ValueError) as error:  # e.g. a damaged identity.json: say so, never make a new key
        print(error, file=sys.stderr)
        return 1
    finally:
        try:  # the node's own relay row goes first; a failure here never stops the exit
            relay_chat.leave(sessions.state_dir())
        except Exception:  # noqa: BLE001 - leaving must not block the exit
            pass
        failed = sessions.remove_own_session()  # a node's own folder goes when it stops
        if failed:
            print(f"could not remove session {failed}", file=sys.stderr)


def list_sessions_command():
    base = sessions.base_dir()
    for session_id, folder in [(sessions.DEFAULT_SESSION, base), *sessions.list_sessions(base)]:
        print(f"{session_id}  {folder}  mode={relay_settings.load_saved(folder)['mode']}")
    return 0


def _tidy_sessions(chat):
    """Remove node folders whose node has ended without cleaning up, each after its relay row (best effort). In the
    background: the start does not wait for the relay. Quiet: a folder that cannot be removed is only named, as a chat
    line (stderr would break the full screen)."""
    def leave(folder):
        relay_chat.leave(folder, timeout=relay_chat.DEAD_LEAVE_SECONDS)

    def tidy():
        for session_id in sessions.remove_dead_sessions(sessions.base_dir(), leave=leave):
            chat.emit(Line(f"could not remove dead session {session_id}"))

    threading.Thread(target=tidy, daemon=True).start()


def _dispatch(args):
    if args.command == "sessions":
        return list_sessions_command()
    if args.command in ("run", "serve"):
        sessions.choose_session(args.session)  # a bad or missing --session is reported by main()
    if args.command in ("run", "serve") and args.udp_port is None:
        args.udp_port = udp_port()  # a bad HERDR_TALK_UDP_PORT is reported by main() and stops the start
    if args.command == "run":
        return run(args)
    if args.command == "serve":
        return serve(args)
    if args.command == "open":
        return open_chat(args)
    if args.command == "dock-start":
        return dock_start(args)
    if args.command == "dock-stop":
        return dock_stop(args)
    return 1


if __name__ == "__main__":
    sys.exit(main())

