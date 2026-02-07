# pyrs

`pyrs` is a Python interpreter in Rust, targeting CPython 3.14 source and bytecode compatibility.

## Status

`pyrs` is in active alpha development.

- Milestones `0-12` are complete.
- Milestones `13-16` remain (long-tail parity, stdlib/packaging closure, performance/observability, extension ecosystem, and release hardening).

## What Works Today

- Substantial pure-Python execution: modules/packages, classes, closures, generators, comprehensions, core async flows.
- CPython `.pyc` execution for a supported subset, including sourceless import fallback paths.
- Broad stdlib foundation (`sys`, import foundations, `os`/`pathlib`, `json`, `re`, `math`, `datetime`, `random`, core `asyncio`/`threading`/`signal`).
- Curated CPython harness suites and project test suite are green.

## Current Limits

- Not full CPython 3.14 parity yet (remaining long-tail language/runtime and stdlib edge semantics).
- C-extension compatibility is not implemented yet (e.g. NumPy remains out of scope until extension milestones).
- Performance/hardening milestones are still ahead.

## Quick Start

Requirements:
- Rust toolchain (`cargo`)

Build:

```bash
cargo build
```

Run source:

```bash
cargo run -- path/to/script.py
```

Run bytecode:

```bash
cargo run -- path/to/module.pyc
```

Disable startup `site` import:

```bash
cargo run -- -S path/to/script.py
```

Inspect AST:

```bash
cargo run -- --ast path/to/script.py
```

Inspect bytecode:

```bash
cargo run -- --bytecode path/to/script.py
```

Run tests:

```bash
cargo test
```

Run parity profile:

```bash
./scripts/run_parity_gate.sh
```

## Using Real CPython Stdlib (Pure-Python Parts)

You can run against a real CPython 3.14 `Lib/` directory now.

Recommended (pinned local CPython tree):

```bash
PYRS_CPYTHON_LIB=/path/to/Python-3.14.3/Lib cargo run -- path/to/script.py
```

How stdlib loading works in `pyrs`:

- `pyrs` resolves stdlib roots from `PYRS_CPYTHON_LIB`, `PYTHONPATH`, `PYTHONHOME`, and known default paths.
- Those directories are added to VM module search paths (`sys.path`-backed resolution).
- Imports are resolved with `PathFinder` + loader contracts:
  - `pyrs.SourceFileLoader` for `.py`
  - `pyrs.SourcelessFileLoader` for supported `.pyc`
  - `pyrs.NamespaceLoader` for namespace packages
- Pure-Python stdlib modules are interpreted by `pyrs` (not linked to CPython runtime internals).

Do we keep a local copy in this repo?

- Not required.
- You point `pyrs` at a filesystem `Lib/` tree.
- If you want reproducibility, keep a local pinned CPython checkout (e.g. `Python-3.14.3/Lib`) and set `PYRS_CPYTHON_LIB`.

## Safety Note for Parity Runs

Curated smoke/parity tests run subprocesses in a constrained mode (`env_clear`, isolated temp cwd/home, timeouts). For stronger isolation, run parity in an OS/container sandbox.

## Key Docs

- Roadmap: `docs/ROADMAP.md`
- Compatibility tracker: `docs/COMPATIBILITY.md`
- Production readiness accounting: `docs/PRODUCTION_READINESS.md`
- Stub/partial ledger: `docs/STUB_ACCOUNTING.md`
- Project context for agent workflows: `AGENTS.md`
