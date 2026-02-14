#!/usr/bin/env bash
set -euo pipefail

ITERATIONS="${1:-7}"
WARMUP="${WARMUP:-1}"

if ! [[ "$ITERATIONS" =~ ^[0-9]+$ ]] || [ "$ITERATIONS" -le 0 ]; then
  echo "usage: $0 [iterations>0]" >&2
  exit 2
fi

PYRS_BIN="${PYRS_BIN:-target/release/pyrs}"
PYTHON_BIN="${PYTHON_BIN:-python3.10}"

PASS_CMD='pass'
IMPORT_CMD='import math, json, re, pathlib'

measure_wall() {
  local bin="$1"
  local mode="$2"
  local cmd="$3"
  python3 - "$bin" "$mode" "$cmd" <<'PY'
import subprocess
import sys
import time
bin_path, mode, command = sys.argv[1], sys.argv[2], sys.argv[3]
argv = [bin_path]
if mode == "nosite":
    argv.extend(["-S", "-c", command])
else:
    argv.extend(["-c", command])
start = time.perf_counter()
subprocess.run(
    argv,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
    check=True,
)
elapsed = time.perf_counter() - start
print(f"{elapsed:.6f}")
PY
}

avg_wall() {
  local bin="$1"
  local mode="$2"
  local cmd="$3"
  local i
  local total="0"
  local elapsed

  for ((i = 0; i < WARMUP; i++)); do
    measure_wall "$bin" "$mode" "$cmd" >/dev/null
  done

  for ((i = 0; i < ITERATIONS; i++)); do
    elapsed="$(measure_wall "$bin" "$mode" "$cmd")"
    if [ -z "$elapsed" ]; then
      echo "failed to measure wall time for $bin ($mode)" >&2
      exit 1
    fi
    total="$(awk -v a="$total" -v b="$elapsed" 'BEGIN {printf "%.6f", a + b}')"
  done
  awk -v total="$total" -v n="$ITERATIONS" 'BEGIN {printf "%.4f", total / n}'
}

if [ ! -x "$PYRS_BIN" ]; then
  echo "missing executable: $PYRS_BIN" >&2
  echo "build first: cargo build --release" >&2
  exit 1
fi

pyrs_pass="$(avg_wall "$PYRS_BIN" "site" "$PASS_CMD")"
pyrs_pass_nosite="$(avg_wall "$PYRS_BIN" "nosite" "$PASS_CMD")"
pyrs_import="$(avg_wall "$PYRS_BIN" "site" "$IMPORT_CMD")"

printf "pyrs startup pass (site)    avg wall: %ss (%s runs, warmup %s)\n" "$pyrs_pass" "$ITERATIONS" "$WARMUP"
printf "pyrs startup pass (-S)      avg wall: %ss (%s runs, warmup %s)\n" "$pyrs_pass_nosite" "$ITERATIONS" "$WARMUP"
printf "pyrs startup import-bundle  avg wall: %ss (%s runs, warmup %s)\n" "$pyrs_import" "$ITERATIONS" "$WARMUP"

if command -v "$PYTHON_BIN" >/dev/null 2>&1; then
  py_pass="$(avg_wall "$PYTHON_BIN" "site" "$PASS_CMD")"
  py_pass_nosite="$(avg_wall "$PYTHON_BIN" "nosite" "$PASS_CMD")"
  py_import="$(avg_wall "$PYTHON_BIN" "site" "$IMPORT_CMD")"
  printf "%s startup pass (site)    avg wall: %ss\n" "$PYTHON_BIN" "$py_pass"
  printf "%s startup pass (-S)      avg wall: %ss\n" "$PYTHON_BIN" "$py_pass_nosite"
  printf "%s startup import-bundle  avg wall: %ss\n" "$PYTHON_BIN" "$py_import"
  awk -v a="$pyrs_pass" -v b="$py_pass" -v ref="$PYTHON_BIN" 'BEGIN {printf "ratio pass-site (pyrs/%s): %.3fx\n", ref, a / b}'
  awk -v a="$pyrs_import" -v b="$py_import" -v ref="$PYTHON_BIN" 'BEGIN {printf "ratio import-bundle (pyrs/%s): %.3fx\n", ref, a / b}'
fi
