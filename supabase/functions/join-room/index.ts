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

    const supabase = createClient(
      Deno.env.get("SUPABASE_URL") ?? "",
      Deno.env.get("SUPABASE_ANON_KEY") ?? "",
      {
        global: {
          headers: { Authorization: authHeader },
        },
      }
    );

    const { room_code, peer_pk, display_name } = await req.json();

    if (!room_code || !peer_pk || !display_name) {
      return new Response(
        JSON.stringify({ error: "room_code, peer_pk, and display_name required" }),
        {
          status: 400,
          headers: { ...corsHeaders, "Content-Type": "application/json" },
        }
      );
    }

    // 방 존재 확인
    const { data: room, error: roomError } = await supabase
      .from("rooms")
      .select("*")
      .eq("code", room_code)
      .single();

    if (roomError || !room) {
      return new Response(
        JSON.stringify({ error: "Room not found" }),
        {
          status: 404,
          headers: { ...corsHeaders, "Content-Type": "application/json" },
        }
      );
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
        return new Response(JSON.stringify({ error: insertError.message }), {
          status: 500,
          headers: { ...corsHeaders, "Content-Type": "application/json" },
        });
      }
    }

    // 방의 모든 멤버 조회
    const { data: members, error: membersError } = await supabase
      .from("room_members")
      .select("peer_pk")
      .eq("room_code", room_code);

    if (membersError) {
      return new Response(JSON.stringify({ error: membersError.message }), {
        status: 500,
        headers: { ...corsHeaders, "Content-Type": "application/json" },
      });
    }

    return new Response(
      JSON.stringify({
        room_code: room_code,
        owner_pk: room.owner_pk,
        members: members?.map((m) => m.peer_pk) || [],
        message: "Joined room successfully",
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
