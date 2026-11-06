# herdr-talk mobile (Expo Go)

Android 에서 herdr-talk 채팅을 Expo Go 로 쓰는 앱. PC 의 `herdr-talk serve` 에 `mobile-bridge/bridge.js` 가 붙고, 앱은 그 브리지에 HTTP 로 붙는다.

## 실행

1. PC 에서 `herdr-talk serve` (또는 `run`) 를 켠다.
2. PC 에서 브리지를 켠다:
   ```sh
   node ../mobile-bridge/bridge.js
   ```
   창에 `token` 과 `app address` 가 나온다.
3. 이 폴더에서 개발 서버를 켠다:
   ```sh
   npx expo start
   ```
4. 폰 (같은 Wi-Fi) 의 Expo Go 로 QR 을 읽는다.
5. 앱 첫 화면에 `app address` (예: `http://192.168.0.10:47860`) 와 `token` 을 입력하고 연결.

## 참고

- 브리지는 토큰 없이 답하지 않는다. 같은 Wi-Fi 에서만 쓴다. 토큰은 브리지 창에만 보인다.
- 포트는 `TALK_BRIDGE_PORT`, 토큰은 `TALK_BRIDGE_TOKEN` 환경변수로 고정할 수 있다.
- 폰은 herdr-talk 피어로 직접 참여하지 않는다. 메시지는 PC 노드를 통해 나간다.
- 테마 (Dark / Light / Amber / Green) 와 CRT (베젤만, 스캔라인 없음) 는 GUI 의 기본값 (Green, CRT 켜짐) 을 따른다.
