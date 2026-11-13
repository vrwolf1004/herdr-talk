-- Room expiry: a room lasts 24 hours from its creation. There is no extension of the time.
-- Run once in the Supabase SQL Editor of the project's owner, AFTER 0004 (not run yet). Nothing here connects to the database.
--
-- Existing rows: expires_at is filled as created_at + 24 hours, then the column becomes NOT NULL.
-- A room that has already passed its time is refused by join-room (HTTP 410) and is removed by the cleanup job below.
--
-- Privileges are unchanged: anon and authenticated may only select. create-room and the cleanup job write the rows.

alter table public.rooms add column if not exists expires_at timestamptz;

update public.rooms set expires_at = created_at + interval '24 hours' where expires_at is null;

alter table public.rooms alter column expires_at set default (now() + interval '24 hours');
alter table public.rooms alter column expires_at set not null;

create index if not exists rooms_expires_at_idx on public.rooms (expires_at);

-- Cleanup (choice: pg_cron, not a delete inside create-room).
-- Why: the function runs with the anon key, and the rooms table grants no delete to anon or authenticated, so a delete
-- inside create-room would be refused; pg_cron runs as the database owner, on a timer, whether or not anyone calls.
-- Deleting a room also deletes its members and messages (on delete cascade in 0003).
-- Needs the pg_cron extension enabled in the project (Database > Extensions). Then run once, by hand:
--
-- create extension if not exists pg_cron;
-- select cron.schedule('delete-expired-rooms', '0 * * * *', $$delete from public.rooms where expires_at < now()$$);
