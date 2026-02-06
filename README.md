# pyrs

`pyrs` is a Python interpreter written in Rust, targeting compatibility with CPython 3.14.

## Status

`pyrs` is in active alpha development.

- Milestones 0-11 are complete (language/runtime foundations, import system, async foundations, and parity/test gate foundations).
- Milestones 12-16 remain (core CPython parity closure, stdlib/packaging usability closure, performance/observability/runtime hooks, extension ecosystem compatibility, and production release hardening).

## What Users Can Expect Today

- Run substantial pure-Python source code (modules/packages, classes, closures, generators, comprehensions, and core async flows).
- Execute a supported subset of CPython 3.14 bytecode (`.pyc`) paths.
- Use a practical stdlib foundation (`sys`, `importlib` basics, `json`, `re`, `math`, `datetime`, `random`, `os`/`pathlib`, and core `asyncio`/`threading`/`signal` pieces).
- Get tracebacks with filename/line/column and inspect parsed AST or emitted bytecode from the CLI.

## Current Limits

- Not full CPython 3.14 parity yet (tokenizer/grammar edge cases and opcode-family completeness are still in progress).
- Stdlib behavior is broad but not complete.
- C-extension compatibility is not implemented yet (for example, NumPy is out of scope right now).
- Performance tuning and production hardening are still upcoming milestones.

## Quick Start

Requirements:

- Rust toolchain (`cargo`)

Build:

```bash
cargo build
```

Run a Python file:

```bash
cargo run -- path/to/script.py
```

Run a `.pyc` file:

```bash
cargo run -- path/to/module.pyc
```

Show AST:

```bash
cargo run -- --ast path/to/script.py
```

Show bytecode:

```bash
cargo run -- --bytecode path/to/script.py
```

Run tests:

```bash
cargo test
```

Run the parity profile:

```bash
./scripts/run_parity_gate.sh
```

## Safety Note for Parity Runs

The curated smoke/parity tests run subprocesses in a constrained mode (`env_clear`, isolated temporary working/home directories, and timeouts). For stronger isolation, run parity suites inside an OS/container sandbox as an extra boundary.

## Key Docs

- Roadmap: `docs/ROADMAP.md`
- Compatibility tracker: `docs/COMPATIBILITY.md`
- Production readiness accounting: `docs/PRODUCTION_READINESS.md`
- Project context for agent workflows: `AGENTS.md`
