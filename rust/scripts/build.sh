#!/usr/bin/env bash
# Build the plugin into bin/ for a local copy linked with `herdr plugin link`.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p herdr-talk
mkdir -p bin
cp target/release/herdr-talk* bin/ 2>/dev/null || true
rm -f bin/*.d bin/*.pdb
