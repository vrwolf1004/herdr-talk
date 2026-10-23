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

## 개선 제안 (2026-10-23)

### 1. 메시지 전송이 피어마다 차례로 실행됨

- **현상**: 메시지는 피어 목록을 하나씩 돌며 `send_to` 로 보낸다(`src/talk.py:176-180`). 각 연결은 제한 시간이 3초다(`src/talk.py:285-288`). 응답하지 않는 주소가 있으면 그 피어마다 최대 3초씩 입력 흐름이 멈춘다. 연결이 거부되면 바로 끝나므로, 느려지는 경우는 응답 없이 막힌 주소일 때다.
- **개선안**: 피어마다 짧은 스레드로 보내거나 전송 큐에 넣어, 입력 루프가 전송을 기다리지 않게 한다. 실패 알림(`peer.unreachable`)은 그대로 둔다.
- **범위**: `src/talk.py` (메시지 전송 부분, `send_to`)
- **확인**: 응답하지 않는 주소를 피어로 넣고 전송 뒤 입력이 바로 돌아오는지 본다. 실행하지 않았다(확인 필요).
→ 완료: 피어에게 동시에 보내고 (상한 8), 한 피어의 실패가 다른 피어를 막지 않는다.

### 2. 입장 알림의 이름은 정리하지 않음

- **현상**: 수신 메시지의 보낸 사람 이름은 제어 문자를 지우고 32자로 자른다(`src/talk.py:278`). 반면 UDP 로 받은 입장 알림은 이름을 그대로 출력하고(`src/talk.py:236-237`), 저장하는 피어 이름에도 길이 제한이 없다(`:229-235`).
- **개선안**: 피어 이름을 저장할 때 `clean_text(name, 32)` 를 적용해 두 경로가 같은 규칙을 쓰게 한다.
- **범위**: `src/talk.py` (`_listen_loop`)
- **확인**: 긴 이름과 제어 문자를 넣은 UDP 알림을 보내 입장 줄과 `/peers` 출력이 정리되는지 본다. 이름 길이 규칙 자체는 [todo.md](todo.md) 의 "Name length rules" 항목과 관련이 있다.
→ 완료: 입장 알림의 이름도 같은 규칙 (제어 문자 제거, 32자) 으로 정리하고, 빈 이름은 unknown 으로 쓴다.
