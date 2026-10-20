# herdr-talk

[English](README.md) | [**한국어**](README.ko.md)

herdr 안에서 다른 PC의 사람과 채팅하는 플러그인입니다. 같은 네트워크에서는 서버 없이 바로 연결됩니다.

> 상태: 1차(같은 네트워크) 구현 단계입니다. 인터넷 너머 채팅(Supabase, 종단 간 암호화)은 2차로 계획되어 있습니다.

## 요구 사항

- herdr 0.9.0 이상
- Windows, Linux, macOS
- Python 3
- 추가 패키지는 필요 없습니다. 입력 줄은 pane 아래에 고정됩니다.
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

채팅 화면은 이렇게 보입니다. 다른 사람의 메시지에는 이름이, 내 메시지에는 `(you)`가 붙고, 입력 줄은 맨 아래에 있습니다:

```text
* 하준 joined (172.31.99.176)
도윤: 안녕하세요
하준 (you): 반갑습니다
>
```

채팅 명령어:

| 입력 | 하는 일 |
|------|---------|
| `<텍스트>` | 모든 상대에게 보내기 |
| `/name <새 이름>` | 내 이름 바꾸기 |
| `/lang <코드>` | 해당 언어의 이름 목록에서 무작위로 새 이름 고르기: `ko en zh ja vi fr de es ru id pt` |
| `/peers` | 최근에 보인 상대 목록 |
| `/help` | 명령어와 단축키 상자 보기 (아무 키나 누르면 닫힘) |
| `/quit` | 채팅 나가기 |

단축키: Backspace는 한 글자 지우기, Ctrl+L은 메시지 지우기, Ctrl+C는 종료입니다.

시작할 때 PC의 언어에 맞는 이름 목록에서 이름이 무작위로 정해집니다(한국어 PC는 한국 이름). 채팅에서 `/lang <코드>`로 다른 나라 이름을 고를 수 있습니다. 스크립트를 직접 실행할 때 이름을 정하려면 `--name`을 씁니다:

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

- [사용법](docs/usage.ko.md): 설치, 명령어, 단축키, 이름, 언어, 네트워크
- [Usage (English)](docs/usage.md)
- [설정](docs/configuration.md): 설정 값, 언어 결정 순서, 시간 상수 (영문)
- [문제 해결](docs/troubleshooting.md): 증상별 확인 방법 (영문)
- [개발](docs/development.md): 코드 구조, 언어·이름·문구·명령 추가 방법 (영문)
- [할 일](docs/todo.md): 개선 목록 (영문)

## 프로젝트 응원하기

프로젝트가 도움이 되셨다면 커피 한 잔으로 개발을 응원해 주세요.

[![Ko-fi에서 응원하기](https://img.shields.io/badge/Ko--fi-%EC%9D%91%EC%9B%90%ED%95%98%EA%B8%B0-FF5E5B?style=for-the-badge&logo=buymeacoffee&logoColor=white)](https://ko-fi.com/dev_ohyt)
