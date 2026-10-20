# 사용법

설치 방법, 채팅, 각 명령의 동작, 이름과 언어 설정을 설명합니다.

## 1. 설치

GitHub에서 설치합니다:

```sh
herdr plugin install vrwolf1004/herdr-talk
```

요구 사항: herdr 0.9.0 이상, Python 3, 두 PC가 같은 네트워크에 있어야 합니다. 플러그인은 Python 표준 라이브러리만 씁니다.

## 2. 채팅 시작

각 PC에서 herdr로 채팅 pane을 엽니다:

```sh
herdr plugin pane open --plugin herdr-talk --entrypoint chat-windows   # Windows
herdr plugin pane open --plugin herdr-talk --entrypoint chat           # Linux, macOS
```

pane이 시작되면 고른 이름과 TCP 포트를 보여 줍니다:

```text
herdr-talk: you are 'Haruto' (TCP port 52311). Type /help for commands.
```

같은 네트워크의 다른 채팅 pane은 처음 발견될 때 `joined` 줄을 보여 줍니다:

```text
* Liam joined (172.31.99.181)
```

## 3. 메시지 보내기

줄을 입력하고 Enter를 누릅니다. 메시지는 최근 10초 안에 보인 모든 상대에게 갑니다. 내 메시지에는 이름 뒤에 `(you)`가 붙습니다:

```text
Liam: hi, anyone here?
Haruto (you): hello!
```

메시지는 저장되지 않습니다. 보낼 때 오프라인이던 상대에게는 나중에 전달되지 않습니다.

제한:

- 메시지는 2000자에서 잘리고, 줄바꿈은 공백으로 바뀝니다.
- 메시지 목록은 마지막 2000줄을 보관합니다.

## 4. 명령

| 입력 | 동작 |
|------|------|
| `<text>` | 모든 상대에게 보냅니다 |
| `/name <new>` | 내 이름을 바꿉니다 (최대 32자) |
| `/lang <code>` | 그 언어의 이름 목록에서 새 이름을 무작위로 고릅니다 |
| `/peers` | 최근에 보인 상대를 주소와 포트와 함께 보여 줍니다 |
| `/help` | 명령과 단축키를 보여 줍니다. 기본 화면에서는 상자로, 전체 화면에서는 한 줄로 나옵니다 (아무 키나 누르면 상자가 닫힙니다). |
| `/quit` | 채팅을 나갑니다 |

`/lang`에 쓰는 언어 코드: `ko en zh ja vi fr de es ru id pt`.

`/lang`과 `/name`은 표시 이름만 바꿉니다. 상대는 약 2초 뒤 다음 발견 메시지에서 새 이름을 봅니다.

## 5. 단축키

기본 화면(`prompt_toolkit`이 설치되지 않은 경우, 기본 설정)에서:

| 키 | 동작 |
|----|------|
| Enter | 줄을 보내거나 명령을 실행합니다 |
| Backspace | 글자 하나를 지웁니다 |
| Ctrl+L | 화면의 메시지 목록을 지웁니다 |
| Ctrl+C, Ctrl+D | 종료합니다 |

`prompt_toolkit`이 설치되어 있으면 전체 화면으로 동작합니다. 이때 **Ctrl+N**을 누르면 입력줄에 `/name `이 채워져 새 이름을 바로 입력할 수 있고, **Ctrl+C** 또는 **Ctrl+D**로 종료합니다.

## 6. 이름

`--name`을 주지 않으면, 채팅은 PC의 언어에 맞는 목록에서 이름을 무작위로 고릅니다:

- 한국어로 설정된 PC는 민준, 서연 같은 한국 이름을 받습니다.
- 영어로 설정된 PC는 Liam, Olivia 같은 영어 이름을 받습니다.
- 목록에 없는 언어는 모든 목록에서 고릅니다.

이름의 중복은 확인하지 않으므로 두 PC가 우연히 같은 이름을 가질 수 있습니다. 바꾸려면 `/name`을 쓰세요.

스크립트를 직접 실행할 때 이름을 정하려면:

```sh
python -X utf8 src/talk.py run --name my-laptop
```

## 7. 나가기

- `/quit`은 언제든 채팅을 나갑니다.
- 채팅 활동이 10분 동안 없으면 자동으로 나갑니다. 전체 화면에서는 메시지나 명령을 입력하면 활동으로 칩니다.
- 기본 화면에서는 아직 입력이 활동 타이머를 갱신하지 않습니다. 그래서 시작 후 10분이 지나면 대화 중이어도 나갑니다. 알려진 문제이며 [TODO](todo.md)에 있습니다.

## 8. 언어

채팅의 고정 문구(메시지, `/help` 상자, 상태 줄)는 언어 설정을 따릅니다. 설정은 다음 순서로 정해집니다:

1. `HERDR_TALK_LANG` 환경 변수
2. `config.json`의 `"lang"` ([설정](configuration.md) 참고)
3. OS 언어

지원 언어: `en`, `ko`, `ru`, `fr`, `es`, `de`, `ja`. 그 밖의 언어는 영어로 나옵니다. 명령 이름(`/name`, `/lang`, `Ctrl+N`)은 모든 언어에서 같습니다.

## 9. 네트워크

- 두 PC가 같은 네트워크에 있어야 하고, 클라이언트 격리(client isolation)가 없어야 합니다.
- 탐색은 UDP `47850` 포트를 쓰고, 메시지는 채팅마다 시작할 때 고르는 임의의 TCP 포트를 씁니다.
- 방화벽에서 둘 다 허용하세요. 상대 목록이 계속 비어 있으면 방화벽부터 확인하세요.
- 이 단계에서는 암호화하지 않습니다. 신뢰할 수 있는 네트워크에서만 쓰세요.

## 10. 문제 해결

증상과 확인 방법은 [문제 해결](troubleshooting.md)에 있습니다.

## 문서

- [Usage (English)](usage.md)
- [설정](configuration.md): 설정 값, 언어 결정 순서, 시간 상수 (영문)
- [개발](development.md): 코드 구조, 언어·이름·문구·명령 추가 방법 (영문)
- [문제 해결](troubleshooting.md): 증상별 확인 방법 (영문)
- [할 일](todo.md): 개선 목록 (영문)
