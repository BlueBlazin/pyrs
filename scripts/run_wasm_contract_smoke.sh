#!/usr/bin/env bash
set -euo pipefail

echo "[wasm-contract] cargo check (native)"
cargo check

echo "[wasm-contract] cargo check wasm contract target"
cargo check --target wasm32-unknown-unknown --test wasm_contract

echo "[wasm-contract] cargo test wasm lib unit harness (compile-only)"
cargo test --target wasm32-unknown-unknown --lib --no-run

echo "[wasm-contract] wasm vm probe lane"
scripts/probe_wasm_vm_compile.sh

echo "[wasm-contract] wasm worker contract summary snapshot"
python3 scripts/generate_wasm_worker_contract_summary.py \
  --out perf/wasm_worker_contract_summary_latest.json

echo "[wasm-contract] wasm execute contract summary snapshot"
python3 scripts/generate_wasm_execute_contract_summary.py \
  --out perf/wasm_execute_contract_summary_latest.json

echo "[wasm-contract] wasm session contract summary snapshot"
python3 scripts/generate_wasm_session_contract_summary.py \
  --out perf/wasm_session_contract_summary_latest.json

echo "[wasm-contract] wasm docs execution matrix summary snapshot"
python3 scripts/generate_wasm_docs_execution_matrix_summary.py \
  --out perf/wasm_docs_execution_matrix_summary_latest.json

echo "[wasm-contract] wasm worker docs contract summary snapshot"
python3 scripts/generate_wasm_worker_docs_contract_summary.py \
  --out perf/wasm_worker_docs_contract_summary_latest.json

echo "[wasm-contract] wasm client-flow docs summary snapshot"
python3 scripts/generate_wasm_client_flow_summary.py \
  --out perf/wasm_client_flow_summary_latest.json

echo "[wasm-contract] wasm module policy summary snapshot"
python3 scripts/generate_wasm_module_policy_summary.py \
  --out perf/wasm_module_policy_summary_latest.json

echo "[wasm-contract] wasm capability summary snapshot"
python3 scripts/generate_wasm_capability_summary.py \
  --out perf/wasm_capability_summary_latest.json

echo "[wasm-contract] wasm host seam audit snapshot"
python3 scripts/audit_wasm_host_seam.py \
  --out perf/wasm_host_seam_audit_latest.json

echo "[wasm-contract] nextest host capability regression"
cargo nextest run --lib wasm_host_capability_matrix_is_explicit --status-level fail --final-status-level fail

echo "[wasm-contract] nextest host unsupported-message regression"
cargo nextest run --lib wasm_host_unsupported_messages_are_stable --status-level fail --final-status-level fail

echo "[wasm-contract] nextest host unsupported-message matrix regression"
cargo nextest run --lib wasm_host_unsupported_message_matrix_matches_supports --status-level fail --final-status-level fail

if [[ "${PYRS_WASM_RUN_BROWSER_SMOKE:-0}" != "1" ]]; then
  echo "[wasm-contract] browser smoke disabled (set PYRS_WASM_RUN_BROWSER_SMOKE=1 to enable)"
  echo "[wasm-contract] compile-only smoke checks passed"
  exit 0
fi

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "[wasm-contract] wasm-pack not installed; browser smoke unavailable"
  exit 1
fi

run_browser_smoke() {
  local browser="$1"
  echo "[wasm-contract] wasm-pack ${browser}: integration contract tests"
  wasm-pack test --headless --"${browser}" -- --test wasm_contract
  echo "[wasm-contract] wasm-pack ${browser}: lib unit tests"
  wasm-pack test --headless --"${browser}" -- --lib
}

echo "[wasm-contract] wasm-pack detected; running optional browser smoke tests"
if run_browser_smoke chrome; then
  echo "[wasm-contract] browser smoke tests passed"
  exit 0
fi

echo "[wasm-contract] chrome smoke failed; trying firefox"
run_browser_smoke firefox
echo "[wasm-contract] browser smoke tests passed (firefox)"
