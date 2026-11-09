-- herdr-talk relay: the peers table, the nonce table, and the write path through the put-peer Edge Function.
-- Run once in the Supabase SQL Editor of the project's owner (not run yet). Nothing here connects to the database.
-- If this file was already run, the two privilege lines under the peers table can be run by hand (they are idempotent).
--
-- Write rule: no client role may insert or update peers. The only writer is the Edge Function `put-peer`, which
-- verifies an Ed25519 signature over the v2 string (pk, noise_pk, name, ts, nonce) and then upserts with the service role (it bypasses RLS).
-- The publishable key only passes the function gate; it is not a write right.
--
-- What is stored: pk (Ed25519 public key, hex), name, noise_pk (X25519 key, hex, required), updated_at.
-- Nothing else: no fingerprint, no IP, no connection log, no messages.

create table if not exists public.peers (
  pk text primary key
    constraint peers_pk_format check (pk ~ '^[0-9a-f]{64}$'),
  name text not null
    constraint peers_name_length check (char_length(name) between 1 and 32)
    constraint peers_name_no_control check (name !~ '[[:cntrl:]]'),
  noise_pk text not null
    constraint peers_noise_pk_format check (noise_pk ~ '^[0-9a-f]{64}$'),
  updated_at timestamptz not null default now()
);

alter table public.peers enable row level security;

-- Read: anyone with the publishable key (public key lookup).
drop policy if exists peers_select on public.peers;
create policy peers_select on public.peers
  for select to anon, authenticated
  using (true);

-- No write right for anon or authenticated: no insert, update or delete policy, and every privilege is revoked first
-- (TRUNCATE, REFERENCES and TRIGGER come from Supabase default grants and bypass row-level security, so they go too).
-- Then only select is granted back, for public key lookup.
revoke all on public.peers from anon, authenticated;
grant select on public.peers to anon, authenticated;

-- Replay protection for put-peer: each nonce is accepted once. RLS on and no policy, so anon cannot read or write it.
create table if not exists public.put_nonces (
  nonce text primary key
    constraint put_nonces_format check (nonce ~ '^[0-9a-f]{32}$'),
  ts bigint not null
);

alter table public.put_nonces enable row level security;
revoke all on public.put_nonces from anon, authenticated;

-- Optional cleanup of expired nonces (older than the 300-second window the function accepts).
-- Run it by hand or from a scheduled job, for example:  select public.purge_put_nonces();
create or replace function public.purge_put_nonces()
returns bigint
language sql
security definer
set search_path = ''
as $$
  with deleted as (
    delete from public.put_nonces
    where ts < extract(epoch from now())::bigint - 300
    returning 1
  )
  select count(*) from deleted;
$$;

revoke execute on function public.purge_put_nonces() from public, anon, authenticated;
