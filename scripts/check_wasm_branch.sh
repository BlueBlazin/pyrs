#!/usr/bin/env bash
set -euo pipefail

echo "[wasm-branch] cargo check (native)"
cargo check

echo "[wasm-branch] cargo check (wasm32-unknown-unknown)"
cargo check --target wasm32-unknown-unknown --no-default-features

echo "[wasm-branch] cargo check wasm contract harness"
cargo check --target wasm32-unknown-unknown --test wasm_contract --no-default-features

echo "[wasm-branch] cargo check wasm32 integration-tests compile set (default)"
cargo check --target wasm32-unknown-unknown --tests --no-default-features

echo "[wasm-branch] cargo check wasm32 integration-tests compile set (vm-probe)"
cargo check --target wasm32-unknown-unknown --tests --no-default-features --features wasm-vm-probe

echo "[wasm-branch] cargo test wasm lib unit harness (compile-only)"
cargo test --target wasm32-unknown-unknown --lib --no-run --no-default-features

echo "[wasm-branch] cargo test wasm lib unit harness (compile-only vm-probe)"
cargo test --target wasm32-unknown-unknown --lib --no-run --no-default-features --features wasm-vm-probe

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

echo "[wasm-branch] wasm api-contract surface summary snapshot"
python3 scripts/generate_wasm_api_contract_surface_summary.py \
  --out perf/wasm_api_contract_surface_summary_latest.json

echo "[wasm-branch] wasm worker docs contract summary snapshot"
python3 scripts/generate_wasm_worker_docs_contract_summary.py \
  --out perf/wasm_worker_docs_contract_summary_latest.json

echo "[wasm-branch] wasm client-flow docs summary snapshot"
python3 scripts/generate_wasm_client_flow_summary.py \
  --out perf/wasm_client_flow_summary_latest.json

echo "[wasm-branch] wasm module policy summary snapshot"
python3 scripts/generate_wasm_module_policy_summary.py \
  --out perf/wasm_module_policy_summary_latest.json

echo "[wasm-branch] wasm capability summary snapshot"
python3 scripts/generate_wasm_capability_summary.py \
  --out perf/wasm_capability_summary_latest.json

echo "[wasm-branch] playground worker contract check"
node scripts/check_playground_worker_contract.mjs \
  --out perf/wasm_playground_worker_contract_latest.json

echo "[wasm-branch] wasm host seam audit snapshot"
python3 scripts/audit_wasm_host_seam.py \
  --out perf/wasm_host_seam_audit_latest.json

echo "[wasm-branch] wasm local evidence-pack snapshot"
python3 scripts/collect_wasm_evidence_pack.py \
  --out-dir perf/wasm_evidence_pack_latest

echo "[wasm-branch] wasm local evidence-pack validation"
python3 scripts/validate_wasm_evidence_pack.py \
  --pack-dir perf/wasm_evidence_pack_latest

echo "[wasm-branch] nextest wasm bridge unit-contract regression"
cargo nextest run --lib wasm_ --status-level fail --final-status-level fail

echo "[wasm-branch] nextest host capability regression"
cargo nextest run --lib wasm_host_capability_matrix_is_explicit --status-level fail --final-status-level fail

echo "[wasm-branch] nextest host unsupported-message regression"
cargo nextest run --lib wasm_host_unsupported_messages_are_stable --status-level fail --final-status-level fail

echo "[wasm-branch] nextest host unsupported-message matrix regression"
cargo nextest run --lib wasm_host_unsupported_message_matrix_matches_supports --status-level fail --final-status-level fail

echo "[wasm-branch] nextest vm smoke regression"
cargo nextest run --test vm callable_instance_dispatch_matches_explicit_dunder_call_path --status-level fail --final-status-level fail

echo "[wasm-branch] all checks passed"
