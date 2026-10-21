# TODO

Improvement list for herdr-talk. Items are grouped by area, most important first inside each group.

## Bugs

- [ ] **Idle timer ignores the plain-terminal path.** `run_ansi()` sends lines to `add_message()`, which does not update `chat.last_activity`. Only `Chat.add_line()` does. So in that path, received messages and your own messages do not reset the 10-minute idle timer, and the pane can leave while people are chatting. Fix: update `last_activity` in `add_message()` too, or move the update into one shared place.
- [ ] **`/help` box is missing in the `prompt_toolkit` view.** The box is drawn only by `run_ansi()`. With `prompt_toolkit` installed, `/help` prints the one-line summary instead.

## Names

- [ ] **Avoid duplicate names on the network.** Two PCs can pick the same name by chance (1 in 10 within one language list). When picking a name at start or with `/lang`, skip names already used by a fresh peer. If every name is taken, fall back to the normal random pick.
- [ ] **Name length rules differ.** `/name` cuts at 32 characters, and the random names are short. Check that a long name still fits the status line and the peer list.

## Language and text

- [ ] **Status line in the `prompt_toolkit` view** still shows the `Ctrl+N` hint in every language. Check that each translation keeps the key names correct.
- [ ] **Unify the one-line `/help` text** (`help.oneline`) with the box text, so the two cannot drift apart.
- [ ] **Check translated help box width** in a real console for Korean and Japanese. The width is calculated by display width, but this has only been checked in a simulated run.
- [ ] **Add language-key checks as a test** (see the test item below), so a missing key in one locale fails before commit.

## Code cleanup

- [ ] **Remove `run_plain()`.** It is not called anywhere. `run_screen()` falls back to `run_ansi()`, not to `run_plain()`, and the docstring still says it falls back to `run_plain()`.
- [ ] **Remove the `__pycache__` folder from the working copy.** `.gitignore` already excludes it, so it does not reach git.
- [ ] **Keep the `Chat.handle()` branches in one place.** Commands are split across `handle()`, `run_ansi()` (the `/help` check), and `run_screen()` (`Ctrl+N`). Move shared behavior into one function.

## Tests

- [ ] **Add a test file** (`tests/test_talk.py`, standard library `unittest`) covering:
  - `wrap_line()`: word breaks, long words cut by character, wide characters
  - `char_width()`: Korean and Japanese count as two columns
  - `default_name()`: picks from the locale's list, and falls back to all lists
  - `Chat.handle()`: `/name`, `/lang`, `/peers`, `/help`, `/quit`
  - i18n: every locale file has the same keys as `en.json`
- [ ] **Run the chat headlessly in CI-like checks.** The current check patches `read_char()` and captures stdout. Turn that into a repeatable script.

## Network and security

- [ ] **수신 메시지 스키마와 표시 문자열 검증**
  - TCP JSON에서 필수 필드, 값 형식, 길이 및 제어 문자를 확인하고, 잘못된 메시지는 버린다. 임의의 `from`/`text`로 인한 잘못된 표시를 재현하는 테스트를 추가한다.
- [ ] **네트워크 스레드와 UI 상태 수명 관리**
  - 수신 루프는 종료 신호 없이 계속 돌고 소켓을 닫지 않는다. 공유 메시지 목록은 네트워크 스레드와 UI가 함께 변경한다. 종료 이벤트·소켓 정리와 잠금 또는 단일 이벤트 큐를 설계하고 종료/동시성 테스트를 추가한다.

- [ ] **Encryption (phase 2).** Messages are plain JSON over TCP. Phase 2 plans end-to-end encryption through Supabase. Phase 1 must stay on trusted networks only.
- [ ] **Limit the message rate.** A peer can send many messages quickly and fill the list. Add a simple per-peer limit.
- [ ] **Check the peer address.** Messages are accepted from any address that sends the right `app` field. Consider checking that the sender matches a peer seen over UDP.
- [ ] **Stop the JSON listener from accepting very slow clients.** `receive_one()` reads without a timeout, so a stuck connection holds a thread.

## Docs and release

- [x] **Restore the Documentation section in both READMEs.** The `docs/` folder now exists (`usage.md`, `configuration.md`, `development.md`), so links to them can be added back.
- [x] **Write `docs/usage.ko.md`.** The main README has a Korean version, but the detailed docs are only in English.
- [ ] **Describe the language setting in the READMEs,** like herdr-timekit does, with a short Language section.
- [x] **Commit the `docs/` folder and this list** after review.
- [ ] **Check the herdr plugin install from GitHub** (`herdr plugin install vrwolf1004/herdr-talk`) after the next push, since the manifest and `src/` files must all be in the repo.

## Later (phase 2 and beyond)

- [ ] Chat across the internet through Supabase, with end-to-end encryption.
- [ ] Message history for peers that were offline (needs a decision on storage first).
- [ ] Optional sound or highlight for mentions of your name.
