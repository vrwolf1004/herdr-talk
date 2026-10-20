# Troubleshooting

Check the symptom in the table first. Each check points to the setting or step to look at.

| Symptom | Check |
|---------|-------|
| `no peers yet` | Both PCs on the same network? Firewall allows UDP `47850` and the TCP port? |
| `! could not reach <name>` | The peer closed its pane or the firewall blocks its TCP port. Peers that have not been seen for 10 seconds drop from the list. |
| Language is not what you expect | Set `HERDR_TALK_LANG=ko` (or another code) to force it. |
| Chat leaves after 10 minutes in the plain view | Known issue: the idle timer is not reset by activity in the plain view (see [TODO](todo.md)). |

## Related

- [Usage](usage.md): commands, keys, names, language, network
- [Configuration](configuration.md): settings and timing constants
