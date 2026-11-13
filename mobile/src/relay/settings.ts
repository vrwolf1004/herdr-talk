// Relay settings for the phone (Supabase only; no local/LAN mode on Android), and the trust confirmation.
// Pure functions: the screens and the app storage call these, and tests run them without React Native.

import { normalizeCode } from "./protocol.ts";

/** A room ID: 8 uppercase letters or digits. Input ignores spaces, dashes, and case. */
export function isRoomCode(text: string): boolean {
  return /^[A-Z0-9]{8}$/.test(text);
}

/** The signed `put-peer` row still uses this PC's short ID; it shares the room-ID alphabet. */
export function isPeerInput(text: string): boolean {
  return isRoomCode(text);
}

export function normalizeRoomCode(text: string): string {
  return text.replace(/[\s-]/g, "").toUpperCase();
}

export type RelaySettings = {
  url: string; // project URL, https://<ref>.supabase.co
  key: string; // publishable key (not a secret, but it is never printed in full)
  name: string; // display name, 1 to 32 characters, no control characters; assigned by the app, never typed
  roomCode: string; // the room ID, 8 uppercase letters or digits
};

// The display name is not a field: the app assigns it (store.loadOrCreateDisplayName).
export const FIELD_IDS = ["url", "key", "roomCode"] as const;
export type FieldId = (typeof FIELD_IDS)[number];

export function emptySettings(): RelaySettings {
  return { url: "", key: "", name: "", roomCode: "" };
}

/** Field ids that are missing or not valid. Empty means the settings can be saved. */
export function missingFields(settings: RelaySettings): FieldId[] {
  const missing: FieldId[] = [];
  if (!isHttpsUrl(settings.url)) missing.push("url");
  if (settings.key.trim().length === 0) missing.push("key");
  if (!isRoomCode(settings.roomCode)) missing.push("roomCode");
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
  return normalizeCode(typed) === normalizeCode(computed) && normalizeCode(computed).length === 8;
}
