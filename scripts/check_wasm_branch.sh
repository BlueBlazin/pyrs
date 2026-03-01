#!/usr/bin/env bash
set -euo pipefail

echo "[wasm-branch] cargo check (native)"
cargo check

echo "[wasm-branch] cargo check (wasm32-unknown-unknown)"
cargo check --target wasm32-unknown-unknown

echo "[wasm-branch] nextest host capability regression"
cargo nextest run --lib wasm_host_capability_matrix_is_explicit --status-level fail --final-status-level fail

echo "[wasm-branch] nextest vm smoke regression"
cargo nextest run --test vm callable_instance_dispatch_matches_explicit_dunder_call_path --status-level fail --final-status-level fail

echo "[wasm-branch] all checks passed"
