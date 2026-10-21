# herdr-talk

[**English**](README.md) | [한국어](README.ko.md)

UI languages: English · Korean · Russian · French · Spanish · German · Japanese

Chat with people on other PCs from inside herdr. Nearby peers connect directly over the same network without a chat server.

## Dashboard design

The chat pane lists messages and keeps the input line at the bottom. Messages are sent directly to peers and are not stored.

## Installation

```sh
herdr plugin install vrwolf1004/herdr-talk
```

## Actions

| Action | What it does |
| --- | --- |
| `open` | Split a chat pane below the current pane (Linux/macOS) |
| `open-windows` | Split a chat pane below the current pane (Windows) |

The action must be run from a herdr pane. See the [usage guide](docs/usage.md) for direct pane opening, chat commands, and network requirements.

## Requirements

- Windows, Linux, or macOS
- herdr 0.9.0 or later
- Python 3; standard library only
- Both PCs on the same network, with discovery and TCP traffic allowed by the firewall

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
