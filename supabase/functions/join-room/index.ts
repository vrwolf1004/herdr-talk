// Supabase Edge Function `join-room` (Deno). Adds the signing peer to a room and returns its members.
// Order: format (400) -> time window (401) -> Ed25519 signature (401) -> nonce not used before (401) -> room checks (404, 410)
// -> membership write (service role). The request is signed with the peer's key (ROOM_TAG herdr-talk-room-v1, action join).
// The nonce is recorded in public.put_nonces (the same table put-peer uses). A failed write gives the nonce back.
// Failures carry no reason beyond messageFor() and the room errors below.
//
// Deploy (owner, once, after the signature step is reviewed): supabase functions deploy join-room --no-verify-jwt
// Secrets: SUPABASE_URL and SUPABASE_SERVICE_ROLE_KEY are provided to the function by Supabase. Set nothing here.

import { createClient } from "https://esm.sh/@supabase/supabase-js@2";
import { checkBeforeNonce, messageFor, statusFor, type RoomBody } from "./checks.ts";

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
  // CORS 요청
  if (req.method === "OPTIONS") {
    return new Response("ok", { headers: corsHeaders });
  }

  try {
    const authHeader = req.headers.get("Authorization");
    if (!authHeader) {
      return jsonResponse(401, { error: "No authorization header" });
    }

    // service role: 서버 측 함수만 이 키를 쓴다 (RLS 를 건너뛴다). 사용자 토큰은 전달하지 않는다.
    // 서명 검사 (아래) 가 요청자가 peer_pk 의 주인임을 확인한다. 그 밖의 사용자 인증은 아직 없다 (별도 단계).
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

    const nowSeconds = Math.floor(Date.now() / 1000);
    const verdict = await checkBeforeNonce(body, "join", nowSeconds);
    if (verdict !== "ok") {
      return jsonResponse(statusFor(verdict), { error: messageFor(verdict) });
    }
    const { room_code, peer_pk, ts, nonce } = body as RoomBody & { room_code: string };

    // 재전송 방지: 이 nonce 를 기록한다. 이미 있으면 (23505) 재전송이다.
    const { error: nonceError } = await supabase.from("put_nonces").insert({ nonce, ts });
    if (nonceError) {
      if (nonceError.code === "23505") {
        return jsonResponse(401, { error: messageFor("nonce") });
      }
      return jsonResponse(500, { error: nonceError.message });
    }
    // Takes the nonce back when a later step fails, so the same request can be sent again.
    const releaseNonce = () => supabase.from("put_nonces").delete().eq("nonce", nonce);

    // 방 존재 확인
    const { data: room, error: roomError } = await supabase
      .from("rooms")
      .select("*")
      .eq("code", room_code)
      .single();

    if (roomError || !room) {
      await releaseNonce();
      return jsonResponse(404, { error: "Room not found" });
    }

    // 기한이 지난 방은 들어갈 수 없다 (410). 정리(삭제)는 migration 0005 의 pg_cron 작업이 한다.
    if (Date.parse(room.expires_at) <= Date.now()) {
      await releaseNonce();
      return jsonResponse(410, { error: "방 기한이 지났습니다" });
    }

    // 이미 멤버인지 확인
    const { data: existing } = await supabase
      .from("room_members")
      .select("*")
      .eq("room_code", room_code)
      .eq("peer_pk", peer_pk)
      .single();

    if (!existing) {
      // 새 멤버 추가
      const { error: insertError } = await supabase
        .from("room_members")
        .insert({
          room_code: room_code,
          peer_pk: peer_pk,
        });

      if (insertError) {
        await releaseNonce();
        return jsonResponse(500, { error: insertError.message });
      }
    }

    // 방의 모든 멤버 조회
    const { data: members, error: membersError } = await supabase
      .from("room_members")
      .select("peer_pk")
      .eq("room_code", room_code);

    if (membersError) {
      return jsonResponse(500, { error: membersError.message });
    }

    return jsonResponse(200, {
      room_code: room_code,
      owner_pk: room.owner_pk,
      members: members?.map((m) => m.peer_pk) || [],
      message: "Joined room successfully",
    });
  } catch (error) {
    return jsonResponse(500, { error: error.message });
  }
});
