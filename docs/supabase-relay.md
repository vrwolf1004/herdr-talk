# Supabase 릴레이

## 현재 상태

Supabase 릴레이는 **1대다 방** 모델이다. 서버가 방을 만들 때 무작위 8자리 방 ID를 만들고 만든 사람에게 돌려준다. 다른 사람은 그 방 ID를 입력해 같은 방에 참가한다.

**폐기됨: 상대 ID 기반 1:1 Supabase 릴레이.** 상대 쌍으로 채널을 만들거나 상대의 ID를 조회해 1:1로 연결하는 이전 방식은 지원하지 않는다.

## 방 사용

1. **설정 코드**(8자리)를 입력하고 TUI에서 **F2**를 누른다(GUI는 **설정 코드 저장**). 앱이 서버에서 URL과 publishable 키를 받아 저장한다. 두 값은 화면에 보이지 않는다.
2. 표시 이름과 **trust 코드**(숫자 8자리)를 입력하고, 마지막 줄에서 저장한다.
3. **방 만들기**(TUI는 Ctrl+N)를 누른다. 서버가 만든 8자리 방 ID가 방 ID 칸에 들어간다. 이때 입력한 trust 코드가 이 방의 참가 코드가 된다.
4. 방 ID와 trust 코드를 다른 참가자에게 전달한다.
5. 다른 참가자는 설정 코드, 표시 이름, 방 ID, trust 코드를 입력해 참가한다. trust 코드가 맞지 않으면 서버가 `403`을 돌려주고 참가되지 않는다. TUI는 채팅을 열기 전에 설정 화면으로 돌아가 `trust 코드가 맞지 않습니다`를 보여 준다. GUI의 같은 동작은 미확인이다.

trust 코드는 운영자가 정하고 참가자가 확인받는 값이다. 서버에는 솔트를 넣은 PBKDF2 해시만 저장하며, 코드 자체는 저장하지 않는다. 10번 연속 틀리면 그 방은 1시간 동안 잠긴다.

방 ID는 `A-Z0-9` 8자리다. 입력할 때 대소문자는 구별하지 않으며, 공백과 대시는 무시한다.

방 인원은 방장을 포함해 **최대 10명**이다. 인원이 찬 방에 새로 참가하면 서버가 `409`(`방 인원이 가득 찼습니다 (10명)`)로 거부한다. 이 제한은 `join-room` 함수 코드에 있으며, 배포된 서버에서의 동작은 미확인이다. 방은 생성 시점부터 **24시간** 사용할 수 있고 연장할 수 없다. 만료된 방에 참가하면 서버는 `410`을 반환한다. 만료 방 정리는 `pg_cron`으로 설계되어 있으며 `0005_room_expiry.sql` 주석에 기록되어 있다. 만료·정리 기능은 **구현됨, 미배포** 상태다.

## 식별 코드와 신뢰

- 방 ID는 8자리 `A-Z0-9` 코드다. 입력에서 대소문자를 구별하지 않고 공백과 대시를 무시한다.
- 레거시 상대 ID는 서명된 peer 행 게시용으로만 남고, 방 참가나 1:1 라우팅에는 쓰지 않는다.
- trust 코드는 숫자 8자리다. 방 참가(`join-room`)에서 서버가 검사하고, 방 생성(`create-room`)에서 해시로 저장한다. LAN의 확인 코드(`/join`)와는 별개다.
- 이전 방(trust 코드 도입 전에 만든 방)에는 trust 코드가 없어 참가할 수 없다.

## LAN과의 관계

`local` 모드는 LAN 전용이다. UDP 브로드캐스트로 상대를 찾고 보이는 **모든** 상대에게 메시지를 보낸다. 이는 1:1 기능이 아니라 다수 참가자 LAN 채팅이다.

`supabase` 모드는 방 기반 인터넷 릴레이다. LAN과 Supabase는 별도 전송 방식이며, 실제 전송·화면 동작의 세부 사항은 미확인인 부분을 추측하지 않는다.

## 운영 배포 절차 (사용자가 직접 실행)

전체 절차와 시크릿 이름은 [운영자 가이드](operator-setup.md)에 있다. 아래는 방 기능의 최소 절차다.

운영 프로젝트 ref는 `coekrspnpcjzcbydarco`다. 아래는 순서대로 한 번씩 실행한다. `db push`는 운영에서 쓰지 않는다.

1. **마이그레이션 적용:** Supabase 대시보드의 SQL 편집기에서 `supabase/migrations/0009_room_trust.sql` 내용을 실행한다. `Success. No rows returned`가 나오면 정상이다.
2. **함수 배포 (저장소 최상위 폴더에서):**

   Supabase CLI가 없으면 npx로 실행한다.

   ```
   npx supabase@latest login
   npx supabase@latest functions deploy create-room --no-verify-jwt --remote=coekrspnpcjzcbydarco
   npx supabase@latest functions deploy join-room --no-verify-jwt --remote=coekrspnpcjzcbydarco
   ```

   CLI를 설치해 쓸 수도 있다. `winget install Supabase.CLI`로 설치한 뒤 PowerShell을 새로 열고 `supabase login`과 위 명령을 `supabase`로 실행한다.

   `login`은 브라우저가 열려 로그인하는 단계다. 토큰을 채팅이나 저장소에 붙여 넣지 않는다. 시크릿은 따로 넣지 않는다. Supabase가 함수에 `SUPABASE_URL`과 `SUPABASE_SERVICE_ROLE_KEY`를 자동으로 넘긴다.

   설정 코드 기능은 다음이 추가로 필요하다: 마이그레이션 `supabase/migrations/0010_setup_codes.sql`, `0011_setup_codes_multi_use.sql`, 함수 `issue-setup-code`, `redeem-setup-code`. 이 문서 작성 시점에 실행 여부는 확인하지 않았다(미확인).

3. **확인:** 새 방을 만들고, 같은 방 ID에 맞는 trust 코드로 참가가 되는지, 틀린 코드가 거부되는지 본다.

## 운영자 설정 코드 발급 (숨김)

설정 화면에서 F9를 누르면 운영자 암호 입력 줄이 열린다(길이만 보임). Enter를 누르면 `issue-setup-code` 함수가 새 설정 코드(20회, 7일)를 준다. 함수는 암호를 서버 시크릿 `OPERATOR_PASSPHRASE`와 비교하고, 코드가 주는 URL과 키는 `SUPABASE_URL`과 시크릿 `APP_PUBLISHABLE_KEY`에서 온다. 시크릿 값은 이 문서와 저장소에 쓰지 않는다. 설정 예: `supabase secrets set OPERATOR_PASSPHRASE=<값> APP_PUBLISHABLE_KEY=<값> --project-ref <ref>`(값은 직접 넣는다). 배포 여부와 함수 동작은 미확인이다.

## 배포 전 보안 과제 (저장소 코드 기준)

아래는 `supabase/migrations/`와 `supabase/functions/`의 파일을 읽어 확인한 내용이다. **실제 프로젝트에 적용·배포됐는지는 확인하지 않았으므로 모두 미확인**이다. 설정 절차는 [운영자 가이드](operator-setup.md)를 본다.

- **읽기 권한 (코드상 닫힘):** 0003이 방, 방 멤버, 방 메시지를 누구나 읽게 열었다. 0006이 그 정책을 지우고 anon과 authenticated의 select 권한도 뺀다. 0007(방 메시지 전달), 0010(설정 코드), `put_nonces`는 anon과 authenticated 권한이 없다. 예외: `peers` 표는 0001에서 anon이 읽을 수 있다(공개키 조회용). 쓰기 권한은 어느 표에도 없다.
- **쓰기 경로:** 모든 쓰기는 Edge Function이 service role로 한다.
- **서명 검사 (코드에 있음):** `create-room`, `join-room`, `put-peer`, `delete-peer`는 요청자의 Ed25519 서명을 검사한다. `post-message`는 보내는 사람, `fetch-messages`는 받는 사람의 서명을 검사한다. 순서는 형식, 시간 창, 서명, nonce 재사용 금지다.
- **trust 코드:** 해시(PBKDF2-SHA256, 솔트 포함)만 저장한다. 연속 10회 틀리면 방이 1시간 잠긴다.
- **설정 코드:** 같은 설정 코드의 상환과 발급 실패는 600초 안에 10회까지다. 초과하면 429다. 상환 한 번은 `claim_setup_code`로 원자적으로 차감한다.
- **메시지 내용:** `0008`은 방 메시지 전달 행의 `ciphertext`를 짝수 길이의 16진수 문자열로 제한한다. 메시지가 실제로 암호화됐는지는 이 문서에서 확인하지 않았다.

**이전에 "배포 전 해결 필요"로 적었던 두 항목**(방 RLS 전체 공개, 방 생성·참가 서명 검증 없음)은 **저장소 코드에서는 해소된 것으로 보인다**. 다만 0006 적용과 함수 배포가 확인되기 전까지는 해결로 보지 않는다.

남은 확인 항목:
- 0006–0011 마이그레이션의 적용 여부 (각 파일 머리는 "not run yet"으로 적혀 있다).
- 배포된 함수가 저장소의 코드와 같은지.
- 운영 프로젝트의 실제 권한과 정책 상태.
- 방 메시지의 암호화 여부.
