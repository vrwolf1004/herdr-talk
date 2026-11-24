# herdr-talk

[**English**](README.md) | [한국어](README.ko.md)

UI languages: English · Korean · Russian · French · Spanish · German · Japanese

Chat with people on other PCs from inside herdr. Nearby peers connect directly over the same network without a chat server.

> **⚠ Urgent fix in progress (2026-11-13)**
> - **Supabase room mode:** room tables were readable by anyone before migration 0006. Migration 0006 is applied on the production project. Request signing and end-to-end checks of the room functions are not fully verified yet.
> - **LAN pairing (SAS code):** implemented, not yet tested on two PCs. Rust and Python nodes are not yet confirmed to connect to each other.
> - Do not use this build for sensitive chats until these are verified.

## Dashboard design

Chat pane (Python edition, the default):

```text
▣ HERDR-TALK                                                    SYS.VER 1.0
────────────────────────────────────────────────────────────────────────
COMMUNICATION LOG
herdr-talk: 당신의 이름은 '서연'입니다 (TCP 포트 55668). 명령어는 /help를 입력하세요.
[23:33] 서연 (나) : 하하
[23:33] 하준 : dho
CONNECTED NODES
HOST/YOU 서연, 하준, 지호
────────────────────────────────────────────────────────────────────────
MESSAGE > 메시지를 입력하세요...
[방장]  상대: 2 · CLOUD SERVICE · room ID: PRXALBYO · setup code: HBJW1MI5   [CONNECTED]  F1 도움말
```

- Each message line is `[HH:MM] name : text`. Your own lines show `(나)` after your name.
- The status line shows the host badge `[방장]`, the peer count, the mode (`BASE STATION` or `CLOUD SERVICE`), and in cloud mode the room ID and setup code. A problem is appended after them.
- `F1` shows the help, `F3` switches the language.

## Installation



Recommended: the Python edition. It needs only Python 3 and no downloaded executables:

```sh
herdr plugin install vrwolf1004/herdr-talk/python
```

Rust edition (optional, downloads an executable):

```sh
herdr plugin install vrwolf1004/herdr-talk/rust
```

The Python edition encrypts the same way as the Rust edition (pure Python, no packages), so the two talk to each other after you compare codes and `/join`. Only an older Python edition without keys needs `"plaintext": true` on the Rust side. See [docs/encryption-protocol.md](docs/encryption-protocol.md).

### Windows GUI (standalone)

A standalone Windows window (not in herdr) is also available:

```sh
# Build from source
cd rust
cargo build --release
cd ../gui
cargo build --release
# Run: gui/target/release/herdr-talk-gui.exe
```

The GUI connects to a running `herdr-talk serve` through its control channel, or starts one automatically. Use Supabase for remote chat, or Local mode for LAN discovery.

## Actions

| Action | What it does |
| --- | --- |
| `open` | Split a chat pane below the current pane |
| `dock-start` | Show the chat as a block in the `herdr-dock` workspace |
| `dock-stop` | Hide the dock block |

The action must be run from a herdr pane. See the [usage guide](docs/usage.md) for direct pane opening, chat commands, and network requirements.

## Requirements

- Windows, Linux, or macOS
- herdr 0.9.0 or later
- Both PCs on the same network, with discovery and TCP traffic allowed by the firewall

## Documentation

- [Usage (English)](docs/usage.md)
- [사용법 (한국어)](docs/usage.ko.md)
- [Configuration](docs/configuration.md)
- [Operator setup (Supabase, setup codes)](docs/operator-setup.md)
- [Phone (Expo Go), Korean](docs/mobile.md)
- [Encryption protocol](docs/encryption-protocol.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Development](docs/development.md)
- [Rust roadmap (Korean)](docs/rust-roadmap.md)
- [TODO](docs/todo.md)
- [Additional sections](docs/readme-extras.md): sections moved out of this README

## Support the project

If this project is useful to you, consider buying the developer a coffee.

[![Support on Ko-fi](https://img.shields.io/badge/Ko--fi-Support-FF5E5B?style=for-the-badge&logo=buymeacoffee&logoColor=white)](https://ko-fi.com/dev_ohyt)
