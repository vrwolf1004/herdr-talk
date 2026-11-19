# 운영자 가이드: Supabase 설정과 설정 코드 발급

이 문서는 herdr-talk 의 Supabase 방 기능을 운영하는 사람을 위한 것이다. 사용자용 설명은 [사용법](usage.ko.md)을 본다.

> **이 문서에 적힌 것은 저장소의 파일을 읽어 확인한 내용이다.** 실제 Supabase 프로젝트에 적용됐는지, 함수가 배포됐는지는 확인하지 않았다. 아래 `미확인` 표시가 그런 항목이다.

## 0. 쓰지 않는 것

- 비밀 값(암호, publishable 키, service role 키, 토큰)은 이 문서나 저장소, 채팅, 로그에 쓰지 않는다.
- 이 문서는 **시크릿 이름**만 적는다: `OPERATOR_PASSPHRASE`, `APP_PUBLISHABLE_KEY`.
- 예시의 `<...>` 자리에는 직접 넣은 값을 쓰고, 파일에 저장하지 않는다.

## 1. 마이그레이션 (SQL 편집기에서 한 번씩 실행)

마이그레이션은 `supabase/migrations/` 에 있다. Supabase 대시보드의 **SQL Editor** 에 파일 내용을 붙여 넣고 실행한다. `supabase db push` 는 운영 프로젝트에 쓰지 않는다.

| 순서 | 파일 | 목적 |
|---|---|---|
| 1 | `0009_room_trust.sql` | 방에 trust 코드 칸을 추가한다 (`trust_salt`, `trust_hash`, `trust_fails`, `trust_locked_until`). 코드 자체는 저장하지 않고 PBKDF2 해시만 저장한다. |
| 2 | `0010_setup_codes.sql` | 설정 코드 표(`setup_codes`)와 실패 기록 표(`setup_code_failures`)를 만든다. 두 표는 anon 과 authenticated 에 아무 권한도 주지 않는다. |
| 3 | `0011_setup_codes_multi_use.sql` | 설정 코드를 여러 사람이 쓰도록 `max_uses`, `use_count` 를 추가하고, 한 번에 하나만 차감하는 `claim_setup_code` 함수를 만든다. |

- 0009 는 0008 뒤에 실행하도록 적혀 있다. 0001–0008 이 이미 적용됐는지는 **미확인**이다.
- 각 파일 머리의 주석은 "not run yet" 으로 쓰여 있다. 실제로 실행됐는지는 **미확인**이다.
- 0006 은 방, 방 멤버, 방 메시지의 읽기 권한을 anon 과 authenticated 에서 뺀다. 적용 여부는 **미확인**이다.

## 2. Edge Functions 배포

저장소의 `supabase/functions/` 에는 함수가 8개 있다. 각 함수 파일 머리에 배포 명령이 적혀 있으며 모두 `--no-verify-jwt` 를 쓴다.

```
npx supabase@latest login
npx supabase@latest functions deploy <함수 이름> --no-verify-jwt --remote=coekrspnpcjzcbydarco
```

`<함수 이름>` 자리에 다음 중 하나를 넣는다.

| 함수 | 역할 | 앱이 호출하는 곳 (저장소 검색 기준) |
|---|---|---|
| `create-room` | 방 만들기 (서명, 무작위 방 ID, trust 해시 저장) | 6곳 |
| `join-room` | 방 참가 (서명, trust 검사, 인원 검사) | 5곳 |
| `put-peer` | 이 PC 의 행 게시 | 5곳 |
| `post-message` | 방 메시지 보내기 | 1곳 |
| `fetch-messages` | 방 메시지 받기 | 1곳 |
| `delete-peer` | 행 삭제 | 6곳 |
| `redeem-setup-code` | 설정 코드를 URL 과 키로 바꾸기 | 4곳 |
| `issue-setup-code` | 운영자가 새 설정 코드 발급 (암호 검사) | 3곳 |

- "앱이 호출하는 곳"은 저장소 안에서 함수 경로 문자열을 찾은 개수다. 테스트 파일을 뺐다. 실제 호출 여부는 **실행해서 확인하지 않았다**.
- 함수는 서버에서 `SUPABASE_URL` 과 `SUPABASE_SERVICE_ROLE_KEY` 를 자동으로 받는다. 이 값을 직접 넣을 필요가 없다.
- 배포된 함수의 실제 버전이 저장소와 같은지는 **미확인**이다.

## 3. 시크릿

운영 프로젝트에 다음 두 시크릿을 넣는다. 값은 직접 정하고 이 문서에 쓰지 않는다.

| 이름 | 쓰는 함수 | 뜻 |
|---|---|---|
| `OPERATOR_PASSPHRASE` | `issue-setup-code` | 운영자 암호. 앱의 F9 입력과 같은 값이어야 한다. |
| `APP_PUBLISHABLE_KEY` | `issue-setup-code` | 설정 코드로 앱에 넘길 publishable 키. |

설정 명령의 모양은 다음과 같다. `<값>` 은 직접 넣는다.

```
supabase secrets set OPERATOR_PASSPHRASE=<값> APP_PUBLISHABLE_KEY=<값> --project-ref coekrspnpcjzcbydarco
```

- 시크릿을 넣은 뒤 `issue-setup-code` 가 503 을 주지 않는지 확인한다. 503 은 시크릿이 없다는 뜻이다. 이 확인은 **미확인**이다.

## 4. 설정 코드 발급 (SQL 편집기)

설정 코드는 운영자가 SQL 편집기에서 만든다. 앱에는 발급 기능이 없다. 아래 예시에서 `<코드>`, `<프로젝트 URL>`, `<publishable 키>` 를 직접 채운다. 코드는 8자리 `A-Z0-9` 이다.

```sql
insert into public.setup_codes (code, url, key, expires_at, max_uses)
values ('<코드 8자리>', '<프로젝트 URL>', '<publishable 키>', now() + interval '7 days', 20);
```

- `max_uses` 는 몇 명이 쓸 수 있는지다. 기본값은 1 이다.
- `expires_at` 이 지나면 쓸 수 없다. 예시는 7일이다.
- 앱의 F9 발급은 같은 표에 **20회, 7일** 짜리 코드를 만든다 (`issue-setup-code`).
- 사용자는 이 코드를 앱의 설정 화면에 입력하고 F2(GUI 는 "Save setup code")를 누른다. 앱이 `redeem-setup-code` 로 URL 과 키를 받아 저장한다. 키는 화면에 보이지 않는다.

## 5. 제한

| 항목 | 값 | 근거 |
|---|---|---|
| 방 인원 | 방장을 포함해 **10명** | `join-room/checks.ts` `ROOM_MEMBER_LIMIT = 10`. 넘는 참가는 `409` |
| 방 trust 코드 연속 실패 | 10회 연속이면 방이 **1시간** 잠김 | `TRUST_FAILS_LIMIT = 10`, `TRUST_LOCK_SECONDS = 3600`. 잠긴 동안 `403` |
| 설정 코드 실패 | 최근 **600초** 안에 10회 틀리면 그 시간 동안 모든 상환 요청이 `429` | `redeem-setup-code` 의 `FAILURE_LIMIT = 10`, `FAILURE_WINDOW_SECONDS = 600`. 발급 함수도 같은 값 |
| 방 기한 | 생성 후 **24시간**, 연장 없음 | `create-room` 주석. 기한이 지난 방 참가는 `410` |

- 위 제한의 값은 저장소 코드에서 읽었다. 배포된 서버에서 같은 값으로 동작하는지는 **미확인**이다.

## 6. F9 운영자 발급 흐름

1. 앱의 설정 화면에서 **F9** 를 누른다. `operator passphrase` 줄이 열리고, 입력한 글자 수만큼 `*` 가 보인다. F9 는 화면 안내에 쓰여 있지 않다.
2. 암호를 입력하고 **Enter** 를 누른다. 앱이 `issue-setup-code` 에 요청한다. 요청은 백그라운드에서 나간다.
3. 성공하면 메시지 줄에 `new setup code: <코드> (20 uses, 7 days; write it down now)` 가 나온다. 코드는 그 자리에서만 보이므로 적어 둔다.
4. 실패하면 `issue failed: <이유>` 가 나온다. 이유는 `the operator passphrase is wrong`, `too many wrong tries; try later`, `the operator secrets are not set on the server` 중 하나이거나 HTTP 상태다.
5. **Esc** 는 암호 줄만 닫는다. 암호와 코드는 파일과 로그에 남기지 않는다(코드 확인 기준).

GUI 에서 같은 흐름이 있는지는 **미확인**이다.

## 7. 확인하지 않은 것

- 0006–0011 마이그레이션이 실제 프로젝트에 적용됐는지.
- 8개 함수가 지금 배포된 버전이 저장소와 같은지.
- 시크릿 두 개가 운영 프로젝트에 들어 있는지.
- 설정 코드 발급과 상환의 실제 HTTP 응답 (단위 테스트와 로컬 가짜 서버만 확인했다).
- 두 PC 이상이 실제 Supabase 방에서 대화하는 동작.
