# Troubleshooting

Check the symptom in the table first. Each check points to the setting or step to look at.

| Symptom | Check |
|---------|-------|
| `no peers yet` | Both PCs on the same network? Firewall allows UDP `47850` and the TCP port? |
| `herdr-talk open: run this from a herdr pane` | Run the `open` action from a herdr pane so it has a `HERDR_PANE_ID`, or open the `chat` pane directly. |
| `! could not reach <name>` | The peer closed its pane or the firewall blocks its TCP port. Peers that have not been seen for 10 seconds drop from the list. |
| Language is not what you expect | Set `HERDR_TALK_LANG=ko` (or another code) to force it. |
| Chat leaves after 10 minutes | The idle timeout is 10 minutes without submitted chat activity. Sending or receiving a message resets it; typing without submitting does not. |
| GUI status shows `내장 모드로 실행 중 (herdr-talk 실패: ...)` | The GUI could not start `herdr-talk serve` and runs the chat itself. The text names the reason. Chat still works. To use `serve`, put `herdr-talk.exe` (a version with `serve`) next to the GUI, or set `HERDR_TALK_EXE`. |
| GUI status stays at `기존 서버 없음 → 자체 서버 실행 중` | It waits up to 10 seconds for the control file, then falls back to embedded mode. |
| Your name got a number (`Liam2`) | Another verified peer with a smaller id had the same name. Use `/name` to pick another. Automatic renames happen at most once every 60 seconds. |
| A leftover `control-<pid>.json` in the state folder | A killed chat could not remove it. It is harmless and can be deleted. |
| `no peers yet` on one PC with two chats | Fixed 2026-11-06 in `net.rs` (`is_local_address` now accepts any of this PC's adapter addresses). Rebuild and restart the running `herdr-talk.exe`. See [mobile.md](mobile.md). |
| `<name> 의 키가 바뀌었습니다. 확인하지 않았고 보내지도 않습니다` | A new key appeared at an address you trusted before. On one PC with several identities this was a false alarm (fixed 2026-11-06). Trust only when you know why: compare codes, then `/trust`. |
| `<name> 에게 보내지 않음: 암호화하지 않는 상대이고 평문이 꺼져 있습니다` | The peer is a plaintext (Python) edition and `"plaintext": true` is off in the Rust `config.json`. Rust drops Python messages silently in the other direction. See [mobile.md](mobile.md). |
| Phone app cannot connect | Use the bridge `app address` of the phone's Wi-Fi, not a virtual adapter (e.g. `192.168.56.1`). Same Wi-Fi, firewall allows the bridge port (47860). After 5 wrong tokens an IP is blocked for 10 minutes. |
| Expo QR missing | Metro started with `CI=1`. Run `npx expo start --host lan` in a normal terminal. If port 8081 is busy Expo uses 8082: check the port in the QR or `exp://` address. |
| Supabase join returns `410` | The room expired. Rooms last 24 hours from creation and cannot be extended; create a new room. Expiry cleanup is implemented but not deployed. |
| A room ID does not match | Enter all 8 `A-Z0-9` characters. Case, spaces, and dashes are ignored. **Retired:** peer ID as a room-join input. |
| Trust-code behavior differs from old documentation | The SAS commit design is decided but not implemented yet. Treat unverified behavior as unconfirmed. |

## Supabase deployment warning

The current relay is a many-participant room model; the peer-ID-based one-to-one relay is **retired**. Room RLS is publicly open and create/join requests lack signature verification. These are unresolved deployment blockers.

## Related

- [Usage](usage.md): commands, keys, names, language, network
- [Configuration](configuration.md): settings, control channel, timing constants
- [Mobile (Expo Go)](mobile.md): phone connection, bridge, decisions
