// Supabase Edge Function `fetch-messages` (Deno). A room member asks for the room messages addressed to its key.
// Order: format (400) -> time window (401) -> Ed25519 signature by the recipient (401) -> nonce not used before (401)
// -> room and membership (404, 410, 403) -> rows for this recipient after `since` (service role). The rows are not
// removed here: they last until the room expires (see 0005 and 0007).
//
// Deploy (owner, once, after review): supabase functions deploy fetch-messages --no-verify-jwt
// Secrets: SUPABASE_URL and SUPABASE_SERVICE_ROLE_KEY are provided to the function by Supabase. Set nothing here.

import { createClient } from "https://esm.sh/@supabase/supabase-js@2";
import { checkBeforeNonce, messageFor, statusFor, type FetchBody } from "./checks.ts";

const corsHeaders = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "POST, OPTIONS",
  "Access-Control-Allow-Headers": "authorization, x-client-info, apikey, content-type",
};

const ROWS_PER_ANSWER = 200;

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { ...corsHeaders, "Content-Type": "application/json" },
  });
}

Deno.serve(async (req) => {
  if (req.method === "OPTIONS") {
    return new Response("ok", { headers: corsHeaders });
  }

  try {
    const supabase = createClient(
      Deno.env.get("SUPABASE_URL") ?? "",
      Deno.env.get("SUPABASE_SERVICE_ROLE_KEY") ?? ""
    );

    let body: unknown;
    try {
      body = await req.json();
    } catch {
      return jsonResponse(400, { error: messageFor("format") });
    }

    const verdict = await checkBeforeNonce(body, Math.floor(Date.now() / 1000));
    if (verdict !== "ok") {
      return jsonResponse(statusFor(verdict), { error: messageFor(verdict) });
    }
    const fetch = body as FetchBody;

    const { error: nonceError } = await supabase.from("put_nonces").insert({ nonce: fetch.nonce, ts: fetch.ts });
    if (nonceError) {
      if (nonceError.code === "23505") {
        return jsonResponse(401, { error: messageFor("nonce") });
      }
      return jsonResponse(500, { error: nonceError.message });
    }
    const releaseNonce = () => supabase.from("put_nonces").delete().eq("nonce", fetch.nonce);

    const { data: room, error: roomError } = await supabase
      .from("rooms")
      .select("expires_at")
      .eq("code", fetch.room_code)
      .single();
    if (roomError || !room) {
      await releaseNonce();
      return jsonResponse(404, { error: "Room not found" });
    }
    if (Date.parse(room.expires_at) <= Date.now()) {
      await releaseNonce();
      return jsonResponse(410, { error: "방 기한이 지났습니다" });
    }

    const { data: member, error: memberError } = await supabase
      .from("room_members")
      .select("peer_pk")
      .eq("room_code", fetch.room_code)
      .eq("peer_pk", fetch.recipient_pk)
      .maybeSingle();
    if (memberError) {
      await releaseNonce();
      return jsonResponse(500, { error: memberError.message });
    }
    if (!member) {
      await releaseNonce();
      return jsonResponse(403, { error: "not a room member" });
    }

    let query = supabase
      .from("room_message_deliveries")
      .select("message_id, sender_pk, ts, nonce, ciphertext, created_at")
      .eq("room_code", fetch.room_code)
      .eq("recipient_pk", fetch.recipient_pk)
      .order("created_at", { ascending: true })
      .limit(ROWS_PER_ANSWER);
    if (fetch.since !== undefined) {
      if (Number.isNaN(Date.parse(fetch.since))) {
        await releaseNonce();
        return jsonResponse(400, { error: messageFor("format") });
      }
      query = query.gt("created_at", fetch.since);
    }
    const { data: rows, error: rowsError } = await query;
    if (rowsError) {
      return jsonResponse(500, { error: rowsError.message });
    }

    return jsonResponse(200, { messages: rows ?? [] });
  } catch (error) {
    return jsonResponse(500, { error: error.message });
  }
});
