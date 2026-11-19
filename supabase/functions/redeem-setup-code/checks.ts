// Pure checks for redeem-setup-code: the code shape and the failure limit. No network, no environment: testable alone.
// Only erasable TypeScript syntax, so the same file runs under Deno (the function) and under Node (tests).

export const CODE_PATTERN = /^[0-9A-Z]{8}$/;
export const FAILURE_WINDOW_SECONDS = 600;
export const FAILURE_LIMIT = 5; // wrong codes in the window before every redeem is refused for a while // wrong codes in the window before every redeem is refused for a while

// What the user typed: spaces and dashes are dropped, letters are made uppercase. Null when the rest is not 8 characters
// of A-Z or 0-9.
export function normalizeSetupCode(text: unknown): string | null {
  if (typeof text !== "string") return null;
  const code = text.replace(/[\s-]/g, "").toUpperCase();
  return CODE_PATTERN.test(code) ? code : null;
}

// True while the failures in the window are at or above the limit.
export function tooManyFailures(recentFailures: number): boolean {
  return recentFailures >= FAILURE_LIMIT;
}
