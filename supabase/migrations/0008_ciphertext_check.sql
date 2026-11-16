-- Fix: the ciphertext check in 0007 used a repetition count {2,32768}, which Postgres regex refuses (max 255),
-- so every insert into room_message_deliveries failed with "invalid repetition count(s)".
-- This checks the even-length hex by a pattern without a count, and the length by char_length.
-- 65535 bytes of ciphertext = 131070 hex characters (the limit used by post-message).
-- Run once in the Supabase SQL Editor of the project's owner (not run yet).

alter table public.room_message_deliveries drop constraint if exists room_message_deliveries_ciphertext_format;

alter table public.room_message_deliveries add constraint room_message_deliveries_ciphertext_format
  check (ciphertext ~ '^([0-9a-f]{2})+$' and char_length(ciphertext) <= 131072);
