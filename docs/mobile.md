# 모바일 (Expo Go) 방 채팅

앱은 Supabase 방 채팅만 한다. PC 브리지(`mobile-bridge/`)와 LAN 연결은 없다. 폰은 PC 노드의 피어가 아니고, 서버의 방을 통해 다른 참가자와 주고받는다. 실행 방법은 [mobile/README.md](../mobile/README.md) 를 본다.

## 구조

```
폰 (Expo Go) --HTTPS--> Supabase Edge Functions (join-room, post-message, fetch-messages) --> Postgres
```

- 방 ID 와 trust 코드로 `join-room` 에 참가하고, 응답의 `owner_pk` 와 `members` 를 화면에 보여 준다.
- 보낼 때는 방의 다른 멤버마다 Noise 로 따로 암호화해 `post-message` 한 번에 묶어 보낸다. 서명과 형식은 PC 와 같다 (`herdr-talk-msg-v1`).
- 받을 때는 3초마다 `fetch-messages` 를 호출하고, 발신자와 nonce 를 검사한 뒤 화면에 보여 준다. 중복은 nonce 캐시로 걸러낸다.

## Supabase 방 상태

Supabase는 1:1 상대 ID 릴레이가 아니라 1대다 방 모델입니다. 서버가 무작위 8자리 방 ID를 만들고, 다른 참가자가 그 코드를 입력해 방에 참가합니다. 방 ID는 `A-Z0-9` 8자리이고 입력에서 대소문자·공백·대시를 무시합니다. 레거시 상대 ID는 서명된 peer 행 게시용으로만 남습니다.

방은 생성 후 24시간만 사용 가능하고 연장할 수 없으며, 만료 방 참가 요청은 `410`입니다. 만료와 pg_cron 정리는 구현됐지만 미배포입니다.

방 RLS 전체 공개와 create/join 서명 검증 부재는 미해결이며 배포 전 해결 필요입니다. 모바일의 Supabase 실제 동작은 미확인입니다.

## 확인하지 못한 것

- 실기기 화면과 Expo Go 실행.
- 실제 Supabase 방에서 두 폰 이상의 join, 보내기, 받기.
- 한국어 상태 줄 (현재는 `peers:` 영문 형식).

## 관련 문서

- [supabase-relay.md](supabase-relay.md): 방 모델, 배포 절차
- [room-messages-design.md](room-messages-design.md): 방 메시지 설계
- [encryption-protocol.md](encryption-protocol.md): 암호화 프로토콜
- [configuration.md](configuration.md): 설정 항목
