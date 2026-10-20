# herdr-talk

[**English**](README.md) | [한국어](README.ko.md)

Chat with people on other PCs from inside herdr. Works directly on the same network without a server.

> Status: phase 1 (same network). Over-the-internet chat through Supabase with end-to-end encryption is planned for phase 2.

## Requirements

- herdr 0.9.0 or later
- Windows, Linux, or macOS
- Python 3
- No extra packages. The input line stays at the bottom of the pane.
- Both PCs on the same network, with discovery and TCP ports allowed by the firewall

## Quick start

1. Install the plugin on both PCs:

   ```sh
   herdr plugin install vrwolf1004/herdr-talk
   ```

2. On each PC, open the chat pane:

   ```sh
   herdr plugin pane open --plugin herdr-talk --entrypoint chat-windows   # Windows
   herdr plugin pane open --plugin herdr-talk --entrypoint chat           # Linux, macOS
   ```

3. Type a message and press Enter. It goes to every peer on the network.

The chat looks like this. Messages from others show their name, your own messages show `(you)`, and the input line stays at the bottom:

```text
* Liam joined (172.31.99.181)
Liam: hi, anyone here?
Riku (you): hello!
>
```

Commands in the chat:

| Input | What it does |
|-------|--------------|
| `<text>` | Send to all peers |
| `/name <new>` | Change your name |
| `/lang <code>` | Pick a random new name from a language list: `ko en zh ja vi fr de es ru id pt` |
| `/peers` | List peers seen recently |
| `/help` | Show the commands and keys. In the plain view it is a box (any key closes it). In the full-screen view it is one line. |
| `/quit` | Leave the chat |

Keys: Backspace deletes one character, Ctrl+L clears the message list, Ctrl+C quits. If `prompt_toolkit` is installed, the chat uses a full-screen view instead, where Ctrl+N fills `/name ` so you can rename yourself.

Each PC gets a random name when it starts, picked from the list for its language (a PC set to Korean gets Korean names). Use `/lang <code>` in the chat to pick another one. To set your own name when running the script directly, use `--name`:

```sh
python -X utf8 src/talk.py run --name my-laptop
```

## How it works

- Each chat instance broadcasts its name and TCP port over UDP port `47850` every 2 seconds.
- Other instances on the network record the peer.
- Messages are sent directly to each peer over TCP. Nothing is stored.

## Limits

- Only works on the same network (same router, no client isolation).
- Messages are not stored. A peer that is offline does not get the message.
- Not encrypted in phase 1. Use only on a network you trust.
- The chat leaves on its own after 10 minutes without activity. In the plain view, activity does not reset this timer yet, so the chat can leave 10 minutes after it starts (known issue, see [TODO](docs/todo.md)).

## Documentation

- [Usage](docs/usage.md): install, commands, keys, names, language, network
- [사용법 (한국어)](docs/usage.ko.md): 설치, 명령어, 단축키, 이름, 언어, 네트워크
- [Configuration](docs/configuration.md): settings, language order, timing constants
- [Troubleshooting](docs/troubleshooting.md): symptoms and checks
- [Development](docs/development.md): code layout, adding languages, names, messages and commands
- [TODO](docs/todo.md): improvement list

## Support the project

If this project is useful to you, consider buying the developer a coffee.

[![Support on Ko-fi](https://img.shields.io/badge/Ko--fi-Support-FF5E5B?style=for-the-badge&logo=buymeacoffee&logoColor=white)](https://ko-fi.com/dev_ohyt)
