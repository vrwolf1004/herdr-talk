-- herdr-talk relay: short ID (8 characters) for each peer row, used to find a peer. Signed write format v3.
-- Run once in the Supabase SQL Editor of the project's owner, AFTER 0001 (not run yet). Nothing here connects to the database.
--
-- Existing rows: a row without a short_id cannot be found by the new lookup. Before the NOT NULL line below can run,
-- the owner clears the rows that have no short_id (they are published again by each PC on its next start):
--     delete from public.peers where short_id is null;
-- Then run the rest. Rows that keep a short_id are not touched.
--
-- Privileges are unchanged from 0001: anon and authenticated may only select. The only writer is put-peer (service role).

alter table public.peers add column if not exists short_id text;

alter table public.peers drop constraint if exists peers_short_id_format;
alter table public.peers add constraint peers_short_id_format check (short_id ~ '^[a-z0-9]{8}$');

-- Each short ID belongs to one key at a time. put-peer answers 409 when another key already has it.
create unique index if not exists peers_short_id_key on public.peers (short_id);

-- Run only after the rows without a short_id are cleared (see above).
alter table public.peers alter column short_id set not null;
