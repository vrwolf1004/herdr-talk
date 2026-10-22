# herdr-talk

[**English**](README.md) | [한국어](README.ko.md)

UI languages: English · Korean · Russian · French · Spanish · German · Japanese

Chat with people on other PCs from inside herdr. Nearby peers connect directly over the same network without a chat server.

## Dashboard design

Chat pane:

```text
herdr-talk: you are 'Haruto' (TCP port 52311). Type /help for commands.
* Liam joined (172.31.99.181)

Liam: hi, anyone here?
Haruto (you): hello!

>
```

This plugin uses a chat pane rather than an Agents-sidebar block. The current
implementation is LAN-only: peers are discovered with UDP broadcast and
messages are sent directly over TCP. Messages are not stored or encrypted, so
use it only on a network you trust.

## Installation

```sh
herdr plugin install vrwolf1004/herdr-talk
```

## Actions

| Action | What it does |
| --- | --- |
| `open` | Split a chat pane below the current pane (Linux/macOS) |
| `open-windows` | Split a chat pane below the current pane (Windows) |
| `dock-start` / `dock-start-windows` | Show the chat as a block in the `herdr-dock` workspace |
| `dock-stop` / `dock-stop-windows` | Hide the dock block |

The action must be run from a herdr pane. See the [usage guide](docs/usage.md) for direct pane opening, chat commands, and network requirements.

## Requirements

- Windows, Linux, or macOS
- herdr 0.9.0 or later
- Python 3; standard library only
- Both PCs on the same network, with discovery and TCP traffic allowed by the firewall
- Optional `prompt_toolkit` enables the full-screen chat view; otherwise the built-in terminal view is used

## Documentation

- [Usage (English)](docs/usage.md)
- [사용법 (한국어)](docs/usage.ko.md)
- [Configuration](docs/configuration.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Development](docs/development.md)
- [TODO](docs/todo.md)

## Support the project

If this project is useful to you, consider buying the developer a coffee.

[![Support on Ko-fi](https://img.shields.io/badge/Ko--fi-Support-FF5E5B?style=for-the-badge&logo=buymeacoffee&logoColor=white)](https://ko-fi.com/dev_ohyt)
