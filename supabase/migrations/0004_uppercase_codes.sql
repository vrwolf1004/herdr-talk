-- herdr-talk relay: short IDs are 8 uppercase letters or digits (A-Z, 0-9), the same as room codes (see 0003).
-- Run once in the Supabase SQL Editor of the project's owner, AFTER 0003 (not run yet). Nothing here connects to the database.
--
-- Existing rows: every short_id made by 0002 is lowercase, so upper() maps each of them to a different uppercase value.
-- No two rows can collide (the old values were all lowercase, so no uppercase row existed), and the unique index stays.
-- The rows are kept, not deleted: each PC reads its saved short ID as uppercase and publishes the same ID again on its next start.
-- The old check constraint refuses uppercase, so it is dropped first and the new one is added after the rows are changed.
--
-- Privileges are unchanged: anon and authenticated may only select. The only writer is put-peer (service role).

alter table public.peers drop constraint if exists peers_short_id_format;

update public.peers set short_id = upper(short_id) where short_id <> upper(short_id);

alter table public.peers add constraint peers_short_id_format check (short_id ~ '^[A-Z0-9]{8}$');
