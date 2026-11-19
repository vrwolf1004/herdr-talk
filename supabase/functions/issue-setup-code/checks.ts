// Pure checks for issue-setup-code: the request shape, the code alphabet, and the passphrase comparison.
// No network, no environment. Only erasable TypeScript syntax, so the same file runs under Deno and Node.

export const CODE_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
export const MAX_USES_LIMIT = 1000;
export const DAYS_LIMIT = 30;

export interface IssueBody {
  passphrase: string;
  max_uses: number;
  days: number;
}

// The body: a passphrase string, max_uses 1 to 1000, days 1 to 30. Anything else is null.
export function checkIssueBody(body: unknown): IssueBody | null {
  if (typeof body !== "object" || body === null) return null;
  const { passphrase, max_uses, days } = body as Record<string, unknown>;
  if (typeof passphrase !== "string" || passphrase.length === 0 || passphrase.length > 200) return null;
  if (!Number.isSafeInteger(max_uses) || (max_uses as number) < 1 || (max_uses as number) > MAX_USES_LIMIT) return null;
  if (!Number.isSafeInteger(days) || (days as number) < 1 || (days as number) > DAYS_LIMIT) return null;
  return { passphrase, max_uses: max_uses as number, days: days as number };
}

// An 8-character code of A-Z and 0-9 from the given random bytes (one byte per character, rejection-free by mod of 36).
// shortcut: the mod 36 bias is below 1 percent per character; use rejection sampling if codes must be perfectly uniform.
export function codeFromBytes(bytes: Uint8Array): string {
  return Array.from(bytes.slice(0, 8), (byte) => CODE_ALPHABET[byte % CODE_ALPHABET.length]).join("");
}

// Equal strings compared over every character (no early exit).
export function sameText(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

// The failure limit (the same numbers as redeem-setup-code/checks.ts; kept here so this function deploys alone).
export const FAILURE_WINDOW_SECONDS = 600;
export const FAILURE_LIMIT = 10;

// True while the failures in the window are at or above the limit.
export function tooManyFailures(recentFailures: number): boolean {
  return recentFailures >= FAILURE_LIMIT;
}
