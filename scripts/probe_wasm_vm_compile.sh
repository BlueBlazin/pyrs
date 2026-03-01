#!/usr/bin/env bash
set -euo pipefail

echo "[wasm-vm-probe] cargo check wasm target with vm probe feature"
cargo check --target wasm32-unknown-unknown --features wasm-vm-probe

echo "[wasm-vm-probe] wasm vm native-link blocker snapshot"
python3 scripts/generate_wasm_vm_link_blockers_summary.py \
  --out perf/wasm_vm_link_blockers_latest.json
