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

Python 판은 평문만 보냅니다. Rust 판과 대화하려면 Rust 쪽 `config.json`에 `"plaintext": true`를 설정해야 합니다.

## 액션

| 액션 | 기능 |
| --- | --- |
| `open` | 현재 pane 아래에 채팅 pane 열기 |
| `dock-start` | `herdr-dock` 워크스페이스에 대화 블록을 띄웁니다 |
| `dock-stop` | 도크 블록을 숨깁니다 |

herdr pane 안에서 액션을 실행해야 합니다. 직접 pane을 여는 방법, 채팅 명령, 네트워크 요구 사항은 [사용법](docs/usage.ko.md)을 참고하세요.

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
