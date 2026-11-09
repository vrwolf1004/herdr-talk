// Relay settings for the phone (Supabase only; no local/LAN mode on Android), and the trust confirmation.
// Pure functions: the screens and the app storage call these, and tests run them without React Native.

import { normalizeCode } from "./protocol.ts";

/** The peer's short ID as typed: 8 lowercase letters or digits. */
export function isPeerInput(text: string): boolean {
  return /^[a-z0-9]{8}$/.test(text);
}

export type RelaySettings = {
  url: string; // project URL, https://<ref>.supabase.co
  key: string; // publishable key (not a secret, but it is never printed in full)
  name: string; // display name, 1 to 32 characters, no control characters
  peer: string; // the peer's short ID, 8 lowercase letters or digits
};

export const FIELD_IDS = ["url", "key", "name", "peer"] as const;
export type FieldId = (typeof FIELD_IDS)[number];

export function emptySettings(): RelaySettings {
  return { url: "", key: "", name: "", peer: "" };
}

/** Field ids that are missing or not valid. Empty means the settings can be saved. */
export function missingFields(settings: RelaySettings): FieldId[] {
  const missing: FieldId[] = [];
  if (!isHttpsUrl(settings.url)) missing.push("url");
  if (settings.key.trim().length === 0) missing.push("key");
  if (!isValidName(settings.name)) missing.push("name");
  if (!isPeerInput(settings.peer)) missing.push("peer");
  return missing;
}

export function isHttpsUrl(url: string): boolean {
  return /^https:\/\/[^\s/]+$/.test(url.trim().replace(/\/+$/, ""));
}

export function isValidName(name: string): boolean {
  const count = [...name].length;
  return count >= 1 && count <= 32 && !/[\u0000-\u001f\u007f-\u009f]/.test(name);
}

/** What the screen shows for the key: its length only. */
export function keyLabel(key: string): string {
  return key.length === 0 ? "-" : `(${key.length} chars)`;
}

/** Confirmation of a peer's code: the typed code must equal the code computed from the two Noise keys. */
export function codeMatches(typed: string, computed: string): boolean {
  return normalizeCode(typed) === normalizeCode(computed) && normalizeCode(computed).length === 32;
}
