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

이 플러그인은 Agents 사이드바 블록 대신 채팅 pane을 사용합니다. 현재 구현은 같은 LAN에서만 동작하며 UDP 브로드캐스트로 상대를 찾고 TCP로 메시지를 직접 보냅니다. 메시지는 저장되거나 암호화되지 않으므로 신뢰할 수 있는 네트워크에서만 사용하세요.

## 설치

```sh
herdr plugin install vrwolf1004/herdr-talk
```

## 액션

| 액션 | 기능 |
| --- | --- |
| `open` | 현재 pane 아래에 채팅 pane 열기 (Linux/macOS) |
| `open-windows` | 현재 pane 아래에 채팅 pane 열기 (Windows) |

herdr pane 안에서 액션을 실행해야 합니다. 직접 pane을 여는 방법, 채팅 명령, 네트워크 요구 사항은 [사용법](docs/usage.ko.md)을 참고하세요.

## 요구 사항

- Windows, Linux, macOS
- herdr 0.9.0 이상
- Python 3, 표준 라이브러리만 사용
- 두 PC가 같은 네트워크에 있고 방화벽에서 탐색과 TCP 통신을 허용해야 함
- 선택 사항인 `prompt_toolkit`을 설치하면 전체 화면 채팅을 사용하며, 없으면 기본 터미널 화면을 사용

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
