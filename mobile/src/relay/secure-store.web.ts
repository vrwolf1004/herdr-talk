// Web: expo-secure-store has no web build, so the same three calls use localStorage.
// Dev-only fallback: values are readable by any script on the page. Never use this build for real keys.
export async function getItemAsync(key: string): Promise<string | null> {
  try {
    return globalThis.localStorage?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

export async function setItemAsync(key: string, value: string): Promise<void> {
  globalThis.localStorage?.setItem(key, value);
}

export async function deleteItemAsync(key: string): Promise<void> {
  globalThis.localStorage?.removeItem(key);
}