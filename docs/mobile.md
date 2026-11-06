# 모바일 (Expo Go) 연결

작성: 2026-11-06. 개발은 2026-11-06 에 마무리했습니다 (dev 종료 후 최종 확인: `cargo test` 95개, Python 113개, GUI 릴리스 빌드 통과). 상태는 PC 에서 직접 확인한 것과 확인하지 못한 것을 나눠 적었습니다. 폰 화면은 볼 수 없어서 사용자 확인으로 남긴 항목이 있습니다. 코드와 문서는 아직 커밋하지 않았습니다.

## 구조

```
폰 (Expo Go) --HTTP--> mobile-bridge/bridge.js --127.0.0.1 제어 채널--> PC 의 herdr-talk 노드 --UDP 발견 + TCP--> 다른 노드 (P2P)
```

- PC 사이는 P2P 입니다: UDP 브로드캐스트 (포트 `47850`) 로 상대를 찾고 TCP 로 직접 보냅니다. 서버 없음, 메시지 저장 없음, 같은 LAN 에서만 동작.
- 폰은 피어가 아닙니다. 메시지는 PC 노드를 거쳐 나갑니다 (`mobile/README.md`).
- 브리지는 `%LOCALAPPDATA%\herdr-talk\control-<pid>.json` (포트, 토큰) 로 노드의 제어 채널에 붙어 `lines`, `peers`, `send` 를 중계합니다.

## 실행 순서

1. PC: herdr-talk 노드 실행 (`herdr-talk serve` 또는 `run`).
2. PC: `node mobile-bridge/bridge.js`. 콘솔에 `token` 과 `app address` 가 나옵니다.
3. PC: `mobile` 에서 `npx expo start --host lan` (QR 이 나옵니다).
4. 폰 (같은 Wi-Fi): Expo Go 로 QR 스캔 또는 `exp://<PC IP>:<포트>` 입력.
5. 폰 앱 첫 화면에 `app address` (예: `http://192.168.0.181:47860`) 와 `token` 입력.

주의:
- 어댑터가 여러 개인 PC 에서는 `app address` 가 여러 줄 나옵니다. 폰이 속한 Wi-Fi 의 주소를 쓰세요 (가상 어댑터 `192.168.56.1` 은 폰에서 안 닿습니다).
- `CI=1` 로 Metro 를 띄우면 QR 과 리로드가 꺼집니다. 일반 터미널에서 `npx expo start --host lan` 으로 실행하세요.
- 포트 8081 이 쓰이고 있으면 Expo 가 8082 로 올라갑니다. QR 이나 `exp://` 주소의 포트를 확인하세요.

## 확인된 것 (2026-11-06, 같은 PC)

- `mobile` 의존성 설치, `tsc --noEmit`, `expo lint`, `expo-doctor` 20/20 통과.
- 폰이 Metro 에서 번들을 받고 브리지에 연결됨.
- 폰 → PC: 폰에서 보낸 메시지가 PC 노드에 도착. PC → 폰: PC 노드가 보낸 메시지가 폰에 표시됨 (사용자 확인).
- 같은 PC 의 Rust 노드끼리 서로 피어로 보임 (아래 "같은 PC 에서 서로 안 보이는 문제" 수정 후).

## 같은 PC 에서 서로 안 보이는 문제

- 증상: 같은 PC·같은 상태 폴더에서 `serve` 와 `run` 을 띄우면 서로 피어로 안 보였습니다.
- 원인 (dev 가 재현으로 확정): `net.rs` 의 `is_local_address` 가 기본 경로 주소 하나만 로컬로 봤습니다. 이 PC 는 어댑터가 둘 (예: Wi-Fi `192.168.0.181`, VirtualBox `192.168.56.1`) 이고 `255.255.255.255` 브로드캐스트는 `192.168.56.1` 로 나가서, 같은 키 공지가 "로컬 주소에서 온 것" 으로 인정되지 않아 버려졌습니다.
- 수정: 루프백 또는 이 PC 어느 어댑터 주소든 (UDP bind 가능하면) 로컬로 판정합니다. 외부 IP 는 bind 가 실패하므로 여전히 거부합니다. 보안 수준은 같습니다.
- 같은 수정으로, 같은 IP 에서 다른 신원이 동시에 떠 있을 때 나던 "키가 바뀌었습니다" 오탐도 줄였습니다 (신뢰한 키가 지금도 그 IP 에서 공지 중이면 다른 신원으로 봄).
- 새 키는 어느 경우든 `/trust` 전까지 신뢰되지 않습니다.
- 실행 중인 `herdr-talk.exe` 는 구버전일 수 있으니 재빌드·재시작이 필요합니다.

## 암호화 범위

| 구간 | 암호화 |
|------|--------|
| Rust ↔ Rust (PC 사이) | 됨 (Noise). 확인된 상대와만. 같은 키는 신뢰 |
| Python ↔ 다른 노드 | 됨. Python 판에도 같은 Noise 암호화를 추가함 (`python/src/noise.py`, [encryption-protocol.md](encryption-protocol.md)). `plaintext` 설정은 키 없는 옛 Python 과 대화할 때만 필요 |
| 폰 ↔ 브리지 | 안 됨. 일반 HTTP 라서 같은 Wi-Fi 에서 토큰·메시지가 평문. 믿을 수 있는 네트워크에서만 사용 |
| 브리지 ↔ PC 노드 | 같은 PC (`127.0.0.1`). 네트워크로 나가지 않음. 토큰 인증 |

- 브리지 토큰은 6자리 숫자입니다. 무작위 대입을 막으려고 IP 별 연속 실패 5회 → 10분 차단이 걸려 있습니다.
- 처음 보는 상대는 `확인 필요. 코드: …` 가 뜹니다. 두 화면의 코드가 같은지 비교한 뒤 `/trust <코드>` 를 입력합니다. 한쪽에서만 하면 반대쪽은 계속 막혀 있으니 양쪽에서 입력하세요.

## Python 판

- 제어 채널 (`control-<pid>.json`, `lines`/`peers`/`send`/`shutdown`, Rust 와 같은 형식) 과 `serve` 서브커맨드가 있어서 브리지가 Python 노드에도 붙습니다.
- 암호화: 표준 라이브러리만으로 `Noise_IK_25519_ChaChaPoly_BLAKE2s` 를 구현했습니다 (X25519, ChaCha20-Poly1305 는 순수 Python). RFC 7748, RFC 8439, Cacophony 공개 벡터로 검증했습니다. 한계: 상수 시간이 아니라서 같은 PC 의 다른 프로세스가 타이밍을 재면 키가 새어 나갈 수 있습니다 (네트워크 공격자는 해당 없음). 필요하면 `cryptography` 로 교체할 수 있습니다.
- 신원 키와 신뢰 목록 (`identity.json`, `trusted.json`) 이 Rust 와 같은 위치·형식이라 같은 상태 폴더를 공유하면 같은 키로 시작해 `/trust` 없이 대화합니다.
- 확인 (2026-11-06, 같은 PC): Python 노드가 브리지 피어로 보이고, Rust → Python, Python → Rust 메시지가 양쪽에 도착함. 상태 폴더가 다른 Rust ↔ Python 은 dev 가 코드 비교와 `/trust` 후 양방향 도착을 확인함.
- 방장 규칙도 Rust 와 같습니다 (Python `run` 에서 방장 인계와 폰 주소·토큰 패널 확인).

## Rust GUI

- `gui/` 는 `talk-core` 를 공유합니다. 소스 수정 없이 새 `talk-core` 로 재빌드해서 위 수정이 포함됩니다 (`cargo build --release`, 결과 `gui\target\release\herdr-talk-gui.exe`).
- 창 기동과 응답은 확인했습니다. 버튼 동작은 직접 눌러 확인한 것이 아닙니다.

## 방장, 폰 주소, 토큰 (구현됨)

- **방장**: 같은 PC (같은 상태 폴더) 에서 `phone.lock` 을 잡은 노드입니다. OS 파일 잠금이라 동시에 시작해도 하나만 되고, 프로세스가 죽으면 OS 가 풀어 줍니다. Rust 와 Python 이 같은 잠금을 씁니다.
- **`phone.json`** (본인만 읽기): `{port, token, urls, owner_pid, owner_port}`. 토큰은 6자리 숫자 (000000~999999, 암호학적 난수). 주소 목록은 기본 경로 어댑터가 먼저이고 사설 대역, 가상 어댑터 (`192.168.56.x`, 링크로컬, Docker) 는 뒤입니다. 네트워크로 보내거나 공지에 싣지 않습니다.
- **이름과 배지**: 첫 노드는 이름이 "방장" (언어별: Host, Gastgeber, ホスト, …) 입니다. `--name` 을 주면 그 이름을 쓰고 상태줄 앞에 `[방장]` 배지가 붙습니다 (Rust TUI, Python, GUI 상단 바). 나중에 시작한 노드는 임의 이름입니다.
- **인계**: 방장이 종료되면 기다리던 노드가 2초 안에 잠금을 잡고 토큰·포트를 그대로 이어받습니다. 이름은 바뀌지 않고 배지만 옮겨 갑니다. 마지막 노드가 나가면 `phone.json` 을 지웁니다. 크래시 뒤에는 다음 방장이 시작할 때까지 파일이 남습니다.
- **화면 표시**: 채팅 줄이 아니라 화면 하단 덮어쓰기 패널입니다 (기록·로그·피어 전송에 안 들어감). 자신이 방장이 될 때, 같은 PC 의 방장이 입장할 때 (`phone.json` 의 `owner_port` 와 로컬 주소 공지가 일치) 자동으로 한 번 뜹니다. 첫 입력·`Ctrl+P` 다시 누르기·30초 뒤에 숨습니다. 방장이 아닌 노드도 `Ctrl+P` 또는 `/phone` 으로 볼 수 있습니다. `/phone new` 는 토큰을 새로 만듭니다 (이전 토큰은 즉시 401).
- **브리지**: 포트와 토큰을 환경변수 → `phone.json` → (없으면 무작위 생성해 콘솔 출력) 순으로 씁니다. `phone.json` 의 토큰은 콘솔에 찍지 않고, 요청마다 다시 읽어서 `/phone new` 가 바로 반영됩니다. IP 별 연속 실패 5회 → 10분 차단은 그대로입니다. 브리지는 아직 사용자가 `node mobile-bridge/bridge.js` 로 직접 실행합니다.
- **Rust GUI**: 자기 전용 숨은 `serve` 를 띄웁니다. 다른 노드가 없으면 그 `serve` 가 방장이 됩니다 (`phone.json` 의 `owner_pid` 부모가 GUI 임을 확인). `Ctrl+P`, `/phone`, 배지가 있습니다.

## 확인한 것 (2026-11-06, 같은 PC)

| 항목 | 결과 |
|------|------|
| `cargo test` | talk-core 81, herdr-talk 14 통과 |
| Python `unittest` | 113 통과 |
| Rust GUI | `cargo build --release` 와 `cargo check` 통과, 실행해서 창 기동과 응답 확인 (방장이 되는 것까지 `phone.json` 소유자로 확인) |
| 첫 노드의 이름과 방장 | "방장", `phone.json` 소유 확인 |
| 나중 노드 (Rust 채팅) | 임의 이름, `방장 입장` 뒤에 주소·토큰 패널 표시 확인 (pane 화면 읽기) |
| 방장 인계 | Rust → Python, Python 종료 후 `phone.json` 삭제 확인 |
| 브리지 | `phone.json` 토큰으로 `/state` 200, 틀린 토큰 401, `/phone new` 후 옛 토큰 401 |
| 폰 ↔ PC | 양방향 도착 (폰 화면 표시는 사용자 확인) |

## 결정 사항 (사용자)

- 1차 목표는 연결, 2차는 방장·폰 주소·토큰. 둘 다 구현했습니다.
- 폰을 진짜 P2P 피어로 만드는 것은 지금 안 합니다. Expo Go 로는 불가 (UDP/TCP 소켓은 네이티브 모듈이라 개발 빌드가 필요). 기술적으로는 가능하지만 Noise 구현, Android 멀티캐스트 권한, 백그라운드 제한 대응이 필요한 별도 프로젝트입니다.

## 미확인 / 남은 일

- 실제 화면 모양: 배지 모양·위치, `Ctrl+P`·아무 키 숨김, 30초 자동 숨김, GUI 자동 표시.
- GUI 가 붙어 있던 노드가 사라지면 GUI 가 같이 사라지는 현상 (크래시 이벤트 없음). dev 확인 필요.
- 패널이 "입장 줄 바로 아래" 가 아니라 하단 덮어쓰기라는 점 (기록에 남기지 않기 위해). 줄 바로 아래가 필요한지 결정.
- 서로 다른 상태 폴더의 Rust 노드 `/trust` 흐름, 서로 다른 PC 사이 통신 (상태 폴더가 다르면 다른 PC 로 취급).
- 폰 구간 암호화 (HTTPS 또는 보안 터널), 방장이 브리지를 자동 실행하는 것.
- 수정한 코드와 문서 커밋.

## 관련 문서

- [encryption-protocol.md](encryption-protocol.md): 암호화 프로토콜
- [troubleshooting.md](troubleshooting.md): 증상별 확인
- [configuration.md](configuration.md): 제어 채널, 평문 설정
- [todo.md](todo.md): 할 일 목록
