# pyrs

`pyrs` is a Python interpreter in Rust, targeting CPython 3.14 source and bytecode compatibility.

## Status

`pyrs` is in active alpha development.

- Milestones `0-12` are complete.
- Milestones `13-16` remain (long-tail parity, stdlib/packaging closure, performance/observability, extension ecosystem, and release hardening).
- Optimization phase-1 checkpoint is complete; remaining throughput gaps are tracked in `docs/OPTIMIZATION_BACKLOG.md`.

## What Works Today

- Substantial pure-Python execution: modules/packages, classes, closures, generators, comprehensions, core async flows.
- CPython `.pyc` execution for a supported subset, including sourceless import fallback paths and exception-table `try`/`except`/`with` baseline semantics.
- Broad stdlib foundation (`sys`, import foundations, `os`/`pathlib`, `json`, `re`, `math`, `datetime`, `random`, `sqlite3` baseline, core `asyncio`/`threading`/`signal`).
- Interactive REPL via `pyrs` with syntax highlighting, history, multiline input, repr-style expression echo, and banner `RSPYTHON`.
- Curated CPython harness suites and project test suite are green.

## Current Limits

- Not full CPython 3.14 parity yet (remaining long-tail language/runtime and stdlib edge semantics).
- C-extension compatibility is not implemented yet (e.g. NumPy remains out of scope until extension milestones).
- Performance/hardening milestones are still ahead.

## Performance Snapshot

Canonical benchmark suite:

```bash
scripts/bench_fib_gate.sh 5
scripts/bench_dispatch_hotpath.sh 5
scripts/bench_dict_backend.sh 5
```

Latest local snapshot (`2026-02-11`):

- `fib(29)x5`: `pyrs ~0.56s` user vs `python3.10 ~0.49s` user (`~1.15x`)
- dispatch hotpath: `pyrs ~0.44-0.50s` vs `python3.10 ~0.054-0.056s` (`~7.9-9.3x`)
- dict microbench: `pyrs ~0.24s` vs `python3.10 ~0.02s`
- pickle hotspot: `pyrs ~5.01s` vs `python3.10 ~0.43s` (`~11.7x`)

## Native Stdlib Layout

- VM-native stdlib handlers are being split out of `src/vm/mod.rs` into `src/vm/stdlib/`.
- Current extracted modules: `src/vm/stdlib/json.rs`, `src/vm/stdlib/re.rs`, `src/vm/stdlib/csv.rs`, `src/vm/stdlib/sqlite3.rs`.
- Direction: prefer CPython official pure-Python stdlib implementations whenever feasible; keep native handlers only where required and track remaining native/stub parity gaps in `docs/STUB_ACCOUNTING.md`.

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

Run interactive REPL (default with no args):

```bash
cargo run --
```

REPL commands:

```text
:help   :clear   :paste   :timing   :reset   :exit/:quit
%time <expr-or-stmt>
%timeit [-n N] [-r R] <expr-or-stmt>
```

REPL keys:

```text
Tab                     insert 4 spaces
Shift-Tab / Ctrl-Space  open completion menu
Esc                     dismiss active suggestion/menu
```

REPL startup script:

```text
default: ~/.pyrsrc
override: PYRS_REPL_INIT=/path/to/init.py
disable: PYRS_REPL_INIT=""
```

REPL theme:

```text
auto detect (default): PYRS_REPL_THEME=auto
force dark palette:    PYRS_REPL_THEME=dark
force light palette:   PYRS_REPL_THEME=light
```

Run from piped stdin (non-interactive mode):

```bash
echo "print(40 + 2)" | cargo run --
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
