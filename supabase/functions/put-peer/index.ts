// Supabase Edge Function `put-peer` (Deno). The only writer of public.peers.
// Order: format (400) -> time window (401) -> Ed25519 signature (401) -> nonce not used before (401) -> upsert (service role).
// A short_id that belongs to another pk makes the unique column fail: PostgREST answers 409, and the client picks another.
// Failures carry no reason in the body. Keys and bodies are never logged.
//
// Deploy (owner, once): supabase functions deploy put-peer
// Secrets: SUPABASE_URL and SUPABASE_SERVICE_ROLE_KEY are provided to the function by Supabase. Set nothing here.

import { checkBeforeNonce, statusFor, type PutBody, type Verdict } from "./checks.ts";

const SERVICE_KEY = Deno.env.get("SUPABASE_SERVICE_ROLE_KEY") ?? "";
const BASE_URL = Deno.env.get("SUPABASE_URL") ?? "";

function restHeaders(extra: Record<string, string> = {}): Record<string, string> {
  return {
    apikey: SERVICE_KEY,
    Authorization: `Bearer ${SERVICE_KEY}`,
    "Content-Type": "application/json",
    ...extra,
  };
}

// Records the nonce. A duplicate (409 from the primary key) means the nonce was used before.
async function takeNonce(nonce: string, ts: number): Promise<"fresh" | "used" | "failed"> {
  const response = await fetch(`${BASE_URL}/rest/v1/put_nonces`, {
    method: "POST",
    headers: restHeaders({ Prefer: "return=minimal" }),
    body: JSON.stringify({ nonce, ts }),
  });
  if (response.status === 409) return "used";
  return response.ok ? "fresh" : "failed";
}

// "taken": the short_id already belongs to another pk (unique violation, 409). The same pk keeps its short_id.
async function upsertPeer(body: PutBody): Promise<"ok" | "taken" | "failed"> {
  const response = await fetch(`${BASE_URL}/rest/v1/peers?on_conflict=pk`, {
    method: "POST",
    headers: restHeaders({ Prefer: "resolution=merge-duplicates,return=minimal" }),
    body: JSON.stringify({
      pk: body.pk,
      noise_pk: body.noise_pk,
      short_id: body.short_id,
      name: body.name,
      updated_at: new Date().toISOString(),
    }),
  });
  if (response.status === 409) return "taken";
  return response.ok ? "ok" : "failed";
}

function answer(verdict: Verdict): Response {
  return new Response(null, { status: statusFor(verdict) });
}

Deno.serve(async (request: Request): Promise<Response> => {
  if (request.method !== "POST") {
    return new Response(null, { status: 405 });
  }
  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return answer("format");
  }
  const nowSeconds = Math.floor(Date.now() / 1000);
  const verdict = await checkBeforeNonce(body, nowSeconds);
  if (verdict !== "ok") return answer(verdict);

  const checked = body as PutBody;
  const nonce = await takeNonce(checked.nonce, checked.ts);
  if (nonce === "used") return answer("nonce");
  if (nonce === "failed") return new Response(null, { status: 500 });

  const written = await upsertPeer(checked);
  if (written === "taken") return new Response(null, { status: 409 });
  if (written === "failed") return new Response(null, { status: 500 });
  return answer("ok");
});
