// Operator: ask the relay for a new setup code (issue-setup-code). Same rules as Python relay_rest.py and Rust relay_rest.rs:
// https only, the passphrase goes in the body, and the answer 200 gives the code; 401, 429 and 503 say why.

import { normalizeSetupCode, isHttpsUrl } from "./settings.ts";
import { TIMEOUT_MS } from "./client.ts";

export const MAX_USES = 20;
export const DAYS = 7;

type Fetch = (url: string, init: { method: string; headers: Record<string, string>; body?: string }) => Promise<{ status: number; text: () => Promise<string> }>;

/** Body of issue-setup-code: passphrase, max_uses 20, days 7. */
export function issueBody(passphrase: string): string {
  return JSON.stringify({ passphrase, max_uses: MAX_USES, days: DAYS });
}

/** The new code from an issue-setup-code answer. Throws with the reason for anything but 200. */
export function parseIssue(status: number, body: string): string {
  if (status === 401) throw new Error("the operator passphrase is wrong");
  if (status === 429) throw new Error("too many wrong tries; try later");
  if (status === 503) throw new Error("the operator secrets are not set on the server");
  if (status !== 200) throw new Error(`issue status ${status}`);
  let code: unknown;
  try {
    code = (JSON.parse(body) as { code?: unknown }).code;
  } catch {
    throw new Error("issue answer is not JSON");
  }
  const checked = typeof code === "string" ? normalizeSetupCode(code) : null;
  if (checked === null) throw new Error("issue answer has no valid code");
  return checked;
}

/** Ask for a new setup code at `url`. Nothing is sent unless the URL is https. Throws with the reason. */
export async function issueSetupCode(url: string, passphrase: string, fetcher: Fetch = fetch as unknown as Fetch): Promise<string> {
  if (!isHttpsUrl(url)) throw new Error("the relay URL must start with https://; nothing was sent");
  let response: Awaited<ReturnType<Fetch>>;
  try {
    response = await Promise.race([
      fetcher(`${url.trim().replace(/\/+$/, "")}/functions/v1/issue-setup-code`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: issueBody(passphrase),
      }),
      new Promise<never>((_, reject) => setTimeout(() => reject(new Error("timeout")), TIMEOUT_MS)),
    ]);
  } catch {
    throw new Error("the server could not be reached");
  }
  return parseIssue(response.status, await response.text());
}
