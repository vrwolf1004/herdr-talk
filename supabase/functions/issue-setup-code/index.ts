// Supabase Edge Function `issue-setup-code` (Deno). The operator makes a setup code from the app's hidden screen.
// The operator passphrase is checked here against the secret OPERATOR_PASSPHRASE; nothing secret is in the app.
// A wrong passphrase is counted with the setup-code failures (setup_code_failures) and refused after FAILURE_LIMIT.
// The code gives the project URL (SUPABASE_URL) and the publishable key from the secret APP_PUBLISHABLE_KEY.
// Deploy (owner, once): supabase functions deploy issue-setup-code --no-verify-jwt --remote=<ref>
// Secrets (owner sets them; never in the repo):
//   supabase secrets set OPERATOR_PASSPHRASE=<passphrase> APP_PUBLISHABLE_KEY=<publishable key> --project-ref <ref>

import { createClient } from "https://esm.sh/@supabase/supabase-js@2";
import { checkIssueBody, codeFromBytes, FAILURE_WINDOW_SECONDS, sameText, tooManyFailures } from "./checks.ts";

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
    const request = checkIssueBody(body);
    if (request === null) {
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

    const passphrase = Deno.env.get("OPERATOR_PASSPHRASE") ?? "";
    const appKey = Deno.env.get("APP_PUBLISHABLE_KEY") ?? "";
    const url = Deno.env.get("SUPABASE_URL") ?? "";
    if (!passphrase || !appKey || !url) {
      return jsonResponse(503, { error: "operator secrets are not set" });
    }
    if (!sameText(request.passphrase, passphrase)) {
      await supabase.from("setup_code_failures").insert({});
      return jsonResponse(401, { error: "passphrase is wrong" });
    }

    // A new code: a few tries in case a random code is already taken (primary key clash).
    const expiresAt = new Date(Date.now() + request.days * 24 * 60 * 60 * 1000).toISOString();
    for (let attempt = 0; attempt < 5; attempt++) {
      const code = codeFromBytes(crypto.getRandomValues(new Uint8Array(8)));
      const { error: insertError } = await supabase.from("setup_codes").insert({
        code,
        url,
        key: appKey,
        expires_at: expiresAt,
        max_uses: request.max_uses,
      });
      if (!insertError) {
        return jsonResponse(200, { code, max_uses: request.max_uses, expires_at: expiresAt });
      }
      if (insertError.code !== "23505") {
        return jsonResponse(500, { error: insertError.message });
      }
    }
    return jsonResponse(500, { error: "could not make a unique code, try again" });
  } catch (error) {
    return jsonResponse(500, { error: error.message });
  }
});
