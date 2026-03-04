#!/usr/bin/env bash
set -euo pipefail

build_wasm_stdlib_subset_pack() {
  echo "[wasm-contract] build curated wasm stdlib source pack"
  python3 scripts/build_wasm_stdlib_subset.py \
    --out-zip website/public/wasm/stdlib_subset_v1.zip \
    --out-pack website/public/wasm/stdlib_subset_v1.json \
    --out-manifest website/public/wasm/stdlib_subset_manifest_v1.json
}

build_wasm_stdlib_subset_pack

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

  echo "[wasm-contract] playground worker contract check"
  node scripts/check_playground_worker_contract.mjs \
    --out perf/wasm_playground_worker_contract_latest.json

  echo "[wasm-contract] wasm host seam audit snapshot"
  python3 scripts/audit_wasm_host_seam.py \
    --out perf/wasm_host_seam_audit_latest.json

  echo "[wasm-contract] wasm stdlib subset summary snapshot"
  python3 scripts/generate_wasm_stdlib_subset_summary.py \
    --manifest website/public/wasm/stdlib_subset_manifest_v1.json \
    --out perf/wasm_stdlib_subset_summary_latest.json

  echo "[wasm-contract] wasm artifact input hash summary snapshot"
  python3 scripts/generate_wasm_artifact_input_hashes.py \
    --out perf/wasm_artifact_input_hashes_latest.json

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
vm_probe_browser_smoke_runner=""

configure_browser_test_timeout() {
  local timeout_seconds="${PYRS_WASM_BROWSER_TEST_TIMEOUT_SECONDS:-180}"
  export WASM_BINDGEN_TEST_TIMEOUT="${timeout_seconds}"
  echo "[wasm-contract] wasm browser test timeout: ${WASM_BINDGEN_TEST_TIMEOUT}s"
}

run_wasm_pack_checked() {
  local label="$1"
  shift
  local log_file
  log_file="$(mktemp)"
  local command_status=0
  if ! "$@" 2>&1 | tee "${log_file}"; then
    command_status=1
  fi
  if command -v rg >/dev/null 2>&1; then
    if rg -q "output filename collision" "${log_file}"; then
      echo "[wasm-contract] ${label}: cargo output filename collision detected; wasm lane must avoid bin/lib wasm artifact name conflicts"
      command_status=1
    fi
  elif grep -q "output filename collision" "${log_file}"; then
    echo "[wasm-contract] ${label}: cargo output filename collision detected; wasm lane must avoid bin/lib wasm artifact name conflicts"
    command_status=1
  fi
  rm -f "${log_file}"
  return "${command_status}"
}

run_browser_smoke() {
  local browser="$1"
  local smoke_status=0
  echo "[wasm-contract] wasm-pack ${browser}: integration contract tests"
  if ! run_wasm_pack_checked \
    "wasm-pack ${browser} integration contract tests" \
    wasm-pack test --headless --"${browser}" --test wasm_contract --no-default-features; then
    smoke_status=1
  fi
  echo "[wasm-contract] wasm-pack ${browser}: lib unit tests"
  if ! run_wasm_pack_checked \
    "wasm-pack ${browser} lib unit tests" \
    wasm-pack test --headless --"${browser}" --lib --no-default-features; then
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

  echo "[wasm-contract] wasm-pack ${browser}: vm-probe state-gate smoke target"
  if run_wasm_pack_checked \
    "wasm-pack ${browser} vm-probe state-gate smoke target" \
    wasm-pack test --headless --"${browser}" --no-default-features --features wasm-vm-probe --test wasm_vm_probe_browser_smoke; then
    vm_probe_browser_smoke_enabled=1
    vm_probe_browser_smoke_runner="browser"
    return 0
  fi

  echo "[wasm-contract] ${browser} vm-probe state-gate smoke failed; trying node fallback"
  local shim_root
  shim_root="$(pwd)/scripts/wasm_node_shims"
  local env_shim_file
  env_shim_file="${shim_root}/env/index.js"
  if [[ ! -f "${env_shim_file}" ]]; then
    echo "[wasm-contract] missing required node env shim: ${env_shim_file}"
    return 1
  fi
  if [[ -n "${NODE_PATH:-}" ]]; then
    export NODE_PATH="${shim_root}:${NODE_PATH}"
  else
    export NODE_PATH="${shim_root}"
  fi
  run_wasm_pack_checked \
    "wasm-pack node vm-probe state-gate smoke target" \
    wasm-pack test --node --no-default-features --features wasm-vm-probe --test wasm_vm_probe_browser_smoke
  vm_probe_browser_smoke_enabled=1
  vm_probe_browser_smoke_runner="node"
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
    args+=(--vm-probe-runner "${vm_probe_browser_smoke_runner}")
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
