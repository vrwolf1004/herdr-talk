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
- [ ] **Check translated help box width** in a real console for Korean and Japanese. The width is calculated by display width, but this has only been checked in a simulated run. (2026-11-03 건너뜀: 한국어·일본어 실제 콘솔을 눈으로 봐야 하는데 이 세션에서는 화면을 볼 수 없음)
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

- [x] **Encryption (phase 2).** Messages are plain JSON over TCP. Phase 2 plans end-to-end encryption; see [rust-roadmap.md](rust-roadmap.md). Phase 1 must stay on trusted networks only. (Noise IK, 저장된 키만 신뢰, `/trust <코드>`로 지문 확인, 미확인·키 변경 상대는 거부, 평문은 `"plaintext": true` 일 때만. 두 PC 실제 채팅은 미확인. `docs/configuration.md#encryption`)
- [x] **Limit the message rate.** A peer can send many messages quickly and fill the list. Add a simple per-peer limit. (주소별 10초에 20개, 넘치면 알림 없이 버림. `MESSAGE_LIMIT`, `admit_sender()`)
- [x] **Check the peer address.** Messages are accepted from any address that sends the right `app` field. Consider checking that the sender matches a peer seen over UDP. (TCP 연결의 출발 주소가 UDP로 최근 발견된 피어 주소일 때만 받음. 시작 직후 공지가 아직 안 온 피어의 첫 메시지는 버려짐 — `shortcut:` 주석 참고)
- [x] **Stop the JSON listener from accepting very slow clients.** `receive_one()` reads without a timeout, so a stuck connection holds a thread. (`RECEIVE_TIMEOUT_SECONDS` 5초 제한)

## Docs and release

- [x] **First Rust release.** (2026-11-03: 태그 `v0.7.0` 푸시, release run 41561342443 전부 success, 릴리스 공개 발행(draft 아님), 체크섬 미서명(sign 생략), Windows `herdr plugin install` 확인. Linux/macOS 설치는 미확인)
- [ ] **Check the Rust version on real setups.** Linux and macOS build and run, a chat between two PCs, the `open` action, and `herdr plugin link`. (이 PC(Windows): `open` 은 가짜 herdr CLI로 분할·실행 호출 순서를 확인, `plugin link --disabled` 로 액션 3개 등록 확인 후 unlink. 실제 herdr 세션에서 실행, 두 PC 채팅, Linux/macOS 는 미확인) (2026-11-03 건너뜀: Linux/macOS·두 PC·실제 herdr 세션이 필요)
- [x] **Keep `release.yml` actions updated.** `upload-rust-binary-action` is pinned to a commit SHA; add Dependabot. (`.github/dependabot.yml` 추가, github-actions 주간. 푸시 후 적용)
- [x] **Build from source at install.** When `cargo` is found, let the installers offer a source build instead of the release binary, for people who avoid downloaded executables. (`HERDR_TALK_BUILD_FROM_SOURCE=1` 이면 `rust/herdr/install.sh`, `install.ps1` 가 소스 빌드. `cargo` 가 없으면 중단. 이 PC에서 두 스크립트 모두 확인)
- [ ] **Fail closed once signing is on.** When the public key is set in the installers, stop the install if `minisign` is missing, unless the user opts in (`HERDR_TALK_ALLOW_UNSIGNED=1` or a source build). In `release.yml`, fail the `sign` job when the key is missing instead of publishing unsigned. Do this in the same change that adds the key. (2026-11-02 보안 리뷰 지적, 키가 없는 지금은 동작 차이 없음) (2026-11-03 건너뜀: 공개키를 넣는 같은 변경에서 해야 하고, 키는 소유자가 만들어야 함)
- [ ] **Code-sign the binaries.** Windows and macOS. Needs certificates, which the owner must buy. (코드 서명은 이번 범위에서 제외)
- [x] **Python edition.** Offer the Python plugin for people who avoid executables, installed from the `python/` folder of this repo. It stays plain text on the LAN, so Rust users must turn on plain-text peers to talk with it. (`python/` 플러그인 id `herdr-talk-python`, 도크 키 분리. 테스트 32개 통과, 두 판 동시 등록 확인. GitHub 설치는 푸시 후 확인)
- [ ] **Sign the release checksum.** (2026-11-02: 0.7.0 은 서명 없이 낸다. 사용자가 위험을 알고 승인. 키가 없으면 sign 단계를 건너뛰고, 설치 스크립트는 SHA-256 만 확인) The installers download the checksum from the same release, so it does not catch a tampered release. Use sigstore or minisign and verify it in the installers. (minisign 으로 구현: 설치 스크립트가 서명을 먼저 검증. 이 PC에서 정상/변조/다른 키/키 없음 경우를 sh, ps1 모두 확인. 남은 일: 소유자가 `minisign -G` 로 키를 만들어 공개키를 `rust/herdr/install.sh`, `install.ps1` 에 넣고, 비밀키를 GitHub 비밀값 `MINISIGN_SECRET_KEY`, `MINISIGN_PASSWORD` 에 넣기)

- [x] **Restore the Documentation section in both READMEs.** The `docs/` folder now exists (`usage.md`, `configuration.md`, `development.md`), so links to them can be added back.
- [x] **Write `docs/usage.ko.md`.** The main README has a Korean version, but the detailed docs are only in English.
- [x] **Describe the language setting in the READMEs,** like herdr-timekit does, with a short Language section. (README.md, README.ko.md에 Language 절 추가)
- [x] **Commit the `docs/` folder and this list** after review.

## Security follow-ups (2026-11-02 review)

- [x] **Check long plaintext messages on screen.** Every row of a wrapped plaintext message must carry the `name (plain)` label, so a continuation row cannot pass as another sender. Test with a long message from the Python edition. (2026-11-03: 원인은 줄바꿈 후 이어지는 화면 행에 보낸 사람 표시가 없던 것. `screen.rs` `rows_of()` 가 받은 메시지의 모든 화면 행에 `보낸 사람: ` 을 붙임(평문 외 암호화 상대 메시지도 같음, 내가 쓴 줄과 알림은 그대로). 보낸 사람이 너무 길어 본문 자리가 8칸 미만이면 예전처럼 한 번만. 단위 테스트 `wrapped_rows_of_a_received_message_all_carry_the_sender`. Python 판으로 실제 전송해 화면으로 보는 것은 안 함)
- [ ] **Choose the trusted name at `/trust`.** (2026-11-09 사용자 결정: B. 명령 형식은 그대로 두고, 지문 확인 화면에 상대 이름·IP·키 지문 앞부분을 함께 표시. 이름 입력 방식(C)은 보류) The saved name now comes from the peer. Show it in the fingerprint prompt and let the user confirm or set a local name, e.g. `/trust <code> <name>`, and refuse a name another trusted peer already has. Changes the command, so decide before the two-PC test. (2026-11-03 건너뜀: 명령 형식이 바뀌는 chef 결정 사항)
- [x] **Old name stays for the own key.** Another pane on the same PC keeps its old name in the list after `/name`, because entries with the own key are not overwritten. (2026-11-03: 같은 PC의 다른 창이 보낸 공지(내 키, 로컬 주소)면 같은 주소의 기존 항목 이름도 갱신. `another_pane_on_this_pc_can_rename_itself`. 실제 창 두 개로는 확인 안 함)
- [ ] **Second network card is not local.** On a PC with several network cards, announcements of the own key from the second card's address are not accepted as local. (2026-11-03 건너뜀: 모든 네트워크 카드 주소를 알아내려면 의존성 추가가 필요해 결정 필요, 그리고 카드 2개인 PC가 필요)
- [ ] **Verified peer on a new address.** A new UDP entry is made and the old one expires after 10 seconds. Updating the address over an authenticated connection is not done. (2026-11-03 건너뜀: 인증된 연결로 주소를 바꾸는 프로토콜 설계가 필요한 chef 결정 사항)

## Phone and mobile bridge (2026-11-06)

Phase 1 (connection) and phase 2 (host, phone URL and token) are implemented and checked on one PC. Development ended 2026-11-06; final check: `cargo test` 95 (talk-core 81, herdr-talk 14), Python 113, GUI release build. See [mobile.md](mobile.md).

Done:

- [x] **Same-PC chats could not see each other** (`serve` + `run` with the same state folder). Cause: `is_local_address` only knew the default-route address; this PC has two adapters. Fixed in `net.rs`, tests added. (원인 확정과 수정은 dev. 이 PC 에서 `serve` 두 개와 브리지로 확인)
- [x] **False "key changed" warning for several identities on one PC.** A trusted key that still announces from the same IP is treated as another identity. A new key is never trusted before `/trust`.
- [x] **Python control channel** (`control-<pid>.json`, `lines`/`peers`/`send`/`shutdown`, `serve` subcommand) so the bridge can attach to a Python node.
- [x] **Python encryption** (Noise_IK_25519_ChaChaPoly_BLAKE2s in pure Python, `python/src/noise.py`, checked against RFC 7748, RFC 8439 and Cacophony vectors). No `plaintext` setting needed any more. Protocol notes in [encryption-protocol.md](encryption-protocol.md). Known limit: not constant time, so a local process that can time it could learn the key; swap `x25519`/`aead` for `cryptography` if that matters.
- [x] **Rust and Python talk to each other** on one PC (both directions, shared state folder, no `/trust` needed because the key is the same). dev also checked two separate state folders with matching codes.
- [x] **Host rule.** The node that gets `phone.lock` at start (no other node on this PC) is the host. It owns `phone.json` (port, 6-digit token, URLs). Rust and Python use the same lock. On exit the next node takes over and keeps the token. The last node removes `phone.json`.
- [x] **Host name and badge.** The first node is named "방장" (`Host`, `Gastgeber`, ... per language) unless `--name` is given. A `[방장]` badge marks the host in the Rust TUI status line, the Python status line and the GUI top bar. A takeover never renames a node.
- [x] **Phone address and token on screen.** A host sees them when it becomes host; any node on this PC sees them when the host joins; `Ctrl+P` or `/phone` shows them on request; `/phone new` re-issues the token. Drawn as an overlay only, never in chat lines, logs or peer messages. Checked with the Rust TUI chat pane.
- [x] **Bridge reads `phone.json`** (port, token), falls back to a random token printed on the console, keeps the lockout (5 failures per IP, 10 minutes).
- [x] **Phone to PC and PC to phone messages** through the bridge. (폰 화면 표시는 사용자 확인)
- [x] **Rust GUI** gets the same `Ctrl+P`, `/phone` and badge. It starts its own hidden `serve`; with no other node that `serve` becomes the host (checked by `phone.json` owner).

Open:

- [ ] **Look at the real screens.** Badge shape and position (TUI, Python, GUI), `Ctrl+P` and any-key hide, the 30-second hide, GUI auto display. Only unit tests and pane text reads so far.
- [ ] **GUI exit when its node goes away.** A GUI that was attached to a Python host disappeared right after that node was stopped (no crash event found). (2026-11-09: 재현 안 됨, 6회 — 순서 A 4회, 순서 B 2회. GUI 는 외부 Python 방장에 붙지 않고 자기 자식 serve 에만 붙음. 원 빌드·state 폴더 조건 미확인. `gui-panic.log` 로 다음 발생 때 원인 확인. 진단 로그는 추가됨)
- [ ] **Phone token after takeover (later).** When the host goes away and the remaining node is the only one on this PC, a new token is issued (`phone.rs` / `phone.py` `try_take`: `other_live_nodes` does not count the node itself). Differs from "takeover keeps token". Deferred 2026-11-09: phone work is not possible on the current network. Decide when phone work resumes.
- [ ] **Panel position.** The panel is an overlay at the bottom, not a line directly under `방장 입장`, so it never lands in the chat history. Ask whether a position directly under the line is needed.
- [ ] **Bridge start.** The host does not start the bridge; the user runs `node mobile-bridge/bridge.js`. Decide whether the host should start it (needs a fixed path and process handling).
- [ ] **Chat across two PCs** (not tested). A different state folder is treated as another PC.
- [ ] **Phone leg encryption**: the bridge is plain HTTP on the LAN. Options: HTTPS with a pinned certificate, or a tunnel.
- [ ] **Phone as a real P2P peer.** Not possible with Expo Go (UDP/TCP sockets need native modules, so a development build). Would need a JS Noise implementation, Android multicast permission and background limits. Separate project.
- [ ] **One intermittent Python test failure** (once in 37 runs, not reproduced). Watch for it.
- [ ] **Stale `control-<pid>.json` files** pile up after killed nodes. Harmless; maybe clean them at start.
- [ ] **Commit the work.** `net.rs`, `chat.rs`, `phone.rs`, `screen.rs`, `gui/src/main.rs`, `talk.py`, `noise.py`, `phone.py`, `store.py`, locales, tests, `bridge.js`, docs.

## Review (2026-11-09)

Found by reading the relay, node-folder and CLI code after commit `222aee8`. Not run; each item says what was checked.

- [ ] (구현 완료 2026-11-11, dev 회신 확인, 미커밋·미배포) **Crashed node leaves its server row behind.** `remove_dead_sessions` (Rust `store.rs`, Python `sessions.py`) deletes a dead node's folder, and that folder holds the key. The row is never sent to `delete-peer`, and the key is gone, so nobody can remove it. Fix: before the folder is removed, sign and send the delete with that folder's key (best effort, same as `leave`). (코드 읽기로 확인. 실제 충돌 종료로는 재현 안 함)
- [ ] (구현 완료 2026-11-11, dev 회신 확인, 미커밋·미배포) **Rust does not delete the relay row when the run ends with an error.** `rust/crates/herdr-talk/src/main.rs` calls `relay_chat::leave` only when `result.is_ok()`, but it still removes the folder (and the key) afterwards. The row stays on the server. Python does the leave in `finally`, so it is not affected. Fix: call `leave` whenever a session exists. (코드 읽기로 확인)
- [ ] (구현 완료 2026-11-11, dev 회신 확인, 미커밋·미배포) **`delete-peer` records the nonce before the delete.** `supabase/functions/delete-peer/index.ts`: `takeNonce` runs before `deletePeer`. If the DELETE fails (500), the nonce stays used, so the same signed body gets 401 on retry. The client signs a new body per try, so the impact is small. Fix: delete the nonce on failure, or take it after the delete. (코드 읽기로 확인)
- [ ] **A reused pid keeps a dead node's folder.** `_has_live_node` / `has_live_node` treat any live pid in a `control-<pid>.json` as the node. After a crash the OS can give that pid to another program, and the folder is kept until that program exits. Fix: store the process start time in the control file and compare it. Low priority. (코드 읽기로 확인)
- [ ] **`herdr_cli()` still has no timeout** (`python/src/talk.py:1366`). `subprocess.run` waits forever if herdr does not answer. The item in `improvements.md` (2026-10-22) is still open. Fix the shared copy in herdr-dock-grid first, then sync (shared-module rule).

## Supabase 방 기능 (2026-11-13)

- [ ] **방 보안 (배포 전 필수)**: `0003_rooms.sql`의 RLS가 anon에게 `rooms`·`room_members`·`room_messages` 전체 조회를 열어 둡니다. `join-room`과 `create-room`에는 서명 검증과 재전송 방지가 없습니다. 메시지는 종단 암호화 전제이므로 서버에는 암호문만 저장하고, 조회는 멤버 검증 후 허용합니다. 구현 전까지 main의 방 모델 커밋(`8cbc628`)을 배포하지 않습니다. 진행 (2026-11-13): `0006_rooms_read_closed.sql` 작성 (실행 안 함). 전제: `create-room`·`join-room` 을 service role 키로 바꾼 뒤에만 실행 (승인 항목 f). service role 전환 완료 (미배포). 서명 검증 1단계 완료 (미커밋·미배포): `create-room`·`join-room` 이 Ed25519 서명, ts, nonce 를 검사하고 nonce 는 `put_nonces` 에 기록. 남은 것: talk-core `join_room_request` 와 gui `create_room` 이 아직 서명 없이 보낸다 (허락 필요), 배포 전 실제 Edge Function 실행 확인.
- [ ] **방 사용기한 24시간 (구현됨, 미커밋·미배포)**: 생성 시점부터 24시간, 연장 없음 가정. `0005_room_expiry.sql`(실행 안 함), `create-room`은 `expires_at` 저장, `join-room`은 만료된 방에 410 응답. 만료 방 정리는 pg_cron (0005 주석). 남은 일: talk-core `relay_rest.rs:295`의 410 처리 한 줄 (공통 모듈, 허락 필요).
- [ ] **8자리 코드 통일 (trust·상대 ID·방 ID, A-Z0-9)**: 구현됨, 미커밋. `0004_uppercase_codes.sql` 작성 (실행 안 함). 라이브 DB 소문자 행 정리와 실행은 사람이 한다. 입력 소문자는 입력 지점에서 대문자로 변환.
- [ ] **trust 코드 방식 (SAS 커밋)**: 결정 (2026-11-13): 짧은 8자 코드 + 커밋 방식. 설계는 dev가 작성 (설계만, `docs/encryption-protocol.md` 새 절은 미작성). 결정 (2026-11-13, 권장안 채택): (a) LAN 에도 SAS 적용. (b) 구버전은 거부 (명확한 오류). (c) 기존 신뢰 목록은 재검증하지 않는다. (d) 코드 8자 (SAS 와 함께). (e) 릴레이 commit 은 채널 envelope 으로 전달. (f) rooms 쓰기는 service role 로 전환 완료. (g) talk-core 허락 범위 확대: 미결정 (LAN 의 net.rs, chat.rs 는 talk-core 라서 구현 전 확인 필요). 8자 지문을 SAS 없이 단독으로 쓰면 안 된다 (키 조작으로 같은 코드를 만들 수 있다).
- [ ] **LAN SAS 테스트 (테스트 전)** (2026-11-13): 로컬/자체 모드로 GUI 두 창 (같은 PC) 을 띄워 상대 목록에 서로 보이는지, 지문 코드가 reveal 뒤에만 나오는지, 양쪽 코드가 같은지, `/trust` 로 신뢰되는지 확인. 주의: Supabase 모드 노드는 LAN 으로 상대를 찾지 않는다 (이번 테스트 실패의 원인은 모드였다). Python 노드는 Rust 맞춤 전이라 Rust 와 연결되지 않는다. 실제 두 PC 확인은 별도.
- [ ] **SAS 보안 검토 지적 반영 (다른 PC 에서 진행)** (2026-11-13, 자동 보안 검토): `python/src/talk.py` 의 SAS 구현. (1) nonce 를 세션 전체에서 하나만 쓴다 → 상대마다, 페어링 시도마다 새로 만든다. (2) `chat.commits` 를 덮어쓴다 → 처음 받은 커밋을 고정 (`setdefault`), 같은 키의 다른 커밋은 연결 거부와 경고. (3) reveal 이 실패하면 nonce 를 버리고 다음 시도에 새로 만든다. (4) 커밋을 먼저 보내고, 상대 커밋을 받은 뒤에만 reveal (ZRTP 식 순서) 이 지켜지는지 확인. (5) Rust `talk-core` (`net.rs`, `chat.rs`) 도 같은 규칙인지 확인하고 불일치는 목록으로 보고 (talk-core 수정은 허락 필요).
- [ ] **방 메시지 결정 (2026-11-13, 추천안 채택)**: (1) 암호화 A: 멤버마다 1:1 Noise fan-out. (2) 저장: 멤버별 행 (테이블 형태 `room_message_deliveries` 또는 `room_messages` 수정은 구현 전 확인). (3) 멤버 상한 8명. (4) 보관 24시간 (방 기한과 같이 만료). (5) SAS 결정 전에는 방 메시지를 열지 않는다. (6) 미확인 멤버는 제외하고 알린다. (7) Rust 먼저. (8) 0006 RLS 닫기와 service role 전환을 방 메시지 전에 끝낸다. 설계: `docs/room-messages-design.md`.
- [ ] **방 메시지 전송 미구현**: `talk-core/src/relay_chat/room_relay.rs`의 `TODO`. talk-core는 공통 모듈이라 허락 필요. 방 메시지에도 `/trust` 확인을 적용해야 한다 (서버가 준 공개키를 그대로 믿지 않는다).
- [ ] **방 참가·만들기 동작**: 방 참가는 실패해도 재시도하지 않는다. 방 만들기는 Rust·Python 모두 동기 호출이라 최대 5초 화면이 멈춘다 (GUI는 백그라운드). 방 이름·안내 문구는 locale 키가 없어 코드에 직접 넣었다.
- [ ] **내장 모드 릴레이 재시작·첫 실행 화면 (GUI)**: qa가 수정 보고 (`gui/src/main.rs`, 미커밋). 실제 실행 확인 필요.
- [ ] **여러 방 소속 시 릴레이 대상**: 설정에는 방 코드가 하나만 저장된다. 새 방을 만들고 저장하면 릴레이가 새 방으로 옮겨 간다. 이전 방 기록은 DB에 남는다.
- [ ] **기준 확정 (2026-11-13)**: Supabase 는 1대다(방). 로컬/자체(LAN)는 직접 연결로 보이는 상대 모두에게 전송 (다수). 1:1 은 Supabase 기존 릴레이뿐. 쌍별 계획 `docs/multi-peer-relay-plan.md`는 1대다 기준으로 갱신이 필요하다 (추천 방식 A 는 대체됨, 이전 "relay 여러 상대" 항목 폐기). 1:1 릴레이는 폐기 확정 (2026-11-13), 제거 3단계 (서버) 와 모바일 1:1 정리만 남음.
- [x] **1:1 제거 1단계 (UI)** (2026-11-13, dev 완료, 미커밋): Python 설정은 URL·키·이름·방 ID 4칸. Rust·GUI 는 방 ID 만 받는다. 남은 것: "이 PC의 ID" 표시 (herdr-talk `settings_form.rs`, GUI `relay_form.rs`) 는 방 모드 행 ID 라서 유지했다. Supabase 모드에서만 숨기는 작업은 아직 안 함.
- [x] **1:1 제거 2단계 (코드)** (2026-11-13, dev 완료, 미커밋): 삭제 `talk-core` `relay.rs`, `relay_realtime.rs`, `relay_e2e_test.rs`, `relay_chat/peer_relay.rs`, Python `relay.py`, `relay_realtime.py`, `relay_replay.py`, `websocket.py` 와 관련 테스트. talk-core 경고 15 → 0. Rust 테스트 148, Python 198 통과. 남긴 것: `relay_replay.rs` (방 `NonceCache` 가 쓴다), `relay_key` 의 `short_id` (행 게시에 쓴다), `put_peer`·`delete_peer` (행 게시에 쓴다).
- [x] Chat across the internet through Supabase, with end-to-end encryption. (2026-11-09: 노드마다 새 세션 폴더, 자동 이름, 상대 이름은 행 이름을 따름. 남은 확인은 `supabase-relay.md` "남은 일") (2026-11-09: Rust TUI·GUI·Python·안드로이드 구현. 폰 ↔ PC 실사용 송수신 확인. 남은 확인은 `supabase-relay.md` "남은 일")
- [ ] **배포 상태 미확인**: 로컬 Supabase CLI 가 없어 함수 배포와 마이그레이션 적용 상태를 확인하지 못했다 (qa, 2026-11-13). CLI 설치 후 읽기 전용으로 확인하거나 대시보드에서 확인한다.
- [ ] **RLS·채널 권한은 코드로만 확인**: 0001~0005 마이그레이션 읽기 기준. 실제 DB 의 정책과 grant 는 확인하지 않았다. 방 보안 항목 (RLS 전체 공개) 과 함께 확인한다.
- [ ] **Edge Function 응답 경로는 코드로만 확인**: `create-room`, `join-room` 의 200·400·401·410 경로를 코드 읽기로 정리했다. 실행과 호출은 하지 않았다 (실제 방 생성·참가는 쓰기 작업).
- [ ] **모바일 방 기능 없음**: `mobile/src/relay` 는 1:1 경로만 있다 (`settings.ts` 단일 peer ID, `client.ts` `findRow`, `protocol.ts` 두 키 채널). 방 생성·참가·멤버 조회가 필요하다. 1:1 제거와 함께 맞춘다. 실기기와 Expo 실행은 확인하지 않았다.
- [ ] **1:1 제거 3단계 (서버, 보류)**: `peers.short_id` 컬럼과 `put-peer`·`delete-peer` 의 짧은 ID 조회를 제거한다. 보류 이유: 행 게시 서명 형식을 바꿔야 하고 (talk-core `put_message`, Python `put_request`, mobile `signPut` 동시 변경), 행 게시를 Rust·Python 이 아직 쓴다. `peers` 테이블은 `rooms.owner_pk`, `room_members.peer_pk` 가 참조하므로 남긴다. 마이그레이션 실행은 사람이 한다.
- [ ] **모바일 앱 SAS 적용 (차후)**: SAS 커밋 방식 (LAN announce 의 commit, reveal 프레임, 8자 코드) 을 `mobile/` 에 맞춘다. PC 판 (Rust, Python) 과 같은 고정 벡터로 확인. 구버전 앱은 거부된다 (호환 결정 (b)). 지금 PC 판 SAS 구현 (`task_5a16ee93214f`) 이 끝난 뒤 진행.
- [ ] **모바일 1:1 경로 정리**: `mobile/src/relay` 의 `engine.ts`, `client.ts` `findRow`, `relay.tsx` 의 peer 칸은 이번 정리 범위 밖이라 남았다. 방 기능 구현과 함께 정리한다.

## Later (phase 2 and beyond)

- [ ] Message history for peers that were offline (needs a decision on storage first).
- [ ] **Android app that sends commands to herdr AI agents.** Pick a target agent on the phone, send it a prompt, and see its state and reply. Needs the relay and encryption first, plus pairing and permission per device: this runs commands on the PC remotely, so an unpaired device must never reach an agent.
- [ ] Optional sound or highlight for mentions of your name.
- [ ] **Continuous local development with Slack.** When a local task finishes, a Claude Code Stop hook posts a completion message to Slack (`chat.postMessage`, Bot Token from `SLACK_BOT_TOKEN`, channel from `SLACK_CHANNEL_ID`). The hook then polls the thread with `conversations.replies` (standard library only, no public endpoint). A reply is returned to Claude as `{"decision":"block","reason":"<reply>"}`, so the same session continues with the next step. A reply such as `종료` stops the loop. Open decisions: project scope vs. global `settings.json`, and the hook timeout. Never store the token in the repository or in chat.
