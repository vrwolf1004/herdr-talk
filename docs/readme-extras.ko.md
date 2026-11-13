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

- **로컬/자체 (LAN)**: 기본값입니다. UDP 브로드캐스트로 상대를 찾고 보이는 모든 상대에게 보내므로, LAN 채팅은 다수 참가자 방식입니다.
- **Supabase**: 1대다 방 릴레이입니다. **방 만들기**를 누르면 서버가 무작위 8자리 방 ID를 만들고, 그 코드를 전달받은 사람이 참가합니다. **폐기됨:** 상대 ID 기반 1:1 릴레이.
- 방 ID는 `A-Z0-9` 8자리이며 입력에서는 대소문자·공백·대시를 무시합니다. 레거시 상대 ID는 서명된 peer 행 게시용으로만 남고 방 참가 입력은 아닙니다. trust 코드도 8자리이고 SAS 커밋 방식은 설계 확정·구현 대기입니다.
- 방은 생성 후 24시간만 사용 가능하고 연장할 수 없습니다. 만료 방 참가 요청은 `410`이며, 만료 정리는 구현됨·미배포 상태입니다.
- **배포 전 해결 필요:** 방 RLS 전체 공개와 create/join 서명 검증 부재는 아직 미해결입니다.

현재 모델과 미확인 항목은 [Supabase 릴레이](docs/supabase-relay.md)를 참고하세요.

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

