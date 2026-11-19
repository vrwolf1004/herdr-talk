// Supabase Edge Function `redeem-setup-code` (Deno). Gives the project URL and publishable key for a setup code.
// A code works up to max_uses times (default 1) and only before its expires_at. A wrong code is counted; at FAILURE_LIMIT wrong codes in
// FAILURE_WINDOW_SECONDS every request is refused (429) until the window passes.
// Deploy (owner, once): supabase functions deploy redeem-setup-code --no-verify-jwt --remote=<ref>
// Secrets: SUPABASE_URL and SUPABASE_SERVICE_ROLE_KEY are provided by Supabase. Set nothing here.

import { createClient } from "https://esm.sh/@supabase/supabase-js@2";
import { normalizeSetupCode, tooManyFailures, FAILURE_WINDOW_SECONDS } from "./checks.ts";

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

    const since = new Date(Date.now() - FAILURE_WINDOW_SECONDS * 1000).toISOString();
    const { count, error: countError } = await supabase
      .from("setup_code_failures")
      .select("id", { count: "exact", head: true })
      .gte("failed_at", since);
    if (countError) {
      return jsonResponse(500, { error: countError.message });
    }
    if (tooManyFailures(count ?? 0)) {
      return jsonResponse(429, { error: "잠시 후 다시 시도하세요" });
    }

    // One use is taken in one step (claim_setup_code, migration 0011): only a known code that is not expired and has uses left.
    const { data: claimed, error: claimError } = await supabase.rpc("claim_setup_code", { p_code: code }).maybeSingle();
    if (claimError) {
      return jsonResponse(500, { error: claimError.message });
    }
    if (!claimed) {
      await supabase.from("setup_code_failures").insert({});
      return jsonResponse(404, { error: "코드가 맞지 않거나 사용 횟수가 다 찼거나 만료되었습니다" });
    }

    return jsonResponse(200, { url: claimed.url, key: claimed.key });
  } catch (error) {
    return jsonResponse(500, { error: error.message });
  }
});
