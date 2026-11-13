import { createClient } from "https://esm.sh/@supabase/supabase-js@2";

const corsHeaders = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "POST, OPTIONS",
  "Access-Control-Allow-Headers": "authorization, x-client-info, apikey, content-type",
};

Deno.serve(async (req) => {
  // CORS 요청
  if (req.method === "OPTIONS") {
    return new Response("ok", { headers: corsHeaders });
  }

  try {
    const authHeader = req.headers.get("Authorization");
    if (!authHeader) {
      return new Response(JSON.stringify({ error: "No authorization header" }), {
        status: 401,
        headers: { ...corsHeaders, "Content-Type": "application/json" },
      });
    }

    // 클라이언트 초기화 (인증된 요청)
    const supabase = createClient(
      Deno.env.get("SUPABASE_URL") ?? "",
      Deno.env.get("SUPABASE_ANON_KEY") ?? "",
      {
        global: {
          headers: { Authorization: authHeader },
        },
      }
    );

    // 요청 본문에서 피어 공개키와 표시 이름 받음
    const { peer_pk, display_name } = await req.json();

    if (!peer_pk || !display_name) {
      return new Response(
        JSON.stringify({ error: "peer_pk and display_name required" }),
        {
          status: 400,
          headers: { ...corsHeaders, "Content-Type": "application/json" },
        }
      );
    }

    // 피어가 이미 다른 방에 소속했는지 확인
    const { data: existing } = await supabase
      .from("room_members")
      .select("room_code")
      .eq("peer_pk", peer_pk)
      .single();

    let roomCode: string;

    if (existing) {
      // 기존 방이 있으면 재사용
      roomCode = existing.room_code;
    } else {
      // 새 방 생성: 8자리 대문자 + 숫자
      roomCode = generateRoomCode();

      // 방 생성
      const { error: roomError } = await supabase.from("rooms").insert({
        code: roomCode,
        owner_pk: peer_pk,
      });

      if (roomError) {
        return new Response(JSON.stringify({ error: roomError.message }), {
          status: 500,
          headers: { ...corsHeaders, "Content-Type": "application/json" },
        });
      }

      // 멤버 추가 (소유자)
      const { error: memberError } = await supabase
        .from("room_members")
        .insert({
          room_code: roomCode,
          peer_pk: peer_pk,
        });

      if (memberError) {
        return new Response(JSON.stringify({ error: memberError.message }), {
          status: 500,
          headers: { ...corsHeaders, "Content-Type": "application/json" },
        });
      }
    }

    return new Response(
      JSON.stringify({
        room_code: roomCode,
        message: "Room created or retrieved successfully",
      }),
      {
        status: 200,
        headers: { ...corsHeaders, "Content-Type": "application/json" },
      }
    );
  } catch (error) {
    return new Response(JSON.stringify({ error: error.message }), {
      status: 500,
      headers: { ...corsHeaders, "Content-Type": "application/json" },
    });
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
