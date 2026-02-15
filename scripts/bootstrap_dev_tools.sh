#!/usr/bin/env bash
set -euo pipefail

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found"
  exit 1
fi

install_cargo_tool() {
  local crate_name="$1"
  local subcommand="$2"

  if cargo "$subcommand" --version >/dev/null 2>&1; then
    echo "[dev-tools] cargo-$subcommand already installed"
    return 0
  fi

  echo "[dev-tools] installing ${crate_name}"
  cargo install "${crate_name}"
}

install_cargo_tool cargo-nextest nextest
install_cargo_tool cargo-bloat bloat
install_cargo_tool cargo-llvm-cov llvm-cov
install_cargo_tool cargo-flamegraph flamegraph

echo "[dev-tools] done"
