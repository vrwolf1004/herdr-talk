# Configuration

herdr-talk has no required settings. Everything below is optional.

## Settings at a glance

| Setting | Where | Default | What it does |
|---------|-------|---------|--------------|
| `--name` | command line | random name for the PC's language | Name shown to others |
| `--tcp-port` | command line | `0` (pick a free port) | TCP port for incoming messages |
| `--udp-port` | command line | `47850` | Discovery port. Use the same value on every PC. |
| `HERDR_TALK_LANG` | environment | not set | Language code: `en`, `ko`, `ru`, `fr`, `es`, `de`, `ja` |
| `"lang"` | `config.json` in the plugin config folder | not set | Same as `HERDR_TALK_LANG` |

Example:

```sh
python -X utf8 src/talk.py run --name my-laptop --tcp-port 47900
```

## Language

The language of the chat's own text is chosen in this order. The first one that is set wins:

1. The `HERDR_TALK_LANG` environment variable
2. `"lang"` in `config.json`, in the plugin config folder (`HERDR_PLUGIN_CONFIG_DIR`)
3. The OS language (Windows UI language, or `LC_ALL` / `LC_MESSAGES` / `LANG`)
4. `en`, when the OS language is not one of the supported languages

Example `config.json`:

```json
{
  "lang": "ko"
}
```

Translations live in `src/locales/<language>.json`. Missing keys fall back to English.

The language does not change the random name list. Names come from the PC's locale, or from the `/lang` command. See [Usage: Names](usage.md#6-names).

## Discovery and timing

These values are constants in `src/talk.py`. Change them only if you have a reason to.

| Constant | Value | Meaning |
|----------|-------|---------|
| `UDP_PORT` | `47850` | Discovery port |
| `ANNOUNCE_SECONDS` | `2` | How often a chat announces itself |
| `PEER_TIMEOUT_SECONDS` | `10` | A peer not seen for this long leaves the list |
| `IDLE_LIMIT_SECONDS` | `600` | Leave after this long with no activity |
| `MAX_TEXT_CHARS` | `2000` | Longest message sent |
| `MAX_MESSAGE_BYTES` | `65536` | Largest network message accepted |
| `MAX_LINES` | `2000` | Lines kept in the message list |
