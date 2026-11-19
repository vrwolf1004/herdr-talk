-- Setup codes: an operator issues an 8-character one-time code; a participant types it in the app and gets the
-- project's publishable key back (redeem-setup-code). The operator issues codes in the SQL editor (no app feature).
-- Nothing is readable or writable by anon or authenticated: only the Edge Function (service role) touches these tables.
-- Run once in the Supabase SQL Editor of the project's owner, AFTER 0009 (not run yet). Nothing here connects to the database.

create table if not exists public.setup_codes (
  code text primary key
    constraint setup_codes_format check (code ~ '^[0-9A-Z]{8}$'),
  url text not null,
  key text not null,
  expires_at timestamptz not null,
  used_at timestamptz,
  created_at timestamptz not null default now()
);

-- Wrong codes are counted here, so a guessing run is refused for a while (see redeem-setup-code).
create table if not exists public.setup_code_failures (
  id bigserial primary key,
  failed_at timestamptz not null default now()
);

alter table public.setup_codes enable row level security;
alter table public.setup_code_failures enable row level security;
revoke all on public.setup_codes from anon, authenticated;
revoke all on public.setup_code_failures from anon, authenticated;

-- Issue one code (example, run by the owner; replace the values):
-- insert into public.setup_codes (code, url, key, expires_at)
-- values ('K7P2X9QM', 'https://coekrspnpcjzcbydarco.supabase.co', '<publishable key>', now() + interval '7 days');
