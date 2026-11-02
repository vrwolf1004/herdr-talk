# TODO

> 2026-11-02: code moved from Python to Rust. Names in finished items refer to the Python code. See [rust-roadmap.md](rust-roadmap.md).

Improvement list for herdr-talk. Items are grouped by area, most important first inside each group.

## Bugs

- [x] **Idle timer ignores the plain-terminal path.** `run_ansi()` sends lines to `add_message()`, which does not update `chat.last_activity`. Only `Chat.add_line()` does. So in that path, received messages and your own messages do not reset the 10-minute idle timer, and the pane can leave while people are chatting. Fix: update `last_activity` in `add_message()` too, or move the update into one shared place. (`Chat.touch()`를 모든 화면이 쓴다. `IdleTimerTest`)
- [x] **`/help` box is missing in the `prompt_toolkit` view.** The box is drawn only by `run_ansi()`. With `prompt_toolkit` installed, `/help` prints the one-line summary instead. (모든 화면이 같은 상자를 쓴다. `ChatHandleTest.test_help_prints_the_box`)

## Names

- [x] **Avoid duplicate names on the network.** Two PCs can pick the same name by chance (1 in 10 within one language list). When picking a name at start or with `/lang`, skip names already used by a fresh peer. If every name is taken, fall back to the normal random pick. (`/lang`에서 최근 피어가 쓰는 이름은 피함. 시작 시 이름은 아직 적용하지 않음)
- [x] **Name length rules differ.** `/name` cuts at 32 characters, and the random names are short. Check that a long name still fits the status line and the peer list. (`/name`도 32자 자르기 후 제어 문자를 공백으로 바꿈, 같은 규칙. 상태 줄은 `fit()`, 목록은 `wrap_line()`으로 잘림. 실제 콘솔 확인은 안 함. `name_is_cut_to_32_and_controls_removed`)

## Language and text

- [x] **Status line in the `prompt_toolkit` view** (모든 언어의 상태 줄에 Ctrl+N, Ctrl+C, /help가 있는지 `StatusLineTest`로 확인) still shows the `Ctrl+N` hint in every language. Check that each translation keeps the key names correct.
- [x] **Unify the one-line `/help` text** (`help.oneline`) with the box text, so the two cannot drift apart. (한 줄 도움말을 없애고 상자 하나로 통일. `help.oneline` 키 삭제)
- [ ] **Check translated help box width** in a real console for Korean and Japanese. The width is calculated by display width, but this has only been checked in a simulated run.
- [x] **Add language-key checks as a test** (see the test item below), so a missing key in one locale fails before commit. (`LocaleKeysTest`. 이 검사로 `open.no_pane` 누락 7개 언어를 찾아 고쳤다)

## Code cleanup

- [x] **Remove `run_plain()`.** It is not called anywhere. `run_screen()` falls back to `run_ansi()`, not to `run_plain()`, and the docstring still says it falls back to `run_plain()`. (삭제)
- [x] **Remove the `__pycache__` folder from the working copy.** `.gitignore` already excludes it, so it does not reach git. (`.gitignore`로 이미 git에서 제외됨)
- [x] **Keep the `Chat.handle()` branches in one place.** Commands are split across `handle()`, `run_ansi()` (the `/help` check), and `run_screen()` (`Ctrl+N`). Move shared behavior into one function. (Rust: 명령 에코를 `Chat::handle()`로 옮김. `Ctrl+N`은 입력창만 채우는 화면 동작이라 화면에 남김. `commands_are_echoed_before_they_run`)

## Tests

- [x] **Add a test file** (`tests/test_talk.py`, standard library `unittest`) covering: (`tests/test_talk.py`: wrap, 폭, 이름, `/help`·`/name`·`/quit`·`/lang`, idle, 수신 검증, locale 키)
  - `wrap_line()`: word breaks, long words cut by character, wide characters
  - `char_width()`: Korean and Japanese count as two columns
  - `default_name()`: picks from the locale's list, and falls back to all lists
  - `Chat.handle()`: `/name`, `/lang`, `/peers`, `/help`, `/quit`
  - i18n: every locale file has the same keys as `en.json`
- [x] **Run the chat headlessly in CI-like checks.** The current check patches `read_char()` and captures stdout. Turn that into a repeatable script. (Rust: `cargo test --workspace` 이 화면 없이 `Chat::handle()`과 수신 경로를 돌린다. CI 워크플로는 아직 없음)

## Network and security

- [x] **수신 메시지 스키마와 표시 문자열 검증** (`receive_one`: 타입·길이 확인, 제어 문자 치환, 빈 본문 버림. `ReceiveTest`)
  - TCP JSON에서 필수 필드, 값 형식, 길이 및 제어 문자를 확인하고, 잘못된 메시지는 버린다. 임의의 `from`/`text`로 인한 잘못된 표시를 재현하는 테스트를 추가한다.
- [x] **네트워크 스레드와 UI 상태 수명 관리** (`STOP` 이벤트로 수신·안내 루프를 끝내고 소켓을 닫음. 메시지 목록은 잠금으로 보호하고 화면은 복사본으로 그림. `ShutdownTest`, `SharedLinesTest`)
  - 처리 1: `STOP` 이벤트로 announce·수신 루프를 끝내고 소켓을 닫는다 (`run()`이 끝날 때 설정)
  - 처리 2: 메시지 목록(`Chat.lines`)에 잠금을 두고, 화면은 복사본으로 그린다
  - 테스트: 종료 신호가 스레드를 제시간에 끝내는지, 동시 추가 중에도 목록이 깨지지 않는지
  - 수신 루프는 종료 신호 없이 계속 돌고 소켓을 닫지 않는다. 공유 메시지 목록은 네트워크 스레드와 UI가 함께 변경한다. 종료 이벤트·소켓 정리와 잠금 또는 단일 이벤트 큐를 설계하고 종료/동시성 테스트를 추가한다.

- [x] **Encryption (phase 2).** Messages are plain JSON over TCP. Phase 2 plans end-to-end encryption; see [rust-roadmap.md](rust-roadmap.md). Phase 1 must stay on trusted networks only. (Noise IK, 저장된 키만 신뢰, `/trust <코드>`로 지문 확인, 미확인·키 변경 상대는 거부, 평문은 `"plaintext": true` 일 때만. 두 PC 실제 채팅은 미확인. `docs/configuration.md#encryption`)
- [x] **Limit the message rate.** A peer can send many messages quickly and fill the list. Add a simple per-peer limit. (주소별 10초에 20개, 넘치면 알림 없이 버림. `MESSAGE_LIMIT`, `admit_sender()`)
- [x] **Check the peer address.** Messages are accepted from any address that sends the right `app` field. Consider checking that the sender matches a peer seen over UDP. (TCP 연결의 출발 주소가 UDP로 최근 발견된 피어 주소일 때만 받음. 시작 직후 공지가 아직 안 온 피어의 첫 메시지는 버려짐 — `shortcut:` 주석 참고)
- [x] **Stop the JSON listener from accepting very slow clients.** `receive_one()` reads without a timeout, so a stuck connection holds a thread. (`RECEIVE_TIMEOUT_SECONDS` 5초 제한)

## Docs and release

- [ ] **First Rust release.** Push tag `v0.7.0`, then check the release workflow, both install scripts, and `herdr plugin install` on Windows, Linux and macOS. Until then the GitHub install fails.
- [ ] **Check the Rust version on real setups.** Linux and macOS build and run, a chat between two PCs, the `open` action, and `herdr plugin link`. (이 PC(Windows): `open` 은 가짜 herdr CLI로 분할·실행 호출 순서를 확인, `plugin link --disabled` 로 액션 3개 등록 확인 후 unlink. 실제 herdr 세션에서 실행, 두 PC 채팅, Linux/macOS 는 미확인)
- [x] **Keep `release.yml` actions updated.** `upload-rust-binary-action` is pinned to a commit SHA; add Dependabot. (`.github/dependabot.yml` 추가, github-actions 주간. 푸시 후 적용)
- [x] **Build from source at install.** When `cargo` is found, let the installers offer a source build instead of the release binary, for people who avoid downloaded executables. (`HERDR_TALK_BUILD_FROM_SOURCE=1` 이면 `rust/herdr/install.sh`, `install.ps1` 가 소스 빌드. `cargo` 가 없으면 중단. 이 PC에서 두 스크립트 모두 확인)
- [ ] **Fail closed once signing is on.** When the public key is set in the installers, stop the install if `minisign` is missing, unless the user opts in (`HERDR_TALK_ALLOW_UNSIGNED=1` or a source build). In `release.yml`, fail the `sign` job when the key is missing instead of publishing unsigned. Do this in the same change that adds the key. (2026-11-02 보안 리뷰 지적, 키가 없는 지금은 동작 차이 없음)
- [ ] **Code-sign the binaries.** Windows and macOS. Needs certificates, which the owner must buy. (코드 서명은 이번 범위에서 제외)
- [x] **Python edition.** Offer the Python plugin for people who avoid executables, installed from the `python/` folder of this repo. It stays plain text on the LAN, so Rust users must turn on plain-text peers to talk with it. (`python/` 플러그인 id `herdr-talk-python`, 도크 키 분리. 테스트 32개 통과, 두 판 동시 등록 확인. GitHub 설치는 푸시 후 확인)
- [ ] **Sign the release checksum.** (2026-11-02: 0.7.0 은 서명 없이 낸다. 사용자가 위험을 알고 승인. 키가 없으면 sign 단계를 건너뛰고, 설치 스크립트는 SHA-256 만 확인) The installers download the checksum from the same release, so it does not catch a tampered release. Use sigstore or minisign and verify it in the installers. (minisign 으로 구현: 설치 스크립트가 서명을 먼저 검증. 이 PC에서 정상/변조/다른 키/키 없음 경우를 sh, ps1 모두 확인. 남은 일: 소유자가 `minisign -G` 로 키를 만들어 공개키를 `rust/herdr/install.sh`, `install.ps1` 에 넣고, 비밀키를 GitHub 비밀값 `MINISIGN_SECRET_KEY`, `MINISIGN_PASSWORD` 에 넣기)

- [x] **Restore the Documentation section in both READMEs.** The `docs/` folder now exists (`usage.md`, `configuration.md`, `development.md`), so links to them can be added back.
- [x] **Write `docs/usage.ko.md`.** The main README has a Korean version, but the detailed docs are only in English.
- [x] **Describe the language setting in the READMEs,** like herdr-timekit does, with a short Language section. (README.md, README.ko.md에 Language 절 추가)
- [x] **Commit the `docs/` folder and this list** after review.
- [ ] **Check the herdr plugin install from GitHub** (`herdr plugin install vrwolf1004/herdr-talk/rust`) after the next push, since the manifest and `rust/` files must all be in the repo.

## Security follow-ups (2026-11-02 review)

- [ ] **Check long plaintext messages on screen.** Every row of a wrapped plaintext message must carry the `name (plain)` label, so a continuation row cannot pass as another sender. Test with a long message from the Python edition.
- [ ] **Choose the trusted name at `/trust`.** The saved name now comes from the peer. Show it in the fingerprint prompt and let the user confirm or set a local name, e.g. `/trust <code> <name>`, and refuse a name another trusted peer already has. Changes the command, so decide before the two-PC test.
- [ ] **Old name stays for the own key.** Another pane on the same PC keeps its old name in the list after `/name`, because entries with the own key are not overwritten.
- [ ] **Second network card is not local.** On a PC with several network cards, announcements of the own key from the second card's address are not accepted as local.
- [ ] **Verified peer on a new address.** A new UDP entry is made and the old one expires after 10 seconds. Updating the address over an authenticated connection is not done.

## Later (phase 2 and beyond)

- [ ] Chat across the internet through Supabase, with end-to-end encryption.
- [ ] Message history for peers that were offline (needs a decision on storage first).
- [ ] **Android app that sends commands to herdr AI agents.** Pick a target agent on the phone, send it a prompt, and see its state and reply. Needs the relay and encryption first, plus pairing and permission per device: this runs commands on the PC remotely, so an unpaired device must never reach an agent.
- [ ] Optional sound or highlight for mentions of your name.
- [ ] **Continuous local development with Slack.** When a local task finishes, a Claude Code Stop hook posts a completion message to Slack (`chat.postMessage`, Bot Token from `SLACK_BOT_TOKEN`, channel from `SLACK_CHANNEL_ID`). The hook then polls the thread with `conversations.replies` (standard library only, no public endpoint). A reply is returned to Claude as `{"decision":"block","reason":"<reply>"}`, so the same session continues with the next step. A reply such as `종료` stops the loop. Open decisions: project scope vs. global `settings.json`, and the hook timeout. Never store the token in the repository or in chat.
