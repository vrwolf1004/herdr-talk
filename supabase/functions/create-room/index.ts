// Supabase Edge Function `create-room` (Deno). Makes a new room owned by the signing peer (24 hours, see 0005).
// Order: format (400) -> time window (401) -> Ed25519 signature (401) -> nonce not used before (401) -> write (service role).
// The request is signed with the peer's key (ROOM_TAG herdr-talk-room-v1, action create). The nonce is recorded in
// public.put_nonces (the same table put-peer uses; the signed strings differ, so they never collide). A failed write
// gives the nonce back, so the same request can be sent again. Failures carry no reason beyond messageFor().
//
// Deploy (owner, once, after the signature step is reviewed): supabase functions deploy create-room --no-verify-jwt
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
    const verdict = await checkBeforeNonce(body, "create", nowSeconds);
    if (verdict !== "ok") {
      return jsonResponse(statusFor(verdict), { error: messageFor(verdict) });
    }
    const { peer_pk, display_name, ts, nonce } = body as RoomBody;

    // 재전송 방지: 이 nonce 를 기록한다. 이미 있으면 (23505) 재전송이다.
    const { error: nonceError } = await supabase.from("put_nonces").insert({ nonce, ts });
    if (nonceError) {
      if (nonceError.code === "23505") {
        return jsonResponse(401, { error: messageFor("nonce") });
      }
      return jsonResponse(500, { error: nonceError.message });
    }
    // Takes the nonce back when the write below fails, so the same request can be sent again.
    const releaseNonce = () => supabase.from("put_nonces").delete().eq("nonce", nonce);

    // 요청마다 새 방: 이미 다른 방에 있어도 새 8자리 코드를 만든다 (room_members 는 여러 방을 허용)
    const roomCode = generateRoomCode();
    // 방 기한: 생성 시점부터 24시간, 연장 없음 (join-room 이 기한이 지난 방을 410 으로 막는다)
    const expiresAt = new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString();

    // 방 생성
    const { error: roomError } = await supabase.from("rooms").insert({
      code: roomCode,
      owner_pk: peer_pk,
      expires_at: expiresAt,
    });

    if (roomError) {
      await releaseNonce();
      return jsonResponse(500, { error: roomError.message });
    }

    // 멤버 추가 (소유자)
    const { error: memberError } = await supabase
      .from("room_members")
      .insert({
        room_code: roomCode,
        peer_pk: peer_pk,
      });

    if (memberError) {
      await releaseNonce();
      return jsonResponse(500, { error: memberError.message });
    }

    return jsonResponse(200, {
      room_code: roomCode,
      expires_at: expiresAt,
      message: "Room created successfully",
    });
  } catch (error) {
    return jsonResponse(500, { error: error.message });
  }
});

// 8자리 대문자 + 숫자 무작위 생성
function generateRoomCode(): string {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
  let code = "";
  for (let i = 0; i < 8; i++) {
    code += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return code;
}
