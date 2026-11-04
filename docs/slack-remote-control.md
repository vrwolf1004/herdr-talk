# Slack으로 herdr 원격 제어 (검토 메모)

상태: 검토 단계. 구현하지 않았고, 아래 내용은 문서와 CLI 도움말을 읽고 정리한 것이다. 직접 만들거나 실행해 본 것은 없다.

## 배경

herdr 안에서 Slack이나 PC 원격 제어를 하려고 했다. 처음에는 경량 에이전트 런타임 zeroclaw(0.8.7)를 시도했다.

- 실행 실패의 원인은 zeroclaw 자체가 아니라 모델 provider 설정이었다.
  - `config.toml`의 `uri`에 API 키 조각이 잘못 들어가 있었다. 지우고 `model`을 `claude-sonnet-5-5`로 바꿨다.
  - 저장된 API 키가 거부됐다. 키를 새로 발급해 다시 넣었다.
  - 그 뒤 `Your credit balance is too low to access the Anthropic API`로 거부됐다. API 크레딧은 claude.ai Pro 구독과 별개라서 충전이 필요하다.
- 구독 토큰을 쓰는 `zeroclaw auth paste-token` 경로가 있지만 동작은 확인하지 못했다. 서드파티 도구에서 구독 토큰 사용이 약관상 허용되는지도 확인하지 못했다.
- zeroclaw는 삭제하는 쪽으로 기울었다. 대안으로 PicoClaw, NullClaw, NanoClaw, IronClaw, Nanobot, TinyClaw가 거론됐다(블로그 요약이라 검증되지 않음). 어느 것을 쓰든 모델 호출에는 API 키나 크레딧이 필요하다.
- Hermes는 크고 무겁다는 이유로 제외했다.

## herdr-talk로 가능한 것과 불가능한 것

herdr-talk는 같은 LAN 안의 사람끼리 하는 채팅 플러그인이다. UDP 브로드캐스트로 상대를 찾고 TCP로 직접 메시지를 보낸다.

- 인터넷 너머나 Slack, 폰으로는 연결되지 않는다.
- 채팅 메시지만 주고받는다. herdr pane을 조작하거나 명령을 실행하는 기능은 없다.
- 따라서 지금 상태로는 원격 제어가 되지 않는다.

## herdr의 제어 수단

`herdr --help`와 `herdr pane --help`로 확인한 제어용 CLI(소켓 API 기반)가 있다.

| 명령 | 기능 |
| --- | --- |
| `herdr pane list` / `current` / `get` | pane 목록과 상태 |
| `herdr pane read` | pane 출력 읽기 |
| `herdr pane send-text` | pane에 문자열 보내기 |
| `herdr pane send-keys` | pane에 키 입력 보내기 |
| `herdr pane wait-output` | 출력이 맞을 때까지 대기 |
| `herdr pane split` / `focus` / `close` 등 | pane 관리 |

`workspace`, `tab`, `agent`, `notification` 하위 명령도 있다. 원격 접속용으로 `--remote <ssh-target>`과 `--machine <label-or-id>` 옵션도 보인다. 이 옵션들은 사용해 보지 않았다.

## 구조 후보

### 후보 1. Slack 봇이 herdr CLI를 직접 호출 (권장)

```
Slack 메시지 -> (Socket Mode 봇, PC에서 실행) -> herdr pane send-text
herdr pane read -> 결과를 Slack 스레드에 답장
```

- Socket Mode는 PC에서 Slack으로 나가는 연결만 쓴다. 포트 개방과 공인 주소가 필요 없다.
- Python 스크립트 100줄 안팎으로 예상한다. 아직 만들지 않았다.
- herdr-talk를 거치지 않는다.

### 후보 2. herdr-talk 제어 채널에 외부 브리지를 붙임

`herdr-talk serve`(또는 `run`)가 `127.0.0.1`에 로컬 제어 채널을 연다. 자세한 규격은 [configuration.md](configuration.md#control-channel)에 있다.

- 주소는 `127.0.0.1`의 TCP 포트이고 다른 PC에서는 접근할 수 없다.
- 포트와 토큰은 상태 폴더의 `control-<pid>.json`에 있다. 토큰은 실행마다 새로 만든다.
- 요청은 한 줄에 JSON 하나다. 명령은 `lines`, `peers`, `send`, `shutdown`(`serve`만)이다.
- 제한은 동시 4 클라이언트, 요청 한 줄 64 KiB, 유휴 5분이다.

외부 브리지 스크립트가 이 채널의 `send`와 `lines`를 쓰면 herdr-talk 코드를 고치지 않고 붙일 수 있다. 다만 이 채널은 채팅 전용이다. pane 제어 기능은 없다. 이 후보는 Slack 메시지를 LAN의 다른 PC 사람에게 채팅으로 전달하고 싶을 때만 의미가 있다.

### 후보 3. herdr-talk 안에 Slack과 pane 제어를 직접 구현 (비권장)

- Rust판, Python판, GUI 세 곳을 고쳐야 한다.
- 채팅 플러그인에 원격 제어가 섞여 보안 경계가 흐려진다.

## 다른 선택지 (herdr와 무관)

- **Claude Tag**: Slack 안의 Claude다. Claude Code에서 `/install-slack-app`으로 설치한다. Slack에서 Claude에게 일을 시키는 방식이고 herdr 세션을 직접 조작하지 않는다. 요건과 요금제 조건은 확인하지 못했다.
- **Claude Code Remote Control**: 다른 기기에서 세션을 이어 쓰는 기능이다. 지원 범위는 확인하지 못했다.
- **computer use**: Claude 데스크톱 앱의 화면 제어 기능이다.

## 보안 요구사항

외부 채널에서 명령을 받으면 그 채널이 곧 PC의 원격 셸이 된다. 구현한다면 다음을 반드시 넣는다.

- 허용할 Slack 사용자 ID를 하나로 고정한다. 그 외 사용자의 메시지는 무시한다.
- 전용 채널이나 DM에서만 동작시킨다.
- 입력을 보낼 pane을 하나로 고정한다.
- 가능하면 실행 가능한 명령을 화이트리스트로 제한한다.
- Slack 봇 토큰과 앱 토큰은 설정 파일이나 환경 변수에 두고 저장소에 커밋하지 않는다.

## 열린 질문

- 목적이 "채팅 중계"인가 "pane 제어"인가? pane 제어라면 후보 1로 충분하고 herdr-talk는 필요 없다.
- Slack 앱 생성(봇 토큰, 앱 토큰 발급)은 사용자가 직접 해야 한다.
- 후보 1의 실제 동작(`send-text`로 에이전트 pane에 입력하고 `read`로 결과를 읽는 흐름)은 시험해 보지 않았다.
- Claude Tag, Remote Control의 요건과 요금제는 확인하지 못했다.
