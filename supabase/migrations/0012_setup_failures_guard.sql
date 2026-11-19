-- Setup-code redeem with the failure check inside the database, under one lock: concurrent requests cannot
-- all pass a count taken before any of them failed. Returns 'locked' (too many failures in the window), 'ok' (with url
-- and key, one use taken) or 'wrong' (a failure is recorded). Only the Edge Functions (service role) may call it.
-- Run once in the Supabase SQL Editor of the project's owner, AFTER 0011 (not run yet). Nothing here connects to the database.

create or replace function public.redeem_setup_code_guarded(p_code text, p_fail_limit int, p_window_seconds int)
returns table (status text, url text, key text)
language plpgsql
security definer
set search_path = public
as $$
declare
  fails int;
  claimed public.setup_codes%rowtype;
begin
  perform pg_advisory_xact_lock(hashtext('setup_code_failures'));
  select count(*) into fails from public.setup_code_failures
   where failed_at > now() - make_interval(secs => p_window_seconds);
  if fails >= p_fail_limit then
    return query select 'locked'::text, null::text, null::text;
    return;
  end if;
  update public.setup_codes
     set use_count = use_count + 1, used_at = now()
   where code = p_code and use_count < max_uses and expires_at > now()
  returning * into claimed;
  if found then
    return query select 'ok'::text, claimed.url, claimed.key;
    return;
  end if;
  insert into public.setup_code_failures default values;
  return query select 'wrong'::text, null::text, null::text;
end;
$$;

revoke all on function public.redeem_setup_code_guarded(text, int, int) from public, anon, authenticated;
grant execute on function public.redeem_setup_code_guarded(text, int, int) to service_role;
