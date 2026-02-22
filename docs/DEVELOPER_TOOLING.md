# Developer Tooling

This project uses a small set of optional developer tools for profiling, coverage, and ABI debugging.

## Install Optional Cargo Tools

```bash
./scripts/bootstrap_dev_tools.sh
```

This installs:

- `cargo-nextest` (`cargo nextest`)
- `cargo-bloat` (`cargo bloat`)
- `cargo-llvm-cov` (`cargo llvm-cov`)
- `cargo-flamegraph` (`cargo flamegraph`)

## AddressSanitizer

Status on this machine:

- `nightly` toolchain is installed.
- `rust-src` is installed.
- `clang` is installed (Apple clang 17).

Recommended setup:

```bash
rustup component add rust-src --toolchain nightly
```

ASan command pattern:

```bash
RUSTFLAGS="-Zsanitizer=address" \
cargo +nightly test -Zbuild-std --target aarch64-apple-darwin --test extension_smoke
```

ASan NumPy import probe:

```bash
RUSTFLAGS="-Zsanitizer=address" \
cargo +nightly run -Zbuild-std --target aarch64-apple-darwin --bin pyrs -- -S -c \
  "import sys; sys.path.insert(0, './.venv-ext314/lib/python3.14/site-packages'); import numpy as np"
```

Notes:

- Sanitizers require nightly (`-Zsanitizer` + `-Zbuild-std`).
- Start with targeted tests (`--test extension_smoke`) before full-suite runs.
- On this target/toolchain (`aarch64-apple-darwin`), rustc does not accept `-Zsanitizer=undefined`; UBSan is currently unavailable through Rust sanitizer flags here.

## C-API No-Op Drift Gate

Run locally:

```bash
python3 scripts/check_capi_noop_inventory.py --manifest perf/capi_noop_inventory.json
```

This verifies that C-API no-op/placeholder exports are tracked in `docs/CAPI_NOOP_INVENTORY.md` and emits a machine-readable manifest.

## Scaffolding Drift Audit

Run locally:

```bash
python3 scripts/audit_scaffolding.py
```

This enforces anti-scaffolding invariants:

- `shims/` is locked to `_ctypes.py` only.
- no retired shim-path references remain in runtime/test/CI code.
- `LOCAL_SHIM_MODULES` remains `_ctypes`-only.
- obsolete local-shim toggle API is absent.
- `docs/NOOP_BUILTIN_INVENTORY.txt` stays in sync with `print_noop_inventory`.
- C-API no-op inventory drift check is green.
