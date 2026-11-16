-- Room messages: one row per recipient (design: docs/room-messages-design.md, section 4). Each row holds one Noise
-- ciphertext from the sender to that recipient, so the server never sees plain text.
-- Run once in the Supabase SQL Editor of the project's owner, AFTER 0006 (not run yet). Nothing here connects to the database.
--
-- message_id is 32 lowercase hex characters (16 random bytes made by the sender), so one message's rows share it.
-- Rows go with their room (on delete cascade): they last as long as the room (24 hours, see 0005).
-- Privileges: anon and authenticated get no access (RLS on, no policy). The functions post-message and fetch-messages
-- use the service role.

create table if not exists public.room_message_deliveries (
  message_id text not null
    constraint room_message_deliveries_id_format check (message_id ~ '^[0-9a-f]{32}$'),
  room_code text not null references public.rooms(code) on delete cascade,
  sender_pk text not null references public.peers(pk) on delete cascade,
  recipient_pk text not null references public.peers(pk) on delete cascade,
  ts bigint not null,
  nonce text not null
    constraint room_message_deliveries_nonce_format check (nonce ~ '^[0-9a-f]{32}$'),
  ciphertext text not null
    constraint room_message_deliveries_ciphertext_format check (ciphertext ~ '^[0-9a-f]{2,32768}$'),
  created_at timestamptz not null default now(),
  primary key (message_id, recipient_pk)
);

create index if not exists room_message_deliveries_recipient
  on public.room_message_deliveries (room_code, recipient_pk, created_at);

alter table public.room_message_deliveries enable row level security;
revoke all on public.room_message_deliveries from anon, authenticated;
