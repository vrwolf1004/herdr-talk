# TODO

> 2026-11-02: code moved from Python to Rust. Names in finished items refer to the Python code. See [rust-roadmap.md](rust-roadmap.md).

Improvement list for herdr-talk. Items are grouped by area, most important first inside each group.

## 0.8.5 (2026-11-24)

- herdr 플러그인 id: Python 판 `herdr-talk`(기본), Rust 판 `herdr-talk-rust`. 도크 키는 판별로 분리.
- 채팅 줄 형식 `[HH:MM] 이름 : 내용`, 시간은 HH:MM.
- GUI·TUI 가 talk-core 를 공유. GUI·Rust TUI·Python TUI 를 core/extend 로 분리.
- 설정 저장 문구: TUI 와 GUI 를 분리. GUI 설정 코드 입력칸은 저장 후에도 유지.
- 제어 채널(serve) 상태에 방 ID·설정 코드 전달. GUI 하단 줄 줄바꿈.
- 실행 스크립트 `run/`: Rust TUI·GUI·Python TUI·모바일. 코드 변경이 없으면 재컴파일할지 묻고, 실행 중인 프로세스가 있으면 빌드를 막음.
- 모바일: 웹용 secure-store 대체, 설정 저장 후 입력값 유지, 상태 줄은 peers·모드만 표시, 기본 방 ID 제거.
- Supabase: put-peer·delete-peer 에 CORS 추가, 운영 배포 완료.

미확인: 실제 기기 모바일 화면, GUI·TUI 실행 화면, sqlite-jdbc 환경 테스트, 웹 배포 후 방 만들기.

## 2026-11-24 작업 현황

완료 (테스트 통과, 커밋 전)
- [x] 채팅 표시 형식 통일 목표: `[HH:MM] 이름 : 내용`. Rust `Line::render`, Rust TUI 메시지 행, Python TUI 메시지 행, 시간 표시 HH:MM.
- [x] 설정 저장 문구: TUI(Rust·Python)는 "설정 코드 확인됨, 설정 저장됨. 나머지 입력 후 마지막 줄에서 Enter", GUI는 "나머지 입력 후 '저장' 버튼을 누르세요"(7개 언어).
- [x] GUI 설정 코드 입력칸: 저장 후 코드를 입력칸에 유지하고, 같은 코드 재저장은 상환하지 않는다(회귀 테스트 추가).
- [x] 제어 채널(serve) 상태에 방 ID·설정 코드 전달(`peers`의 `room_status`). GUI 가 serve 에 붙을 때도 하단 줄에 표시.
- [x] GUI 창 기본 폭 400px, 하단 줄은 줄바꿈(잘림 방지).
- [x] 실행 스크립트: `run/run-py-tui.bat`(exe 빌드 후 실행), `run/run-rust-tui.bat`, `run/run-gui.bat`(CLI+GUI 빌드). `python/build-py-tui.bat`로 PyInstaller 단일 exe(`distherdr-talk-py.exe`) 빌드.

진행 중 (워커)
- [ ] dev1: GUI 문구 i18n 이전, GUI 채팅 줄 [HH:MM], 이어서 GUI core/extend 분리, 이어서 Rust TUI(herdr-talk) core/extend 분리.
- [ ] dev2: Python TUI core/extend 분리 (talk.py 는 진입점과 조립만).

미확인 / 남은 것
- [ ] Rust 테스트 실패 일부(`net::tests`, `relay_chat::room_relay::tests`): 표시 형식 변경으로 기대값이 옛 형식. 기대값 정리 필요(dev1 확인).
- [ ] GUI 실제 화면 확인: 하단 줄 room ID·설정 코드, 저장 문구, 시간 표시. 옛 창과 옛 serve 를 닫고 재실행해야 반영됨.
- [ ] 실환경 F2·설정 코드 상환(회사 또는 실서버).
- [ ] 다른 언어 번역(GUI 저장 문구) 원어민 검토.
- [ ] 커밋 범위: dev1·dev2 변경이 talk-core 와 herdr-talk 에 섞여 있어 커밋 전에 나눠야 함.

## 보안 리뷰 결정 (2026-11-19)

- [x] **LAN 인원 제한 기준 변경:** 확인된(/join) 상대만 10명(나 포함)에 센다. 확인되지 않은 상대는 인원을 차지하지 않는다. (반영, Rust·Python)
- [x] **20초 강퇴 규칙 유지:** 확인하지 않은 상대는 목록에서 숨긴다. (결정)
- [x] **설정 코드 상환 실패 제한 전체 유지:** 10분 안 실패 10회면 전체 상환을 막는다. IP별 제한은 너무 엄격해서 쓰지 않는다. 악용 시 잠깐 전체가 막히는 위험은 감수한다. (결정)
- [x] **발급과 상환의 실패 횟수 공유 유지:** 같은 카운터를 쓴다. (결정)
- [x] **방 trust 잠금 완화:** 이미 멤버인 사람은 잠금에 막히지 않게 했다(배포 완료). 새 참가자는 여전히 잠금의 영향을 받는다. (결정: 방 잠금 유지)
- [x] **확인 코드 8자리 유지:** 자릿수가 줄어 강도는 낮다는 점을 감수한다. (결정)
- [ ] **LAN 전체 상대 수 상한:** 확인되지 않은 상대가 무한히 쌓이면 목록 메모리가 커진다. 전체 상대 상한(예: 20명)을 두는 안을 검토 중. 승인 대기.
- [ ] **실패 횟수 경쟁 조건:** 동시에 여러 요청이 들어오면 제한을 몇 번 넘을 수 있다. 데이터베이스에서 원자적으로 세도록 고치는 안. 우선순위 결정 대기.

## 다국어·F3·정렬 (2026-11-20, 커밋 `ebb16d5`, 브랜치 `ui-i18n-f3`)

완료
- [x] **설정·첫 실행 화면 다국어:** Python, Rust TUI, GUI 모두 번역 키 사용. 7개 locale 키 집합 동일.
- [x] **F3 언어 전환:** TUI 채팅·설정·첫 실행, GUI 세 화면. 저장은 `config.json`의 `lang`. 설정 폴더가 없으면 이번 실행에서만 바뀌고 안내를 보여 준다.
- [x] **설정 입력 칸 정렬:** Python, Rust 모두 글자 폭 기준으로 맞춤(최소 22열). 7개 언어 테스트.
- [x] **설정 코드 표시:** 저장된 설정 코드를 그대로 보여 준다(요청 사항).
- [x] **GUI 설정 입력 칸·버튼 높이 30px.**

남은 것
- [ ] **main 병합:** `ui-i18n-f3` 브랜치를 main에 합칠지 결정.
- [ ] **실환경 확인:** 실제 터미널의 F2·F3·F9 입력, 한글·일본어 칸 정렬, GUI 창의 F3과 입력 칸 높이.
- [ ] **GUI 채팅 창 번역 (진행 중, dev1):** `gui/src/main.rs`의 `CONNECTED NODES`, `COMMUNICATION LOG`, `BASE STATION : CONNECTED` 등 영어 문구.
- [ ] **SYS.VER 1.0:** 상태 문구라 영어로 두었다. 번역할지 결정.
- [ ] **설정 코드 전체 표시 (보안):** 자동 보안 검토가 마스킹을 권했다. 요청 사항이라 그대로 두었다. 마스킹 여부 결정 필요.

## 이번 작업 상태 (2026-11-19 최신)

완료
- [x] **설정 코드:** 상환(F2 / Save setup code)과 저장. Python, Rust TUI, GUI 모두 같은 흐름.
- [x] **운영자 발급 (F9):** 앱의 숨은 키로 운영자 암호를 입력하면 서버가 설정 코드를 만든다. 세 화면 모두 구현.
- [x] **설정 화면 순서:** 설정 코드, Trust code, 표시 이름, 방 ID. 세 화면 모두 같은 순서.
- [x] **채팅 상태 줄:** `상대: N · CLOUD SERVICE · room ID · setup code`. 성공 문구는 붙이지 않고 오류만 붙인다. 세 화면 모두.
- [x] **trust 게이트:** 틀리면 채팅을 열지 않고 설정 화면으로 돌아간다. talk-core, Python core, Rust 시작 흐름, GUI 설정 창.
- [x] **인원 10명:** LAN은 나를 포함해 10명, Supabase 방은 서버 `join-room`에서 10명(409).
- [x] **실패 제한:** 설정 코드 실패 10회(10분), 방 trust 실패 10회 잠금(1시간).
- [x] **저장 규칙 함수:** talk-core `check_save`, `check_create`. Rust TUI 사용, GUI 연결 대기 중이던 것은 GUI 워커 보고 기준으로 확인 필요.
- [x] **Python 설정 화면 이동:** `python/src/extend/settings_form.py`.
- [x] **모바일:** trust, 설정 코드, 403/409 메시지, 상태 줄 반영. 타입 검사와 릴레이 테스트 통과.
- [x] **문서:** 운영자 가이드(`docs/operator-setup.md`), 테스트 결과(`docs/test-results.md`), 사용자 문서 정리.
- [x] **서버 함수 배포:** `join-room`(10명, trust), `redeem-setup-code`(사용 횟수, 실패 제한), `issue-setup-code`(F9 발급). 배포 명령은 사용자 승인 후 실행.
- [x] **테스트:** Rust 워크스페이스, GUI, Python, Supabase 함수, 모바일 모두 통과 (세부는 `docs/test-results.md`).

남은 것
- [x] **운영 적용 확인 (2026-11-19):** 마이그레이션 0006–0011 적용, 설정 코드 테이블과 함수(`claim_setup_code`), 함수 7개 ACTIVE, 시크릿 2개 존재 확인.
- [ ] **미확인 실환경 테스트:** 설정 코드 발급과 상환의 실제 응답, 틀린 trust 403, 11번째 참가 409, LAN 10명, 실제 터미널 화면, F2·F9 키 입력. (세부는 아래 "개선점")
- [ ] **모바일 기기 실행:** 변경된 화면과 방 참가를 실제 기기에서 확인. 모바일 1:1 경로 정리 포함.
- [ ] **모바일 브리지 결정:** 폰에서 LAN 채팅이 필요한지 정한다. 필요 없으면 브리지와 관련 문서를 지운다.
- [ ] **코어 2단계:** Python `Chat`과 네트워크 함수를 `core/`로 옮긴다(talk-core `chat.rs`, `net.rs`와 맞춤).
- [ ] **extend 2단계:** `talk.py`의 화면 그리기를 `extend/`로 분리한다.
- [ ] **커밋 범위 결정:** 1:1 제거 2단계 코드(미커밋)와 이번 작업을 나눠서 커밋할지 정한다.
- [ ] **죽은 로케일 키 정리:** `tui.settings.label.*`, `mode.status` 등 쓰지 않는 키를 지울지 정한다.

## LAN 참가 (2026-11-19 결정)

- [x] (이후 10명 제한으로 바뀜) **LAN 은 1대1, `/trust` 를 `/join` 으로.** 한 명이 이미 있으면 다른 PC 의 announce 는 무시한다 (Rust net.rs, Python talk.py). `/join CODE` 또는 `/join N` 으로 받아들이고, 상대 목록의 '확인 대기' 는 '참가 대기' 로 표시한다. 8자리 확인 코드는 숫자만 쓴다 (SAS, 10^8 가지). GUI 확인 버튼과 문서도 `/join` 으로 맞췄다. 방 메시지의 `/trust` 문구 (room.none_trusted, room.left_out) 는 방 쪽 설계가 남은 것이라 따로 정한다.
- [x] **20초 안에 join 하지 않는 상대는 강퇴.** 처음 announce 한 뒤 20초 안에 `/join` 하지 않으면 목록에서 빼고, 이 대화가 끝날 때까지 그 키의 announce 와 메시지를 무시한다. 키가 바뀌면 새 키로 다시 20초를 잰다. 강퇴 때 `peer.kicked` 줄을 한 번 보여 준다. 테스트: Rust chat.rs, Python test_encryption.py (JoinLimitTests).
- [x] **`/set` 과 HMAC 안, 방 만들기 때 trust 값 넣는 안:** 폐기. 설정 코드와 trust 방식으로 대체됨.
- [x] **워커 세션 작업:** 이번 라운드 워커는 모두 보고를 받았다. 이전 세션 4곳은 더 이상 작업을 맡기지 않는다.

## 개선점 (2026-11-19, 테스트 결과 기준: `docs/test-results.md`)

- [x] **trust 403 경로 테스트 추가 (Rust, GUI)**: 틀린 trust에서 `room_refuses`가 true를 돌려주고 설정 화면으로 돌아가는지 테스트가 없다.
- [x] **상태 줄 전체 문자열 테스트 (Rust, GUI)**: `[방장] 상대: N · CLOUD SERVICE · room ID · setup code`와 "오류만 붙는다" 규칙을 테스트로 고정한다.
- [x] **F2 중간 저장 전체 경로 테스트 (Rust, GUI)** (2026-11-24, dev1: 로컬 mock 서버 성공·404 거부 테스트): 상환 성공 후 `save_connection`까지 가는 흐름을 mock 서버로 테스트한다.
- [x] **F9 운영자 발급 키 입력 테스트 (GUI)**: 빈 암호만 테스트되어 있다. 실제 키 입력과 발급 응답 처리를 확인한다.
- [x] **GUI 하단 바 성공 문구 정리**: 지금은 "relay: room joined" 문구만 숨긴다. Python처럼 성공 문구는 숨기고 오류만 보이게 한다.
- [x] **GUI 메시지 덮어쓰기 방지**: 늦게 도착한 trust 확인 결과가 더 새로운 메시지를 덮어쓸 수 있다. 메시지 순서를 맞춘다.
- [x] **Python 도우미 `_join_room` 정리**: 실제 흐름에서 쓰지 않는 함수이고 테스트에서만 쓴다. 지우거나 실제 흐름과 맞춘다.
- [ ] **trust 확인 시간 제한**: `room_refuses`가 기존 HTTP 시간 제한만 써서, 서버가 느리면 첫 화면이 늦게 뜬다. 짧은 시간 제한을 따로 둘지 정한다.
- [ ] **실환경 확인**: 설정 코드 발급, 상환, 방 참가, 틀린 trust 403, 11번째 참가 409를 실제 서버에서 확인한다.
- [ ] **LAN 10명 실제 확인**: 여러 PC 또는 노드로 10명 참가와 11번째 거부를 확인한다.
- [ ] **모바일 기기 실행**: 모바일 변경(trust, 설정 코드, 상태 줄)을 실제 기기에서 확인한다. 1:1 경로 정리도 함께 본다.
- [ ] **모바일 브리지 결정**: 폰에서 LAN 채팅이 필요한지 정하고, 필요 없으면 브리지와 관련 문서를 지운다.
- [ ] **코어 2단계**: Python `Chat`과 네트워크 함수를 `core/`로 옮긴다(talk-core `chat.rs`, `net.rs`와 맞춘다).
- [ ] **extend 2단계**: `talk.py`의 화면 그리기를 `extend/`로 분리한다.
- [ ] **Rust 설정 화면 테스트 보강**: 중간 저장과 최종 저장 규칙을 화면 단위로 확인한다.

## Bugs

- [x] **Idle timer ignores the plain-terminal path.** `run_ansi()` sends lines to `add_message()`, which does not update `chat.last_activity`. Only `Chat.add_line()` does. So in that path, received messages and your own messages do not reset the 10-minute idle timer, and the pane can leave while people are chatting. Fix: update `last_activity` in `add_message()` too, or move the update into one shared place. (`Chat.touch()`를 모든 화면이 쓴다. `IdleTimerTest`)
- [x] **`/help` box is missing in the `prompt_toolkit` view.** The box is drawn only by `run_ansi()`. With `prompt_toolkit` installed, `/help` prints the one-line summary instead. (모든 화면이 같은 상자를 쓴다. `ChatHandleTest.test_help_prints_the_box`)

## Names

- [x] **Avoid duplicate names on the network.** Two PCs can pick the same name by chance (1 in 10 within one language list). When picking a name at start or with `/lang`, skip names already used by a fresh peer. If every name is taken, fall back to the normal random pick. (`/lang`에서 최근 피어가 쓰는 이름은 피함. 시작 시 이름은 아직 적용하지 않음)
- [x] **Name length rules differ.** `/name` cuts at 32 characters, and the random names are short. Check that a long name still fits the status line and the peer list. (`/name`도 32자 자르기 후 제어 문자를 공백으로 바꿈, 같은 규칙. 상태 줄은 `fit()`, 목록은 `wrap_line()`으로 잘림. 실제 콘솔 확인은 안 함. `name_is_cut_to_32_and_controls_removed`)

## Language and text

- [x] **Status line in the `prompt_toolkit` view** (모든 언어의 상태 줄에 Ctrl+N, Ctrl+C, /help가 있는지 `StatusLineTest`로 확인) still shows the `Ctrl+N` hint in every language. Check that each translation keeps the key names correct.
- [x] **Unify the one-line `/help` text** (`help.oneline`) with the box text, so the two cannot drift apart. (한 줄 도움말을 없애고 상자 하나로 통일. `help.oneline` 키 삭제)
- [x] **Check translated help box width** (2026-11-16 워커: ja·ko 실제 콘솔 캡처로 박스 정렬 확인) in a real console for Korean and Japanese. The width is calculated by display width, but this has only been checked in a simulated run. (2026-11-03 건너뜀: 한국어·일본어 실제 콘솔을 눈으로 봐야 하는데 이 세션에서는 화면을 볼 수 없음)
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
- [x] **Choose the trusted name at `/trust`.** (2026-11-16: 지문 화면에 'Key: 지문' (store::fingerprint, A-Z0-9 8자리) 행 추가, Rust·Python 같은 값. /trust 완료 문구 (trust.done) 에는 지문 없음 — 필요하면 결정) (2026-11-16 확인: 이름, IP, 8자리 코드는 지문 화면에 있음. 키 지문 앞부분은 아직 표시하지 않음 — 표시할지 chef 결정 필요.) (2026-11-09 사용자 결정: B. 명령 형식은 그대로 두고, 지문 확인 화면에 상대 이름·IP·키 지문 앞부분을 함께 표시. 이름 입력 방식(C)은 보류) The saved name now comes from the peer. Show it in the fingerprint prompt and let the user confirm or set a local name, e.g. `/trust <code> <name>`, and refuse a name another trusted peer already has. Changes the command, so decide before the two-PC test. (2026-11-03 건너뜀: 명령 형식이 바뀌는 chef 결정 사항)
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

- [x] **Look at the real screens.** (2026-11-16 추가: GUI Ctrl+P 재표시 3번째 입력에서 확인, 35초 뒤 자동 숨김 확인. Rust TUI: 방장 채팅 진입 시 폰 주소 표시, Ctrl+P 로 숨김 확인 (2026-11-16). 2026-11-16 Rust 워커: Ctrl+P 재표시와 30초 자동 숨김 확인 (캡처 ctrlp-shown/hidden/reshown). 같은 날 발견한 패닉 (메시지가 적을 때 폰 주소 오버레이를 그리다 범위 초과, screen.rs chat_frame) 은 고침) (2026-11-16 GUI 워커: 방장 첫 채팅 진입 시 폰 주소 팝업 자동 표시 확인, Ctrl+P 1회로 숨김 확인. 다시 Ctrl+P 로 표시는 재현 못 함 (코드 gui/src/main.rs:590-597 는 정상으로 보임). 남음: GUI Ctrl+P 재표시, 30초 자동 숨김) (2026-11-16: TUI 배지·Ctrl+P·키 입력 시 숨김은 Python 실제 캡처로 확인, Rust 채팅 배지 캡처 확인. 남음: GUI 배지·Ctrl+P·자동 표시, Rust Ctrl+P) Badge shape and position (TUI, Python, GUI), `Ctrl+P` and any-key hide, the 30-second hide, GUI auto display. Only unit tests and pane text reads so far.
- [ ] **GUI exit when its node goes away.** A GUI that was attached to a Python host disappeared right after that node was stopped (no crash event found). (2026-11-09: 재현 안 됨, 6회 — 순서 A 4회, 순서 B 2회. GUI 는 외부 Python 방장에 붙지 않고 자기 자식 serve 에만 붙음. 원 빌드·state 폴더 조건 미확인. `gui-panic.log` 로 다음 발생 때 원인 확인. 진단 로그는 추가됨)
- [ ] **Phone token after takeover (later).** When the host goes away and the remaining node is the only one on this PC, a new token is issued (`phone.rs` / `phone.py` `try_take`: `other_live_nodes` does not count the node itself). Differs from "takeover keeps token". Deferred 2026-11-09: phone work is not possible on the current network. Decide when phone work resumes.
- [ ] **Panel position.** The panel is an overlay at the bottom, not a line directly under `방장 입장`, so it never lands in the chat history. Ask whether a position directly under the line is needed.
- [ ] **Bridge start.** The host does not start the bridge; the user runs `node mobile-bridge/bridge.js`. Decide whether the host should start it (needs a fixed path and process handling).
- [ ] **Chat across two PCs** (not tested). A different state folder is treated as another PC.
- [ ] **Phone leg encryption**: the bridge is plain HTTP on the LAN. Options: HTTPS with a pinned certificate, or a tunnel.
- [ ] **Phone as a real P2P peer.** Not possible with Expo Go (UDP/TCP sockets need native modules, so a development build). Would need a JS Noise implementation, Android multicast permission and background limits. Separate project.
- [x] **One intermittent Python test failure** (2026-11-16 워커: 12회 연속 실행 0회 실패, 재현 안 됨. 계속 지켜볼 것) (once in 37 runs, not reproduced). Watch for it.
- [x] **Stale `control-<pid>.json` files** pile up after killed nodes. Harmless; maybe clean them at start.
- [x] **Commit the work.** `net.rs`, `chat.rs`, `phone.rs`, `screen.rs`, `gui/src/main.rs`, `talk.py`, `noise.py`, `phone.py`, `store.py`, locales, tests, `bridge.js`, docs. 2026-11-17: 변경은 모두 커밋됨 (main d64ba35, python-core-room 5bd6743).

## Review (2026-11-09)

Found by reading the relay, node-folder and CLI code after commit `222aee8`. Not run; each item says what was checked.

- [x] (구현 완료 2026-11-11, dev 회신 확인, 미커밋·미배포) **Crashed node leaves its server row behind.** `remove_dead_sessions` (Rust `store.rs`, Python `sessions.py`) deletes a dead node's folder, and that folder holds the key. The row is never sent to `delete-peer`, and the key is gone, so nobody can remove it. Fix: before the folder is removed, sign and send the delete with that folder's key (best effort, same as `leave`). (코드 읽기로 확인. 실제 충돌 종료로는 재현 안 함)
- [x] (구현 완료 2026-11-11, dev 회신 확인, 미커밋·미배포) **Rust does not delete the relay row when the run ends with an error.** `rust/crates/herdr-talk/src/main.rs` calls `relay_chat::leave` only when `result.is_ok()`, but it still removes the folder (and the key) afterwards. The row stays on the server. Python does the leave in `finally`, so it is not affected. Fix: call `leave` whenever a session exists. (코드 읽기로 확인)
- [x] (구현 완료 2026-11-11, dev 회신 확인, 미커밋·미배포) **`delete-peer` records the nonce before the delete.** `supabase/functions/delete-peer/index.ts`: `takeNonce` runs before `deletePeer`. If the DELETE fails (500), the nonce stays used, so the same signed body gets 401 on retry. The client signs a new body per try, so the impact is small. Fix: delete the nonce on failure, or take it after the delete. (코드 읽기로 확인)
- [x] **A reused pid keeps a dead node's folder.** (2026-11-16 chef 승인: 제어 파일 `started` 필드 (Windows Unix 초, Linux /proc starttime), Rust `store.rs` `live_node`, Python `sessions.control_live`. 교차 검증 완료: Rust serve 가 쓴 started=1794808716 과 Python process_start_time 이 같음, Python control_live 의 True/False/필드 없음/종료 후 경우 확인. Linux /proc 분기는 이 PC 에서 미검증) `_has_live_node` / `has_live_node` treat any live pid in a `control-<pid>.json` as the node. After a crash the OS can give that pid to another program, and the folder is kept until that program exits. Fix: store the process start time in the control file and compare it. Low priority. (코드 읽기로 확인)
- [x] **`herdr_cli()` still has no timeout** (2026-11-16: herdr-talk 안의 중복 구현을 없애고 `herdr_api.cli()` 를 호출하도록 바꿈. 공유 모듈 herdr-dock-grid 와 사본은 수정하지 않음. 타임아웃 테스트 HerdrCliTimeoutTest 2개. Python 테스트 221 OK) (`python/src/talk.py:1366`). `subprocess.run` waits forever if herdr does not answer. The item in `improvements.md` (2026-10-22) is still open. Fix the shared copy in herdr-dock-grid first, then sync (shared-module rule).

## Supabase 방 기능 (2026-11-13)

- [ ] **방 보안 (배포 전 필수)**: `0003_rooms.sql`의 RLS가 anon에게 `rooms`·`room_members`·`room_messages` 전체 조회를 열어 둡니다. `join-room`과 `create-room`에는 서명 검증과 재전송 방지가 없습니다. 메시지는 종단 암호화 전제이므로 서버에는 암호문만 저장하고, 조회는 멤버 검증 후 허용합니다. 구현 전까지 main의 방 모델 커밋(`8cbc628`)을 배포하지 않습니다. 진행 (2026-11-13): `0006_rooms_read_closed.sql` 작성 (실행 안 함). 전제: `create-room`·`join-room` 을 service role 키로 바꾼 뒤에만 실행 (승인 항목 f). service role 전환 완료 (미배포). 서명 검증 1단계 완료 (미커밋·미배포): `create-room`·`join-room` 이 Ed25519 서명, ts, nonce 를 검사하고 nonce 는 `put_nonces` 에 기록. 남은 것: talk-core `join_room_request` 와 gui `create_room` 이 아직 서명 없이 보낸다 (허락 필요), 배포 전 실제 Edge Function 실행 확인. 2026-11-17 확인: 코드에는 0006 (공개 조회 차단), 함수의 서명·nonce 검증, 서비스 롤 접근이 있음. 운영 DB 에 0006 이 적용됐는지와 함수 배포 상태는 사용자가 확인해야 함. 2026-11-17 운영 DB 확인 (사용자): 정책과 권한 없음 (0006 적용), 마이그레이션 0005·0007·0008 적용됨.
- [ ] **방 사용기한 24시간 (커밋됨, 운영 배포 여부 확인 필요)**: 생성 시점부터 24시간, 연장 없음 가정. `0005_room_expiry.sql`(실행 안 함), `create-room`은 `expires_at` 저장, `join-room`은 만료된 방에 410 응답. 만료 방 정리는 pg_cron (0005 주석). 남은 일: talk-core `relay_rest.rs:295`의 410 처리 한 줄 (공통 모듈, 허락 필요).
- [ ] **8자리 코드 통일 (trust·상대 ID·방 ID, A-Z0-9)**: 커밋됨, 운영 배포 여부 확인 필요. `0004_uppercase_codes.sql` 작성 (실행 안 함). 라이브 DB 소문자 행 정리와 실행은 사람이 한다. 입력 소문자는 입력 지점에서 대문자로 변환.
- [ ] **trust 코드 방식 (SAS 커밋)**: 결정 (2026-11-13): 짧은 8자 코드 + 커밋 방식. 설계는 dev가 작성 (설계만, `docs/encryption-protocol.md` 새 절은 미작성). 결정 (2026-11-13, 권장안 채택): (a) LAN 에도 SAS 적용. (b) 구버전은 거부 (명확한 오류). (c) 기존 신뢰 목록은 재검증하지 않는다. (d) 코드 8자 (SAS 와 함께). (e) 릴레이 commit 은 채널 envelope 으로 전달. (f) rooms 쓰기는 service role 로 전환 완료. (g) talk-core 허락 범위 확대: 미결정 (LAN 의 net.rs, chat.rs 는 talk-core 라서 구현 전 확인 필요). 8자 지문을 SAS 없이 단독으로 쓰면 안 된다 (키 조작으로 같은 코드를 만들 수 있다).
- [x] **LAN SAS 테스트 (테스트 전)** (2026-11-16 GUI 워커 추가 확인: 창 B 에서도 /trust 후 '확인됨, 이제 암호화'. 지문 코드는 reveal 교환 뒤에만 표시됨 (net.rs:431-451, 443-444 / chat.rs:584-586 코드로 확인). 두 창 확인됨) (2026-11-13; 2026-11-16 GUI 워커: 창 두 개 (PID 3352 '지호', 2316 '하준') 로 LOCAL 연결, 서로 상대 표시, 두 창 코드 CTZRW9EA 같음, 창 A 에서 /trust 후 '확인됨, 이제 암호화' 확인. 미확인: 창 B 의 /trust, 지문 코드가 reveal 뒤에만 나오는지 (입장 직후 로그에 보임). 캡처 g126 폴더): 로컬/자체 모드로 GUI 두 창 (같은 PC) 을 띄워 상대 목록에 서로 보이는지, 지문 코드가 reveal 뒤에만 나오는지, 양쪽 코드가 같은지, `/trust` 로 신뢰되는지 확인. 주의: Supabase 모드 노드는 LAN 으로 상대를 찾지 않는다 (이번 테스트 실패의 원인은 모드였다). Python 노드는 Rust 맞춤 전이라 Rust 와 연결되지 않는다. 실제 두 PC 확인은 별도.
- [ ] **SAS 보안 검토 지적 반영 (다른 PC 에서 진행)** (2026-11-13, 자동 보안 검토): `python/src/talk.py` 의 SAS 구현. (1) nonce 를 세션 전체에서 하나만 쓴다 → 상대마다, 페어링 시도마다 새로 만든다. (2) `chat.commits` 를 덮어쓴다 → 처음 받은 커밋을 고정 (`setdefault`), 같은 키의 다른 커밋은 연결 거부와 경고. (3) reveal 이 실패하면 nonce 를 버리고 다음 시도에 새로 만든다. (4) 커밋을 먼저 보내고, 상대 커밋을 받은 뒤에만 reveal (ZRTP 식 순서) 이 지켜지는지 확인. (5) Rust `talk-core` (`net.rs`, `chat.rs`) 도 같은 규칙인지 확인하고 불일치는 목록으로 보고 (talk-core 수정은 허락 필요).
- [x] **방 참가 재시도와 방 하나 제한** (2026-11-17 결정, 구현 완료, 미커밋): 참가 실패 시 10초 뒤 1회 재시도, 성공하면 기회를 되돌림 (Rust JoinRetry, Python JoinRetryTests). 방은 하나만 허용 (다른 코드 저장과 Ctrl+N 거부, 같은 방 허용), 알림 settings.one_room_only / room.only_one 7개 언어. 테스트: Rust talk-core 179, herdr-talk 44; Python 253. 미확인: 실제 네트워크 재시도, GUI 알림 화면.
- [x] **내장 모드 릴레이 재시작·첫 실행 화면 (GUI)** (2026-11-16 GUI 워커: 빌드 성공, 첫 화면 캡처, LOCAL CONNECT 후 방장 채팅 화면 `[방장] BASE STATION : CONNECTED` 확인. 재시작 경로는 코드로 확인: main.rs:306-311 저장 시 릴레이 재시작, main.rs:331-333 serve 모드 relay_reload, relay_chat/mod.rs:56 start_with 가 기존 relay 를 stop 후 재시작): qa가 수정 보고 (`gui/src/main.rs`, 미커밋). 실제 실행 확인 필요.
- [x] **1:1 제거 1단계 (UI)** (2026-11-13, dev 완료, 미커밋): Python 설정은 URL·키·이름·방 ID 4칸. Rust·GUI 는 방 ID 만 받는다. 남은 것: "이 PC의 ID" 표시 (herdr-talk `settings_form.rs`, GUI `relay_form.rs`) 는 방 모드 행 ID 라서 유지했다. Supabase 모드에서만 숨기는 작업은 아직 안 함.
- [x] **1:1 제거 2단계 (코드)** (2026-11-13, dev 완료, 미커밋): 삭제 `talk-core` `relay.rs`, `relay_realtime.rs`, `relay_e2e_test.rs`, `relay_chat/peer_relay.rs`, Python `relay.py`, `relay_realtime.py`, `relay_replay.py`, `websocket.py` 와 관련 테스트. talk-core 경고 15 → 0. Rust 테스트 148, Python 198 통과. 남긴 것: `relay_replay.rs` (방 `NonceCache` 가 쓴다), `relay_key` 의 `short_id` (행 게시에 쓴다), `put_peer`·`delete_peer` (행 게시에 쓴다).
- [x] Chat across the internet through Supabase, with end-to-end encryption. (2026-11-09: 노드마다 새 세션 폴더, 자동 이름, 상대 이름은 행 이름을 따름. 남은 확인은 `supabase-relay.md` "남은 일") (2026-11-09: Rust TUI·GUI·Python·안드로이드 구현. 폰 ↔ PC 실사용 송수신 확인. 남은 확인은 `supabase-relay.md` "남은 일")
- [ ] **배포 상태 (2026-11-17 일부 확인)**: 운영 Edge Function 목록 확인 — create-room, join-room, post-message, fetch-messages, put-peer 배포됨 (put-peer 는 최신 코드와 같은지 미확인). delete-peer 는 없음 (배포 필요, 사용자 실행).
- [ ] **RLS·채널 권한은 코드로만 확인**: 0001~0005 마이그레이션 읽기 기준. 실제 DB 의 정책과 grant 는 확인하지 않았다. 방 보안 항목 (RLS 전체 공개) 과 함께 확인한다.
- [ ] **Edge Function 응답 경로는 코드로만 확인**: `create-room`, `join-room` 의 200·400·401·410 경로를 코드 읽기로 정리했다. 실행과 호출은 하지 않았다 (실제 방 생성·참가는 쓰기 작업).
- [ ] **모바일 방 기능 없음**: `mobile/src/relay` 는 1:1 경로만 있다 (`settings.ts` 단일 peer ID, `client.ts` `findRow`, `protocol.ts` 두 키 채널). 방 생성·참가·멤버 조회가 필요하다. 1:1 제거와 함께 맞춘다. 실기기와 Expo 실행은 확인하지 않았다.
- [ ] **1:1 제거 3단계 (서버, 보류)**: `peers.short_id` 컬럼과 `put-peer`·`delete-peer` 의 짧은 ID 조회를 제거한다. 보류 이유: 행 게시 서명 형식을 바꿔야 하고 (talk-core `put_message`, Python `put_request`, mobile `signPut` 동시 변경), 행 게시를 Rust·Python 이 아직 쓴다. `peers` 테이블은 `rooms.owner_pk`, `room_members.peer_pk` 가 참조하므로 남긴다. 마이그레이션 실행은 사람이 한다.
- [ ] **모바일 앱 SAS 적용 (차후)**: SAS 커밋 방식 (LAN announce 의 commit, reveal 프레임, 8자 코드) 을 `mobile/` 에 맞춘다. PC 판 (Rust, Python) 과 같은 고정 벡터로 확인. 구버전 앱은 거부된다 (호환 결정 (b)). 지금 PC 판 SAS 구현 (`task_5a16ee93214f`) 이 끝난 뒤 진행.
- [ ] **모바일 1:1 경로 정리**: `mobile/src/relay` 의 `engine.ts`, `client.ts` `findRow`, `relay.tsx` 의 peer 칸은 이번 정리 범위 밖이라 남았다. 방 기능 구현과 함께 정리한다.

## UI 디자인 (2026-11-16)

디자인 기준은 [ui-style.md](ui-style.md). GUI 는 레트로 CRT, TUI (Rust, Python) 는 클래식 터미널 (DOS/BBS). 색 팔레트와 규칙은 그 문서가 기준이다.

- [x] GUI 선택 화면: BASE STATION 메인, Supabase 보조 링크 (구현, 커밋 `14ced09`)
- [x] GUI 설정 화면: 가운데 정렬, 스크롤, 하단 고정 버튼, 표시 이름 고유 코드, 공개키 작은 글씨 (구현, 커밋 `14ced09`)
- [x] 하단 상태 줄에 릴레이 상태 문구 (GUI, 구현)
- [x] `made by Rust` / `made by Python` 푸터를 맨 아래 오른쪽 끝으로 (두 TUI 구현)
- [x] Rust, Python TUI 깜빡임 수정: 전체 지우기 제거, 바뀐 행만 그리기, 같은 프레임 건너뛰기 (구현, 실제 화면 확인은 사용자)
- [ ] **TUI 클래식 터미널 디자인 적용 (Rust, Python)**: 거의 완료. 헤더, 구분선, 제목, 본문, 키 안내, 상태 줄, 팔레트, 명칭 통일, 커서 깜빡임 수정 (2026-11-17, 첫 화면과 설정 화면은 커서 숨김, 채팅은 입력 줄). 남음: 사용자가 실제 두 창에서 채팅 커서 위치 확인 (151).
- [ ] 클래식 TUI 확정 후 실제 두 창으로 화면 확인 (사용자)
- [x] **상대 목록 패널** (2026-11-16: Rust 채팅에 `CONNECTED NODES` 줄 추가, 캡처 확인): Rust TUI 에는 CONNECTED NODES 패널이 없다. 클래식 TUI 에 텍스트 목록으로 넣을지 결정 필요.
- [x] **Rust 설정의 좌우 키** (2026-11-16: 모드 행의 좌우키·Space 제거, 포커스는 필드 1~4): 연결 방식 줄을 지웠으나 Left/Right 키는 남아 있다. 키도 지울지 결정 필요.
- [x] **Rust TUI 의 Display name 칸** (2026-11-16: 읽기 전용 8자리 대문자 코드, GUI 와 같은 방식): GUI 는 고유 코드 (읽기 전용) 로 바꿨으나 TUI 는 아직 입력 칸. 같은 방식으로 맞출지 결정 필요.
- [x] **Python 설정 화면의 MODE 줄** (2026-11-16: MODE 행과 Space 제거, 포커스는 필드 4칸. Up/Down 은 미지원으로 확정 (read_char 가 화살표를 못 읽음, 2026-11-16 결정)): Space 로 모드 바꾸는 기능이 남아 있다. 지울지 결정 필요.
- [x] **TUI 입력 안내 문구** (2026-11-16: 채팅 안내와 입력 힌트를 로케일 키 `chat.guide`, `chat.input_hint` 로 옮김, 7개 언어. 번역 검토 완료: ko 'Enter: 보내기' 로 고침, 나머지 유지. Ctrl+N 안내 추가, 7개 언어 폭 79칸 이하): Python 은 고정 문구 (로케일 미사용). 로케일로 옮길지 결정.
- [x] **제작자 표시 위치 (2026-11-16 결정)**: Rust 는 맨 위 줄 오른쪽 끝, Python 은 맨 아래 줄 오른쪽 끝. Rust 구현 완료 (`screen.rs`, 화면 확인). Python 은 이미 맨 아래 오른쪽.
- [x] **Python 맨 아래 줄 혼잡 (2026-11-16)**: 푸터 "made by Python" 을 맨 아래 줄에 혼자 두고, 상태 문구는 그 위 줄로 올렸다 (`talk.py` `screen_frame`, 최소 높이 10행, 메시지 영역 `rows - 9`). 코어 locale `status.line` 의 키 안내는 그대로 둔다 (`i18n.rs` 와 `test_talk.py` 가 키 이름을 요구하는 테스트가 있어서). 테스트 217 통과, 실제 창에서 확인.

## 코드 분석 (2026-11-16)

미커밋 소스 diff 를 읽어 찾았다 (정적 분석, 재현 테스트 안 함). 백엔드 테스트는 전부 통과 (Rust 207, Python 215, Edge Function 46). 아래는 테스트가 잡지 못하는 항목이다.

- [x] **[높음] 설정 화면 포커스가 보이지 않는 칸에 머묾**: `rust/crates/herdr-talk/src/settings_form.rs`. 포커스는 이제 필드 1~4 만 돈다 (`FIELDS`), 처음 포커스는 1. 모드 행의 좌우키·Space 처리와 `toggle_mode` 는 지웠다 (보이지 않는 키였고, 모드는 첫 실행 선택으로 정한다. 이전 "좌우 키 지울지" 결정을 지우는 쪽으로 처리). 테스트 `focus_starts_on_a_drawn_field_and_cycles_over_the_fields_only`, `letters_typed_on_the_first_screen_reach_a_field`. 실제 화면 확인은 안 함.
- [x] **[높음] 방 메시지가 조용히 사라질 수 있음**: 릴레이 스레드가 끝나면 `relay_chat/mod.rs` 가 자기 inbox 를 `detach_relay` 로 비운다 (다른 릴레이의 inbox 는 건드리지 않음, `stop` 의 `Arc` 로 구분). `relay_send` 는 수신자가 없으면 `relay.not_sent` 를 표시한다. 테스트 `a_line_to_a_relay_that_has_ended_is_reported_not_lost`, `a_finished_relay_thread_detaches_only_its_own_inbox`. talk-core 수정은 사용자 결정으로 진행했다. 실제 방 조인 실패 상황으로는 확인 안 함. 릴레이 스레드가 조인 실패로 끝나도 inbox 가 비워지지 않는다. `relay_send` 의 `let _ = inbox.send(..)` 가 오류를 무시하므로 입력한 메시지가 화면에도 안내에도 나타나지 않는다. 수정: 스레드 종료 시 inbox 를 비우거나, 전송 실패 시 `relay.not_sent` 를 표시한다.
- [x] **[중간] 대화명 규칙이 Rust 와 Python 에서 다름**: `python/src/relay_rest.py` `valid_name` 이 Rust `is_printable` 과 같은 범주 (Cc, Cf, Co, Cs, Zl, Zp, Zs, 공백 제외) 를 거부하도록 했다. 남은 차이: 미할당 (Cn) 과 비문자 코드 포인트는 Python 이 받고 Rust 는 일부를 거부한다 (코드에 `shortcut:` 주석). 테스트 `test_name_refuses_control_format_and_space_separator_characters`.
- [x] **[중간] 설정 오류가 연결 중으로 표시됨**: `herdr-talk/src/screen.rs` `status_tag` 가 `incomplete`, `invalid` 도 오류로 본다. 테스트 `the_status_tag_follows_the_relay_text` 에 두 경우 추가.
- [x] **[낮음] Python 릴레이가 이름 검사 실패 시 조인을 시도함**: `python/src/relay_chat.py` `_run`. 빈 이름이 아닌데 유효하지 않으면 게시·조인 없이 `relay: name is not valid; not joined yet` 을 표시하고 끝낸다. 빈 이름은 예전 동작 그대로 (게시를 건너뛰고 조인). 테스트 `test_a_bad_name_is_not_published_or_joined_and_the_reason_is_shown`. Rust 쪽은 이미 이름 검사 실패 시 조인하지 않는지 확인하지 않았다.
- [x] **[낮음] 타임스탬프 재매칭 (보류)** (2026-11-16: Line 에 seq 를 붙이고 Stamps 가 seq 로 매칭. 테스트 equal_texts_keep_their_own_times_when_the_first_is_dropped): `herdr-talk/src/screen.rs` `Stamps::times`. 같은 내용의 줄이 여럿이고 위쪽이 잘려 나가면 시간이 잘못 옮겨질 수 있다. 고치려면 줄마다 순번을 저장해야 해서 `Line` 구조 변경이 필요 (talk-core). 표시만 영향이라 보류.
- [x] **[낮음] Python 채팅 화면 커서 위치**: `python/src/talk.py` `screen_frame` 이 실제 커서를 입력 줄의 `_` 위치에 둔다 (마지막 쓰기 뒤에 `ESC[줄;칸H`). 디자인의 `_` 글자는 그대로 둔다.

- [x] **코어 알림 문구 로케일화 (2026-11-16)**: `room_relay.rs`(방 게시 실패, 확인된 멤버 없음, 제외 인원), `net.rs`(구버전, 다른 확인 값), `chat.rs`(코드 대기) 의 한국어 고정 문구를 키 `room.*`, `net.*`, `peers.code_pending` 로 옮기고 7개 언어에 넣음. talk-core 테스트 통과.
- [x] **모드 명칭 통일 (2026-11-16)**: LAN = BASE STATION, Supabase = CLOUD SERVICE. 7개 언어 mode.local / mode.supabase, 첫 메뉴 (Rust, Python), GUI 링크, 사용자 문서 (usage.ko.md, readme-extras.ko.md, ui-style.md) 를 맞춤.
- [x] **상태 태그 위치 (2026-11-16)**: Rust 와 Python 모두 상태 줄 맨 끝 [SYSTEM READY]. Python 과 Rust 캡처 비교는 워커가 진행 중.
- [x] **방 행 시작 시각 검사 (2026-11-16, 보안 보완 중)**: 이 노드가 시작하기 전 ts 의 행은 버림 (Python core/room.py, Rust talk-core). 프로토콜은 그대로. 방 코드와 받는 사람 바인딩은 프로토콜 변경이라 나중 결정. 2026-11-17: Rust·Python 구현과 테스트 완료, 커밋 5bd6743.
- [x] **구분선 색과 입력 힌트 (Rust, 2026-11-16)**: 구분선을 테두리 녹색 #417D55 로, 입력 힌트는 talk-core chat.input_hint (7개 언어) 로 읽음. HERDR_TALK_LANG=en 에서 확인.
- [x] **LAN 대화 줄 형식 (Python)** (2026-11-16: `[시각] 이름 > 내용` 으로 바꿈, Rust 와 같음)
- [ ] **Python 방 메시지**: 구현·커밋 완료 (python-core-room). 운영 상호운용 테스트 진행 중: 방 K101SWW9 생성, rust-supa 참가 확인 (2026-11-17). 막힌 점: py-supa 가 예전 공지 형식으로 떠 있어 '구버전' 판정 → 공지 수정 (Python talk.py 의 commits 항상 포함) 적용됨, 노드 재시작 허락 대기. 이후 /trust 와 메시지 확인 남음.
- [ ] **운영 테스트 정리 (2026-11-17)**: 테스트로 생긴 운영 데이터 — 방 K101SWW9, rust-supa 의 방 행. 메시지는 아직 없음. 정리는 사용자 확인 후 삭제.
- [ ] **키 입력 사고 확인 (2026-11-17)**: 테스트 중 키 입력이 Orca, Chrome(Gmail), herdr-talk-gui (PID 8524) 로 샜을 가능성. 사용자가 Gmail 탭, Orca 창, GUI 창 상태를 확인해야 함.
- [ ] **Rust 실행 파일 교체 정리**: rust/target/debug/herdr-talk.exe.old 는 잠금이 풀리면 삭제 (실행 중인 운영 노드가 쓰는 중이었음).
- [x] **TUI 팔레트를 GUI·Rust·Python 에서 같은 스펙 색으로 (2026-11-16)**: Python 을 16색 ANSI 에서 24비트 색으로 바꿈 (relay_form.py 상수, 배경 #050A08 을 모든 행에). 값은 docs/ui-style.md 3절과 gui retro 팔레트 일치. Python 테스트 221 OK, 실제 창 캡처로 배경·테두리·제목·흐린 글씨 확인. 2026-11-16: Rust 도 색을 ANSI 24비트 문자열로 직접 쓰도록 고쳐 출력에 38;2/48;2 확인. 남음: Rust 와 Python 의 실제 화면 색 나란히 비교.
- [x] **Rust 채팅 상태 문구의 키 안내 중복** (2026-11-16: 7개 언어 status.line 에서 키 안내를 뺌, 안내는 chat 가이드 줄이 맡음. Python 가이드에 Ctrl+N 추가. Rust 가이드 줄에 /help 추가 완료: Enter: SEND · /help: COMMANDS · Ctrl+N: NAME · Ctrl+P: PHONE · Ctrl+R: SETTINGS · Ctrl+C: QUIT): 맨 위 상태 문구의 `Ctrl+N: 이름 변경 /help Ctrl+C: 종료` 가 아래 키 안내 줄과 겹친다. Python 과 같은 문제 (코어 locale `status.line`). 정리 방향 결정 필요.

- [x] **방 메시지 이름: 대화명 통일, 식별은 pk** (2026-11-16: Rust 방 메시지를 LAN 과 같은 [시각] 이름 > 내용 줄로 바꿈 (sender 있는 Line). Python 은 방 메시지가 아직 없음 (보류). 이전 확인: 표시 이름은 신뢰 목록 이름 (trusted_name), 식별은 pk, 같은 이름이면 pk 앞 4자 (room_relay.rs:183-219). 남은 차이: 방 메시지가 일반 알림 행으로 나가 LAN 의 [시각] 이름 > 내용 형식과 다름 — 형식을 맞출지 chef 결정 필요.) (기존 메모: 이전 디스패치 (task_940ef9b529a3) 가 보고 없이 턴을 끝내서 escalation 이 남아 있음. 작업이 완료됐는지 코드로 확인하고, 안 됐으면 다시 맡기거나 정리할 것.

## Later (phase 2 and beyond)

- [ ] Message history for peers that were offline (needs a decision on storage first).
- [ ] **Android app that sends commands to herdr AI agents.** Pick a target agent on the phone, send it a prompt, and see its state and reply. Needs the relay and encryption first, plus pairing and permission per device: this runs commands on the PC remotely, so an unpaired device must never reach an agent.
- [ ] Optional sound or highlight for mentions of your name.
- [ ] **Continuous local development with Slack.** When a local task finishes, a Claude Code Stop hook posts a completion message to Slack (`chat.postMessage`, Bot Token from `SLACK_BOT_TOKEN`, channel from `SLACK_CHANNEL_ID`). The hook then polls the thread with `conversations.replies` (standard library only, no public endpoint). A reply is returned to Claude as `{"decision":"block","reason":"<reply>"}`, so the same session continues with the next step. A reply such as `종료` stops the loop. Open decisions: project scope vs. global `settings.json`, and the hook timeout. Never store the token in the repository or in chat.
