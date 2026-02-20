# VM Error Model Refactor Plan (Remove String-Based Classification)

Status: in progress (phase 1 + initial phase 2 landed).

## Landed So Far

1. `RuntimeError` now carries optional typed exception payload:
   - `message: String`
   - `exception: Option<Box<ExceptionObject>>`
2. Added typed constructors/helpers:
   - `with_exception`, `from_exception`, `exception_name`
   - `type_error`, `value_error`, `attribute_error`, `index_error`, `key_error`
3. `RuntimeError::new(...)` now auto-extracts exception type/message from:
   - prefixed messages (`TypeError: ...`)
   - traceback tails (`Traceback ... \nValueError: ...`)
4. VM exception conversion is now centralized through:
   - `runtime_error_to_exception_object(...)`
   - `ensure_exception_default_attrs(...)`
5. `runtime_error_matches_exception(...)` now matches on typed payload first, with legacy string fallback.
6. Major call sites that previously compared `classify_runtime_error(&err.message)` are now using typed matching helpers.
7. `runtime_error_from_active_exception(...)` now preserves the original `ExceptionObject` in `RuntimeError.exception` while retaining traceback text for compatibility.
8. Explicit `raise ... from ...` now preserves `__context__` (in addition to `__cause__` + `__suppress_context__`) in VM raise plumbing.
9. VM execution error handling no longer builds exceptions from string parsing directly; it now uses centralized `runtime_error_to_exception_object(...)`.
10. Replaced prefixed `RuntimeError::new(\"XError: ...\")` callsites with typed constructors across VM/stdlib surfaces (136 callsites), reducing fallback classifier pressure.
11. `runtime_error_to_exception_object(...)` and `runtime_error_matches_exception(...)` now prefer extracted/typed exception names before invoking legacy classifier heuristics.
12. `RuntimeError::new(...)` exception extraction now has a fast reject path for non-exception freeform messages to keep the compatibility lane cheap.
13. `runtime_error_matches_exception(...)` no longer falls back to `classify_runtime_error(...)`; matches now depend on typed payloads, extracted exception names, or traceback-tail exception lines.
14. Core attribute lookup surfaces were migrated to emit typed `AttributeError` (`RuntimeError::attribute_error(...)`) instead of untyped text in major VM paths (`vm_builtin_metadata`, `vm_execution`, `builtins_core`).
15. `__len__` parity improved for non-int and negative returns:
   - non-int now raises typed `TypeError` (`'<type>' object cannot be interpreted as an integer`)
   - negative values now raise typed `ValueError` (`__len__() should return >= 0`)
16. Generator resume error parity improved in `vm_native_dispatch`:
   - re-entrancy now raises typed `ValueError` (`generator already executing`)
   - non-generator and invalid initial `send` paths now raise typed `TypeError`.
17. Await/yield-from iterator protocol paths now raise typed exceptions in `vm_native_dispatch`:
   - `object is not awaitable` / `object is not iterable` / `yield from expects iterable` now raise `TypeError`
   - `__iter__()` / `__await__()` non-iterator contract failures now raise `TypeError`
   - `generator ignored GeneratorExit` now raises typed `RuntimeError`.
18. Added VM conformance regressions for these contracts in `/Users/$USER/pyrs/tests/vm.rs`:
   - `executes_generator_yield_from_non_iterable_raises_type_error`
   - `executes_await_non_awaitable_raises_type_error`
   - `executes_await_requires_iterator_from_dunder_await`

## Why This Exists

Today, VM control flow still depends on parsing error text:

- `RuntimeError` is currently a message-only struct:
  - `/Users/$USER/pyrs/src/runtime/mod.rs:7703`
- Exception typing is inferred from message text:
  - `/Users/$USER/pyrs/src/vm/mod.rs:9204` (`classify_runtime_error`)
  - `/Users/$USER/pyrs/src/vm/mod.rs:9572` (`runtime_error_matches_exception`)
- Unhandled exceptions are frequently converted to formatted traceback strings and then re-parsed later:
  - `/Users/$USER/pyrs/src/vm/vm_execution.rs:6007`
  - `/Users/$USER/pyrs/src/vm/stdlib/csv.rs:749`

This is fragile and non-idiomatic:

1. semantics can change when wording changes.
2. `except` behavior depends on message shape.
3. metadata (`errno`, import `name`, `cause/context`) is reconstructed by heuristics.
4. broad `contains(...)` checks are difficult to reason about and maintain.

## Refactor Goals

1. Remove message-string classification from runtime control flow.
2. Carry typed exception data end-to-end.
3. Preserve CPython-visible behavior exactly (exception type, args, attrs, cause/context, traceback formatting at boundary).
4. Keep one temporary compatibility lane for legacy message-only producers, then delete it.

## Non-Goals

1. No user-visible semantic changes beyond fixing existing mismatches.
2. No rewrite of unrelated bytecode/compiler logic.
3. No C-API surface expansion in this refactor itself.

## Baseline (Code Audit Snapshot)

Repository-wide indicators (from `src/`):

- `RuntimeError::new(...)` call sites: `4370`
- `classify_runtime_error(...)` uses: `20`
- `runtime_error_matches_exception(...)` uses: `19`
- direct `RuntimeError::new("TypeError: ...")`-style prefixed literals: `117`
- direct `err.message == ...` / `err.message.contains(...)` style checks: `91+`

Highest-concentration files for `RuntimeError::new(...)`:

- `/Users/$USER/pyrs/src/vm/vm_native_dispatch.rs` (`588`)
- `/Users/$USER/pyrs/src/vm/builtins_io.rs` (`472`)
- `/Users/$USER/pyrs/src/vm/builtins_core.rs` (`458`)
- `/Users/$USER/pyrs/src/vm/vm_execution.rs` (`379`)
- `/Users/$USER/pyrs/src/vm/builtins_os.rs` (`373`)

## Proposed Error Model (Idiomatic Rust)

Replace message-only runtime errors with typed payloads.

### 1) New Runtime Error Shape

```rust
pub enum RuntimeError {
    Exception(ExceptionPayload),
    Internal(InternalVmError),
    // Temporary migration-only variant; remove at end.
    LegacyMessage(String),
}

pub struct ExceptionPayload {
    pub object: ExceptionObject,      // canonical Python exception object
    pub traceback: Vec<TraceFrame>,   // structured traceback frames
}
```

Key point: the VM moves `ExceptionObject` directly, not a string description.

### 2) Construction Helpers (no string prefixes)

Add constructors on `RuntimeError`:

- `type_error(msg)`
- `value_error(msg)`
- `attribute_error(msg)`
- `index_error(msg)`
- `key_error(msg)`
- `import_error(msg, name)`
- `module_not_found(msg, name)`
- `os_error(errno, strerror, msg, filename, filename2)`
- `from_exception_object(ExceptionObject)` (for active exception propagation)

All constructors populate `ExceptionObject.attrs` (`args`, `errno`, `strerror`, `name`, etc.) directly.

### 3) Matching Helpers

Replace string parsing checks with typed checks:

- `err.exception_name() -> Option<&str>`
- `err.is_exception("TypeError")`
- `err.matches_exception_or_subclass("OSError")`

These operate on exception type data, not text.

### 4) Formatting Boundary

Traceback rendering happens only at boundary points:

- CLI output
- unraisable hook/log paths
- C-API bridges that need text (`PyErr_*` compat glue)

Internal VM flow should never need `format_traceback(...)->String` as transport.

## Core Implementation Strategy

### A. Normalize Exception Propagation First

Refactor these paths to move typed exceptions:

- `/Users/$USER/pyrs/src/vm/vm_execution.rs:6007` (`raise_exception*`, `unwind_exception`, `handle_runtime_error`)
- `/Users/$USER/pyrs/src/vm/stdlib/csv.rs:749` (`runtime_error_from_active_exception`; relocate to core VM module)
- `/Users/$USER/pyrs/src/vm/mod.rs:1729` (`runtime_error_to_exception_value`)

Target behavior: once an exception exists in `active_exception`, conversions preserve its full structure (`name`, `message`, `attrs`, `cause/context`) without string round-trips.

### B. Remove Classifier Dependence in Hot Control Paths

Refactor call sites that currently branch on classified strings:

- `/Users/$USER/pyrs/src/vm/vm_runtime_methods.rs`
- `/Users/$USER/pyrs/src/vm/vm_execution.rs`
- `/Users/$USER/pyrs/src/vm/stdlib/csv.rs`
- `/Users/$USER/pyrs/src/vm/stdlib/sqlite3.rs`
- `/Users/$USER/pyrs/src/vm/builtins_io.rs`
- `/Users/$USER/pyrs/src/vm/builtins_core.rs`
- `/Users/$USER/pyrs/src/vm/vm_native_dispatch.rs`

Pattern:

- before: `classify_runtime_error(&err.message) == "TypeError"`
- after: `err.matches_exception_or_subclass("TypeError")`

### C. Metadata Parsing Removal

Delete message parsing helpers once typed metadata is populated at source:

- `extract_os_error_errno`
- `infer_os_error_errno`
- `extract_os_error_strerror`
- `extract_import_error_name`
- traceback/prefix parsing helpers used only for type recovery

### D. C-API and Bridge Compatibility

C-API layers currently forward `err.message` frequently.
Keep compatibility by adding one rendering helper:

- `err.render_for_capi()` -> CPython-like text

This allows typed internals while preserving bridge behavior.

## Migration Phases

### Phase 0 (Scaffold)

1. Add new `RuntimeError` enum + helpers.
2. Keep `LegacyMessage` variant and compatibility constructor (`RuntimeError::new`) temporarily.
3. Add lints/tests forbidding *new* classifier dependencies.

### Phase 1 (Core Runtime Loop)

1. Convert `raise_exception*`, `unwind_exception`, `handle_runtime_error` to typed payloads.
2. Convert `runtime_error_from_active_exception` to return typed exception directly.
3. Ensure traceback is carried structurally, not as a pre-rendered transport string.

### Phase 2 (High-Churn Modules)

Convert largest producers/consumers first:

1. `vm_native_dispatch`
2. `builtins_io`
3. `builtins_core`
4. `vm_execution`
5. `builtins_os`

### Phase 3 (Stdlib + Extension Bridges)

1. Convert `csv`, `sqlite3`, `pickle`, `re`, `expat`, etc. branching logic.
2. Convert CPython bridge surfaces to use typed-to-string rendering only at boundary.

### Phase 4 (Delete Legacy)

1. Remove `LegacyMessage`.
2. Remove `classify_runtime_error` and `runtime_error_matches_exception`.
3. Remove all message-parsing inference helpers.

## Test Plan

1. Unit tests for typed constructors and subclass matching.
2. Differential tests vs CPython for exception class + attrs + chaining.
3. Focus suites:
   - `_io`, `csv`, `sqlite3`, importlib, descriptor/object model.
4. Invariant tests:
   - no internal control-flow branch on message text.
   - traceback formatting only at boundary functions.
5. Regression guard:
   - CI grep gate fails if new code introduces `classify_runtime_error(...)`.

## Risk and Mitigation

Risk: broad blast radius (many call sites).

Mitigation:

1. staged migration with compatibility variant.
2. convert highest-traffic paths first to reduce hidden regressions.
3. keep strict CPython differential tests active at each phase.

Risk: C-API integration expecting text.

Mitigation:

1. keep explicit `render_for_capi()` adapter.
2. validate with NumPy/scipy gate scripts after each batch.

## Acceptance Criteria

1. `classify_runtime_error` removed from runtime control flow.
2. `runtime_error_matches_exception` removed.
3. no message-text-based type inference remains in core VM.
4. exception matching and attrs are driven by typed exception payloads.
5. CPython parity suites stay green or improve (no semantic regressions).

## Immediate Implementation Order (Recommended)

1. Introduce new `RuntimeError` enum and compatibility adapters.
2. Convert `vm_execution` exception loop + `runtime_error_from_active_exception`.
3. Replace classifier branches in `vm_execution`/`vm_runtime_methods`/`csv`/`sqlite3`.
4. Convert `_io` and `builtins_core` message equality checks to typed checks.
5. Remove legacy classifier/parser helpers and lock with CI grep gate.
