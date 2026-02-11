#!/usr/bin/env bash
set -euo pipefail

ITERATIONS="${1:-5}"
WARMUP="${WARMUP:-1}"

if ! [[ "$ITERATIONS" =~ ^[0-9]+$ ]] || [ "$ITERATIONS" -le 0 ]; then
  echo "usage: $0 [iterations>0]" >&2
  exit 2
fi

PYRS_BIN="${PYRS_BIN:-target/release/pyrs}"
PYRS_CMD='fib = lambda n: n if n < 2 else fib(n-1) + fib(n-2); [fib(29) for _ in range(5)]'
PYRS_SINGLE='fib = lambda n: n if n < 2 else fib(n-1) + fib(n-2); print(fib(29))'
PYTHON_BIN="${PYTHON_BIN:-python3.10}"

measure_user() {
  local bin="$1"
  local cmd="$2"
  local out
  out=$({ /usr/bin/time -p "$bin" -c "$cmd" >/dev/null; } 2>&1)
  printf '%s\n' "$out" | awk '/^user / {print $2; exit}'
}

avg_user() {
  local bin="$1"
  local cmd="$2"
  local i
  local total="0"
  local user

  for ((i = 0; i < WARMUP; i++)); do
    measure_user "$bin" "$cmd" >/dev/null
  done

  for ((i = 0; i < ITERATIONS; i++)); do
    user="$(measure_user "$bin" "$cmd")"
    if [ -z "$user" ]; then
      echo "failed to measure user time for $bin" >&2
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

pyrs_gate="$(avg_user "$PYRS_BIN" "$PYRS_CMD")"
pyrs_single="$(avg_user "$PYRS_BIN" "$PYRS_SINGLE")"

printf "pyrs fib(29)x5 avg user: %ss (%s runs, warmup %s)\n" "$pyrs_gate" "$ITERATIONS" "$WARMUP"
printf "pyrs fib(29)   avg user: %ss (%s runs, warmup %s)\n" "$pyrs_single" "$ITERATIONS" "$WARMUP"

if command -v "$PYTHON_BIN" >/dev/null 2>&1; then
  py_gate="$(avg_user "$PYTHON_BIN" "$PYRS_CMD")"
  py_single="$(avg_user "$PYTHON_BIN" "$PYRS_SINGLE")"
  printf "%s fib(29)x5 avg user: %ss\n" "$PYTHON_BIN" "$py_gate"
  printf "%s fib(29)   avg user: %ss\n" "$PYTHON_BIN" "$py_single"
  awk -v a="$pyrs_gate" -v b="$py_gate" -v ref="$PYTHON_BIN" 'BEGIN {printf "ratio gate (pyrs/%s): %.3fx\n", ref, a / b}'
fi
