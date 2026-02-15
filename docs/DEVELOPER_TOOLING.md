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
