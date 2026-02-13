#!/usr/bin/env bash
set -euo pipefail

# Milestone 11 parity profile.
# This is the first-class gate for CPython harness, differential tests,
# fuzz/property checks, and curated real-world smoke suites.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export PYRS_PARITY_STRICT=1

echo "[parity] running builtin parity gate"
./scripts/run_builtin_parity_gate.sh

echo "[parity] running focused parity gate suites"
cargo test -q \
  --test cpython_harness \
  --test differential_cpython \
  --test fuzz_expr \
  --test fuzz_parser_vm \
  --test realworld_smoke

echo "[parity] all parity gate suites passed"
