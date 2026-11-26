# herdr-talk mobile (Expo Go)

Android 에서 herdr-talk 방 채팅을 Expo Go 로 쓰는 앱. Supabase 설정 코드로 서버를 연결하고, 방 ID 와 trust 코드로 방에 참가한다. PC 브리지는 없다.

## 실행

1. 이 폴더에서 개발 서버를 켠다:
   ```sh
   npx expo start
   ```
2. 폰의 Expo Go 로 QR 을 읽는다.
3. 앱 화면의 `settings` 에서 설정 코드를 입력하고 `Save setup code` 를 누른다 (URL 과 키는 저장되고 화면에 보이지 않는다).
4. 표시 이름은 앱이 정한다. trust 코드(숫자 8자리)와 방 ID(8자리)를 입력하고 `join` 을 누른다. 새 방은 `+` 로 만든다.

## 참고

- 방 메시지는 방의 다른 멤버마다 따로 암호화해 서버에 보낸다. 서버는 평문을 보지 않는다.
- 상태 줄은 `peers: N · CLOUD SERVICE` 형식이다. 메시지는 3초마다 받는다.
- 설정 코드, 방 ID, trust 코드는 앱 설정에 저장된다. 설정 화면의 `clear` 는 저장된 설정을 지운다.
- 테스트: `npm run test:relay`. 타입 검사: `npx tsc --noEmit`.
- 실기기와 실제 Supabase 방에서의 동작은 이 문서 작성 시점에 확인하지 않았다. 설계는 [docs/supabase-relay.md](../docs/supabase-relay.md)와 [docs/room-messages-design.md](../docs/room-messages-design.md)를 본다.
