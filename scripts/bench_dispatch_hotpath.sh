#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

mkdir -p perf

echo "[dispatch-bench] building release binary"
cargo build --release >/dev/null

PYTHON_BIN="${PYRS_CP_PYTHON_BIN:-python3}"
if ! command -v "$PYTHON_BIN" >/dev/null 2>&1; then
  echo "[dispatch-bench] missing python interpreter: $PYTHON_BIN" >&2
  exit 1
fi

BENCH_SOURCE_FILE="perf/dispatch_hotpath_bench.py"
cat >"$BENCH_SOURCE_FILE" <<'PY'
class Acc:
    def add(self, value):
        return value + 1

acc = Acc()
value = 0
for _ in range(250_000):
    value = acc.add(value)

payload = [1, 2, 3, 4, 5, 6, 7, 8]
total = 0
for _ in range(300_000):
    total += len(payload)

if value != 250_000 or total != 2_400_000:
    raise SystemExit(1)
PY

measure_seconds() {
  "$PYTHON_BIN" - "$@" <<'PY'
import subprocess
import sys
import time

command = sys.argv[1:]
start = time.perf_counter()
subprocess.run(command, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
elapsed = time.perf_counter() - start
print(f"{elapsed:.4f}")
PY
}

echo "[dispatch-bench] measuring pyrs"
PYRS_SEC="$(measure_seconds target/release/pyrs "$BENCH_SOURCE_FILE")"

echo "[dispatch-bench] measuring cpython"
CPYTHON_SEC="$(measure_seconds "$PYTHON_BIN" "$BENCH_SOURCE_FILE")"

RATIO="$("$PYTHON_BIN" - "$PYRS_SEC" "$CPYTHON_SEC" <<'PY'
import sys
pyrs = float(sys.argv[1])
cpy = float(sys.argv[2])
if cpy <= 0.0:
    print("inf")
else:
    print(f"{pyrs / cpy:.4f}")
PY
)"

OUT_FILE="perf/dispatch_hotpath_bench.txt"
{
  echo "# Dispatch hotpath benchmark report"
  echo "pyrs_dispatch_hotpath_sec=$PYRS_SEC"
  echo "cpython_dispatch_hotpath_sec=$CPYTHON_SEC"
  echo "pyrs_vs_cpython_dispatch_ratio=$RATIO"
} >"$OUT_FILE"

echo "[dispatch-bench] wrote $OUT_FILE"
cat "$OUT_FILE"
