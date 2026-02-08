#!/usr/bin/env bash
set -euo pipefail

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found"
  exit 1
fi

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "error: cargo-llvm-cov is required (install with: cargo install cargo-llvm-cov)"
  exit 1
fi

echo "[coverage] running cargo llvm-cov summary"
output="$(cargo llvm-cov --workspace --summary-only)"
echo "$output"

total_line="$(printf '%s\n' "$output" | awk '$1 == "TOTAL" { line = $0 } END { print line }')"
if [[ -z "$total_line" ]]; then
  echo "error: failed to find TOTAL row in llvm-cov output"
  exit 1
fi

regions_cov="$(printf '%s\n' "$total_line" | awk '{gsub(/%/, "", $4); print $4}')"
functions_cov="$(printf '%s\n' "$total_line" | awk '{gsub(/%/, "", $7); print $7}')"
lines_cov="$(printf '%s\n' "$total_line" | awk '{gsub(/%/, "", $10); print $10}')"

echo "[coverage] totals: regions=${regions_cov}% functions=${functions_cov}% lines=${lines_cov}%"

enforce="${PYRS_COVERAGE_ENFORCE:-0}"
if [[ "$enforce" != "1" ]]; then
  echo "[coverage] report-only mode (set PYRS_COVERAGE_ENFORCE=1 to enforce floors)"
  exit 0
fi

min_regions="${PYRS_COVERAGE_MIN_REGIONS:-0}"
min_functions="${PYRS_COVERAGE_MIN_FUNCTIONS:-0}"
min_lines="${PYRS_COVERAGE_MIN_LINES:-0}"

check_floor() {
  local label="$1"
  local value="$2"
  local min_value="$3"
  awk -v value="$value" -v min="$min_value" 'BEGIN { exit !(value + 0 >= min + 0) }'
  if [[ $? -ne 0 ]]; then
    echo "error: ${label} coverage ${value}% is below floor ${min_value}%"
    exit 1
  fi
}

check_floor "regions" "$regions_cov" "$min_regions"
check_floor "functions" "$functions_cov" "$min_functions"
check_floor "lines" "$lines_cov" "$min_lines"

echo "[coverage] floor checks passed"
