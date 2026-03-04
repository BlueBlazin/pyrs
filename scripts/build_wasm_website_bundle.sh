#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "wasm-bindgen-cli not found; installing with cargo..." >&2
  cargo install wasm-bindgen-cli --locked
fi

if [ -d website/public/wasm ]; then
  rm -rf website/public/wasm
fi

cargo build \
  --lib \
  --target wasm32-unknown-unknown \
  --profile release-wasm \
  --no-default-features \
  --features wasm-vm-probe

WASM_INPUT="target/wasm32-unknown-unknown/release-wasm/pyrs.wasm"
if [ ! -f "${WASM_INPUT}" ]; then
  echo "expected wasm artifact not found: ${WASM_INPUT}" >&2
  exit 1
fi

wasm-bindgen \
  "${WASM_INPUT}" \
  --out-dir website/public/wasm \
  --target web \
  --out-name pyrs \
  --typescript

python3 scripts/generate_wasm_env_shim.py \
  --wasm "${WASM_INPUT}" \
  --bindgen-js website/public/wasm/pyrs.js \
  --out-shim website/public/wasm/pyrs_env.js

python3 scripts/build_wasm_stdlib_subset.py \
  --out-zip website/public/wasm/stdlib_subset_v1.zip \
  --out-manifest website/public/wasm/stdlib_subset_manifest_v1.json

echo "WASM bundle generated in website/public/wasm/ using release-wasm (size-focused profile)."
