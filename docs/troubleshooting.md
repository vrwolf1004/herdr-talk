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

## Related

- [Usage](usage.md): commands, keys, names, language, network
- [Configuration](configuration.md): settings, control channel, timing constants
