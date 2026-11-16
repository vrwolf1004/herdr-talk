# UI 스타일 기준 (2026-11-16 결정)

herdr-talk 의 화면은 두 종류로 나눈다. 같은 색 팔레트를 쓰되 레이아웃 규칙은 종류별로 다르다.

## 1. GUI (Windows 창, `gui/`)

레트로 CRT 디자인. 검정 배경, 녹색 네온 테두리, 모노스페이스, 약한 스캔라인.

- 선택 화면: `BASE STATION` 이 메인 (`[PRIMARY] BASE STATION`, `CONNECT`), Supabase 는 보조 텍스트 링크 (`ALTERNATIVE CONNECTION` 아래 밑줄 링크).
- 설정 화면 (Supabase 전용): 제목 `SUPABASE SETTINGS` 가운데 정렬, 콘텐츠 최대 680px 가운데 열, 가운데 스크롤 영역, 하단 고정 버튼 (`CANCEL`, `SAVE SETTINGS`, 오른쪽 정렬).
  - 섹션: `SECTION 01 SUPABASE CONNECTION` (URL, publishable 키), `SECTION 02 ROOM SETTINGS` (표시 이름 = 읽기 전용 고유 코드, 방 ID 8자리, `CREATE ROOM`), `SECTION 03 DEVICE IDENTITY` (DEVICE ID 와 COPY 버튼, 공개키는 작은 흐린 글씨).
  - 로컬/자체, 연결 방식 선택, 안내 문구는 설정 화면에 두지 않는다.
- 채팅 화면: `BASE STATION : CONNECTED`, `COMMUNICATION LOG`, `CONNECTED NODES`, `SEND` 버튼.
- 하단 상태 줄: 왼쪽 시스템 상태 (또는 릴레이 상태 문구), 오른쪽 `BASE STATION`.
- 푸터: 없음 (GUI 는 창 하단 상태 줄 사용).

## 2. TUI (터미널, `rust/crates/herdr-talk`, `python/src/talk.py`, `python/src/relay_form.py`)

클래식 터미널 (DOS/BBS). GUI 처럼 보이는 카드, 버튼, 둥근 패널, 큰 박스, 네온 글로우, 마우스 중심 UI 는 쓰지 않는다.

화면 구조 (위에서 아래):

```
HERDR-TALK                                              SYS.VER 1.0
--------------------------------------------------------
SCREEN TITLE

MAIN CONTENT

--------------------------------------------------------
KEYBOARD GUIDE
[SYSTEM STATUS]                          made by Rust / Python   <- 맨 아래 줄 오른쪽 끝
```

- 메뉴: 버튼 대신 텍스트 메뉴. 선택 항목은 `▶ 01. LOCAL NETWORK`, 설명은 다음 줄. 선택 항목만 주 녹색.
- 입력: 박스 반복 없이 `DISPLAY NAME : [USER NAME________]` 형태. 포커스는 커서나 강조색.
- 채팅: `[12:30:21] USER_A > Hello.` 형식, 하단 `MESSAGE > _`.
- 상태 태그: `[SYSTEM READY]`, `[CONNECTING...]`, `[CONNECTED]`, `[DISCONNECTED]`, `[ERROR]`. 정상 녹색, 대기 노란색, 오류 빨간색.
- 푸터 (제작자 표시, 흐린 본문색): Rust 와 Python 모두 맨 아래 줄 오른쪽 끝 (`made by Rust` / `made by Python`).
  - Rust: 상태 태그는 같은 줄 왼쪽. 맨 위 줄은 `HERDR-TALK` 와 `SYS.VER 1.0` (2026-11-16 사용자 결정으로 되돌림).
  - Python: 맨 아래 줄에는 제작자 표시만 둔다. 상태 태그는 바로 위 줄.

## 3. 공통 색 팔레트

| 용도 | HEX |
|---|---|
| 배경 | #050A08 |
| 패널 (GUI 전용) | #0A140F |
| 주 녹색 (PRIMARY) | #46FF82 |
| 테두리 녹색 | #417D55 |
| 본문 텍스트 | #B4DCBE |
| 흐린 텍스트 (MUTED) | #526B59 (실제 화면에서는 #7D9A84 로 올림, 가독성) |
| 경고 | #FFB84D |
| 오류 | #FF5555 |

## 4. 이름과 식별

- 화면에 보이는 이름은 **대화명** 하나로 통일한다. LAN 과 방 메시지 모두 같은 대화명을 쓴다.
- 식별은 이름이 아니라 **공개키**로 한다. 같은 대화명이 둘 이상이면 화면에서만 구분하도록 공개키 앞 4자를 붙인다 (예: `방장 (a1b2)`).
- 설정의 표시 이름 칸은 입력이 아니라 **자동 생성 고유 코드** (8자리, 읽기 전용) 로 한다.
- 첫 노드도 `방장`이라는 이름을 쓰지 않는다. 나라별 랜덤 이름을 쓰고, 방장 역할은 상태 줄 배지로만 표시한다.

## 5. 코드 표시

- 지문 코드와 방 ID, 상대 ID 는 8자리 한 덩어리로 표시한다 (예: `CDHQTQ8I`). 대시는 쓰지 않는다.
- 입력은 대시와 공백을 무시하고 대문자로 바꿔서 처리한다.
- 상대 표시는 세 줄: 상태 줄, `  코드: CDHQTQ8I`, `  상대 화면의 코드와 같은지 비교한 뒤 /trust 를 입력하세요`.
- 공개키는 화면에 작은 흐린 글씨로만 보인다 (복사 버튼 없음).

## 6. 원칙

- 화면 표현만 바꾼다. 키, 명령, 네트워크, 저장, 채팅 로직은 바꾸지 않는다.
- Rust 와 Python 의 최종 화면은 같은 규칙을 따른다. 구현 방식은 달라도 된다.
- 깜빡임: 화면 전체를 지우지 않고, 바뀐 행만 다시 그린다. 같은 프레임은 출력하지 않는다.
