# Improvements

- Written: 2026-10-22, commit `a15c974` (`git log -1 --format=%h`)
- Shared dock module: `src/dock_layout.py` is a copy. `load_dock_layout()` (`src/talk.py:37`) uses the installed herdr-dock-grid original first and falls back to this copy. The copy matches the herdr-dock-grid 0.7.1 original (checked with diff).
- Task list: [todo.md](todo.md). Items already listed there are linked, not moved.
- Labels: "confirmed" = seen in the current code. "needs check" = inferred from the code and not run.

## Items

- **[Medium] The herdr CLI helper in `talk.py` has no timeout** (confirmed in code; hang not reproduced)
  - Evidence: `src/talk.py:552-557` (`herdr_cli()`, used in `open_chat()` at `:571-572`). `src/herdr_api.py:42` and `:140` have the same gap.
  - If herdr does not answer, `subprocess.run` waits forever. The call runs only when the chat pane is opened, so the impact is limited to that step. Fix the shared copy in herdr-dock-grid first, then sync it (shared-module rule).

- **[Low] Broad `except` picks the plain-terminal path for any error** (needs check)
  - Evidence: `src/talk.py:535` (`except Exception:  # no usable console for prompt_toolkit`)
  - Any error while building the prompt_toolkit app falls back to `run_ansi()` without a message, so a real bug looks like a missing console. Narrow the exception, or log the reason to stderr before falling back.

- **[Low] No CI** (confirmed)
  - Evidence: there is no `.github/` folder in the repository. The workflow in herdr-stockpulse (`.github/workflows/test.yml`) can be used as a base.

## Related items in todo.md

- Name length rules: [todo.md](todo.md), "Names"
- Peer address check, message rate limit, encryption: [todo.md](todo.md), "Network and security"
- `Chat.handle()` branch cleanup: [todo.md](todo.md), "Code cleanup"
