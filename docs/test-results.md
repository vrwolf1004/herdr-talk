# 테스트 결과 (2026-11-19)

이번 작업 라운드의 최신 실행 결과입니다. 코드는 실행 시점 기준이며, 커밋과 배포는 하지 않았습니다.

## 1. 자동 테스트

| 영역 | 명령 | 결과 | 비고 |
|---|---|---|---|
| Rust 워크스페이스 (TUI) | `cargo test --workspace` | herdr-talk 54 통과 / talk-core 197 통과, 실패 0 | 빌드 경고 0 |
| Rust GUI | `cargo test` (gui/) | 17 통과, 실패 0 | 빌드 경고 0 |
| Python TUI | `python -m unittest discover -s tests` | 280 통과 (OK) | `-W error`에서도 280 통과 |
| Supabase `redeem-setup-code` | `node --test checks.test.ts` | 3 통과 | 설정 코드 형식, 실패 제한 |
| Supabase `join-room` | `node --test checks.test.ts` | 10 통과 | 서명, 형식, trust 형식 |
| Supabase `create-room` | `node --test checks.test.ts` | 10 통과 | `join-room`과 같은 파일 |
| Supabase `issue-setup-code` | `node --test checks.test.ts` | 3 통과 | 발급 요청 형식, 코드 문자, 암호 비교 |
| 모바일 | `npx tsc --noEmit` | 오류 0 | |
| 모바일 릴레이 | `npm run test:relay` | 28 통과, 실패 0 | |
| 모바일 린트 | eslint | 오류 0, 경고 2 | 기존 경고 (`relay.tsx` 의존성, `names.ts` 배열 타입) |

## 2. 확인하지 못한 항목

| 항목 | 상태 | 이유 |
|---|---|---|
| trust 403 → 설정 화면 복귀 (Rust) | 미확인 | `room_refuses`의 true 경로 테스트 없음 |
| trust 403 → 설정 화면 복귀 (Python) | 코드 읽기만 확인 | 테스트 기준으로만 확인 |
| trust 403 메시지 흐름 (GUI) | 미확인 | 테스트 없음 |
| 상태 줄 전체 문자열 `[방장] 상대: N · CLOUD SERVICE · room ID · setup code` | 미확인 (Rust, GUI) | 전체 문자열과 "오류만 붙는 규칙" 테스트 없음 |
| 중간 저장 F2 → 상환 → 저장 전체 경로 | 미확인 (Rust, GUI) | 네트워크 없음. Python은 mock으로 확인 |
| F9 운영자 발급 키 입력 | 미확인 (GUI) | 빈 암호만 테스트됨. Python과 Rust는 화면 로직만 테스트 |
| 실제 서버 응답 (403, 409, 발급) | 미확인 | 실환경 호출 없음 |
| 실제 터미널 화면 렌더링 | 미확인 | 창을 띄우지 않음 |
| 모바일 기기 실행 | 미확인 | 기기에서 실행하지 않음 |

## 3. 발견한 문제 (수정 전)

| 문제 | 영향 | 상태 |
|---|---|---|
| GUI 하단 바가 "relay: room joined" 문구만 숨김 | 다른 성공 문구가 그대로 보일 수 있음 | 미수정 |
| GUI에서 늦게 도착한 trust 확인 결과가 더 새로운 메시지를 덮어씀 | 메시지가 엉킬 수 있음 | 미수정 |
| Python `_join_room` 도우미의 성공 문구 | 실제 흐름에서는 쓰이지 않음 (테스트 전용) | 실제 상태 줄에는 영향 없음 |

## 4. 다음 단계

- trust 403 경로 테스트 추가 (Rust, GUI)
- 상태 줄 전체 문자열 테스트 추가 (Rust, GUI)
- 실환경에서 설정 코드 발급, 상환, 방 참가 확인
