#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PYRS_BIN="${PYRS_BIN:-target/release/pyrs}"
CPYTHON_BIN="${CPYTHON_BIN:-/Library/Frameworks/Python.framework/Versions/3.14/bin/python3}"
CPYTHON_LIB="${CPYTHON_LIB:-/Users/$USER/Downloads/Python-3.14.3/Lib}"
OUTPUT_FILE="${OUTPUT_FILE:-perf/dict_backend_bench.txt}"
PREV_PICKLE_SEC="${PREV_PICKLE_SEC:-39.87}"

mkdir -p "$(dirname "$OUTPUT_FILE")"

echo "[bench] building release binary"
cargo build --release >/dev/null

run_timed() {
  local label="$1"
  shift
  local log_file
  log_file="$(mktemp)"
  /usr/bin/time -p "$@" >"$log_file" 2>&1
  local real_sec
  real_sec="$(awk '/^real /{print $2}' "$log_file" | tail -n 1)"
  echo "$label=$real_sec" >>"$OUTPUT_FILE"
  rm -f "$log_file"
}

cat >"$OUTPUT_FILE" <<EOF
# Dict backend benchmark report
# Generated: $(date -u +"%Y-%m-%dT%H:%M:%SZ")
# PREV_PICKLE_SEC=$PREV_PICKLE_SEC
EOF

PYRS_DICT_CODE="$(cat <<'PY'
import time
N=5000
d={}
for i in range(N):
    d[i]=i
s=0
for i in range(N):
    s += d[i]
for i in range(0,N,3):
    d.pop(i,None)
print("sum", s)
PY
)"
CPY_DICT_CODE="$PYRS_DICT_CODE"

PYRS_PICKLE_CODE="$(cat <<PY
import sys,unittest
sys.path=['$CPYTHON_LIB']
from test.test_pickle import CDumpPickle_LoadPickle
suite=unittest.TestSuite([CDumpPickle_LoadPickle("test_framed_write_sizes_with_delayed_writer")])
res=unittest.TextTestRunner(verbosity=0).run(suite)
print("ok",res.wasSuccessful())
PY
)"
CPY_PICKLE_CODE="$(cat <<PY
import sys,unittest
sys.path.insert(0,'$CPYTHON_LIB')
from test.test_pickle import CDumpPickle_LoadPickle
suite=unittest.TestSuite([CDumpPickle_LoadPickle("test_framed_write_sizes_with_delayed_writer")])
res=unittest.TextTestRunner(verbosity=0).run(suite)
print("ok",res.wasSuccessful())
PY
)"

echo "[bench] running pyrs dict microbench"
run_timed "pyrs_dict_microbench_sec" "$PYRS_BIN" -c "$PYRS_DICT_CODE"
echo "[bench] running cpython dict microbench"
run_timed "cpython_dict_microbench_sec" "$CPYTHON_BIN" -c "$CPY_DICT_CODE"

echo "[bench] running pyrs pickle hotspot"
run_timed "pyrs_pickle_hotspot_sec" "$PYRS_BIN" -c "$PYRS_PICKLE_CODE"
echo "[bench] running cpython pickle hotspot"
run_timed "cpython_pickle_hotspot_sec" "$CPYTHON_BIN" -c "$CPY_PICKLE_CODE"

pyrs_pickle="$(awk -F= '/^pyrs_pickle_hotspot_sec=/{print $2}' "$OUTPUT_FILE")"
cpy_pickle="$(awk -F= '/^cpython_pickle_hotspot_sec=/{print $2}' "$OUTPUT_FILE")"

cat >>"$OUTPUT_FILE" <<EOF
pyrs_vs_cpython_pickle_ratio=$(awk "BEGIN{printf \"%.4f\", $pyrs_pickle/$cpy_pickle}")
pickle_delta_vs_prev_sec=$(awk "BEGIN{printf \"%.4f\", $pyrs_pickle-$PREV_PICKLE_SEC}")
EOF

echo "[bench] wrote $OUTPUT_FILE"
cat "$OUTPUT_FILE"
