# 추가 항목 (README에서 옮김)

## 헤드리스 모드와 제어 채널

`herdr-talk serve`는 화면 없이 채팅을 실행합니다. GUI나 다른 로컬 프로그램이 쓰기 위한 모드이며, Ctrl+C, 종료 신호, `shutdown` 요청 중 하나로 끝납니다.

`run`과 `serve`는 모두 로컬 제어 채널을 엽니다. `127.0.0.1`의 TCP 포트이고 실행마다 새로 만드는 토큰으로 보호합니다. 포트와 토큰은 상태 폴더의 `control-<pid>.json`에 쓰이며(현재 사용자만 읽을 수 있음), 채팅이 끝나면 지워집니다. 명령은 `lines`, `peers`, `send`이고 `serve`는 `shutdown`도 받습니다. [설정](docs/configuration.md#control-channel)을 참고하세요.

## Windows GUI

`gui/`는 채팅을 자체 창으로 보여 주는 별도의 Windows 앱입니다(Rust, eframe). 먼저 `herdr-talk serve`를 숨겨서 실행해 연결을 시도하고, 실패하면 창 안에서 채팅을 직접 실행합니다(내장 모드). 처음에는 입장 버튼이 있는 소개 화면이 나옵니다. 창마다 독립된 채팅 노드입니다. 상단 바에서 테마(Dark, Light, Amber, Green)를 고르고 CRT 체크박스(기본 켜짐: 스캔라인과 베젤)를 켜고 끌 수 있습니다. 글꼴은 Galmuri11(`gui/assets`, SIL OFL)입니다. [사용법](docs/usage.ko.md#11-windows-gui)을 참고하세요.

```sh
cd gui
cargo run --release
```

## 연결 방식: 로컬/자체 또는 Supabase

- **로컬/자체 (LAN)**: 기본값. 같은 네트워크의 PC끼리 서버 없이 바로 채팅합니다.
- **Supabase**: 메시지가 Supabase 릴레이로만 갑니다. LAN 으로는 보내지 않습니다.
- 채팅을 띄울 때마다 새 **노드** (TUI, GUI, Python 실행 하나) 가 된다. 노드마다 키, 짧은 ID, 신뢰 목록이 `sessions\<id>\` 에 따로 있다. 같은 PC의 창 두 개는 노드 두 개다. `--session <id>` 로 같은 노드를 다시 쓴다.
- Supabase 모드에서는 같은 PC의 노드끼리 대화하지 않는다. 각 노드가 릴레이로 폰과 따로 대화한다. LAN 으로는 아무것도 보내지 않는다.
 첫 화면 (GUI) 또는 첫 실행 (TUI) 에서 고르고, `Ctrl+R` (설정) 또는 상단 "연결 방식" 버튼으로 바꿉니다.
- Supabase 설정은 URL, publishable 키, 상대의 짧은 ID (8자리) 입니다. 표시 이름은 자동으로 배정됩니다.
- 채팅을 띄울 때마다 새 **노드** (TUI, GUI, Python 실행 하나) 가 된다. 노드마다 키, 짧은 ID, 신뢰 목록이 `sessions\<id>\` 에 따로 있다. 같은 PC의 창 두 개는 노드 두 개다. `--session <id>` 로 같은 노드를 다시 쓴다.
- Supabase 모드에서는 같은 PC의 노드끼리 대화하지 않는다. 각 노드가 릴레이로 폰과 따로 대화한다.
 짧은 ID 는 PC마다 한 번 만들어집니다 (`short-id.txt`).
- 채팅 전에 양쪽에서 `/trust <코드>` 로 서로 확인합니다.

## 폰 (Expo Go)

Android 폰이 `mobile-bridge/bridge.js` 와 `mobile/` 의 Expo 앱으로 채팅에 참여할 수 있습니다. 폰은 피어가 아닙니다. 폰은 브리지와 HTTP 로 통신하고, 브리지가 같은 PC 의 herdr-talk 노드와 통신합니다. 이 구간은 평문 HTTP 라서 믿을 수 있는 Wi-Fi 에서만 쓰세요.

- 한 PC 에서 처음 시작한 노드가 **방장** 입니다 (`--name` 이 없으면 이름이 "방장", `[방장]` 배지). 방장이 폰 주소와 6자리 토큰 (`phone.json`) 을 가집니다. 방장이 종료되면 다음 노드가 이어받습니다.
- 내가 방장이 될 때, 방장이 입장할 때, `Ctrl+P` 또는 `/phone` 을 누를 때 채팅 화면에 폰 주소와 토큰이 보입니다. `/phone new` 는 새 토큰을 만듭니다. 화면에만 그리고 채팅 줄이나 로그에는 남기지 않습니다.
- `node mobile-bridge/bridge.js` 를 실행하고 `mobile/` 에서 `npx expo start --host lan` 을 실행한 뒤, 앱에 주소와 토큰을 입력합니다.

자세한 내용, 확인 결과, 남은 일: [docs/mobile.md](docs/mobile.md).

## 이름

이미 다른 상대가 같은 이름을 쓰면, id가 큰 쪽이 번호를 붙입니다(`Liam`은 `Liam2`, 그다음 `Liam3`). 키가 id와 맞는 상대만 판단에 쓰며, 자동 이름 변경은 60초에 한 번까지입니다. [사용법](docs/usage.ko.md#6-이름)을 참고하세요.

## 언어

채팅 화면 문구는 PC의 언어를 따릅니다. 영어, 한국어, 러시아어, 프랑스어, 스페인어, 독일어, 일본어를 지원하며, 그 밖의 언어는 영어로 표시합니다. `HERDR_TALK_LANG` 환경 변수나 `config.json`의 `"lang"`으로 바꿀 수 있습니다. 자세한 내용은 [설정](docs/configuration.md#language)을 참고하세요.

