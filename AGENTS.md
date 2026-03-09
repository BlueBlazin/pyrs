# Project Context: Python Interpreter in Rust (`pyrs`)

## Vision
Build a production-grade Python interpreter in Rust with source + bytecode + extension API compatibility for CPython 3.14, minimal third-party dependencies, and architecture that can later support JIT.

## Non-Negotiable Engineering Rule
- Do not use quick fixes as a substitute for correct design.
- Prioritize root-cause, foundational solutions over tactical patches.
- Any temporary workaround must be explicitly marked and tracked with closure criteria.
- For stdlib-facing behavior, implement from CPython reference first (`Modules/*.c`, `Objects/*.c`, `Lib/*.py`) and Python 3.14 docs, then validate with tests.
- CPython 3.14 semantics are the only correctness target: never keep/introduce custom pyrs-specific behavior where CPython differs.
- Tests must encode CPython behavior, not current pyrs behavior. If a test passes with non-CPython semantics, the test is wrong and must be corrected.
- Avoid bootstrap-only mock surfaces that diverge from CPython architecture (e.g. prefer native `_module` substrate + CPython `Lib/*.py` layer instead of replacement modules when CPython provides one).
- For NumPy/scientific-stack bring-up, do not use trial-and-error patch churn: drive fixes from CPython source + Python 3.14 C-API docs, close root causes in the ABI substrate.

## Reporting Discipline
- End each progress update with the immediate next `3-6` concrete steps.
- Create a git commit after each completed piece of work or subtask; do not let progress sit uncommitted for long stretches.

## Command Execution Hygiene
- Prefer direct command execution from the existing working directory (no wrapper like `cd ... && ...` unless direct execution fails).
- Prefer setting environment variables in a separate step before running commands; use inline `ENV=... cmd` only as a fallback when the direct approach is not viable.
- Hard rule for this workspace: do not run commands in inline-env form (`ENV=... cmd`) when a separate environment setup step is possible.
- Do not prepend `mkdir -p perf` to profiling commands; `perf/` already exists in this workspace.
- For profiling commands (especially `cargo flamegraph`), never run inline env form (for example `MY_ENV_VAR=foo cargo flamegraph ...`); set env vars in a separate step first, then run the command cleanly.

## Local CPython Source/Lib Baseline
- Use local untracked CPython checkout at `.local/Python-3.14.3` as the primary reference root.
- Use `.local/Python-3.14.3/Lib` as the default `PYRS_CPYTHON_LIB` for local probes.
- Keep `/.local/` untracked (git-ignored); never commit the copied CPython tree.

## Test Execution Cadence
- Default local Rust test runner is `cargo nextest run` (targeted and full runs).
- Use `cargo test` only when `nextest` is not suitable (for example, behavior that specifically depends on `cargo test` execution semantics).
- Default to targeted tests for touched surfaces first.
- After any high-risk semantic/runtime change, run and pass targeted local tests for the touched behavior before committing.
- Run full suite only for major checkpoints (multi-subsystem refactors, milestone closure, or explicit user request).
- Do not set `RUST_TEST_THREADS=1` by default. Use single-threaded test execution only for targeted race/flakiness diagnosis, then return to default parallel execution.

## Scope and Constraints
- Target version: CPython 3.14
- Platform support priority: Linux and macOS (`x86_64` + `aarch64`) are required release targets; Windows support is non-priority for now.
- Current goals:
  - Production grade python 3.14 interpreter
- Current non-goals:
  - JIT implementation (for now, we will later add support)
- Architecture constraints:
  - Packrat parser aligned to CPython grammar
  - AST -> bytecode IR pipeline
  - CPython-like runtime object model, refcount + cycle GC, and GIL
  - Minimal, justified dependencies only

## Execution Policy
- CPython behavior is the source of truth:
  - `Modules/*.c`
  - `Objects/*.c`
  - `Lib/*.py`
- For re-entrant extension callback paths, never pass context pointers via `&mut ctx as *mut ...`; use `std::ptr::addr_of_mut!(ctx)` and raw-pointer plumbing to avoid aliasing UB.
- Prefer official CPython pure-Python stdlib implementations where feasible.
- Keep native handlers as substrate/accelerator layers, not replacement semantics.
- When citing Python docs, always pin URLs to `https://docs.python.org/3.14/...` (do not use unversioned `.../3/...` links).
- Local shim policy:
  - local shim fallback is now `_ctypes`-only; fallback is enabled by default and can be disabled with `PYRS_DISABLE_LOCAL_SHIMS=1`.
- Keep docs updated in the same checkpoint as behavior changes.
- Keep worktrees clean; commit small focused checkpoints.
- End-of-round rule: finish each coding/reporting round with a clean `git status` (no uncommitted tracked/untracked changes).
- End every assistant turn with immediate next `3-6` concrete steps.

## Test Loop Policy
- Fast local loops: targeted unit/integration tests first.
- Strict stdlib harness is opt-in for frequent local loops:
  - `PYRS_RUN_STRICT_STDLIB=1`
  - `PYRS_PARITY_STRICT=1`
- Deferred strict pickle lane is opt-in until closure:
  - `PYRS_RUN_DEFERRED_PICKLE=1`
  - `PYRS_DEFERRED_PICKLE_TIMEOUT_SECS` (default `max(PYRS_STRICT_HARNESS_TIMEOUT_SECS, 600)`)

## Performance Policy
- Canonical benchmark gates:
- `scripts/bench_fib_gate.sh 5`
- `scripts/bench_dispatch_hotpath.sh 5`
- `scripts/bench_dict_backend.sh 5`
- `scripts/bench_startup_gate.sh 7`
- All optimization work must update the relevant benchmark artifact or source-backed reference doc in the same checkpoint.

## Canonical Documents (may be outdated)
- Docs index and ownership map: `docs/README.md`
- CPython benchmark runner: `docs/CPYTHON_COMPAT_BENCHMARK.md`
- CPython compatibility priorities: `docs/CPYTHON_COMPAT_PRIORITIES.md`
- Extension capability matrix: `docs/EXTENSION_CAPABILITY_MATRIX.md`
- Extension packaging/build contract: `docs/EXTENSION_PACKAGING_CONTRACT.md`
- Extension C-API v1 slice: `docs/EXTENSION_CAPI_V1.md`
- NumPy bring-up tracker: `docs/NUMPY_BRINGUP_GATE.md`
- Builtin parity gate: `docs/BUILTIN_PARITY.md`
- Language feature manifest/inventory: `docs/LANGUAGE_FEATURE_MANIFEST.md`, `docs/LANGUAGE_FEATURE_INVENTORY.md`
- VM architecture map: `docs/VM_ARCHITECTURE_MAP.md`
- Unicode-name table provenance: `docs/UNICODE_NAME_DATA.md`

## Reference Artifacts
- Dict backend CPython mapping: `docs/DICT_BACKEND_CPYTHON_MAPPING.md`
- Clone audit artifacts: `docs/CLONE_BASELINE.txt`, `docs/CLONE_AUDIT.md`
- C-API no-op inventory: `docs/CAPI_NOOP_INVENTORY.md`
- No-op inventory snapshot: `docs/NOOP_BUILTIN_INVENTORY.txt`
