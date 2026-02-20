# VM Error Model Refactor Plan (Remove String-Based Classification)

Status: phase-2 control-flow refactor complete; compatibility lane still active in `RuntimeError::new(...)`.

## Current Architecture (2026-02-20)

1. VM runtime control flow no longer classifies errors from freeform strings:
   - `runtime_error_matches_exception(...)` uses typed exception payload/subclass checks only.
   - `runtime_error_to_exception_object(...)` no longer re-parses text to guess exception types.
2. Compatibility classification moved to the construction boundary:
   - `RuntimeError::new(...)` attaches typed `ExceptionObject` where message payloads still come from legacy text-only producers.
   - This keeps `except` semantics stable while legacy callsites are migrated to typed constructors.
3. Structured attrs are attached at construction-time for key families:
   - import errors (`msg`, `name`, `path`)
   - OS errors (`errno`, `strerror`)
4. Remaining refactor closure work is now migration debt:
   - reduce remaining message-only `RuntimeError::new(...)` producers,
   - eventually remove the compatibility classifier from `RuntimeError::new(...)`.

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
19. Converted high-frequency message-only producers to typed constructors across core runtime paths:
   - `TypeError`: unsupported constructors/conversions (`list/tuple/bytes/int/float`), invalid receiver contracts, unsupported membership/operand paths, and non-exception `except` targets.
   - `IndexError`: common `index out of range` paths.
   - `ValueError`: byte-range violations and selected argument-domain violations.
   - `OverflowError`: selected integer overflow paths.
   - `StopIteration`: direct message-only producers moved to typed `RuntimeError::stop_iteration(...)`.
20. Added targeted VM regressions in `/Users/$USER/pyrs/tests/vm.rs` for typed exception parity on:
   - builtin constructor unsupported-type contracts,
   - membership/index protocol contract failures,
   - bytes/bytearray range validation.
21. Codec error-handler semantics now use typed `LookupError` with CPython-style messaging:
   - unknown handlers now raise `LookupError("unknown error handler name '<name>'")`
   - conversion landed across codec normalize/encode/decode paths in `/Users/$USER/pyrs/src/vm/mod.rs`.
22. Bad file-descriptor paths now raise typed `OSError` with structured attrs:
   - added `RuntimeError::os_error_with_errno(...)` + `RuntimeError::bad_file_descriptor()`
   - migrated fd lookup/mutation paths in `/Users/$USER/pyrs/src/vm/builtins_os.rs` and `/Users/$USER/pyrs/src/vm/builtins_io.rs`.
23. Remaining `RuntimeError::new(\"...\")` inventory was reduced again; the highest-frequency leftovers are now dominated by narrower domain-specific payload/format/VM-internal diagnostics instead of core protocol/type contracts.
24. Converted additional constructor/protocol contract failures to typed `TypeError`:
   - `memoryview() expects bytes-like object`
   - `type() bases must be tuple/list` and class-base resolution failures (`class base must be a class object`)
   - `set() expects at most one argument`
   - `__iter__()` / `__next__()` / `close()` / `write()` argument-count failures in IO/dispatch paths.
25. Converted closed-file operations to typed `ValueError` (`I/O operation on closed file.`) across IO surfaces.
26. Added conformance regressions for this batch in `/Users/$USER/pyrs/tests/vm.rs`:
   - `constructor_contract_errors_for_memoryview_type_and_set_are_typed`
   - `io_method_arity_and_closed_file_contracts_are_typed`
27. `memoryview.cast(...)` keyword/positional argument validation now follows CPython argument-clinic semantics:
   - duplicate positional+keyword `format`/`shape` now raises typed `TypeError` with CPython-style text (`argument for cast() given by name ... and position ...`)
   - over-arity now raises typed `TypeError` with dynamic count (`takes at most 2 arguments` / `takes at most 2 keyword arguments`).
28. Additional typed-constructor conversion landed for parsing/conversion contracts:
   - `invalid JSON number` / `invalid unicode escape` / `invalid UTF-8 in JSON string` -> typed `ValueError`
   - `float() invalid literal` -> typed `ValueError`
   - `range() got multiple values` / `unpack expects iterable` / invalid file-object contracts -> typed `TypeError`
   - unsupported codec encoding normalization now raises typed `LookupError`.
29. CSV unknown-dialect failures now produce typed `Error` exceptions via explicit `RuntimeError::with_exception("Error", ...)` instead of relying on classifier text matching.
30. Added additional regression tests in `/Users/$USER/pyrs/tests/vm.rs` for this conversion wave:
   - `range_duplicate_argument_error_is_typed`
   - `unpack_non_iterable_error_is_typed`
   - `float_invalid_literal_error_is_typed_value_error`
   - `csv_unknown_dialect_error_is_typed_error`
31. Current highest-frequency remaining `RuntimeError::new(...)` buckets are now mostly:
   - VM-internal diagnostics (`name index out of range`, `constant index out of range`, stack underflow),
   - specialized memoryview format/tolist long-tail (`memoryview.tolist() unsupported format`, `memoryview: unsupported format`),
   - narrower domain contract messages (`setstate() expects one argument`, `path/pattern/name must be ...`).
32. Additional contract conversions landed this slice:
   - `type() first argument must be string`, `enumerate() expects iterable`,
     `path/pattern/name must be str/bytes`, `setstate()` arity, `tuple.count/index` receiver/arity,
     and `__mro_entries__ must return a tuple` now emit typed `TypeError`.
33. Added conformance regressions in `/Users/$USER/pyrs/tests/vm.rs`:
   - `regex_pattern_type_contract_errors_are_typed`
   - `mro_entries_non_tuple_contract_error_is_typed`
34. Memoryview unsupported-format closure:
   - `memoryview_format_for_view(...)` now raises typed `NotImplementedError` for unsupported format specs with CPython-style messaging (`memoryview: format <fmt> not supported`).
   - `memoryview.tolist()` unsupported-format paths now raise typed `NotImplementedError` (`memoryview: unsupported format`) instead of message-only runtime errors.
35. Added regression `memoryview_tolist_unsupported_format_raises_not_implemented` to lock this behavior.
36. `range(...)` contract typing was completed across runtime call paths:
   - zero-arg arity now raises typed `TypeError` (`range expected at least 1 argument, got 0`),
   - invalid keyword names now raise typed `TypeError` with the offending keyword,
   - step-zero remains typed `ValueError`.
37. VM Python-function argument binding now emits typed `TypeError` (instead of untyped runtime errors) for:
   - unexpected keyword arguments,
   - duplicate/multiple values for arguments,
   - generic argument-count/required-kwonly mismatches.
38. Fallback `_random` module `randrange(...)` binding was corrected to CPython parameter semantics (`start, stop=None, step=1`):
   - duplicate keyword+positional combinations now raise typed `TypeError` naming the argument,
   - missing required `start` now raises typed `TypeError`,
   - `step == 0` now raises typed `ValueError`.
39. Added typed-conformance regressions in `/Users/$USER/pyrs/tests/vm.rs`:
   - `range_error_contracts_are_typed`
   - `randrange_duplicate_and_empty_range_contracts_are_typed`
40. Random-module argument/contract typing closure landed in `/Users/$USER/pyrs/src/vm/builtins_numeric_time.rs`:
   - `seed/random/randint/getrandbits/choice/choices/shuffle` now emit typed exceptions for signature and contract failures.
   - unexpected keyword errors now include the offending keyword name.
41. Empty-sequence random selection paths now raise typed `IndexError`:
   - `choice([])` and `choices([], k>0)`.
42. Random-domain validation now raises typed `ValueError` for:
   - negative `getrandbits(k)` bit counts,
   - invalid `choices()` weight/cumulative-weight numeric domains and cardinality checks.
43. Added additional regressions in `/Users/$USER/pyrs/tests/vm.rs`:
   - `random_empty_population_contracts_are_typed`
   - `random_argument_contracts_are_typed`
44. Additional core builtin contract conversions now emit typed exceptions:
   - `ord(...)` arity/type/length contract paths -> typed `TypeError`.
   - `dict(...)` arity/shape contract paths -> typed `TypeError`/`ValueError` (sequence-element-length domain).
   - `all(...)` / `any(...)` arity/type contracts -> typed `TypeError`.
45. `divmod(...)` division-by-zero paths now raise typed `ZeroDivisionError` via `RuntimeError::zero_division_error(...)` across integer/bigint/float branches.
46. `namedtuple._make(...)` receiver/class/iterable contract failures now raise typed `TypeError` in both core VM and builtin dispatch paths.
47. Added regression in `/Users/$USER/pyrs/tests/vm.rs`:
   - `core_contract_errors_are_typed_for_ord_dict_all_divmod_and_namedtuple_make`.
48. IO open-path contract typing closure landed in `/Users/$USER/pyrs/src/vm/builtins_io.rs`:
   - `io.open(...)` arity/duplicate-arg/unexpected-keyword/mode-type contract paths now emit typed `TypeError`.
   - mode/newline/buffering/binary-option incompatibility paths now emit typed `ValueError`.
   - bad-fd paths now emit typed `OSError` via `RuntimeError::bad_file_descriptor()`.
49. `io.open(..., opener=...)` exception propagation now preserves the active exception object when opener callbacks raise (no generic string replacement).
50. `FileIO.__init__` contract typing closure landed:
   - missing/duplicate/unexpected argument paths now emit typed `TypeError`,
   - invalid mode-string/type paths now emit typed `ValueError`/`TypeError` as appropriate.
51. Added IO typed-conformance regressions in `/Users/$USER/pyrs/tests/vm.rs`:
   - `io_open_contract_errors_are_typed`
   - `io_fileio_contract_errors_are_typed`.
52. `IncrementalNewlineDecoder` argument-contract typing closure landed in `/Users/$USER/pyrs/src/vm/builtins_io.rs`:
   - `__init__`/`decode` duplicate-value, missing-arg, and unexpected-keyword paths now emit typed `TypeError`.
   - decoder contract violations now emit typed `TypeError`:
     - `decoder=None` with non-`str` input,
     - wrapped decoder returning non-`str`.
53. Added IO typed-conformance regression in `/Users/$USER/pyrs/tests/vm.rs`:
   - `_io_incremental_newline_decoder_contract_errors_are_typed`.

## Historical Problem Statement (Pre-Refactor)

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
