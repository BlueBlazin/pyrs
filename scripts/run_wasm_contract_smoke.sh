#!/usr/bin/env bash
set -euo pipefail

echo "[wasm-contract] cargo check (native)"
cargo check

echo "[wasm-contract] cargo check wasm contract target"
cargo check --target wasm32-unknown-unknown --test wasm_contract

if [[ "${PYRS_WASM_RUN_BROWSER_SMOKE:-0}" != "1" ]]; then
  echo "[wasm-contract] browser smoke disabled (set PYRS_WASM_RUN_BROWSER_SMOKE=1 to enable)"
  echo "[wasm-contract] compile-only smoke checks passed"
  exit 0
fi

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "[wasm-contract] wasm-pack not installed; browser smoke unavailable"
  exit 1
fi

echo "[wasm-contract] wasm-pack detected; running optional browser smoke tests"
if wasm-pack test --headless --chrome -- --test wasm_contract; then
  echo "[wasm-contract] browser smoke tests passed"
  exit 0
fi

echo "[wasm-contract] chrome smoke failed; trying firefox"
wasm-pack test --headless --firefox -- --test wasm_contract
echo "[wasm-contract] browser smoke tests passed (firefox)"
