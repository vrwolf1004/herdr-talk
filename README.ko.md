# herdr-talk

[English](README.md) | [**한국어**](README.ko.md)

지원 언어: 영어 · 한국어 · 러시아어 · 프랑스어 · 스페인어 · 독일어 · 일본어

herdr 안에서 다른 PC의 사람과 채팅합니다. 같은 네트워크의 상대와 채팅 서버 없이 직접 연결됩니다.

## 대시보드 디자인

채팅 pane:

```text
herdr-talk: 내 이름은 'Haruto' (TCP port 52311). 명령은 /help를 입력하세요.
* Liam 접속 (172.31.99.181)

Liam: hi, anyone here?
Haruto (나): hello!

>
```

이 플러그인은 Agents 사이드바 블록 대신 채팅 pane을 사용합니다. 현재 구현은 같은 LAN에서만 동작하며 UDP 브로드캐스트로 상대를 찾고 TCP로 메시지를 직접 보냅니다. 확인된 상대와의 메시지는 암호화됩니다(Noise). 메시지는 저장되지 않습니다. 자세한 내용은 [설정: 암호화](docs/configuration.md#encryption)를 참고하세요.

## 설치

```sh
herdr plugin install vrwolf1004/herdr-talk/rust
```

Rust 도구 체인이 없거나 실행 파일 내려받기를 피하려면 Python 판을 쓰세요. Python 3만 있으면 됩니다.

```sh
herdr plugin install vrwolf1004/herdr-talk/python
```

Python 판도 Rust 판과 같은 방식으로 암호화합니다 (순수 Python, 추가 패키지 없음). 코드를 비교하고 `/trust` 하면 두 판이 서로 대화합니다. 키가 없는 옛 Python 판과 대화할 때만 Rust 쪽 `config.json`에 `"plaintext": true`가 필요합니다. [docs/encryption-protocol.md](docs/encryption-protocol.md) 참고.

## 액션

| 액션 | 기능 |
| --- | --- |
| `open` | 현재 pane 아래에 채팅 pane 열기 |
| `dock-start` | `herdr-dock` 워크스페이스에 대화 블록을 띄웁니다 |
| `dock-stop` | 도크 블록을 숨깁니다 |

herdr pane 안에서 액션을 실행해야 합니다. 직접 pane을 여는 방법, 채팅 명령, 네트워크 요구 사항은 [사용법](docs/usage.ko.md)을 참고하세요.

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

## 요구 사항

- Windows, Linux, macOS
- herdr 0.9.0 이상
- 두 PC가 같은 네트워크에 있고 방화벽에서 탐색과 TCP 통신을 허용해야 함

## 언어

채팅 화면 문구는 PC의 언어를 따릅니다. 영어, 한국어, 러시아어, 프랑스어, 스페인어, 독일어, 일본어를 지원하며, 그 밖의 언어는 영어로 표시합니다. `HERDR_TALK_LANG` 환경 변수나 `config.json`의 `"lang"`으로 바꿀 수 있습니다. 자세한 내용은 [설정](docs/configuration.md#language)을 참고하세요.

## 문서

- [사용법 (한국어)](docs/usage.ko.md)
- [Usage (English)](docs/usage.md)
- [설정](docs/configuration.md)
- [문제 해결](docs/troubleshooting.md)
- [개발](docs/development.md)
- [할 일](docs/todo.md)

## 프로젝트 응원하기

프로젝트가 도움이 되셨다면 커피 한 잔으로 개발을 응원해 주세요.

[![Ko-fi에서 응원하기](https://img.shields.io/badge/Ko--fi-%EC%9D%91%EC%9B%90%ED%95%98%EA%B8%B0-FF5E5B?style=for-the-badge&logo=buymeacoffee&logoColor=white)](https://ko-fi.com/dev_ohyt)
