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

## Local Test Runner Policy

Use `cargo nextest run` as the default local Rust test runner.

Examples:

```bash
# full suite
cargo nextest run

# targeted integration test
cargo nextest run --test vm

# single test by name
cargo nextest run --test differential_cpython differential_traceback_identifier_caret_span_matches_cpython

# quieter summary output (optional)
cargo nextest run --status-level fail --final-status-level fail
```

Use `cargo test` only when you specifically need `cargo test` semantics.

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
export RUSTFLAGS="-Zsanitizer=address"
cargo +nightly test -Zbuild-std --target aarch64-apple-darwin --test extension_smoke
```

ASan NumPy import probe:

```bash
export RUSTFLAGS="-Zsanitizer=address"
cargo +nightly run -Zbuild-std --target aarch64-apple-darwin --bin pyrs -- -S -c \
  "import sys; sys.path.insert(0, './.venv-ext314/lib/python3.14/site-packages'); import numpy as np"
```

Notes:

- Sanitizers require nightly (`-Zsanitizer` + `-Zbuild-std`).
- Start with targeted tests (`--test extension_smoke`) before full-suite runs.
- On this target/toolchain (`aarch64-apple-darwin`), rustc does not accept `-Zsanitizer=undefined`; UBSan is currently unavailable through Rust sanitizer flags here.
- Unset `RUSTFLAGS` after sanitizer runs to avoid contaminating normal builds (`unset RUSTFLAGS`).

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

## Extension Smoke Runtime Controls

`tests/extension_smoke.rs` compiles native probes and runs many subprocess imports, so it has explicit controls for reliability/perf triage:

- `PYRS_EXTENSION_SMOKE_TIMEOUT_SECS`:
  - per-subprocess import timeout (default: `120`).
- `PYRS_EXTENSION_SMOKE_COMPILE_TIMEOUT_SECS`:
  - per-compile timeout for native probe builds (default: `120`).
- `PYRS_EXTENSION_SMOKE_TIMING`:
  - if set to `1`/`true`, emits per-stage timing lines (`compile`, `compile_cache_hit`, `subprocess`).
- `PYRS_EXTENSION_SMOKE_CACHE`:
  - enabled by default; set to `0` to disable extension build cache.
- `PYRS_EXTENSION_SMOKE_CACHE_DIR`:
  - override cache directory (default: `target/extension_smoke_cache`).

Notes:

- Cache is content-addressed by probe source + compile mode/flags.
- Cache is automatically bypassed for path-sensitive probes that validate `__FILE__` / `PyErr_ProgramText(...)` semantics.
