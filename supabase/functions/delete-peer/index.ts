// Supabase Edge Function `delete-peer` (Deno). Deletes a node's own row from public.peers when the node stops.
// Order: format (400) -> time window (401) -> Ed25519 signature (401) -> nonce not used before (401) -> delete (service role).
// A failed delete (500) gives its nonce back, so the same request can be sent again.
// The row is removed only when both pk and short_id match. Failures carry no reason in the body. Keys and bodies are never logged.
//
// Deploy (owner, once): supabase functions deploy delete-peer --no-verify-jwt
// Secrets: SUPABASE_URL and SUPABASE_SERVICE_ROLE_KEY are provided to the function by Supabase. Set nothing here.

import { checkBeforeNonce, deleteWithNonce, statusFor, type DeleteBody, type Verdict } from "./checks.ts";

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

// Records the nonce in the same table put-peer uses (the signed strings differ, so the two never collide).
// A duplicate (409 from the primary key) means the nonce was used before.
async function takeNonce(nonce: string, ts: number): Promise<"fresh" | "used" | "failed"> {
  const response = await fetch(`${BASE_URL}/rest/v1/put_nonces`, {
    method: "POST",
    headers: restHeaders({ Prefer: "return=minimal" }),
    body: JSON.stringify({ nonce, ts }),
  });
  if (response.status === 409) return "used";
  return response.ok ? "fresh" : "failed";
}

// Takes back a nonce this request recorded, after its delete failed.
async function releaseNonce(nonce: string): Promise<void> {
  await fetch(`${BASE_URL}/rest/v1/put_nonces?nonce=eq.${nonce}`, {
    method: "DELETE",
    headers: restHeaders({ Prefer: "return=minimal" }),
  });
}

// The row goes only when pk and short_id both match. Zero rows deleted is still success (already gone).
async function deletePeer(body: DeleteBody): Promise<boolean> {
  const response = await fetch(`${BASE_URL}/rest/v1/peers?pk=eq.${body.pk}&short_id=eq.${body.short_id}`, {
    method: "DELETE",
    headers: restHeaders({ Prefer: "return=minimal" }),
  });
  return response.ok;
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

  const checked = body as DeleteBody;
  const status = await deleteWithNonce({
    takeNonce: () => takeNonce(checked.nonce, checked.ts),
    deletePeer: () => deletePeer(checked),
    releaseNonce: () => releaseNonce(checked.nonce),
  });
  return new Response(null, { status });
});
