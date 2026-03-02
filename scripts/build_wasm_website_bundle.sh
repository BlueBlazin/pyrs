#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "wasm-pack is required. Install with: cargo install wasm-pack --locked" >&2
  exit 1
fi

if [ -d website/public/wasm ]; then
  rm -rf website/public/wasm
fi

wasm-pack build \
  --target web \
  --out-dir website/public/wasm \
  --out-name pyrs \
  --release \
  -- \
  --profile release-wasm \
  --features wasm-vm-probe

echo "WASM bundle generated in website/public/wasm/ using release-wasm (size-focused profile)."
