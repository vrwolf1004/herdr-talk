# Development

## Layout

```
herdr-talk/
├── README.md, README.ko.md     short overview (English, Korean)
├── herdr-plugin.toml           herdr manifest: open actions and chat panes for each OS
├── docs/
│   ├── usage.md                how to install, chat, commands, keys, names, language, network
│   ├── usage.ko.md             same as usage.md, in Korean
│   ├── configuration.md        settings, language order, timing constants
│   ├── troubleshooting.md      symptoms and checks
│   ├── development.md          this file
│   └── todo.md                 improvement list
└── src/
    ├── talk.py                 entry point: discovery, messages, screens, commands
    ├── i18n.py                 language choice and t(key, **values)
    └── locales/<language>.json translated strings (en, ko, ru, fr, es, de, ja)
```

## Running locally

Run the script directly. Use `-X utf8` so Korean and Japanese text prints correctly.

```sh
python -X utf8 src/talk.py run --name test
```

Run it in a real terminal. The full-screen view needs a console.

To test on one machine, start two instances with different names. Both see each other, because discovery works through UDP broadcast on the same machine.

The herdr panes run the same command without `--name`, so each pane picks its own name. A pane keeps the code it started with, so close and reopen it after a change.

## How the code is organized

- **Discovery**: `announce()` sends the name and TCP port by UDP broadcast every `ANNOUNCE_SECONDS`. `listen_for_peers()` records each peer in `Chat.peers`. `fresh_peers()` drops peers not seen for `PEER_TIMEOUT_SECONDS`.
- **Messages**: `send_to()` opens a TCP connection per message and sends one JSON object. `receive_messages()` accepts connections, and `receive_one()` shows the message.
- **Commands**: `Chat.handle()` runs one typed line. It returns `False` to leave the chat.
- **Screens**: `run_screen()` uses `prompt_toolkit` when it is installed. If the import fails, or the full-screen app cannot start (no usable console), it falls back to `run_ansi()`, which redraws the screen with plain ANSI codes. The help box (`HELP_BOX`) is drawn only by `run_ansi()`. `run_plain()` is not called anywhere (see [TODO](todo.md)).
- **Idle timer**: `leave_when_idle()` (plain view) and `leave_when_idle_screen()` (full-screen view) check `Chat.last_activity`. Only `Chat.add_line()` updates it, so the plain view does not reset it yet (see [TODO](todo.md)).
- **Names**: `NAMES_BY_LANGUAGE` maps a language code to its Windows language name and a list of names. `default_name()` picks from the list that matches the PC's locale.

## Adding a language for names

1. Add an entry to `NAMES_BY_LANGUAGE` in `src/talk.py`, with the code, the Windows language name, and 10 names.
2. If the chat text should also be in that language, add it to `src/i18n.py` (`SUPPORTED`) and add `src/locales/<code>.json`.

## Adding or changing a message

Every language file in `src/locales/` must have the same keys.

1. Add the key to all seven files. Keep the same placeholders (`{name}`, `{ip}`, and so on) in each one.
2. Use it with `t("key", name=...)` in `src/talk.py`.
3. Check the width of help box lines. Each line uses display width, so Korean and Japanese characters count as two columns.

Quick check that the keys match:

```sh
python -X utf8 -c "
import json, glob
en = set(json.load(open('src/locales/en.json', encoding='utf-8')))
for f in sorted(glob.glob('src/locales/*.json')):
    keys = set(json.load(open(f, encoding='utf-8')))
    print(f, 'missing:', sorted(en - keys), 'extra:', sorted(keys - en))
"
```

## Adding a command

1. Add a branch to `Chat.handle()` in `src/talk.py`.
2. Add a line to the help box (`help.*` keys in every locale file), and update `help.oneline` for the full-screen view.
3. Document it in `docs/usage.md` and in both READMEs.

## herdr panes

`herdr-plugin.toml` defines an `open` action (`open-windows` on Windows) and
two direct chat panes:

- `chat-windows` for Windows, which runs `python`
- `chat` for Linux and macOS, which runs `python3`

Both panes use `split` placement and run `src/talk.py run`. The `open` action
requires a current herdr pane, splits a new pane below it, then runs the same
chat command there with `pane.run`.

## Checks before a commit

- `python -m py_compile src/talk.py src/i18n.py`
- The key check above prints no missing or extra keys.
- Run the chat once and check the changed screen or command.
