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

policy_file="${PYRS_COVERAGE_POLICY_FILE:-docs/COVERAGE_GATE_POLICY.json}"
if [[ ! -f "$policy_file" ]]; then
  echo "error: coverage policy file not found: $policy_file"
  exit 1
fi

policy_values="$(
  python3 - "$policy_file" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    policy = json.load(handle)

ratchet = policy.get("ratchet_floors", {})
targets = policy.get("long_term_targets", {})
ignore = policy.get("ignore_filename_regex", [])
tests = policy.get("targeted_test_bins", [])

if not isinstance(ignore, list):
    ignore = []
if not isinstance(tests, list):
    tests = []

print(f"POLICY_MIN_REGIONS={ratchet.get('regions', 0)}")
print(f"POLICY_MIN_FUNCTIONS={ratchet.get('functions', 0)}")
print(f"POLICY_MIN_LINES={ratchet.get('lines', 0)}")
print(f"POLICY_TARGET_REGIONS={targets.get('regions', 0)}")
print(f"POLICY_TARGET_FUNCTIONS={targets.get('functions', 0)}")
print(f"POLICY_TARGET_LINES={targets.get('lines', 0)}")
print("POLICY_IGNORE_REGEX=" + "|".join(str(value) for value in ignore))
print("POLICY_TARGETED_TEST_BINS=" + ",".join(str(value) for value in tests))
PY
)"

policy_min_regions=""
policy_min_functions=""
policy_min_lines=""
policy_target_regions=""
policy_target_functions=""
policy_target_lines=""
policy_ignore_regex=""
policy_targeted_test_bins=""

while IFS='=' read -r key value; do
  case "$key" in
    POLICY_MIN_REGIONS) policy_min_regions="$value" ;;
    POLICY_MIN_FUNCTIONS) policy_min_functions="$value" ;;
    POLICY_MIN_LINES) policy_min_lines="$value" ;;
    POLICY_TARGET_REGIONS) policy_target_regions="$value" ;;
    POLICY_TARGET_FUNCTIONS) policy_target_functions="$value" ;;
    POLICY_TARGET_LINES) policy_target_lines="$value" ;;
    POLICY_IGNORE_REGEX) policy_ignore_regex="$value" ;;
    POLICY_TARGETED_TEST_BINS) policy_targeted_test_bins="$value" ;;
  esac
done <<<"$policy_values"

min_regions="${PYRS_COVERAGE_MIN_REGIONS:-$policy_min_regions}"
min_functions="${PYRS_COVERAGE_MIN_FUNCTIONS:-$policy_min_functions}"
min_lines="${PYRS_COVERAGE_MIN_LINES:-$policy_min_lines}"
target_regions="${PYRS_COVERAGE_TARGET_REGIONS:-$policy_target_regions}"
target_functions="${PYRS_COVERAGE_TARGET_FUNCTIONS:-$policy_target_functions}"
target_lines="${PYRS_COVERAGE_TARGET_LINES:-$policy_target_lines}"
ignore_regex="${PYRS_COVERAGE_IGNORE_REGEX:-$policy_ignore_regex}"
targeted_test_bins_csv="${PYRS_COVERAGE_TEST_BINS:-$policy_targeted_test_bins}"

if [[ -z "$targeted_test_bins_csv" ]]; then
  echo "error: no coverage test bins configured (policy file: $policy_file)"
  exit 1
fi

IFS=',' read -r -a targeted_test_bins <<<"$targeted_test_bins_csv"

echo "[coverage] policy file: $policy_file"
echo "[coverage] ratchet floors: regions=${min_regions}% functions=${min_functions}% lines=${min_lines}%"
echo "[coverage] long-term targets: regions=${target_regions}% functions=${target_functions}% lines=${target_lines}%"
if [[ -n "$ignore_regex" ]]; then
  echo "[coverage] ignoring files matching regex: $ignore_regex"
fi

echo "[coverage] cleaning previous llvm-cov artifacts"
cargo llvm-cov clean --workspace

coverage_run_cmd=(cargo llvm-cov --no-report --bin pyrs -q)
echo "[coverage] running targeted coverage bins"
for test_bin in "${targeted_test_bins[@]}"; do
  if [[ -z "$test_bin" ]]; then
    continue
  fi
  echo "[coverage] include test target: $test_bin"
  coverage_run_cmd+=(--test "$test_bin")
done

"${coverage_run_cmd[@]}"

summary_cmd=(cargo llvm-cov report --summary-only)
if [[ -n "$ignore_regex" ]]; then
  summary_cmd+=(--ignore-filename-regex "$ignore_regex")
fi

echo "[coverage] generating summary report"
output="$("${summary_cmd[@]}")"
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
