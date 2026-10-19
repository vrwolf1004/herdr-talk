# herdr-talk

[**English**](README.md) | [한국어](README.ko.md)

Chat with people on other PCs from inside herdr. Works directly on the same network without a server.

> Status: phase 1 (same network). Over-the-internet chat through Supabase with end-to-end encryption is planned for phase 2.

## Requirements

- herdr 0.9.0 or later
- Windows, Linux, or macOS
- Python 3
- Optional: `pip install prompt_toolkit` for a fixed input line at the bottom. Without it, the chat still works with a plain `> ` prompt.
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

Commands in the chat:

| Input | What it does |
|-------|--------------|
| `<text>` | Send to all peers |
| `/peers` | List peers seen recently |
| `/help` | Show commands |
| `/quit` | Leave the chat |

Each PC shows its name when it starts. Use `--name` to change it when running the script directly:

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

## Documentation

- [Usage](docs/usage.md)
- [Development](docs/development.md)
