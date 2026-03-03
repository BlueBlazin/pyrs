#!/usr/bin/env bash
set -euo pipefail

echo "[wasm-vm-probe] cargo check wasm target with vm probe feature"
cargo check --target wasm32-unknown-unknown --no-default-features --features wasm-vm-probe

echo "[wasm-vm-probe] cargo build wasm target with vm probe feature (release-wasm artifact)"
cargo build --lib --target wasm32-unknown-unknown --profile release-wasm --no-default-features --features wasm-vm-probe

echo "[wasm-vm-probe] wasm vm env-import summary snapshot"
python3 scripts/generate_wasm_vm_env_import_summary.py \
  --wasm target/wasm32-unknown-unknown/release-wasm/pyrs.wasm \
  --out perf/wasm_vm_env_import_summary_latest.json

echo "[wasm-vm-probe] cargo test wasm lib harness compile-only with vm probe feature"
cargo test --target wasm32-unknown-unknown --lib --no-run --no-default-features --features wasm-vm-probe

echo "[wasm-vm-probe] cargo check wasm contract harness with vm probe feature"
cargo check --target wasm32-unknown-unknown --test wasm_contract --no-default-features --features wasm-vm-probe

echo "[wasm-vm-probe] nextest wasm bridge unit-contract regression (vm-probe)"
cargo nextest run --lib wasm_ --features wasm-vm-probe --status-level fail --final-status-level fail

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
