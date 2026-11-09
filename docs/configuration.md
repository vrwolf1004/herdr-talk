# Configuration

herdr-talk has no required settings. Everything below is optional.

## Settings at a glance

| Setting | Where | Default | What it does |
|---------|-------|---------|--------------|
| `--name` | command line | random name for the PC's language | Name shown to others |
| `--tcp-port` | command line | `0` (pick a free port) | TCP port for incoming messages |
| `--udp-port` | command line | `47850` | Discovery port. Use the same value on every PC. |
| `HERDR_TALK_UDP_PORT` | environment | `47850` | Discovery port for separate test networks, so test nodes do not mix with a real LAN. Read by Rust and Python, and by the GUI and the `serve` it starts. A value that is not 1–65535 is an error, not a fallback. `--udp-port` wins when given. |
| `HERDR_TALK_LANG` | environment | not set | Language code: `en`, `ko`, `ru`, `fr`, `es`, `de`, `ja` |
| `"lang"` | `config.json` in the plugin config folder | not set | Same as `HERDR_TALK_LANG` |
| `"plaintext"` | `config.json` in the plugin config folder | `false` | Allow plain text to peers that do not encrypt (an old Python edition without keys). See [Encryption](#encryption). |
| `identity.json`, `trusted.json` | herdr-talk state folder (`HERDR_PLUGIN_STATE_DIR`) | created on first start | This PC's key and the peers you verified. Do not share `identity.json`. |
| `control-<pid>.json` | herdr-talk state folder | written while `run` or `serve` is up | Port and token of the [control channel](#control-channel). Removed when the chat ends. |
| `sessions\<id>\` | `%LOCALAPPDATA%\herdr-talk\sessions\` | one folder per launch | Each launch of `run`, `serve` or the GUI is its own node: its own key, short ID, trusted peers and `relay.json`. `--session <id>` reuses one. The old folder `%LOCALAPPDATA%\herdr-talk` is the default node and is kept. |
| `relay.json` | herdr-talk state folder | not set (Local) | Connection settings. `mode`: `local` (LAN only, the default; a missing or unknown value means local) or `supabase` (messages only through the relay; needs `url`, `key`, `name` and `peer`). Other keys: `url`, `key`, `name`, `peer` (the peer's Ed25519 key). Environment variables `HERDR_TALK_RELAY_URL` and `HERDR_TALK_RELAY_KEY` win over the file. |
| `relay-signing.key`, `short-id.txt` | herdr-talk state folder | created on first relay start | This PC's Ed25519 signing key for the relay row, and the short ID (8 letters or digits) other people type to find this PC. Do not share `relay-signing.key`. |
| `HERDR_TALK_EXE` | environment, GUI only | not set | Path of `herdr-talk.exe` for the [Windows GUI](usage.md#11-windows-gui) to start. |

Example:

```sh
bin/herdr-talk run --name my-laptop --tcp-port 47900
```

## Language

The language of the chat's own text is chosen in this order. The first one that is set wins:

1. The `HERDR_TALK_LANG` environment variable
2. `"lang"` in `config.json`, in the plugin config folder (`HERDR_PLUGIN_CONFIG_DIR`)
3. The OS language (Windows UI language, or `LC_ALL` / `LC_MESSAGES` / `LANG`)
4. `en`, when the OS language is not one of the supported languages

Example `config.json`:

```json
{
  "lang": "ko"
}
```

Translations live in `rust/crates/talk-core/locales/<language>.json`. Missing keys fall back to English.

The language does not change the random name list. Names come from the PC's locale, or from the `/lang` command. See [Usage: Names](usage.md#6-names).

## Encryption

Messages between herdr-talk peers are encrypted with Noise (`Noise_IK_25519_ChaChaPoly_BLAKE2s`). Each PC has a key pair, stored in `identity.json` in the state folder. Its peers see the public half in the discovery announcement.

- **Verify before you chat.** A new peer shows a code of 32 hex digits in eight groups, for example `1a2b-3c4d-5e6f-7a8b-9c0d-1e2f-3a4b-5c6d`. The code is computed from both PCs' keys, so both screens show the same value. Compare it with the other PC's screen, then type `/trust <code>` on both sides. Only verified keys are remembered in `trusted.json`. If two different keys match one code, `/trust` refuses.
- **Unverified peers get nothing.** Messages are not sent to them, and their messages are not shown. The handshake is refused before any reply.
- **A changed key is refused.** A new key from an address you trusted before is shown as a warning and is not trusted.
- **Plain text is off by default.** Peers that do not encrypt (an old Python edition without keys) are skipped unless `"plaintext": true` is set in `config.json`. Even then, a peer that has a key is never sent plain text.
  With it off, you see `peer.plain_off` when you send to such a peer, and its plain messages are dropped without a line. The current Python edition has keys, `identity.json`/`trusted.json` in the same format and the same `/trust` flow, and needs no `plaintext` setting. The protocol is in [encryption-protocol.md](encryption-protocol.md). Several Rust chats that share one state folder share one key, so they trust each other (encrypted) and need no `/trust`.
- **Limits.** `/trust` only proves what you compared. Compare the whole code on both screens before you type it. On Windows, `identity.json` relies on the default permissions of your user profile folder.

## Phone host (방장)

One node per PC owns the phone's address and token. Both editions follow the same rules, so Rust and Python nodes in one state folder elect one host between them.

- **Who is the host.** The node that holds `phone.lock` in the state folder (an OS file lock, taken atomically, so nodes that start together get exactly one host). Only this PC counts: nodes on other PCs have their own host and their own phone bridge.
- **`phone.json`** (readable by this user only): `{"port", "token", "urls", "owner_pid", "owner_port"}`. The token is six digits (`000000` to `999999`) from the OS random source. `urls` are address candidates, the default-route adapter first, then other private addresses, virtual adapters (VirtualBox `192.168.56.x`, link-local, Docker) last. `port` is `TALK_BRIDGE_PORT` or 47860. Nothing in it is sent over the network or announced.
- **Name.** The host is decided when a node starts, before its first announcement. Started without `--name`, the host is called `방장` (`host.name`: `Host`, `Gastgeber`, `Anfitrión`, `Hôte`, `ホスト`, `Хост` in the other languages); the usual name-clash numbering still applies (`방장2` when another PC's host has the name). Any other node, started later on the same PC, gets a random name or its `--name` and does not show the phone address by itself (Ctrl+P or `/phone`; it still shows when the host joins). Started with `--name`, the node keeps that name and the status row shows a `[방장]` badge (`host.badge`) while it is the host. The badge is the same for a host with the default name.
- **Name on hand-off.** A node that takes over keeps the name it has: names are never changed by a take-over. Only the status-row badge moves to it, and it shows the phone address once. The old host's name goes with it.
- **When the host ends** the OS gives the lock back and a waiting node takes it within two seconds (whichever polls first, not strictly the oldest), keeps the token and port and writes its own pid. The last node to leave removes `phone.json`. After a crash the file stays until the next host starts.
- **Showing it.** The address and token are drawn over the bottom of the chat screen only, never as a chat line, in no log and not to peers. They appear when this node becomes the host, and when the host of this PC joins (its chat port in `phone.json` matches the announcement from a local address). They hide on the first key typed, with Ctrl+P again, or after 30 seconds. **Ctrl+P** or `/phone` shows them again, on any node (a node that is not the host reads the host's file). The terminal chat (Rust and Python) and the Windows window have it.
- **`/phone new`** makes a new token in `phone.json`. The bridge reads the file on every request, so phones that used the old token get 401 and must enter the new one.
- **Bridge.** `mobile-bridge/bridge.js` takes port and token from `TALK_BRIDGE_PORT` / `TALK_BRIDGE_TOKEN` when set, else from `phone.json`, else it makes a random token and prints it. The six-digit token is guarded by the bridge's block of an address after 5 wrong tries for 10 minutes. The host does not start the bridge: run `node mobile-bridge/bridge.js` yourself.

## Control channel

The Python edition has the same channel (`python src/talk.py run` or `serve`, same `control-<pid>.json`, same `token`/`lines`/`peers`/`send`/`shutdown` requests, same state folder order), so the mobile bridge works with either edition.

`herdr-talk run` and `herdr-talk serve` open a local channel for a GUI or script. `serve` is the same chat without a screen and ends on Ctrl+C, a termination signal, or `shutdown`.

- **Address:** a TCP port on `127.0.0.1`, chosen at start. It is not reachable from other PCs.
- **File:** `control-<pid>.json` in the state folder, with `{"port", "token", "pid"}`. The chat removes it on exit; a file left by a killed process is harmless, and clients skip a file whose connection fails. On Unix the folder is made `0700` and the file `0600`; on Windows the file inherits the user-only default permissions of `%LOCALAPPDATA%`.
- **Token:** 32 random bytes in hex, new for every run. The first line a client sends must be `{"token": "..."}`; the answer is `{"ok": true}`, or `{"ok": false, "error": "bad token"}` and the connection is not used.
- **Requests:** one JSON object per line, one JSON answer per line.

| Request | Answer |
|---------|--------|
| `{"cmd": "lines"}` | `{"ok": true, "lines": [{"text", "mine"}]}`, the last 300 lines |
| `{"cmd": "peers"}` | `{"ok": true, "peers": [{"name", "ip", "port"}], "me": {"name", "port"}}` |
| `{"cmd": "send", "text": "..."}` | `{"ok": true}`. Same as typing the line: messages and `/commands` both work |
| `{"cmd": "shutdown"}` | `{"ok": true}` and the chat ends. Only `serve` accepts it |

Limits: 4 clients at once, 64 KiB per request line, a connection idle for 5 minutes is closed.

## Names

When a fresh, verified peer has your name and a smaller id, you add a number (`Name2`, then `Name3`; at most 30 characters of the name are kept). A peer is verified when its announced key is present and `store::peer_id(key)` equals its announced id; trusting the key is not required. Peers without a key never start a clash. The id decides who changes, and the port only breaks a tie between equal ids (two panes on one PC share an id). An automatic rename happens at most once every 60 seconds per node; `/name` and `/lang` are not limited. The Python edition follows the same rule.

## Discovery and timing

These values are constants in `rust/crates/talk-core/src/net.rs` and `chat.rs`. Change them only if you have a reason to.

| Constant | Value | Meaning |
|----------|-------|---------|
| `UDP_PORT` | `47850` | Discovery port (`HERDR_TALK_UDP_PORT` overrides it) |
| `ANNOUNCE_SECONDS` | `2` | How often a chat announces itself |
| `PEER_TIMEOUT_SECONDS` | `10` | A peer not seen for this long leaves the list |
| `NAME_SETTLE` | `2100 ms` | A new chat listens this long before its first announcement, so a name clash is settled first |
| `AUTO_RENAME_GAP` | `60 s` | Shortest time between two automatic name changes |
| `IDLE_LIMIT_SECONDS` | `600` | Leave after this long with no activity |
| `MAX_TEXT_CHARS` | `2000` | Longest message sent |
| `MAX_MESSAGE_BYTES` | `65536` | Largest network message accepted |
| `MAX_LINES` | `2000` | Lines kept in the message list |
