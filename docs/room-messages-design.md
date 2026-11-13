# 방 메시지 전송 설계

- 상태: **설계만.** 코드·마이그레이션·DB 는 바뀌지 않았다. 구현은 사용자 결정 뒤에 단계별로 한다.
- 관련: [supabase-relay.md](supabase-relay.md), [encryption-protocol.md](encryption-protocol.md), [todo.md](todo.md) 의 "Supabase 방 기능" 항목.
- 작성: 2026-11-13 (dev). 확인하지 못한 것은 본문에서 **미확인** 으로 표시했다.

## 1. 목표와 범위

- 목표: Supabase 모드에서 방 멤버가 보낸 글을 **같은 방의 멤버 전원이 받는다** (1대다).
- 서버는 평문을 보지 않는다. 서버에 저장되는 것은 암호문과 메타데이터(누가, 어느 방, 언제) 뿐이다.
- 범위 안: 텍스트 메시지 송수신, 재전송 방지, `/trust` 확인된 멤버만 대상, 미확인 멤버 표시.
- 범위 밖: 파일 전송, 메시지 기록 조회 UI (과거 메시지 불러오기), 방 멤버 제거, 방 키 교체.

## 2. 현재 상태

- `talk-core/src/relay_chat/room_relay.rs` 의 `join_room_and_listen` 은 방에 참가하고 `rx` 로 받은 글을 `"Would send to room: …"` 로 **화면에만** 찍는다 (TODO 위치: 같은 파일 송신 루프 안). 서버로는 보내지 않는다.
- 받는 쪽은 아무것도 하지 않는다. 방 메시지를 읽는 코드는 없다.
- `supabase/migrations/0003_rooms.sql` 의 `room_messages` 는 `(id, room_code, sender_pk, content, created_at)` 한 칸짜리 `content` 이다. `content` 에 무엇을 넣을지 정해지지 않았다 (암호문으로 쓰기로 한 적 없음).
- `join-room` 은 방 멤버 목록 (`members`, 공개키 배열) 을 돌려준다. 방 기한 24시간과 410 응답이 있다 (0005).
- `relay_replay.rs` 의 `NonceCache` 는 방 relay 에 연결되어 있으나 방 메시지 검사에는 쓰이지 않는다 (`_cache` 인자로만 전달됨).
- 미확인: 실제 DB 의 `room_messages` 정책과 권한 (0006 은 읽기 권한을 닫는 파일이고 실행 여부는 사람이 확인해야 한다).

## 3. 암호화 방식

공통 전제: 각 멤버의 Noise 정적 키 (X25519) 는 `/trust` 로 확인된 것만 쓴다. 서버가 준 값은 믿지 않는다.

### 선택지 A: 멤버마다 1:1 Noise 암호화 (fan-out)

- 보내는 쪽은 확인된 멤버 수만큼 같은 글을 각 멤버용 Noise IK 메시지로 따로 암호화한다. 기존 `talk-core` 의 1:1 암호화 (`seal_payload`, 페이로드 v1) 를 그대로 쓴다.
- 장점: 기존 `/trust` 키와 페이로드 형식을 그대로 쓴다. 멤버 추가·삭제·키 교체가 쉽다 (그 멤버 하나만 다루면 된다). 키 배포 규칙이 없다.
- 단점: 메시지 1개당 암호문이 멤버 수만큼 생긴다. 서버 저장량과 보내기 시간이 멤버 수에 비례한다. 최대 8명 (`docs/multi-peer-relay-plan.md` 의 상한 제안) 이면 실용적이다.

### 선택지 B: 방 공용 키 (그룹 키)

- 방마다 대칭 키를 만들어 모든 멤버에게 배포하고 그 키로 한 번만 암호화한다.
- 장점: 암호문이 1개다. 저장과 전송이 가볍다.
- 단점: 키 배포 (누가 누구에게 키를 주나, 키를 어떻게 서로 확인하나) 가 필요하다. 멤버가 나가거나 들어올 때마다 키를 바꿔야 한다 (나간 사람이 옛 키로 계속 읽는 것을 막기 위해). 이 키 교환도 `/trust` 로 확인해야 하므로 SAS 와 얽힌다. 설계와 검증 부담이 크다.

### 추천: A

- 이유: 지금 있는 `/trust`, 1:1 암호화, 테스트 벡터를 그대로 쓸 수 있다. 방 메시지 기능을 가장 작게 시작할 수 있다. 멤버 수 상한 (8명) 에서는 비용이 작다. B 는 멤버 변동이 잦아질 때 다시 검토한다.

## 4. room_messages 저장 형식

### 선택지 1: 멤버별 행

- 메시지 1개가 멤버 수만큼 행이 된다. 각 행은 `(message_id, recipient_pk, ciphertext)`.
- 장점: 받는 사람은 자기 행만 읽는다. 서버가 "누구에게 무엇이 갔나" 를 행 단위로 본다 (메타데이터 노출은 늘어난다).
- 단점: 행 수가 많다. 같은 메시지의 행을 묶는 키가 필요하다.

### 선택지 2: 한 행에 암호문 목록

- 메시지 1개가 한 행이고, `ciphertexts` 는 `{recipient_pk: 암호문}` 의 JSON 이다.
- 장점: 행이 1개라 저장과 삭제가 단순하다.
- 단점: 받는 사람이 모든 암호문을 내려받는다 (멤버 수 × 크기). 누가 받을 수 있는지 한 행에 다 보인다.

### 추천: 선택지 1 (멤버별 행)

- 받는 쪽이 필요한 행만 읽게 하고, 행 단위 RLS 로 "내 행만 조회" 를 걸 수 있다 (0006 의 방향과 맞는다). 행 수가 늘어나는 것은 멤버 수 상한으로 감당한다.

### 스키마 초안 (마이그레이션 아님, 검토용)

```sql
-- 초안. 실행하지 않는다. 0007 로 만들 때 0006 의 RLS 와 함께 검토한다.
create table if not exists public.room_message_deliveries (
  message_id uuid not null,                       -- 한 메시지의 모든 행을 묶는 값 (클라이언트가 만든 무작위 값)
  room_code text not null references public.rooms(code) on delete cascade,
  sender_pk text not null references public.peers(pk) on delete cascade,
  recipient_pk text not null references public.peers(pk) on delete cascade,
  ts bigint not null,                              -- 보낸 시각 (초), 재전송 창 검사용
  nonce text not null check (nonce ~ '^[0-9a-f]{32}$'),
  ciphertext text not null,                        -- hex, Noise 메시지 (서버는 해독하지 않는다)
  created_at timestamptz not null default now(),
  primary key (message_id, recipient_pk)
);
create index if not exists room_message_deliveries_recipient
  on public.room_message_deliveries (recipient_pk, created_at);
```

- 기존 `room_messages` 테이블은 건드리지 않고 새 테이블을 쓰는 안이다. 대체할지는 결정 항목이다.
- 보관 기간: 방과 같이 24시간 뒤 삭제 (`rooms` 가 cascade 로 지운다). 결정 항목.

## 5. /trust 적용

- 방 멤버 목록은 `join-room` 이 준 공개키다. 서버가 준 값이므로 그대로 믿지 않는다.
- 송신: 자기 `/trust` 목록에 있는 멤버 (그 키가 확인된 멤버) 에게만 보낸다.
- 수신: 보낸 사람의 키가 `/trust` 로 확인된 키일 때만 화면에 보인다. 확인되지 않은 키의 메시지는 버린다 (표시하지 않는다).
- 미확인 멤버 표시: 멤버 목록에서 미확인 멤버를 "미확인 (/trust 필요)" 로 보여 준다. 보낼 때는 "N명 중 M명에게 보냄, 미확인 K명 제외" 한 줄을 남긴다. 누구에게 안 갔는지 숨기지 않는다 (사용자가 빠진 것을 알 수 있어야 한다).
- SAS 와의 관계: `docs/todo.md` 의 "trust 코드 방식 (SAS 커밋)" 결정이 끝나야 `/trust` 의 확인 방식이 정해진다. 이 설계는 "`/trust` 로 확인된 키" 라는 말만 쓰므로 SAS 결정과 독립이다. 다만 8자 지문을 SAS 없이 쓰면 키를 골라 같은 코드를 만들 수 있으므로, SAS 가 정해지기 전에는 방 메시지를 열지 않는다 (권장).
- 미확인: 방 멤버 키를 `/trust` 한 적이 없는 상대에게 메시지를 열면 어떤 사용자 흐름이 되는지 (멤버 목록에서 바로 확인하는지) 는 화면 설계가 필요하다.

## 6. 재전송 방지와 순서

- 재전송 방지: 받는 쪽은 `relay_replay::NonceCache` 를 쓴다. 메시지 nonce 와 ts 를 검사한다 (창 300초, 같은 nonce 는 한 번만). 1:1 relay 와 같은 규칙이다.
- 재시작 시 캐시가 비므로 재시작 직후 창 안에서 옛 메시지를 다시 받을 수 있다 (`relay_replay.rs` 의 기존 한계, 그대로 둔다).
- 순서: 서버 `created_at` 과 클라이언트 `ts` 가 다를 수 있다. 화면에는 받은 순서대로 보이고, 같은 방 안에서 전역 순서는 보장하지 않는다. 같은 보낸 사람의 메시지는 `ts`, `nonce` 가 같은 `message_id` 로 묶이므로 중복 표시를 막는다.
- 미확인: 긴 연결 끊김 뒤 놓친 메시지를 다시 가져오는 방식 (`created_at > 마지막 본 시각` 조회) 은 정하지 않았다.

## 7. 구현 단계와 검증

| 단계 | 내용 | 검증 |
|---|---|---|
| 0 | 결정 항목 확정 (8번) | 사용자 답 기록 |
| 1 | talk-core: 방 송신 (fan-out, `/trust` 확인된 멤버만), 수신 (NonceCache, 발신자 키 검사) | talk-core 단위 테스트, 벡터 추가, `cargo test -p talk-core --locked` |
| 2 | Rust TUI: 방 메시지 표시, 미확인 멤버 줄, 송신 결과 한 줄 | `cargo test -p herdr-talk --locked`, 실제 화면은 사용자 확인 |
| 3 | GUI: 같은 표시 (gui/src/main.rs 는 qa 가 쓰는 중이라 조율 필요) | `cargo check --manifest-path gui/Cargo.toml`, 화면은 사용자 확인 |
| 4 | Python: 같은 동작 | `python -m unittest discover -s tests -p "test_*.py"` |
| 5 | 모바일: 방 생성·참가·멤버 목록·송수신 (1:1 정리와 함께) | `npm run test:relay` 와 실기기 (미확인) |
| 6 | 마이그레이션 (`room_message_deliveries`, RLS) 과 Edge Function 초안 검토 | 사람이 DB 에서 실행·확인 (에이전트는 실행하지 않는다) |

- 단계 1 이 나머지의 전제다. 단계 2·3·4 는 1 뒤에 병행할 수 있다.
- 각 단계마다 "두 PC 에서 실제로 보고 받기" 를 사람이 확인한다 (자동 테스트로 대신하지 않는다).

## 8. 결정 필요 (사용자)

1. 암호화 방식: A (멤버별 1:1 fan-out, 추천) 인가, B (방 공용 키) 인가.
2. 저장 형식: 멤버별 행 (추천) 인가, 한 행 암호문 목록인가. 새 테이블 `room_message_deliveries` 를 쓸지, `room_messages` 를 고쳐 쓸지.
3. 멤버 수 상한: 8명 (제안) 으로 할지.
4. 메시지 보관 기간: 방과 같이 24시간인지, 더 짧게 할지.
5. SAS 결정 (`docs/todo.md`) 이 끝날 때까지 방 메시지를 열지 않을지 (권장: 기다린다).
6. 미확인 멤버 처리: 보낼 때 제외하고 한 줄로 알릴지 (권장), 아니면 보내기를 막을지.
7. 구현 순서: Rust 먼저 (talk-core 허락이 필요한 TODO 포함), Python 과 모바일은 뒤로 미룰지.
8. 0006 (RLS 닫기) 와 service role 전환을 방 메시지 전에 끝낼지 (권장: 먼저).

DONE
만든 파일: docs/room-messages-design.md (새 파일 하나)
결정 필요 목록: 위 8번 항목 (암호화 방식, 저장 형식과 테이블, 상한 8명, 보관 기간, SAS 선행 여부, 미확인 멤버 처리, 구현 순서, 0006 선행 여부)
