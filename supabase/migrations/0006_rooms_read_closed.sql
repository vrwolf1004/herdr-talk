-- 방 테이블 직접 조회 차단 (anon, authenticated). 읽기는 Edge Function (service role) 만 한다.
--
-- 실행 전제 (반드시 이 순서):
--   1) create-room, join-room 이 anon 키 대신 service role 키 (Edge Function secret) 로 DB 에 접근하도록 바꾼다.
--      지금은 사용자 Authorization 헤더 (anon 키) 로 접근해서, 이 마이그레이션을 먼저 실행하면 함수가 깨진다.
--   2) 그 다음 이 파일을 실행한다. 실행은 사람이 한다 (chef 는 파일만 작성).
--
-- 효과: RLS 정책 제거 + select 권한 회수. RLS 가 켜져 있어 정책이 없으면 anon 은 아무 행도 보지 못한다.
-- 0003 의 using (true) 정책 (전체 공개) 을 없앤다. 방 보안 항목의 1차 수정이며, 서명 검증은 별도 단계다.

drop policy if exists rooms_select on public.rooms;
drop policy if exists room_members_select on public.room_members;
drop policy if exists room_messages_select on public.room_messages;

revoke select on public.rooms from anon, authenticated;
revoke select on public.room_members from anon, authenticated;
revoke select on public.room_messages from anon, authenticated;
