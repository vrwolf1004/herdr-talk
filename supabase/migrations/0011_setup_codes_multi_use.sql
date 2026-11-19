-- Setup codes can be used by several people: max_uses says how many times a code may be redeemed (default 1, as before),
-- use_count counts the redeems so far. A redeem is one atomic update (claim_setup_code), so two people cannot take the last use.
-- Run once in the Supabase SQL Editor of the project's owner, AFTER 0010 (not run yet). Nothing here connects to the database.

alter table public.setup_codes add column if not exists max_uses int not null default 1
  constraint setup_codes_max_uses_positive check (max_uses >= 1);
alter table public.setup_codes add column if not exists use_count int not null default 0;

-- Rows made before this migration: a used row counts as used once.
update public.setup_codes set use_count = 1 where used_at is not null and use_count = 0;

-- Claim one use of a code: returns the URL and key when the code is known, not expired and has uses left. Otherwise no row.
create or replace function public.claim_setup_code(p_code text)
returns table (url text, key text)
language sql
as $$
  update public.setup_codes
     set use_count = use_count + 1, used_at = now()
   where code = p_code and use_count < max_uses and expires_at > now()
  returning setup_codes.url, setup_codes.key;
$$;

revoke all on function public.claim_setup_code(text) from public, anon, authenticated;
grant execute on function public.claim_setup_code(text) to service_role;

-- Issue a code for N people (example, owner runs it; replace the values):
-- insert into public.setup_codes (code, url, key, expires_at, max_uses)
-- values ('T5N9Q1ZB', 'https://coekrspnpcjzcbydarco.supabase.co', '<publishable key>', now() + interval '7 days', 20);
