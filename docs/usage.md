# Usage

How to chat, what each command does, and how names and languages work.

## Start a chat

Open the chat pane from herdr on each PC:

```sh
herdr plugin pane open --plugin herdr-talk --entrypoint chat-windows   # Windows
herdr plugin pane open --plugin herdr-talk --entrypoint chat           # Linux, macOS
```

When the pane starts, it shows the name it picked and its TCP port:

```text
herdr-talk: you are 'Haruto' (TCP port 52311). Type /help for commands.
```

Other chat panes on the same network show a `joined` line when they are first seen:

```text
* Liam joined (172.31.99.181)
```

## Send messages

Type a line and press Enter. The message goes to every peer seen in the last 10 seconds. Your own messages show `(you)` after your name:

```text
Liam: hi, anyone here?
Haruto (you): hello!
```

Messages are not stored. A peer that is offline when you send does not get the message later.

Limits:

- A message is cut to 2000 characters, and line breaks become spaces.
- The message list keeps the last 2000 lines.

## Commands

| Input | What it does |
|-------|--------------|
| `<text>` | Send to every peer |
| `/name <new>` | Change your name (up to 32 characters) |
| `/lang <code>` | Pick a random new name from the list for a language |
| `/peers` | List peers seen recently, with their address and port |
| `/help` | Show a box with the commands and keys (any key closes it) |
| `/quit` | Leave the chat |

Language codes for `/lang`: `ko en zh ja vi fr de es ru id pt`.

`/lang` and `/name` change only your display name. Peers see the new name on their next discovery message, about 2 seconds later.

## Keys

In the plain pane (no `prompt_toolkit`):

| Key | What it does |
|-----|--------------|
| Enter | Send the line, or run the command |
| Backspace | Delete one character |
| Ctrl+L | Clear the message list on screen |
| Ctrl+C, Ctrl+D | Quit |

If `prompt_toolkit` is installed, the chat uses a full-screen view instead. There, **Ctrl+N** fills the input with `/name ` so you can type a new name, and **Ctrl+C** or **Ctrl+D** quits.

## Names

When you do not pass `--name`, the chat picks a random name from the list for the PC's language:

- A PC set to Korean gets a Korean name, such as 민준 or 서연.
- A PC set to English gets an English name, such as Liam or Olivia.
- For a language not in the list, the name is picked from all the lists.

Names are not checked for uniqueness, so two PCs can show the same name by chance. Use `/name` to change it.

To choose the name when starting the script directly:

```sh
python -X utf8 src/talk.py run --name my-laptop
```

## Leaving

- `/quit` leaves at any time.
- The chat leaves on its own after 10 minutes with no chat activity. Typing a message or a command counts as activity.

## Language

The chat's own text (messages, `/help` box, status line) follows the language setting. The setting is chosen in this order:

1. The `HERDR_TALK_LANG` environment variable
2. `"lang"` in `config.json` (see [Configuration](configuration.md))
3. The OS language

Languages: `en`, `ko`, `ru`, `fr`, `es`, `de`, `ja`. Other languages fall back to English. The command names (`/name`, `/lang`, `Ctrl+N`) stay the same in every language.

## Network

- Both PCs must be on the same network with no client isolation.
- Discovery uses UDP port `47850`, and messages use a random TCP port that each chat picks at start.
- Allow both in the firewall. If the peer list stays empty, check the firewall first.
- Nothing is encrypted in this phase. Use it only on a network you trust.

## Troubleshooting

| Symptom | Check |
|---------|-------|
| `no peers yet` | Both PCs on the same network? Firewall allows UDP `47850` and the TCP port? |
| `! could not reach <name>` | The peer closed its pane or the firewall blocks its TCP port. Peers that have not been seen for 10 seconds drop from the list. |
| Language is not what you expect | Set `HERDR_TALK_LANG=ko` (or another code) to force it. |
