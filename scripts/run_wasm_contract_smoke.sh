#!/usr/bin/env bash
set -euo pipefail

# Default behavior runs full local gate checks; browser-focused CI lanes can
# set PYRS_WASM_SKIP_CORE_SMOKE=1 after core checks have already passed.
if [[ "${PYRS_WASM_SKIP_CORE_SMOKE:-0}" != "1" ]]; then
  echo "[wasm-contract] cargo check (native)"
  cargo check

  echo "[wasm-contract] cargo check wasm contract target"
  cargo check --target wasm32-unknown-unknown --test wasm_contract --no-default-features

  echo "[wasm-contract] cargo check wasm32 integration-tests compile set (default)"
  cargo check --target wasm32-unknown-unknown --tests --no-default-features

  echo "[wasm-contract] cargo check wasm32 integration-tests compile set (vm-probe)"
  cargo check --target wasm32-unknown-unknown --tests --no-default-features --features wasm-vm-probe

  echo "[wasm-contract] cargo test wasm lib unit harness (compile-only)"
  cargo test --target wasm32-unknown-unknown --lib --no-run --no-default-features

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

  echo "[wasm-contract] wasm api-contract surface summary snapshot"
  python3 scripts/generate_wasm_api_contract_surface_summary.py \
    --out perf/wasm_api_contract_surface_summary_latest.json

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

  echo "[wasm-contract] wasm local evidence-pack snapshot"
  python3 scripts/collect_wasm_evidence_pack.py \
    --out-dir perf/wasm_evidence_pack_latest

  echo "[wasm-contract] wasm local evidence-pack validation"
  python3 scripts/validate_wasm_evidence_pack.py \
    --pack-dir perf/wasm_evidence_pack_latest

  echo "[wasm-contract] nextest wasm bridge unit-contract regression"
  cargo nextest run --lib wasm_ --status-level fail --final-status-level fail

  echo "[wasm-contract] nextest host capability regression"
  cargo nextest run --lib wasm_host_capability_matrix_is_explicit --status-level fail --final-status-level fail

  echo "[wasm-contract] nextest host unsupported-message regression"
  cargo nextest run --lib wasm_host_unsupported_messages_are_stable --status-level fail --final-status-level fail

  echo "[wasm-contract] nextest host unsupported-message matrix regression"
  cargo nextest run --lib wasm_host_unsupported_message_matrix_matches_supports --status-level fail --final-status-level fail
else
  echo "[wasm-contract] skipping core smoke checks (PYRS_WASM_SKIP_CORE_SMOKE=1)"
fi

if [[ "${PYRS_WASM_RUN_BROWSER_SMOKE:-0}" != "1" ]]; then
  echo "[wasm-contract] browser smoke disabled (set PYRS_WASM_RUN_BROWSER_SMOKE=1 to enable)"
  if [[ "${PYRS_WASM_SKIP_CORE_SMOKE:-0}" == "1" ]]; then
    echo "[wasm-contract] browser-only mode complete"
  else
    echo "[wasm-contract] compile-only smoke checks passed"
  fi
  exit 0
fi

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "[wasm-contract] wasm-pack not installed; browser smoke unavailable"
  exit 1
fi

vm_probe_browser_smoke_enabled=0

configure_browser_test_timeout() {
  local timeout_seconds="${PYRS_WASM_BROWSER_TEST_TIMEOUT_SECONDS:-180}"
  export WASM_BINDGEN_TEST_TIMEOUT="${timeout_seconds}"
  echo "[wasm-contract] wasm browser test timeout: ${WASM_BINDGEN_TEST_TIMEOUT}s"
}

run_browser_smoke() {
  local browser="$1"
  local smoke_status=0
  echo "[wasm-contract] wasm-pack ${browser}: integration contract tests"
  if ! wasm-pack test --headless --"${browser}" --test wasm_contract --no-default-features; then
    smoke_status=1
  fi
  echo "[wasm-contract] wasm-pack ${browser}: lib unit tests"
  if ! wasm-pack test --headless --"${browser}" --lib --no-default-features; then
    smoke_status=1
  fi
  return "${smoke_status}"
}

run_vm_probe_state_gate_browser_smoke() {
  local browser="$1"
  if [[ "${PYRS_WASM_RUN_VM_PROBE_BROWSER_STATE_GATE_SMOKE:-0}" != "1" ]]; then
    echo "[wasm-contract] vm-probe state-gate browser smoke disabled (set PYRS_WASM_RUN_VM_PROBE_BROWSER_STATE_GATE_SMOKE=1 to enable)"
    return 0
  fi

  echo "[wasm-contract] wasm-pack node: vm-probe state-gate smoke target"
  # vm-probe wasm binaries currently include native-host `env` imports that are
  # shimmed in the website bundle pipeline, but not by wasm-bindgen-test's
  # browser harness. Run vm-probe state-gate smoke under the node runner to
  # keep hard signal without browser-loader deadlocks.
  local shim_root
  shim_root="$(pwd)/scripts/wasm_node_shims"
  if [[ -n "${NODE_PATH:-}" ]]; then
    export NODE_PATH="${shim_root}:${NODE_PATH}"
  else
    export NODE_PATH="${shim_root}"
  fi
  wasm-pack test --node --no-default-features --features wasm-vm-probe --test wasm_vm_probe_browser_smoke
  vm_probe_browser_smoke_enabled=1
  return 0
}

emit_browser_smoke_baseline() {
  local browser="$1"
  local fallback_from="${2:-}"
  local args=(
    --browser "${browser}"
  )
  if [[ -n "${fallback_from}" ]]; then
    args+=(--fallback-from "${fallback_from}")
  fi
  if [[ "${vm_probe_browser_smoke_enabled}" == "1" ]]; then
    args+=(--vm-probe-state-gate)
  fi
  python3 scripts/generate_wasm_browser_smoke_baseline.py "${args[@]}"
}

configure_browser_test_timeout

echo "[wasm-contract] wasm-pack detected; running optional browser smoke tests"
if run_browser_smoke chrome; then
  run_vm_probe_state_gate_browser_smoke chrome
  emit_browser_smoke_baseline chrome
  echo "[wasm-contract] browser smoke tests passed"
  exit 0
fi

echo "[wasm-contract] chrome smoke failed; trying firefox"
run_browser_smoke firefox
run_vm_probe_state_gate_browser_smoke firefox
emit_browser_smoke_baseline firefox chrome
echo "[wasm-contract] browser smoke tests passed (firefox)"
