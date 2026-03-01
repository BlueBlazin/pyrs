#!/usr/bin/env bash
set -euo pipefail

echo "[wasm-vm-probe] cargo check wasm target with vm probe feature"
cargo check --target wasm32-unknown-unknown --features wasm-vm-probe

echo "[wasm-vm-probe] cargo test wasm lib harness compile-only with vm probe feature"
cargo test --target wasm32-unknown-unknown --lib --no-run --features wasm-vm-probe

echo "[wasm-vm-probe] cargo check wasm contract harness with vm probe feature"
cargo check --target wasm32-unknown-unknown --test wasm_contract --features wasm-vm-probe

echo "[wasm-vm-probe] wasm vm native-link blocker snapshot"
python3 scripts/generate_wasm_vm_link_blockers_summary.py \
  --out perf/wasm_vm_link_blockers_latest.json

echo "[wasm-vm-probe] wasm worker contract summary snapshot (vm-probe)"
python3 scripts/generate_wasm_worker_contract_summary.py \
  --vm-probe \
  --out perf/wasm_worker_contract_summary_vm_probe_latest.json

echo "[wasm-vm-probe] wasm execute contract summary snapshot (vm-probe)"
python3 scripts/generate_wasm_execute_contract_summary.py \
  --vm-probe \
  --out perf/wasm_execute_contract_summary_vm_probe_latest.json
