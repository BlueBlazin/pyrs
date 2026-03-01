#!/usr/bin/env bash
set -euo pipefail

echo "[wasm-branch] cargo check (native)"
cargo check

echo "[wasm-branch] cargo check (wasm32-unknown-unknown)"
cargo check --target wasm32-unknown-unknown

echo "[wasm-branch] cargo check wasm contract harness"
cargo check --target wasm32-unknown-unknown --test wasm_contract

echo "[wasm-branch] cargo test wasm lib unit harness (compile-only)"
cargo test --target wasm32-unknown-unknown --lib --no-run

echo "[wasm-branch] wasm vm probe lane"
scripts/probe_wasm_vm_compile.sh

echo "[wasm-branch] wasm worker contract summary snapshot"
python3 scripts/generate_wasm_worker_contract_summary.py \
  --out perf/wasm_worker_contract_summary_latest.json

echo "[wasm-branch] wasm execute contract summary snapshot"
python3 scripts/generate_wasm_execute_contract_summary.py \
  --out perf/wasm_execute_contract_summary_latest.json

echo "[wasm-branch] wasm session contract summary snapshot"
python3 scripts/generate_wasm_session_contract_summary.py \
  --out perf/wasm_session_contract_summary_latest.json

echo "[wasm-branch] wasm docs execution matrix summary snapshot"
python3 scripts/generate_wasm_docs_execution_matrix_summary.py \
  --out perf/wasm_docs_execution_matrix_summary_latest.json

echo "[wasm-branch] wasm module policy summary snapshot"
python3 scripts/generate_wasm_module_policy_summary.py \
  --out perf/wasm_module_policy_summary_latest.json

echo "[wasm-branch] wasm capability summary snapshot"
python3 scripts/generate_wasm_capability_summary.py \
  --out perf/wasm_capability_summary_latest.json

echo "[wasm-branch] wasm host seam audit snapshot"
python3 scripts/audit_wasm_host_seam.py \
  --out perf/wasm_host_seam_audit_latest.json

echo "[wasm-branch] nextest host capability regression"
cargo nextest run --lib wasm_host_capability_matrix_is_explicit --status-level fail --final-status-level fail

echo "[wasm-branch] nextest host unsupported-message regression"
cargo nextest run --lib wasm_host_unsupported_messages_are_stable --status-level fail --final-status-level fail

echo "[wasm-branch] nextest host unsupported-message matrix regression"
cargo nextest run --lib wasm_host_unsupported_message_matrix_matches_supports --status-level fail --final-status-level fail

echo "[wasm-branch] nextest vm smoke regression"
cargo nextest run --test vm callable_instance_dispatch_matches_explicit_dunder_call_path --status-level fail --final-status-level fail

echo "[wasm-branch] all checks passed"
