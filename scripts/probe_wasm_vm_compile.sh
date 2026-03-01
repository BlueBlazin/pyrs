#!/usr/bin/env bash
set -euo pipefail

echo "[wasm-vm-probe] cargo check wasm target with vm probe feature"
cargo check --target wasm32-unknown-unknown --features wasm-vm-probe
