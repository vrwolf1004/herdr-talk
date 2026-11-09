// Phone storage for the relay: settings and the trusted peer keys in AsyncStorage, the two private keys in
// expo-secure-store (never in AsyncStorage). Random bytes come from expo-crypto (the platform CSPRNG).

import AsyncStorage from "@react-native-async-storage/async-storage";
import { NativeModules } from "react-native";
import * as SecureStore from "expo-secure-store";
import { getRandomBytes } from "expo-crypto";
import { bytesToHex, hexToBytes } from "./bytes.ts";
import { publicKeyOf } from "./noise.ts";
import { emptySettings, isPeerInput, isValidName, type RelaySettings } from "./settings.ts";
import { type Keys } from "./engine.ts";
import { signingPublicKey } from "./protocol.ts";
import { namesFor } from "./names.ts";

// Settings: encrypted in expo-secure-store (Android Keystore). The old AsyncStorage copy is only read once, to move it.
const SETTINGS_KEY = "herdr-talk.relay.settings";
const SECURE_SETTINGS_KEY = "herdr-talk.relay.settings-secure";
const TRUSTED_KEY = "herdr-talk.relay.trusted";
const SIGNING_SEED_KEY = "herdr-talk.relay.signing-seed";
const NOISE_PRIVATE_KEY = "herdr-talk.relay.noise-private";
const SHORT_ID_KEY = "herdr-talk.relay.short-id";
const DISPLAY_NAME_KEY = "herdr-talk.relay.display-name";
const SHORT_ID_ALPHABET = "abcdefghijklmnopqrstuvwxyz0123456789";

/** Random bytes from expo-crypto (the platform CSPRNG). Throws when no bytes come back: never a weak key. */
export function randomBytes(length: number): Uint8Array {
  const bytes = getRandomBytes(length);
  if (bytes.length !== length) {
    throw new Error('no secure random source on this device');
  }
  return bytes;
}

function parseSettings(text: string | null): RelaySettings | null {
  if (!text) return null;
  try {
    const value = JSON.parse(text) as Partial<RelaySettings>;
    return { ...emptySettings(), ...value };
  } catch {
    return null;
  }
}

/**
 * App defaults from mobile/.env (EXPO_PUBLIC_RELAY_URL, _KEY, _PEER). Expo inlines them at build time.
 * The display name is not part of these defaults: the app assigns it (loadOrCreateDisplayName). Nothing here logs or prints a value.
 */
export function defaultSettings(): RelaySettings {
  return {
    url: process.env.EXPO_PUBLIC_RELAY_URL ?? "",
    key: process.env.EXPO_PUBLIC_RELAY_KEY ?? "",
    name: "",
    peer: process.env.EXPO_PUBLIC_RELAY_PEER ?? "",
  };
}

export type LoadedSettings = { settings: RelaySettings; fromDefaults: boolean };

/**
 * The saved settings if there are any (a saved value always wins over the defaults); otherwise the defaults.
 * Saved values are read from the encrypted store; a copy left in AsyncStorage by an older build is moved over once.
 */
export async function loadSettings(): Promise<LoadedSettings> {
  const secure = parseSettings(await SecureStore.getItemAsync(SECURE_SETTINGS_KEY));
  if (secure) return { settings: secure, fromDefaults: false };
  const legacy = parseSettings(await AsyncStorage.getItem(SETTINGS_KEY));
  if (legacy) {
    await SecureStore.setItemAsync(SECURE_SETTINGS_KEY, JSON.stringify(legacy));
    await AsyncStorage.removeItem(SETTINGS_KEY);
    return { settings: legacy, fromDefaults: false };
  }
  return { settings: defaultSettings(), fromDefaults: true };
}

/** This phone's language as the platform reports it (for example ko_KR); the JS engine's locale when that is missing. */
function phoneLocale(): string {
  const native: unknown = NativeModules.I18nManager?.localeIdentifier;
  return typeof native === "string" && native.length > 0 ? native : Intl.DateTimeFormat().resolvedOptions().locale;
}

/** A person's name for this phone's language, picked at random (talk-core's list, see names.ts). */
export function defaultDisplayName(): string {
  const names = namesFor(phoneLocale());
  const [high, low] = randomBytes(2);
  return names[((high << 8) | low) % names.length];
}

/** Save to the encrypted store. A failure throws (the caller shows a message without any value). */
export async function saveSettings(settings: RelaySettings): Promise<void> {
  await SecureStore.setItemAsync(SECURE_SETTINGS_KEY, JSON.stringify(settings));
  await AsyncStorage.removeItem(SETTINGS_KEY);
}

/** Remove every saved relay setting (the trusted peer keys are kept). */
export async function clearSettings(): Promise<void> {
  await SecureStore.deleteItemAsync(SECURE_SETTINGS_KEY);
  await AsyncStorage.removeItem(SETTINGS_KEY);
}

/** Noise public keys (hex) of the peers the user confirmed with the code. */
export async function loadTrusted(): Promise<string[]> {
  const text = await AsyncStorage.getItem(TRUSTED_KEY);
  if (!text) return [];
  try {
    const value: unknown = JSON.parse(text);
    return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
  } catch {
    return [];
  }
}

export async function addTrusted(noisePublicHex: string): Promise<string[]> {
  const trusted = await loadTrusted();
  if (!trusted.includes(noisePublicHex)) trusted.push(noisePublicHex);
  await AsyncStorage.setItem(TRUSTED_KEY, JSON.stringify(trusted));
  return trusted;
}

/** A new short ID: 8 random letters or digits. Bytes from 252 up are skipped, so all 36 characters are equally likely. */
export function newShortId(): string {
  let id = "";
  while (id.length < 8) {
    const [byte] = randomBytes(1);
    if (byte < 252) id += SHORT_ID_ALPHABET[byte % SHORT_ID_ALPHABET.length];
  }
  return id;
}

/** This phone's short ID, made on the first use and kept in secure storage. A damaged value is an error, not replaced. */
export async function loadOrCreateShortId(): Promise<string> {
  const saved = await SecureStore.getItemAsync(SHORT_ID_KEY);
  if (!saved) {
    const id = newShortId();
    await SecureStore.setItemAsync(SHORT_ID_KEY, id);
    return id;
  }
  if (!isPeerInput(saved)) throw new Error("short ID in secure storage is damaged");
  return saved;
}

/** This phone's display name: assigned once, kept in secure storage, and used for every row write. Any other saved name is replaced. */
export async function loadOrCreateDisplayName(): Promise<string> {
  const saved = await SecureStore.getItemAsync(DISPLAY_NAME_KEY);
  if (saved && isValidName(saved)) return saved;
  const name = defaultDisplayName();
  await SecureStore.setItemAsync(DISPLAY_NAME_KEY, name);
  return name;
}

/** Forget the assigned display name (the next assignment is a new name). */
export async function clearDisplayName(): Promise<void> {
  await SecureStore.deleteItemAsync(DISPLAY_NAME_KEY);
}

/** After the relay answered 409 (another key has this short ID): keep a new one. */
export async function replaceShortId(): Promise<string> {
  const id = newShortId();
  await SecureStore.setItemAsync(SHORT_ID_KEY, id);
  return id;
}

/** The two private keys, made on the first run and kept only in secure storage. */
export async function loadOrCreateKeys(): Promise<{ keys: Keys; signingSeed: Uint8Array; signingPublicHex: string }> {
  let seedHex = await SecureStore.getItemAsync(SIGNING_SEED_KEY);
  if (!seedHex) {
    seedHex = bytesToHex(randomBytes(32));
    await SecureStore.setItemAsync(SIGNING_SEED_KEY, seedHex);
  }
  let noiseHex = await SecureStore.getItemAsync(NOISE_PRIVATE_KEY);
  if (!noiseHex) {
    noiseHex = bytesToHex(randomBytes(32));
    await SecureStore.setItemAsync(NOISE_PRIVATE_KEY, noiseHex);
  }
  const noisePrivate = hexToBytes(noiseHex);
  const signingSeed = hexToBytes(seedHex);
  return {
    keys: { noisePrivate, noisePublic: publicKeyOf(noisePrivate) },
    signingSeed,
    signingPublicHex: bytesToHex(signingPublicKey(signingSeed)),
  };
}
