# Usage

How to install, chat, what each command does, and how names and languages work.

## 1. Install

Install from GitHub:

```sh
herdr plugin install vrwolf1004/herdr-talk/rust
```

Requirements: herdr 0.9.0 or later, and both PCs on the same network. The install downloads a prebuilt binary, so nothing else is needed.

The install checks the binary's SHA-256 checksum. Releases are not signed yet, so the checksum catches a damaged download but not a release replaced on GitHub. This is the same trust as installing any plugin from its GitHub repository. If you prefer, build from source with `HERDR_TALK_BUILD_FROM_SOURCE=1`, or use the Python edition.

## 2. Start a chat

From a herdr pane on each PC, run the `open` action to split a chat pane below
the current pane:

```sh
herdr plugin action invoke open --plugin herdr-talk           # Linux, macOS
```

The action requires a current herdr pane (`HERDR_PANE_ID`). You can also open
the chat pane directly with `herdr plugin pane open --plugin herdr-talk
--entrypoint chat`.
After Ctrl+C, the chat command exits and the split pane returns to its shell
prompt.

When the pane starts, it shows the name it picked and its TCP port:

```text
herdr-talk: you are 'Haruto' (TCP port 52311). Type /help for commands.
```

Other chat panes on the same network show a `joined` line when they are first seen:

```text
* Liam joined (172.31.99.181)
```

## 3. Send messages

Type a line and press Enter. The message goes to every peer seen in the last 10 seconds. Your own messages show `(you)` after your name:

```text
Liam: hi, anyone here?
Haruto (you): hello!
```

Messages are not stored. A peer that is offline when you send does not get the message later.

Limits:

- A message is cut to 2000 characters, and line breaks become spaces.
- The message list keeps the last 2000 lines.

## 4. Commands

| Input | What it does |
|-------|--------------|
| `<text>` | Send to every peer |
| `/name <new>` | Change your name (up to 32 characters) |
| `/lang <code>` | Pick a random new name from the list for a language |
| `/peers` | List peers seen recently, with their address, port and state (verified, or the code to check) |
| `/trust <code>` | Verify the peer whose code matches. Compare the code on both screens first. See [Configuration: Encryption](configuration.md#encryption) |
| `/help` | Show the commands and keys. |
| `/quit` | Leave the chat |

Language codes for `/lang`: `ko en zh ja vi fr de es ru id pt`.

`/lang` and `/name` change only your display name. Peers see the new name on their next discovery message, about 2 seconds later.

## 5. Keys

| Key | What it does |
|-----|--------------|
| Enter | Send the line, or run the command |
| Backspace | Delete one character |
| Ctrl+L | Clear the message list on screen |
| Ctrl+N | Fill the input with `/name ` |
| Ctrl+C, Ctrl+D | Quit |

## 6. Names

When you do not pass `--name`, the chat picks a random name from the list for the PC's language:

- A PC set to Korean gets a Korean name, such as 민준 or 서연.
- A PC set to English gets an English name, such as Liam or Olivia.
- For a language not in the list, the name is picked from all the lists.

If another peer already uses your name, one of you changes it by itself, by these rules:

- The node with the larger id adds a number: `Liam` becomes `Liam2`, or `Liam3` if `Liam2` is taken. A name that ends in a digit gets the number after it (`Room101` becomes `Room1012`). Only the first 30 characters of the name are kept.
- Only peers whose public key matches their id count. A peer without a key (the Python edition) cannot take a name from you.
- An automatic rename happens at most once every 60 seconds per node. A name you type with `/name` or `/lang` is never held back.
- A new chat waits about 2 seconds for announcements before it announces itself, so a clash with a peer already online is settled first. The same rule applies on `/name` and when a peer appears later.
- The Python edition follows the same numbering and the same 60 second limit, but it has no keys, so it cannot check who announced a name.

Use `/name` to pick another name at any time.

To choose the name when starting the script directly:

```sh
bin/herdr-talk run --name my-laptop
```

## 7. Leaving

- `/quit` leaves at any time.
- The chat leaves on its own after 10 minutes without chat activity. Sending or receiving a message and command output reset the timer in either view. Typing without submitting does not count as activity.

## 8. Language

The chat's own text (messages, `/help` box, status line) follows the language setting. The setting is chosen in this order:

1. The `HERDR_TALK_LANG` environment variable
2. `"lang"` in `config.json` (see [Configuration](configuration.md))
3. The OS language

Languages: `en`, `ko`, `ru`, `fr`, `es`, `de`, `ja`. Other languages fall back to English. The command names (`/name`, `/lang`, `Ctrl+N`) stay the same in every language.

## 9. Network

- Both PCs must be on the same network with no client isolation.
- Discovery uses UDP port `47850`, and messages use a random TCP port that each chat picks at start.
- Allow both in the firewall. If the peer list stays empty, check the firewall first.
- Discovery is UDP broadcast and messages go directly over TCP. Messages to verified peers are encrypted. Plain text is off unless you turn it on for peers that do not encrypt. See [Configuration: Encryption](configuration.md#encryption).
- Messages are accepted only from an address that announced itself as a peer, and at most 20 messages per address in 10 seconds are kept. Others are dropped without a notice.

## 10. Dock block

Run `dock-start` to show a chat block in the shared `herdr-dock` workspace. The block shows `talk: N online` in the agents list. Clicking it focuses the pane, and the chat window is that pane.

- The block leaves after 10 minutes without chat activity, as in a normal chat. Run `dock-start` again to bring it back.
- `dock-stop` hides the block and releases its slot.
- The plugin starts the block on herdr startup as well.

> `herdr-dock-grid` is optional. The plugin allocates its dock pane by the same rules without it.

## 11. Windows GUI

`gui/` holds a separate Windows app (Rust, eframe) that shows the chat in its own window. It is not part of the herdr plugin and has its own version (`gui/Cargo.toml`).

```sh
cd gui
cargo run --release
```

How it starts:

1. It looks for `herdr-talk.exe`: next to the GUI program, `HERDR_TALK_EXE`, `rust/target/release`, then the newest herdr plugin install.
2. It starts `herdr-talk serve` without a console window and connects to it through the control channel. The status line shows the steps: `기존 서버 없음 → 자체 서버 실행 중`, then `자체 서버 연결됨 (port N)`.
3. If that fails (program not found, no control file in 10 seconds, repeated exits), it runs the chat inside the window: `내장 모드로 실행 중`, with the reason. It does not switch back later.
4. When the window closes, the `serve` process it started is asked to end (killed only if it does not).

Every window is its own chat node, so several windows on one PC see each other as peers. The window never uses another window's `serve` or a chat pane. Note: a `herdr-talk.exe` from a plugin install older than the one with `serve` has no `serve` command, so the GUI falls back to embedded mode.

Screens and controls:

- **Start screen:** the name, a short description, the current status and an enter button. Press it to open the chat.
- **Chat:** message list, input line, and a peer panel with your own entry on top.
- **Theme select:** Dark, Light, Amber, Green (default Green). It changes the colors and the window title bar color (Windows 11).
- **CRT checkbox:** on by default. It draws scanlines and a bezel around the window. Turn it off for a plain look.
- **Font:** Galmuri11, bundled in the program (`gui/assets`, SIL Open Font License). Windows system fonts cover any character it lacks.

## 12. Headless mode and control channel

```sh
herdr-talk serve [--name NAME] [--tcp-port N] [--udp-port N]
```

`serve` is the chat without a screen. It takes no `--dock` or `--pane`. It ends on Ctrl+C, a termination signal, or a `shutdown` request, and removes its control file.

`run` and `serve` open a control channel on `127.0.0.1` (random port, new token per run) and write `control-<pid>.json` in the state folder. Only your user can read the file. Details and the request format are in [Configuration: Control channel](configuration.md#control-channel).

## 13. Troubleshooting

Symptoms and checks are in [Troubleshooting](troubleshooting.md).

## Documentation

- [Usage (한국어)](usage.ko.md)
- [Configuration](configuration.md): settings, language order, timing constants
- [Development](development.md): code layout, adding languages, names, messages and commands
- [Troubleshooting](troubleshooting.md): symptoms and checks
- [TODO](todo.md): improvement list
