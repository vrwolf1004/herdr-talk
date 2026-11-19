-- Room trust code: the owner sets an 8-digit code at create-room, and each participant types it at join-room.
-- Only a salted PBKDF2 hash is stored (trust_salt, trust_hash), never the code. A room is locked for an hour after 10
-- wrong codes in a row (trust_fails, trust_locked_until). Rooms made before this migration have no code and cannot be joined.
-- Run once in the Supabase SQL Editor of the project's owner, AFTER 0008 (not run yet). Nothing here connects to the database.
-- Privileges are unchanged: anon and authenticated may only select; the Edge Functions write the rows.

alter table public.rooms add column if not exists trust_salt text;
alter table public.rooms add column if not exists trust_hash text;
alter table public.rooms add column if not exists trust_fails int not null default 0;
alter table public.rooms add column if not exists trust_locked_until timestamptz;
