// Supabase Edge Function `post-message` (Deno). A room member posts one message: one Noise ciphertext per recipient.
// Order: format (400) -> time window (401) -> Ed25519 signature by the sender (401) -> nonce not used before (401,
// recorded in public.put_nonces) -> room and membership (404, 410, 403) -> one row per recipient (service role).
// A failed write gives the nonce back, so the same request can be sent again. Failures carry no reason beyond {error}.
// The server stores ciphertexts only: it cannot read the messages.
//
// Deploy (owner, once, after review): supabase functions deploy post-message --no-verify-jwt
// Secrets: SUPABASE_URL and SUPABASE_SERVICE_ROLE_KEY are provided to the function by Supabase. Set nothing here.

import { createClient } from "https://esm.sh/@supabase/supabase-js@2";
import { checkBeforeNonce, messageFor, statusFor, type PostBody } from "./checks.ts";

const corsHeaders = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "POST, OPTIONS",
  "Access-Control-Allow-Headers": "authorization, x-client-info, apikey, content-type",
};

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
    const post = body as PostBody;

    // Replay protection: this nonce is recorded once (23505 when it is already there).
    const { error: nonceError } = await supabase.from("put_nonces").insert({ nonce: post.nonce, ts: post.ts });
    if (nonceError) {
      if (nonceError.code === "23505") {
        return jsonResponse(401, { error: messageFor("nonce") });
      }
      return jsonResponse(500, { error: nonceError.message });
    }
    // Takes the nonce back when a later step fails, so the same request can be sent again.
    const releaseNonce = () => supabase.from("put_nonces").delete().eq("nonce", post.nonce);

    const { data: room, error: roomError } = await supabase
      .from("rooms")
      .select("expires_at")
      .eq("code", post.room_code)
      .single();
    if (roomError || !room) {
      await releaseNonce();
      return jsonResponse(404, { error: "Room not found" });
    }
    if (Date.parse(room.expires_at) <= Date.now()) {
      await releaseNonce();
      return jsonResponse(410, { error: "방 기한이 지났습니다" });
    }

    // The sender and every recipient must be members of this room.
    const keys = [post.sender_pk, ...post.deliveries.map((delivery) => delivery.recipient_pk)];
    const { data: members, error: membersError } = await supabase
      .from("room_members")
      .select("peer_pk")
      .eq("room_code", post.room_code)
      .in("peer_pk", keys);
    if (membersError) {
      await releaseNonce();
      return jsonResponse(500, { error: membersError.message });
    }
    const found = new Set((members ?? []).map((member) => member.peer_pk));
    if (keys.some((key) => !found.has(key))) {
      await releaseNonce();
      return jsonResponse(403, { error: "not a room member" });
    }

    const rows = post.deliveries.map((delivery) => ({
      message_id: post.message_id,
      room_code: post.room_code,
      sender_pk: post.sender_pk,
      recipient_pk: delivery.recipient_pk,
      ts: post.ts,
      nonce: post.nonce,
      ciphertext: delivery.ciphertext,
    }));
    const { error: insertError } = await supabase.from("room_message_deliveries").insert(rows);
    if (insertError) {
      await releaseNonce();
      return jsonResponse(500, { error: insertError.message });
    }

    return jsonResponse(200, { message_id: post.message_id, delivered: rows.length });
  } catch (error) {
    return jsonResponse(500, { error: error.message });
  }
});
