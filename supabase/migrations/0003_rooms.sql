-- Room-based relay: groups of peers sharing an 8-character code.
-- Users join a room with the code; all messages in the room go through Supabase.

create table if not exists public.rooms (
  code text primary key
    constraint rooms_code_format check (code ~ '^[A-Z0-9]{8}$'),
  owner_pk text not null
    constraint rooms_owner_fk references public.peers(pk) on delete cascade,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

-- Members: peer_pk -> room_code mapping (many-to-many).
create table if not exists public.room_members (
  id bigserial primary key,
  room_code text not null references public.rooms(code) on delete cascade,
  peer_pk text not null references public.peers(pk) on delete cascade,
  joined_at timestamptz not null default now(),
  unique(room_code, peer_pk)
);

-- Messages in rooms.
create table if not exists public.room_messages (
  id bigserial primary key,
  room_code text not null references public.rooms(code) on delete cascade,
  sender_pk text not null references public.peers(pk) on delete cascade,
  content text not null,
  created_at timestamptz not null default now()
);

alter table public.rooms enable row level security;
alter table public.room_members enable row level security;
alter table public.room_messages enable row level security;

-- Read rooms: anyone with publishable key can list/read.
drop policy if exists rooms_select on public.rooms;
create policy rooms_select on public.rooms
  for select to anon, authenticated
  using (true);

-- Read room members: anyone can see who's in a room.
drop policy if exists room_members_select on public.room_members;
create policy room_members_select on public.room_members
  for select to anon, authenticated
  using (true);

-- Read room messages: anyone can read messages.
drop policy if exists room_messages_select on public.room_messages;
create policy room_messages_select on public.room_messages
  for select to anon, authenticated
  using (true);

-- Revoke write access from anon/authenticated (only Edge Functions can write).
revoke all on public.rooms from anon, authenticated;
revoke all on public.room_members from anon, authenticated;
revoke all on public.room_messages from anon, authenticated;
grant select on public.rooms to anon, authenticated;
grant select on public.room_members to anon, authenticated;
grant select on public.room_messages to anon, authenticated;
