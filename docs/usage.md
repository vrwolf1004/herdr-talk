# Usage

How to install, chat, what each command does, and how names and languages work.

## 1. Install

Install from GitHub:

```sh
herdr plugin install vrwolf1004/herdr-talk/rust
```

Requirements: herdr 0.9.0 or later, and both PCs on the same network. The install downloads a prebuilt binary, so nothing else is needed.

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

Names are not checked for uniqueness, so two PCs can show the same name by chance. Use `/name` to change it.

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

## 11. Troubleshooting

Symptoms and checks are in [Troubleshooting](troubleshooting.md).

## Documentation

- [Usage (한국어)](usage.ko.md)
- [Configuration](configuration.md): settings, language order, timing constants
- [Development](development.md): code layout, adding languages, names, messages and commands
- [Troubleshooting](troubleshooting.md): symptoms and checks
- [TODO](todo.md): improvement list
