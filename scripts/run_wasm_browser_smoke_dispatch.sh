#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/run_wasm_browser_smoke_dispatch.sh [options]

Dispatches the wasm-track workflow, waits for completion, downloads artifacts,
and validates the browser-smoke baseline summary when present.

Options:
  --ref <branch>        Git ref/branch for workflow dispatch (default: codex/wasm)
  --workflow <name>     Workflow file or name (default: wasm-track.yml)
  --run-id <id>         Existing run id to watch/download (skip dispatch)
  --out-dir <dir>       Download root directory
                        (default: perf/wasm-browser-smoke-run)
  --skip-download       Skip artifact download + baseline validation
  -h, --help            Show this help
EOF
}

require_cmd() {
  local cmd="$1"
  if ! command -v "${cmd}" >/dev/null 2>&1; then
    echo "[wasm-browser-dispatch] missing required command: ${cmd}" >&2
    exit 1
  fi
}

branch_ref="codex/wasm"
workflow_name="wasm-track.yml"
run_id=""
download_root="perf/wasm-browser-smoke-run"
skip_download=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ref)
      branch_ref="$2"
      shift 2
      ;;
    --workflow)
      workflow_name="$2"
      shift 2
      ;;
    --run-id)
      run_id="$2"
      shift 2
      ;;
    --out-dir)
      download_root="$2"
      shift 2
      ;;
    --skip-download)
      skip_download=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "[wasm-browser-dispatch] unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

require_cmd gh
require_cmd python3

if [[ -z "${run_id}" ]]; then
  previous_run_id="$(
    gh run list \
      --workflow "${workflow_name}" \
      --branch "${branch_ref}" \
      --event workflow_dispatch \
      --limit 1 \
      --json databaseId \
      --jq '.[0].databaseId // empty'
  )"

  echo "[wasm-browser-dispatch] dispatching workflow '${workflow_name}' on ref '${branch_ref}'"
  gh workflow run "${workflow_name}" --ref "${branch_ref}"

  echo "[wasm-browser-dispatch] waiting for workflow_dispatch run id"
  for _ in {1..30}; do
    candidate_run_id="$(
      gh run list \
        --workflow "${workflow_name}" \
        --branch "${branch_ref}" \
        --event workflow_dispatch \
        --limit 1 \
        --json databaseId \
        --jq '.[0].databaseId // empty'
    )"
    if [[ -n "${candidate_run_id}" ]] && [[ "${candidate_run_id}" != "${previous_run_id}" ]]; then
      run_id="${candidate_run_id}"
      break
    fi
    sleep 2
  done

  if [[ -z "${run_id}" ]]; then
    echo "[wasm-browser-dispatch] failed to discover dispatched run id" >&2
    exit 1
  fi
fi

echo "[wasm-browser-dispatch] watching run ${run_id}"
gh run watch "${run_id}" --exit-status

run_url="$(
  gh run view "${run_id}" --json url --jq '.url'
)"
run_head_sha="$(
  gh run view "${run_id}" --json headSha --jq '.headSha'
)"
echo "[wasm-browser-dispatch] run complete: ${run_url}"

if [[ "${skip_download}" == "1" ]]; then
  echo "[wasm-browser-dispatch] skipping artifact download (--skip-download)"
  echo "[wasm-browser-dispatch] run-url: ${run_url}"
  exit 0
fi

download_dir="${download_root}/run-${run_id}"
mkdir -p "${download_dir}"

echo "[wasm-browser-dispatch] downloading artifacts into ${download_dir}"
gh run download "${run_id}" --dir "${download_dir}"

baseline_path="$(
  find "${download_dir}" -type f -name wasm_browser_smoke_baseline_latest.json | head -n 1
)"
if [[ -z "${baseline_path}" ]]; then
  echo "[wasm-browser-dispatch] browser baseline summary not found in downloaded artifacts" >&2
  exit 1
fi

echo "[wasm-browser-dispatch] validating baseline summary: ${baseline_path}"
python3 scripts/validate_wasm_browser_smoke_baseline.py --summary "${baseline_path}"

hash_json="${download_dir}/wasm-artifact-hashes.json"
hash_md="${download_dir}/wasm-artifact-hashes.md"
run_log="${download_dir}/workflow-run.log"
echo "[wasm-browser-dispatch] extracting artifact hash summary"
if gh run view "${run_id}" --log > "${run_log}"; then
  if python3 scripts/extract_wasm_ci_artifact_hashes.py \
    --run-id "${run_id}" \
    --run-url "${run_url}" \
    --head-sha "${run_head_sha}" \
    --log-file "${run_log}" \
    --format json \
    --out "${hash_json}"; then
    python3 scripts/extract_wasm_ci_artifact_hashes.py \
      --run-id "${run_id}" \
      --run-url "${run_url}" \
      --head-sha "${run_head_sha}" \
      --log-file "${run_log}" \
      --format markdown \
      --out "${hash_md}" >/dev/null
    echo "[wasm-browser-dispatch] artifact hash files:"
    echo "  - ${hash_json}"
    echo "  - ${hash_md}"
    echo "  - ${run_log}"
  else
    echo "[wasm-browser-dispatch] warning: failed to extract artifact hashes from saved run log"
  fi
else
  echo "[wasm-browser-dispatch] warning: failed to download workflow run log for hash extraction"
fi

echo "[wasm-browser-dispatch] baseline capture complete"
echo "[wasm-browser-dispatch] run-url: ${run_url}"
echo "[wasm-browser-dispatch] artifacts: ${download_dir}"
