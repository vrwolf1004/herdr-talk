#!/usr/bin/env bash
# herdr's `[[build]]` step: put this platform's binary in bin/, from the release or, with HERDR_TALK_BUILD_FROM_SOURCE=1, from source.
set -euo pipefail

NAME="herdr-talk"
# Every line this step prints starts with the plugin's name, as the action lines do.
SAY="talk"
REPO="vrwolf1004/herdr-talk"
# minisign public key that signs the release checksums (herdr/install.ps1 has the same key).
# Empty until the release key exists; the install stops until it is set.
MINISIGN_PUBKEY=""

# The root from this script's place: build commands may not receive herdr's runtime env.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="$ROOT/bin"

# Source build: herdr runs this without a terminal, so the choice is an env var, not a prompt.
if [ "${HERDR_TALK_BUILD_FROM_SOURCE:-}" = "1" ]; then
  command -v cargo >/dev/null 2>&1 || { echo "$SAY: HERDR_TALK_BUILD_FROM_SOURCE=1 but cargo is not on PATH" >&2; exit 1; }
  (cd "$ROOT" && cargo build --release -p "$NAME")
  mkdir -p "$BIN_DIR"
  install -m 0755 "$ROOT/target/release/$NAME" "$BIN_DIR/$NAME"
  echo "$SAY: built $BIN_DIR/$NAME from source"
  exit 0
fi

if [ -z "$MINISIGN_PUBKEY" ]; then
  echo "$SAY: no release signing key is set in herdr/install.sh; use HERDR_TALK_BUILD_FROM_SOURCE=1 to build instead" >&2
  exit 1
fi
command -v minisign >/dev/null 2>&1 || {
  echo "$SAY: minisign is needed to verify the release (brew install minisign, apt install minisign); or set HERDR_TALK_BUILD_FROM_SOURCE=1" >&2
  exit 1
}

# The release tag matches the manifest version, so a checkout always pulls its own release.
VERSION="$(grep -m1 '^version' "$ROOT/herdr-plugin.toml" | sed -E 's/.*"([^"]+)".*/\1/')"
TAG="v${VERSION}"

# Map the running platform to the release target triple.
os="$(uname -s)"
arch="$(uname -m)"
case "$os-$arch" in
  Darwin-arm64)              target="aarch64-apple-darwin" ;;
  Darwin-x86_64)             target="x86_64-apple-darwin" ;;
  Linux-aarch64 | Linux-arm64) target="aarch64-unknown-linux-musl" ;;
  Linux-x86_64)              target="x86_64-unknown-linux-musl" ;;
  *)
    echo "$SAY: no prebuilt binary for $os-$arch, build with 'cargo build --release' and copy target/release/herdr-talk into bin/" >&2
    exit 1
    ;;
esac

archive="${NAME}-${target}.tar.gz"
# taiki-e's checksum sidecar drops the archive extension: <name>-<target>.sha256, not <archive>.sha256.
checksum="${NAME}-${target}.sha256"
base="https://github.com/${REPO}/releases/download/${TAG}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# GitHub's CDN can 404 a fresh release's asset for minutes, so every failure retries, here
# rather than with curl's retry flags, which older curls (RHEL 8, CentOS 7) lack.
dl() {
  for _ in 1 2 3 4 5 6; do
    curl -fsSL "$1" -o "$2" && return
    sleep 3
  done
  return 1
}

echo "$SAY: downloading $archive ($TAG)"
dl "$base/$archive" "$tmp/$archive"
dl "$base/$checksum" "$tmp/$checksum"
dl "$base/$checksum.minisig" "$tmp/$checksum.minisig"

# The signature covers the checksum file, so checking the archive against it proves the release.
echo "$SAY: verifying signature"
minisign -q -V -P "$MINISIGN_PUBKEY" -m "$tmp/$checksum" -x "$tmp/$checksum.minisig" ||
  { echo "$SAY: signature check failed for $checksum" >&2; exit 1; }

echo "$SAY: verifying checksum"
expected="$(awk '{print $1}' "$tmp/$checksum")"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp/$archive" | awk '{print $1}')"
else
  actual="$(shasum -a 256 "$tmp/$archive" | awk '{print $1}')"
fi
if [ "$expected" != "$actual" ]; then
  echo "$SAY: checksum mismatch (expected $expected, got $actual)" >&2
  exit 1
fi

mkdir -p "$BIN_DIR"
tar -xzf "$tmp/$archive" -C "$tmp"
install -m 0755 "$tmp/$NAME" "$BIN_DIR/$NAME"
echo "$SAY: installed $BIN_DIR/$NAME"
