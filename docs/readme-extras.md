# Additional sections (moved from README)

## Headless mode and control channel

`herdr-talk serve` runs the chat without a screen, for a GUI or another local program. It ends on Ctrl+C, a termination signal, or a `shutdown` request.

`run` and `serve` both open a local control channel: a TCP port on `127.0.0.1`, protected by a per-run token. The port and token are written to `control-<pid>.json` in the state folder (readable only by your user) and removed when the chat ends. The commands are `lines`, `peers` and `send`; `serve` also accepts `shutdown`. See [configuration](docs/configuration.md#control-channel).

## Windows GUI

`gui/` is a separate Windows app (Rust, eframe) that shows the chat in its own window. It first tries to start `herdr-talk serve` hidden and connect to it; if that fails, it runs the chat inside the window (embedded mode). A start screen with an enter button comes first. Each window is its own chat node. The top bar has a theme select (Dark, Light, Amber, Green) and a CRT checkbox (on by default: scanlines and a bezel). The text uses the Galmuri11 font (`gui/assets`, SIL OFL). See the [usage guide](docs/usage.md#11-windows-gui).

```sh
cd gui
cargo run --release
```

## Connection mode: Local or Supabase

- **Local (LAN)**: the default. PCs on the same network find each other and chat with no server.
- **Supabase**: messages go only through the Supabase relay. Nothing is sent over the LAN. Pick it on the first screen (GUI) or the first run (TUI), or change it with `Ctrl+R` (settings) or the 연결 방식 button.
- The Supabase settings are URL, publishable key, and the other side's 8-character short ID. The display name is assigned automatically.
- Each launch (`run`, `serve`, the GUI) is its own **node**, with its own key, short ID and trusted peers, kept in `sessions\<id>\`. Two windows on one PC are two nodes. `--session <id>` reuses one.
- In Supabase mode, nodes on the same PC do not chat with each other: each one talks to the phone through the relay.
 The short ID is made once per PC (`short-id.txt`).
- Verify each other with `/trust <code>` on both sides before chatting. The relay shows the same code as the LAN does.

## Phone (Expo Go)

An Android phone can join the chat through `mobile-bridge/bridge.js` and the Expo app in `mobile/`. The phone is not a peer: it talks to the bridge over HTTP and the bridge talks to a herdr-talk node on the same PC. Use it only on a trusted Wi-Fi, because that leg is plain HTTP.

- The first node on a PC is the **host** (named "Host" unless `--name` is given, with a `[Host]` badge). It owns the phone URL and a 6-digit token in `phone.json`. The next node takes over when the host exits.
- The chat shows the phone address and token when you become host or when the host joins, and on `Ctrl+P` or `/phone`. `/phone new` makes a new token. They are drawn on screen only, never in chat lines or logs.
- Run `node mobile-bridge/bridge.js`, then `npx expo start --host lan` in `mobile/`, and enter the address and token in the app.

Details, checks and open items: [docs/mobile.md](docs/mobile.md).

## Names

If another peer already uses your name, the node with the larger id adds a number (`Liam` becomes `Liam2`, then `Liam3`). Only peers whose key matches their id are considered, and an automatic rename happens at most once every 60 seconds. See [usage](docs/usage.md#6-names).

## Language

Chat text follows the PC's language: English, Korean, Russian, French, Spanish, German, or Japanese, otherwise English. Set `HERDR_TALK_LANG` or `"lang"` in `config.json` to override it. See [configuration](docs/configuration.md#language).

