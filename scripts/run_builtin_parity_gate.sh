#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if ! command -v python3 >/dev/null 2>&1; then
  echo "error: python3 not found"
  exit 1
fi

PYRS_BIN="${PYRS_BIN:-target/debug/pyrs}"
if [[ ! -x "$PYRS_BIN" ]]; then
  echo "[builtin-parity] building pyrs binary"
  cargo build -q --bin pyrs
fi

default_cpython_bin="python3"
if [[ -x "/Library/Frameworks/Python.framework/Versions/3.14/bin/python3" ]]; then
  default_cpython_bin="/Library/Frameworks/Python.framework/Versions/3.14/bin/python3"
fi
CPYTHON_BIN="${PYRS_CPYTHON_BIN:-$default_cpython_bin}"
OUTPUT_JSON="${PYRS_BUILTIN_PARITY_REPORT:-perf/builtin_parity_report.json}"

python3 scripts/check_builtin_parity.py \
  --check \
  --cpython-bin "$CPYTHON_BIN" \
  --pyrs-bin "$PYRS_BIN" \
  --missing-allowlist tests/builtin_missing_allowlist.txt \
  --probe-allowlist tests/builtin_probe_allowlist.txt \
  --output-json "$OUTPUT_JSON"
