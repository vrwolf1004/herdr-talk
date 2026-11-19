// Supabase Edge Function `redeem-setup-code` (Deno). Gives the project URL and publishable key for a setup code.
// A code works up to max_uses times (default 1) and only before its expires_at. A wrong code is counted; at FAILURE_LIMIT wrong codes in
// FAILURE_WINDOW_SECONDS every request is refused (429) until the window passes.
// Deploy (owner, once): supabase functions deploy redeem-setup-code --no-verify-jwt --remote=<ref>
// Secrets: SUPABASE_URL and SUPABASE_SERVICE_ROLE_KEY are provided by Supabase. Set nothing here.

import { createClient } from "https://esm.sh/@supabase/supabase-js@2";
import { normalizeSetupCode, FAILURE_LIMIT, FAILURE_WINDOW_SECONDS } from "./checks.ts";

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
      return jsonResponse(400, { error: "invalid request" });
    }
    const code = normalizeSetupCode(typeof body === "object" && body !== null ? (body as { code?: unknown }).code : undefined);
    if (code === null) {
      return jsonResponse(400, { error: "invalid request" });
    }

    // The failure check, the use and the failure record happen in one database call under one lock (migration 0012),
    // so concurrent wrong codes cannot all pass the limit together.
    const { data, error: rpcError } = await supabase
      .rpc("redeem_setup_code_guarded", { p_code: code, p_fail_limit: FAILURE_LIMIT, p_window_seconds: FAILURE_WINDOW_SECONDS })
      .maybeSingle();
    if (rpcError) {
      return jsonResponse(500, { error: rpcError.message });
    }
    if (data?.status === "locked") {
      return jsonResponse(429, { error: "잠시 후 다시 시도하세요" });
    }
    if (data?.status !== "ok") {
      return jsonResponse(404, { error: "코드가 맞지 않거나 사용 횟수가 다 찼거나 만료되었습니다" });
    }

    return jsonResponse(200, { url: data.url, key: data.key });
  } catch (error) {
    return jsonResponse(500, { error: error.message });
  }
});
