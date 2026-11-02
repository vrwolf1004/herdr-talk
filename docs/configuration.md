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
| `"plaintext"` | `config.json` in the plugin config folder | `false` | Allow plain text to peers that do not encrypt (the Python edition). See [Encryption](#encryption). |
| `identity.json`, `trusted.json` | herdr-talk state folder (`HERDR_PLUGIN_STATE_DIR`) | created on first start | This PC's key and the peers you verified. Do not share `identity.json`. |

Example:

```sh
bin/herdr-talk run --name my-laptop --tcp-port 47900
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

Translations live in `rust/crates/talk-core/locales/<language>.json`. Missing keys fall back to English.

The language does not change the random name list. Names come from the PC's locale, or from the `/lang` command. See [Usage: Names](usage.md#6-names).

## Encryption

Messages between herdr-talk peers are encrypted with Noise (`Noise_IK_25519_ChaChaPoly_BLAKE2s`). Each PC has a key pair, stored in `identity.json` in the state folder. Its peers see the public half in the discovery announcement.

- **Verify before you chat.** A new peer shows a code of 32 hex digits in eight groups, for example `1a2b-3c4d-5e6f-7a8b-9c0d-1e2f-3a4b-5c6d`. The code is computed from both PCs' keys, so both screens show the same value. Compare it with the other PC's screen, then type `/trust <code>` on both sides. Only verified keys are remembered in `trusted.json`. If two different keys match one code, `/trust` refuses.
- **Unverified peers get nothing.** Messages are not sent to them, and their messages are not shown. The handshake is refused before any reply.
- **A changed key is refused.** A new key from an address you trusted before is shown as a warning and is not trusted.
- **Plain text is off by default.** Peers that do not encrypt (the Python edition) are skipped unless `"plaintext": true` is set in `config.json`. Even then, a peer that has a key is never sent plain text.
- **Limits.** `/trust` only proves what you compared. Compare the whole code on both screens before you type it. On Windows, `identity.json` relies on the default permissions of your user profile folder.

## Discovery and timing

These values are constants in `rust/crates/talk-core/src/net.rs` and `chat.rs`. Change them only if you have a reason to.

| Constant | Value | Meaning |
|----------|-------|---------|
| `UDP_PORT` | `47850` | Discovery port |
| `ANNOUNCE_SECONDS` | `2` | How often a chat announces itself |
| `PEER_TIMEOUT_SECONDS` | `10` | A peer not seen for this long leaves the list |
| `IDLE_LIMIT_SECONDS` | `600` | Leave after this long with no activity |
| `MAX_TEXT_CHARS` | `2000` | Longest message sent |
| `MAX_MESSAGE_BYTES` | `65536` | Largest network message accepted |
| `MAX_LINES` | `2000` | Lines kept in the message list |
