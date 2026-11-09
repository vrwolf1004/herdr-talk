// Byte helpers for the relay: hex, base64 (RFC 4648, padded) and UTF-8. No native code, no Node-only APIs.

const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

export function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

export function bytesToHex(bytes: Uint8Array): string {
  let out = "";
  for (const byte of bytes) {
    out += byte.toString(16).padStart(2, "0");
  }
  return out;
}

export function concatBytes(...parts: Uint8Array[]): Uint8Array {
  const total = parts.reduce((sum, part) => sum + part.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

export function utf8Encode(text: string): Uint8Array {
  const out: number[] = [];
  for (const ch of text) {
    const code = ch.codePointAt(0) ?? 0;
    if (code < 0x80) {
      out.push(code);
    } else if (code < 0x800) {
      out.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
    } else if (code < 0x10000) {
      out.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
    } else {
      out.push(0xf0 | (code >> 18), 0x80 | ((code >> 12) & 0x3f), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
    }
  }
  return Uint8Array.from(out);
}

export function utf8Decode(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; ) {
    const first = bytes[i];
    const length = first < 0x80 ? 1 : first >= 0xf0 ? 4 : first >= 0xe0 ? 3 : first >= 0xc0 ? 2 : 0;
    if (length === 0 || i + length > bytes.length) {
      throw new Error("invalid UTF-8");
    }
    let code = length === 1 ? first : first & (0xff >> (length + 1));
    for (let k = 1; k < length; k++) {
      code = (code << 6) | (bytes[i + k] & 0x3f);
    }
    out += String.fromCodePoint(code);
    i += length;
  }
  return out;
}

export function base64Encode(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const a = bytes[i];
    const b = i + 1 < bytes.length ? bytes[i + 1] : 0;
    const c = i + 2 < bytes.length ? bytes[i + 2] : 0;
    const n = (a << 16) | (b << 8) | c;
    out += B64[(n >> 18) & 63] + B64[(n >> 12) & 63];
    out += i + 1 < bytes.length ? B64[(n >> 6) & 63] : "=";
    out += i + 2 < bytes.length ? B64[n & 63] : "=";
  }
  return out;
}

/** Padded standard base64. Null for anything else (the same rule as the Rust side). */
export function base64Decode(text: string): Uint8Array | null {
  if (text.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(text)) {
    return null;
  }
  const out: number[] = [];
  for (let i = 0; i < text.length; i += 4) {
    const pad = text.endsWith("==", i + 4) ? 2 : text[i + 3] === "=" ? 1 : 0;
    let n = 0;
    for (let k = 0; k < 4; k++) {
      const ch = text[i + k];
      const value = ch === "=" ? 0 : B64.indexOf(ch);
      n = (n << 6) | value;
    }
    out.push((n >> 16) & 0xff);
    if (pad < 2) out.push((n >> 8) & 0xff);
    if (pad < 1) out.push(n & 0xff);
  }
  return Uint8Array.from(out);
}
