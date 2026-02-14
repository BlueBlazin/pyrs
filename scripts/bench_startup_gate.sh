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

measure_user() {
  local bin="$1"
  local mode="$2"
  local cmd="$3"
  local out
  if [ "$mode" = "nosite" ]; then
    out=$({ /usr/bin/time -p "$bin" -S -c "$cmd" >/dev/null; } 2>&1)
  else
    out=$({ /usr/bin/time -p "$bin" -c "$cmd" >/dev/null; } 2>&1)
  fi
  printf '%s\n' "$out" | awk '/^user / {print $2; exit}'
}

avg_user() {
  local bin="$1"
  local mode="$2"
  local cmd="$3"
  local i
  local total="0"
  local user

  for ((i = 0; i < WARMUP; i++)); do
    measure_user "$bin" "$mode" "$cmd" >/dev/null
  done

  for ((i = 0; i < ITERATIONS; i++)); do
    user="$(measure_user "$bin" "$mode" "$cmd")"
    if [ -z "$user" ]; then
      echo "failed to measure user time for $bin ($mode)" >&2
      exit 1
    fi
    total="$(awk -v a="$total" -v b="$user" 'BEGIN {printf "%.6f", a + b}')"
  done
  awk -v total="$total" -v n="$ITERATIONS" 'BEGIN {printf "%.4f", total / n}'
}

if [ ! -x "$PYRS_BIN" ]; then
  echo "missing executable: $PYRS_BIN" >&2
  echo "build first: cargo build --release" >&2
  exit 1
fi

pyrs_pass="$(avg_user "$PYRS_BIN" "site" "$PASS_CMD")"
pyrs_pass_nosite="$(avg_user "$PYRS_BIN" "nosite" "$PASS_CMD")"
pyrs_import="$(avg_user "$PYRS_BIN" "site" "$IMPORT_CMD")"

printf "pyrs startup pass (site)    avg user: %ss (%s runs, warmup %s)\n" "$pyrs_pass" "$ITERATIONS" "$WARMUP"
printf "pyrs startup pass (-S)      avg user: %ss (%s runs, warmup %s)\n" "$pyrs_pass_nosite" "$ITERATIONS" "$WARMUP"
printf "pyrs startup import-bundle  avg user: %ss (%s runs, warmup %s)\n" "$pyrs_import" "$ITERATIONS" "$WARMUP"

if command -v "$PYTHON_BIN" >/dev/null 2>&1; then
  py_pass="$(avg_user "$PYTHON_BIN" "site" "$PASS_CMD")"
  py_pass_nosite="$(avg_user "$PYTHON_BIN" "nosite" "$PASS_CMD")"
  py_import="$(avg_user "$PYTHON_BIN" "site" "$IMPORT_CMD")"
  printf "%s startup pass (site)    avg user: %ss\n" "$PYTHON_BIN" "$py_pass"
  printf "%s startup pass (-S)      avg user: %ss\n" "$PYTHON_BIN" "$py_pass_nosite"
  printf "%s startup import-bundle  avg user: %ss\n" "$PYTHON_BIN" "$py_import"
  awk -v a="$pyrs_pass" -v b="$py_pass" -v ref="$PYTHON_BIN" 'BEGIN {printf "ratio pass-site (pyrs/%s): %.3fx\n", ref, a / b}'
  awk -v a="$pyrs_import" -v b="$py_import" -v ref="$PYTHON_BIN" 'BEGIN {printf "ratio import-bundle (pyrs/%s): %.3fx\n", ref, a / b}'
fi
