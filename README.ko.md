# herdr-talk

[English](README.md) | [**한국어**](README.ko.md)

herdr 안에서 다른 PC의 사람과 채팅하는 플러그인입니다. 같은 네트워크에서는 서버 없이 바로 연결됩니다.

> 상태: 1차(같은 네트워크) 구현 단계입니다. 인터넷 너머 채팅(Supabase, 종단 간 암호화)은 2차로 계획되어 있습니다.

## 요구 사항

- herdr 0.9.0 이상
- Windows, Linux, macOS
- Python 3
- 선택: 입력 줄을 화면 아래에 고정하려면 `pip install prompt_toolkit`을 설치합니다. 없어도 채팅은 되고, 입력 줄은 일반 `> ` 프롬프트로 나옵니다.
- 두 PC가 같은 네트워크에 있고, 방화벽이 탐색과 메시지 포트를 막지 않아야 합니다

## 빠른 시작

1. 두 PC에 플러그인을 설치합니다:

   ```sh
   herdr plugin install vrwolf1004/herdr-talk
   ```

2. 각 PC에서 채팅 pane을 엽니다:

   ```sh
   herdr plugin pane open --plugin herdr-talk --entrypoint chat-windows   # Windows
   herdr plugin pane open --plugin herdr-talk --entrypoint chat           # Linux, macOS
   ```

3. 메시지를 입력하고 Enter를 누르면 같은 네트워크의 모든 상대에게 갑니다.

채팅 명령어:

| 입력 | 하는 일 |
|------|---------|
| `<텍스트>` | 모든 상대에게 보내기 |
| `/peers` | 최근에 보인 상대 목록 |
| `/help` | 명령어 보기 |
| `/quit` | 채팅 나가기 |

시작할 때 이름이 표시됩니다. 스크립트를 직접 실행할 때는 `--name`으로 바꿀 수 있습니다:

```sh
python -X utf8 src/talk.py run --name my-laptop
```

## 동작 방식

- 각 채팅 프로그램은 2초마다 UDP `47850` 포트로 자기 이름과 TCP 포트를 알립니다.
- 같은 네트워크의 다른 프로그램은 그 정보를 받아 상대 목록에 넣습니다.
- 메시지는 상대에게 TCP로 바로 보냅니다. 어디에도 저장하지 않습니다.

## 한계

- 같은 네트워크(같은 공유기, 기기 간 통신 허용)에서만 동작합니다.
- 메시지를 저장하지 않으므로, 꺼져 있는 상대는 메시지를 받지 못합니다.
- 1차에는 암호화가 없습니다. 믿을 수 있는 네트워크에서만 쓰세요.

## 문서

- [사용법](docs/usage.md)
- [개발](docs/development.md)
