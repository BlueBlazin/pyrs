# Project Context: Python Interpreter in Rust (`pyrs`)

## Vision
Build a production-grade Python interpreter in Rust with source + bytecode compatibility for CPython 3.14, minimal third-party dependencies, and architecture that can later support JIT and extension work.

## Non-Negotiable Engineering Rule
- Do not use quick fixes as a substitute for correct design.
- Prioritize root-cause, foundational solutions over tactical patches.
- Any temporary workaround must be explicitly marked and tracked with closure criteria in:
  - `docs/STUB_ACCOUNTING.md`, or
  - `docs/ALGO_AUDIT_BACKLOG.md`.
- Do not use test-by-test attribute patching as a development strategy.
- For stdlib-facing behavior, implement from CPython reference first (`Modules/*.c`, `Objects/*.c`, `Lib/*.py`) and Python 3.14 docs, then validate with tests.
- CPython 3.14 semantics are the only correctness target: never keep/introduce custom pyrs-specific behavior where CPython differs.
- Tests must encode CPython behavior, not current pyrs behavior. If a test passes with non-CPython semantics, the test is wrong and must be corrected.
- Avoid bootstrap-only mock surfaces that diverge from CPython architecture (e.g. prefer native `_module` substrate + CPython `Lib/*.py` layer instead of replacement modules when CPython provides one).
- For NumPy/scientific-stack bring-up, do not use trial-and-error patch churn: drive fixes from CPython source + Python 3.14 C-API docs, close root causes in the ABI substrate, and record each blocker/fix in `docs/NUMPY_BRINGUP_GATE.md`.

## Reporting Discipline
- End each progress update with the immediate next `3-6` concrete steps.

## Command Execution Hygiene
- Prefer direct command execution from the existing working directory (no wrapper like `zsh -lc 'cd ... && ...'` unless direct execution fails).
- Prefer setting environment variables in a separate step before running commands; use inline `ENV=... cmd` only as a fallback when the direct approach is not viable.
- Hard rule for this workspace: do not run commands in inline-env form (`ENV=... cmd`) when a separate environment setup step is possible.
- Do not set `RUST_TEST_THREADS=1` by default. Use single-threaded test execution only for targeted race/flakiness diagnosis, then return to default parallel execution.

## Active Execution Lock (2026-02-16)
- Primary focus is C-API closure for native scientific-stack support.
- Use two-lane execution:
  - Lane A: CPython 3.14 Stable ABI (`abi3`) closure.
  - Lane B: explicit non-abi3 surfaces required by NumPy/scientific stack.
- P0 safety lock:
  - treat C-API wrapper lifetime as a first-class blocker (`docs/CAPI_LIFETIME_MODEL.md`);
  - no patch-only pinning strategy as final state;
  - migrate to VM-global registry + explicit borrowed/new/stolen ownership semantics.
- Before starting a new coding round, re-check:
  - `docs/CAPI_PLAN.md` (execution lock + abi3 status),
  - `docs/CAPI_LIFETIME_MODEL.md` (lifetime invariants + migration phase),
  - `docs/CAPI_NOOP_EXECUTION_ORDER.md` (ordered closure checklist for remaining C-API no-op rows),
  - `docs/NUMPY_BRINGUP_GATE.md` (current blocker + gate state),
  - `perf/abi3_manifest_latest.json` (coverage baseline).
- Required cadence for each ABI batch:
  1. refresh manifest
  2. add/extend conformance tests
  3. implement ABI/runtime substrate deltas
  4. refresh NumPy gate artifact
  5. commit checkpoint
- Do not drift into unrelated long-tail work while this lock is active.
- Every assistant status update must end with an explicit list of the immediate next `3-6` steps.

## Scope and Constraints
- Target version: CPython 3.14
- Current goals:
  - Run Python source code
  - Execute CPython 3.14 bytecode (`.pyc`)
- Current non-goals:
  - JIT implementation
  - Full CPython C-API / C-extension compatibility
- Architecture constraints:
  - Packrat parser aligned to CPython grammar
  - AST -> bytecode IR pipeline
  - CPython-like runtime object model, refcount + cycle GC, and GIL
  - Minimal, justified dependencies only

## Milestone State
- Milestones 0-12: complete
- Milestone 13: in progress (active)
- Milestones 14-16: pending

Milestone 13 closes only when P0 blockers in `docs/PRODUCTION_READINESS.md` and `docs/STUB_ACCOUNTING.md` are fully closed.

## Current Snapshot (2026-02-14)
- Error-reporting parity checkpoint (2026-02-23, latest):
  - Added local PEP references used for implementation:
    - `docs/references/pep-0626.rst`
    - `docs/references/pep-0657.rst`
    - execution plan in `docs/ERROR_REPORTING_PLAN.md`.
  - Exception-constructor kwargs parity updated in `instantiate_exception_type`:
    - `AttributeError(..., name=?, obj=?)`,
    - `NameError(..., name=?)`,
    - `ImportError/ModuleNotFoundError(..., name=?, path=?)`,
    - unexpected kwargs now raise typed `TypeError` with CPython-style text.
  - Traceback formatting now includes source lines and caret spans when source text is available:
    - source text cache added to VM and wired for file/`-c`/REPL/import/eval/exec/compile paths.
    - frame lines now render in CPython shape (`File "...", line N, in ...`) with caret.
    - caret fallback now infers identifier-span highlights when end columns are unavailable
      and suppresses keyword-only highlights (e.g. no caret under bare `raise` keyword lines).
  - CLI/REPL parser failures now emit `SyntaxError`-style diagnostics (`File/line/source/caret`)
    instead of raw parser offset strings.
    - parse-error diagnostics now map parser-internal expectation wording to CPython-style
      user-facing classes/messages for core syntax cases (`invalid syntax`, indentation class,
      unclosed delimiter message).
    - delimiter diagnostics now include CPython-style unmatched/mismatch forms and triple-quote
      unterminated-string message shape.
    - unclosed `(`/`[` followed by `:` now emits CPython-style `invalid syntax` at the
      colon position (instead of always reporting `'<delimiter>' was never closed`).
  - Exception objects now retain propagated traceback-frame metadata (`traceback_frames`) so
    chained exceptions (`__context__` / `__cause__`) can render separate traceback blocks.
  - chained exception output now follows CPython flow:
    - context/cause sections render independent `Traceback (most recent call last):` blocks with
      delimiter text (`During handling...` / `The above exception was the direct cause...`).
  - CPython `.pyc` `co_linetable` decoding now maps instruction ranges into `Location {line,column,end_line,end_column}`.
  - traceback frame capture now honors `reraise_lasti_override` when set, preserving original
    source line fidelity for reraised exceptions in exception-table cleanup flows
    (fixes line-0 fallback in `.pyc` context-chain tracebacks).
  - VM unwind paths now preserve existing traceback frame stacks for reraises:
    - bare `raise` and opcode `RERAISE` now keep prior traceback frames instead of
      re-rooting at cleanup handler lines.
    - compiler-generated rethrows in `with` and `try/finally` cleanup now emit
      `Opcode::Reraise` (instead of `Raise 1`) for CPython traceback-line parity.
  - explicit `raise exc` now preserves existing traceback chains while appending the current
    raise site (CPython ordering parity for both source and `.pyc` execution paths).
  - exception `__traceback__` is now materialized as a runtime traceback object chain
    (`tb_next`/`tb_lineno`/`tb_lasti`/`tb_frame`) instead of always `None`, and
    `with_traceback(...)` / direct `__traceback__` writes now apply CPython traceback-or-None
    type contracts.
    - compatibility note: runtime traceback objects currently use `tb_lasti = -1` fallback and
      synthesized frame metadata; `tb_lasti`+`co_positions` precision closure remains tracked.
  - code-object location APIs landed for traceback/PEP alignment:
    - `code.co_positions()` now returns an iterator of 4-tuples
      `(start_line, end_line, start_col, end_col)`,
    - `code.co_lines()` now returns an iterator of 3-tuples
      `(start_offset, end_offset, line)`.
  - traceback `tb_lasti` propagation now preserves per-frame instruction offsets through
    exception frame capture/materialization:
    - `ExceptionTracebackFrame` now stores `lasti`,
    - traceback objects expose `tb_lasti` from captured frame instruction offsets
      (instead of fixed fallback),
    - traceback frame synthetic code objects now materialize enough instruction/location rows
      to keep `_get_code_position(co, tb_lasti)` in-range.
  - `compile(..., flags=_ast.PyCF_ONLY_AST)` baseline now materializes `_ast` node objects
    for `exec`/`eval` modes, including statement/expression shapes used by traceback caret
    heuristics (`Module`, `Assign`, `Return`, `Expr`, `Call`, `Name`, and context nodes).
  - `_ast` bootstrap surface was expanded with missing node classes consumed by stdlib `ast.py`
    and traceback matching paths (`Module`, `Assign`, `Return`, `Expr`, `Pass`).
  - `_ast` class metadata parity improved:
    - AST node classes now publish `_fields`, `__match_args__`, and `_attributes`,
      enabling positional `match` class-pattern behavior used by stdlib traceback helpers.
    - expression/statement conversion now covers additional core nodes used by traceback
      anchor extraction (`BinOp`, `Compare`, `UnaryOp`, `BoolOp`, `IfExp`, `NamedExpr`,
      and `Slice`) instead of falling back to placeholder constants.
  - `_ast` bootstrap class inventory now includes missing operator/comparator/context nodes
    required by those conversion paths (`And`, `Or`, `Not`, `Is`, `IsNot`, `In`, `NotIn`,
    `BoolOp`, `IfExp`, `NamedExpr`).
  - `_ast` hierarchy wiring now mirrors CPython base-class structure for core abstract roots:
    - `mod`/`stmt`/`expr`/`expr_context`/`operator`/`unaryop`/`boolop`/`cmpop` now inherit from `AST`,
    - concrete nodes are wired to their abstract families (e.g. `Assign -> stmt`,
      `Name/Call/BinOp/Compare/... -> expr`, `Add/Sub/... -> operator`, `Eq/Lt/... -> cmpop`).
  - `compile(..., PyCF_ONLY_AST)` statement-surface coverage expanded beyond minimal bootstrap:
    - `Delete`, `Raise`, `Assert`, `If`, `While`, `For`/`AsyncFor`, `With`/`AsyncWith`,
      `Try`/`TryStar`, `Import`/`ImportFrom`, `Global`/`Nonlocal`, `Break`, `Continue`,
      plus helper nodes `alias`, `withitem`, and `ExceptHandler`.
  - `compile(..., PyCF_ONLY_AST)` conversion now includes function/class definition surfaces:
    - `FunctionDef` / `AsyncFunctionDef` / `ClassDef`,
    - `arguments` / `arg`,
    - `type_param` / `TypeVar` / `ParamSpec` / `TypeVarTuple`,
    - decorator propagation via `StmtKind::Decorated`.
  - `compile(..., PyCF_ONLY_AST)` statement conversion now includes assignment variants:
    - `AugAssign` (augmented-op to `_ast` operator-node mapping),
    - `AnnAssign` with CPython `simple` field semantics
      (`1` for name targets, `0` for non-name targets).
  - `compile(..., PyCF_ONLY_AST)` now includes structural pattern matching node conversion:
    - `Match` + `match_case`,
    - `pattern` abstract root and concrete `Match*` families:
      `MatchValue`, `MatchSingleton`, `MatchSequence`, `MatchMapping`,
      `MatchClass`, `MatchStar`, `MatchAs`, `MatchOr`.
  - `compile(..., PyCF_ONLY_AST)` expression conversion now covers previously fallback-heavy
    families:
    - `Lambda`, `Await`,
    - `ListComp` / `SetComp` / `DictComp` / `GeneratorExp`,
    - `Yield` / `YieldFrom`,
    - generator-clause helper node `comprehension`.
  - type-parameter AST conversion now preserves star-kind semantics from parser type params:
    - `T` -> `_ast.TypeVar`,
    - `*Ts` -> `_ast.TypeVarTuple`,
    - `**P` -> `_ast.ParamSpec`,
    - with name fields normalized to unprefixed identifiers.
  - location-attribute propagation for AST helper nodes improved:
    - `alias`, `keyword`, and `ExceptHandler` conversions now populate location attrs
      (`lineno`, `col_offset`, `end_lineno`, `end_col_offset`) rather than leaving
      `_attributes` unset.
  - `_ast` metadata/hierarchy parity was extended for these node families:
    - metadata now includes CPython-shaped `_fields` / `_attributes` for `FunctionDef`,
      `AsyncFunctionDef`, `ClassDef`, `arguments`, `arg`, `type_param`, `TypeVar`,
      `ParamSpec`, `TypeVarTuple`,
    - hierarchy now wires `FunctionDef`/`AsyncFunctionDef`/`ClassDef -> stmt`,
      `arguments`/`arg`/`type_param -> AST`,
      `TypeVar`/`ParamSpec`/`TypeVarTuple -> type_param`,
    - corrected `withitem._attributes` to CPython parity (`()`).
    - pattern hierarchy now wired to CPython families:
      `Match -> stmt`, `match_case -> AST`, `pattern -> AST`,
      concrete `Match*` pattern classes -> `pattern`.
  - additional AST hierarchy regressions are now covered:
    - `tests/vm.rs::compile_only_ast_honors_core_ast_hierarchy`
    - `tests/vm.rs::compile_only_ast_honors_operator_hierarchy`.
    - `tests/vm.rs::compile_only_ast_covers_common_statement_nodes`.
    - `tests/vm.rs::compile_only_ast_covers_function_class_and_type_param_nodes`.
    - `tests/vm.rs::compile_only_ast_covers_augassign_and_annassign_nodes`.
    - `tests/vm.rs::compile_only_ast_covers_match_and_pattern_nodes`.
    - `tests/vm.rs::compile_only_ast_sets_location_attrs_on_alias_keyword_and_excepthandler`.
    - `tests/vm.rs::compile_only_ast_covers_lambda_await_comprehension_and_yield_nodes`.
    - `tests/vm.rs::compile_only_ast_preserves_type_param_kinds_for_star_and_doublestar`.
  - differential CPython parity gates were expanded for AST-compile surfaces:
    - `tests/differential_cpython.rs::differential_compile_only_ast_assign_fields_and_match_args`
    - `tests/differential_cpython.rs::differential_compile_only_ast_operator_hierarchy_parity`.
    - `tests/differential_cpython.rs::differential_compile_only_ast_function_class_and_type_param_parity`.
    - `tests/differential_cpython.rs::differential_compile_only_ast_augassign_and_annassign_parity`.
    - `tests/differential_cpython.rs::differential_compile_only_ast_match_and_pattern_parity`.
    - `tests/differential_cpython.rs::differential_compile_only_ast_alias_keyword_and_handler_location_attrs_parity`.
    - `tests/differential_cpython.rs::differential_compile_only_ast_lambda_await_comprehension_and_yield_parity`.
    - `tests/differential_cpython.rs::differential_compile_only_ast_type_param_kind_parity_for_star_and_doublestar`.
  - native codec keyword-argument parity improved for traceback formatting paths:
    - `str.encode`, `str.decode`, and `bytes.decode` now accept `encoding=`/`errors=` kwargs
      with duplicate/unexpected-keyword contract checks.
  - traceback footer exception formatting now resolves display text from exception `args` where
    available and applies CPython KeyError single-arg `repr(arg)` behavior.
  - compiler now enforces CPython semantic syntax errors (with span-backed diagnostics):
    - `'return' outside function`,
    - `'break' outside loop`,
    - `'continue' not properly in loop`,
    - `'await' outside function`,
    - `'yield' outside function`,
    - `'yield from' outside function`,
    - `'return' with value in async generator`,
    - `global/nonlocal` declaration-order and scope errors
      (`used prior`, `assigned before`, module-level `nonlocal`, missing nonlocal binding,
      parameter/global conflict, parameter/nonlocal conflict, nonlocal/global conflict).
  - CLI/REPL compile diagnostics now render as `SyntaxError` (instead of `compile error: ...`);
    `-c` semantic compile errors omit source+caret to match CPython command-mode behavior.
  - syntax-error source rendering now follows CPython indentation display shape (normalized
    leading indentation with adjusted caret offsets; `unexpected indent` still omits caret).
  - `str(KeyError(<arg>))` now follows CPython single-arg behavior (`repr(arg)`).
  - Unhandled exception propagation no longer re-wraps traceback text as nested `RuntimeError`/`<Exc>: Traceback ...`.
  - New regressions:
    - `tests/vm.rs::exception_constructor_keyword_parity_matches_cpython`
    - `tests/vm.rs::traceback_output_preserves_exception_type_without_traceback_rewrap`
    - `tests/vm.rs::code_object_co_positions_and_co_lines_iterators_have_expected_shape`
    - `tests/vm.rs::traceback_helpers_can_read_exception_traceback_attr`
    - `tests/vm.rs::traceback_tb_lasti_maps_into_code_positions`
    - `tests/vm.rs::compile_only_ast_returns_assign_and_call_shape`
    - `tests/vm.rs::compile_only_ast_supports_positional_match_patterns`
    - `tests/vm.rs::compile_only_ast_covers_binop_compare_and_slice_shapes`
    - `tests/vm.rs::traceback_caret_infers_identifier_span_without_keyword_noise`
    - `tests/vm.rs::traceback_caret_skips_statement_keyword_ranges`
    - `tests/vm.rs::keyerror_single_arg_string_uses_repr_semantics`
    - `tests/vm.rs::rejects_return_outside_function_with_syntax_message`
    - `tests/vm.rs::rejects_yield_outside_function_with_syntax_message`
    - `tests/vm.rs::rejects_await_outside_async_function_with_syntax_message`
    - `tests/vm.rs::rejects_async_generator_return_with_value_with_syntax_message`
    - `tests/vm.rs::rejects_global_used_prior_declaration_with_syntax_message`
    - `tests/vm.rs::rejects_global_assigned_prior_declaration_with_syntax_message`
    - `tests/vm.rs::rejects_module_nonlocal_with_cpython_message`
    - `tests/vm.rs::rejects_nonlocal_without_binding_with_cpython_message`
    - `tests/vm.rs::rejects_parameter_and_global_conflict_with_cpython_message`
    - `tests/vm.rs::rejects_parameter_and_nonlocal_conflict_with_cpython_message`
    - `tests/vm.rs::rejects_nonlocal_global_conflict_with_global_first`
    - `tests/vm.rs::rejects_nonlocal_global_conflict_with_nonlocal_first`
    - `tests/pyc_translate.rs::translates_cpython_linetable_into_instruction_ranges`.
    - `tests/differential_cpython.rs::differential_traceback_context_chain_matches_cpython_shape`
    - `tests/differential_cpython.rs::differential_traceback_direct_cause_matches_cpython_shape`.
    - `tests/differential_cpython.rs::differential_traceback_identifier_caret_span_matches_cpython`.
    - `tests/differential_cpython.rs::differential_traceback_suppressed_context_matches_cpython_shape`.
    - `tests/differential_cpython.rs::differential_traceback_mixed_cause_and_context_chain_matches_cpython_shape`.
    - `tests/differential_cpython.rs::differential_syntax_error_shape_matches_cpython`.
    - `tests/differential_cpython.rs::differential_invalid_syntax_span_matches_cpython`
    - `tests/differential_cpython.rs::differential_unclosed_delimiter_shape_matches_cpython`
    - `tests/differential_cpython.rs::differential_indentation_error_shape_matches_cpython`.
    - `tests/differential_cpython.rs::differential_unmatched_closing_delimiter_matches_cpython`
    - `tests/differential_cpython.rs::differential_mismatched_closing_delimiter_matches_cpython`
    - `tests/differential_cpython.rs::differential_unterminated_triple_quoted_string_matches_cpython`
    - `tests/differential_cpython.rs::differential_unexpected_indent_matches_cpython`
    - `tests/differential_cpython.rs::differential_unindent_mismatch_matches_cpython`
    - `tests/differential_cpython.rs::differential_class_header_colon_inside_unclosed_paren_is_invalid_syntax`
    - `tests/differential_cpython.rs::differential_function_header_colon_inside_unclosed_paren_is_invalid_syntax`
    - `tests/differential_cpython.rs::differential_open_bracket_with_colon_is_invalid_syntax`.
    - `tests/differential_cpython.rs::differential_pyc_traceback_identifier_caret_span_matches_cpython`.
    - `tests/differential_cpython.rs::differential_pyc_traceback_context_chain_matches_cpython_shape`.
    - `tests/differential_cpython.rs::differential_pyc_traceback_suppressed_context_matches_cpython_shape`.
    - `tests/differential_cpython.rs::differential_pyc_traceback_direct_cause_matches_cpython_shape`.
    - `tests/differential_cpython.rs::differential_pyc_traceback_mixed_cause_and_context_chain_matches_cpython_shape`.
    - `tests/differential_cpython.rs::differential_traceback_reraise_preserves_original_fault_line`.
    - `tests/differential_cpython.rs::differential_pyc_traceback_reraise_preserves_original_fault_line`.
    - `tests/differential_cpython.rs::differential_traceback_raise_exc_keeps_original_traceback_chain`.
    - `tests/differential_cpython.rs::differential_pyc_traceback_raise_exc_keeps_original_traceback_chain`.
    - `tests/differential_cpython.rs::differential_traceback_with_traceback_restores_supplied_chain`.
    - `tests/differential_cpython.rs::differential_pyc_traceback_with_traceback_restores_supplied_chain`.
    - `tests/differential_cpython.rs::differential_semantic_syntax_return_outside_function_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_break_outside_loop_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_continue_outside_loop_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_await_outside_function_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_yield_outside_function_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_yield_from_outside_function_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_return_with_value_in_async_generator_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_global_used_prior_declaration_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_global_assigned_prior_declaration_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_nonlocal_at_module_level_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_nonlocal_without_binding_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_global_used_prior_declaration_file_caret_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_parameter_and_global_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_parameter_and_nonlocal_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_nonlocal_and_global_conflict_global_first_matches_cpython`
    - `tests/differential_cpython.rs::differential_semantic_syntax_nonlocal_and_global_conflict_nonlocal_first_matches_cpython`.
- VM protocol-dispatch checkpoint (2026-02-23, latest):
  - `LOAD_SPECIAL` now probes CPython-proxy attributes even when `class_of_value(...)` is unavailable for the receiver.
  - memoryview context-manager methods are now bound in special-method lookup for both native memoryview values and CPython-proxy memoryview receivers.
  - `IMPORT_FROM` now re-canonicalizes to `sys.modules[requested_name]` and retries attribute lookup on missing-attribute misses.
    - closes stale-module alias edge cases where module execution rewires `sys.modules` (e.g. `decimal` -> `_pydecimal`) during import.
  - `import *` now resolves explicit `__all__` entries through module attribute lookup instead of silently skipping missing globals.
    - missing `__all__` names now raise `AttributeError` (CPython parity) instead of incorrectly succeeding.
  - closed regressions:
    - `tests/vm.rs::pickle_zero_copy_bytearray_roundtrips_across_protocols`
    - `tests/vm.rs::pickle_zero_copy_bytes_oob_buffers_preserve_identity`.
    - `tests/vm.rs::statistics_mean_supports_basic_int_dataset`.
    - `tests/vm.rs::from_import_reads_attribute_from_replaced_sys_modules_entry`
    - `tests/vm.rs::from_import_star_raises_when_all_contains_missing_name`.
    - `tests/differential_cpython.rs::differential_from_import_reads_attribute_from_replaced_sys_modules_entry`
    - `tests/differential_cpython.rs::differential_from_import_star_missing_all_entry_raises_attribute_error`.
- C-API no-op closure checkpoint (2026-02-22, latest):
  - Batch 1 from `docs/CAPI_NOOP_EXECUTION_ORDER.md` is closed:
    - `PyGILState_{Ensure,Release,GetThisThreadState}`,
    - `PyEval_{SaveThread,RestoreThread,AcquireThread,ReleaseThread,AcquireLock,ReleaseLock,InitThreads,ThreadsInitialized}`,
    - `PyMutex_{Lock,Unlock}`,
    - `PyThread_init_thread`, `PyThread_ReInitTLS`.
  - C-API compatibility header now declares `PyEval_SaveThread`, `PyEval_RestoreThread`, and `PyMutex` APIs (`include/pyrs_cpython_compat.h`).
  - extension smoke regressions were upgraded to verify semantics (not symbol presence only):
    - `tests/extension_smoke.rs::cpython_compat_eval_abi_batch27_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_gilstate_abi_batch30_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_thread_abi_batch53_apis_work`.
  - Batch 2 from `docs/CAPI_NOOP_EXECUTION_ORDER.md` is closed:
    - `PyErr_CheckSignals`,
    - `Py_EnterRecursiveCall`, `Py_LeaveRecursiveCall`, `_Py_CheckRecursiveCall`.
  - signal and recursion parity updates:
    - `PyErr_SetInterrupt` / `PyErr_SetInterruptEx` now post pending-interrupt state;
      `PyErr_CheckSignals` consumes that state and raises typed `KeyboardInterrupt` on demand.
    - recursion-control APIs match CPython 3.14 non-overflow behavior in supported runtime paths
      (successful/no-op calls under normal stack conditions).
  - extension smoke regressions were updated for this closure:
    - `tests/extension_smoke.rs::cpython_compat_error_abi_batch21_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_internal_ref_gc_abi_batch68_apis_work`.
  - Batch 3 from `docs/CAPI_NOOP_EXECUTION_ORDER.md` is closed:
    - `PyObject_GC_Track`, `PyObject_GC_UnTrack`, `PyObject_GC_IsFinalized`,
      `PyObject_ClearWeakRefs`.
  - GC/weakref lifecycle parity updates:
    - VM-global C-API registry now tracks GC track/untrack overrides and pointer finalized state.
    - `PyObject_ClearWeakRefs` now enforces compat-owned dealloc precondition (`refcnt == 0`),
      clears runtime weakrefs for target objects, and marks finalized state for C-API visibility.
    - runtime weakref wrappers now honor explicit weakref-clear state in addition to finalized
      `__del__` state when resolving `weakref.ref()` targets.
  - extension smoke regressions were updated for this closure:
    - `tests/extension_smoke.rs::cpython_compat_gc_weakref_lifecycle_abi_batch70_apis_work`.
  - C-API compatibility header now declares GC lifecycle and weakref-clear exports used by
    extension probes:
    - `PyObject_GC_Track`, `PyObject_GC_UnTrack`, `PyObject_GC_IsTracked`,
      `PyObject_GC_IsFinalized`, `PyObject_ClearWeakRefs`.
  - Batch 4 from `docs/CAPI_NOOP_EXECUTION_ORDER.md` is closed:
    - `PyType_Modified`, `PyType_ClearCache`.
  - type-cache coherence parity updates:
    - VM now exposes explicit type-cache invalidation paths:
      - global invalidation (`clear_all_type_caches`),
      - per-type + subtype invalidation (`invalidate_type_cache_for_class_id`).
    - `PyType_Modified` now refreshes class attrs from `tp_dict` for mapped type objects and
      invalidates runtime class/inline caches for affected type hierarchies.
    - `PyType_ClearCache` now performs global type-cache invalidation and returns a monotonic
      non-zero version tag.
  - extension smoke regressions were updated for this closure:
    - `tests/extension_smoke.rs::cpython_compat_type_cache_coherence_abi_batch71_apis_work`.
  - Batch 5 from `docs/CAPI_NOOP_EXECUTION_ORDER.md` is closed:
    - `Py_NewInterpreter`, `Py_EndInterpreter`.
  - interpreter-lifecycle parity updates:
    - `Py_NewInterpreter` now creates a distinct interpreter state + thread state and swaps
      current thread state to the new subinterpreter state.
    - `Py_EndInterpreter` now enforces current-thread-state precondition, tears down all thread
      states bound to the target interpreter, and then deletes interpreter state.
    - `PyInterpreterState_Get` and `PyThreadState_GetInterpreter` now return interpreter pointers
      derived from thread-state ownership instead of always returning main-interpreter token.
  - extension smoke regressions were updated for this closure:
    - `tests/extension_smoke.rs::cpython_compat_interpreter_lifecycle_abi_batch72_apis_work`.
  - Batch 6 from `docs/CAPI_NOOP_EXECUTION_ORDER.md` is closed:
    - `PyTraceMalloc_Track`, `PyTraceMalloc_Untrack`,
    - `PyUnstable_Object_IsUniquelyReferenced`,
    - `PyUnstable_Object_IsUniqueReferencedTemporary`,
    - `PyUnstable_Object_EnableDeferredRefcount`.
  - observability/unstable API parity updates:
    - `PyTraceMalloc_Track`/`PyTraceMalloc_Untrack` now maintain a VM process-level trace table
      keyed by `(domain, ptr)` with update+idempotent-untrack semantics.
      - current policy: this substrate is deterministic and always-on; it is not yet coupled to
        stdlib `tracemalloc.start/stop` toggles.
    - `PyUnstable_Object_IsUniquelyReferenced` now resolves refcount-based uniqueness while
      suppressing effectively-immortal runtime values.
    - `PyUnstable_Object_IsUniqueReferencedTemporary` now gates unique-ref checks on top-frame
      presence for object identity handles.
    - `PyUnstable_Object_EnableDeferredRefcount` now follows explicit GIL-mode policy:
      deterministic no-op (`0`) in current runtime mode.
  - extension smoke regressions were updated for this closure:
    - `tests/extension_smoke.rs::cpython_compat_observability_unstable_abi_batch73_apis_work`.
- C-API lifetime-model checkpoint (2026-02-22, latest):
  - owned-pointer free path now clears active thread-state exception pointers when they
    alias the freed pointer (`current_exception`, `exc_info.exc_value`, `exc_state.exc_value`).
  - escaped compat-object pinning now preserves owned `ob_type` children to prevent freed
    type-pointer reuse across C-API contexts.
  - `_thread` lock substrate now reuses a heap-cached lock class instead of allocating a new
    class object per lock instance, removing stale lock-type pointer churn.
  - NumPy random stability/parity recovery:
    - `np.random.default_rng().random()` succeeds,
    - `rng.integers(0, 2, size=4)` and `rng.integers(0, 2, 4)` both succeed,
    - `callable(np.random.default_rng().integers)` is `True`.
  - latest scientific-stack gate status (`perf/numpy_gate_direct_latest.json`):
    - `scipy_import`: `PASS`,
    - `pandas_import` / `pandas_series_sum`: `FAIL` (`pandas._libs.tslibs.dtypes`: `type 'type' has no attribute 'keys'`),
    - `matplotlib_import` / `matplotlib_pyplot_smoke`: `FAIL` (`TypeError: attempted to call non-function: û`).
- C-API lifetime-model checkpoint (2026-02-20, latest):
  - VM-global pointer registry landed in `src/vm/capi_registry.rs` (provenance/lifecycle/ref-kind tracking).
  - registry is now wired into core compat allocation and teardown paths (`src/vm/vm_extensions.rs`, `src/vm/mod.rs`), including external-pin accounting and pending/free state transitions.
  - high-traffic proxy/callable call-result conversions now use owned-reference mapping; call/attr/vectorcall argument conversions use borrowed-reference mapping.
  - new NumPy lifetime stress regressions landed in `tests/vm.rs`:
    - `numpy_axis_sum_and_repr_stress_stays_stable`
    - `numpy_repeated_array_ops_and_reprs_stay_stable`
  - removed VM-side legacy external-pin/freed-allocation sets and moved that state handling into the VM-global CAPI registry.
  - CI now includes a nightly ASan lifetime lane (`sanitizer-stability` job in `.github/workflows/parity-gate.yml`).
- C-API lifetime-model checkpoint (2026-02-21, latest):
  - fixed deterministic post-NumPy teardown aborts (`SIGABRT` / pointer-not-allocated) rooted in stale owned-pointer state across realloc/free paths.
  - list-buffer `realloc` now migrates owned-pointer + registry pin state when the buffer address changes.
  - context-drop free paths now remove compat/list-buffer/aux pointers from owned-pointer sets before free to prevent stale ownership reuse on recycled addresses.
  - removed `ModuleCapiContext` legacy owned-pointer shadow set (`cpython_owned_ptrs`); ownership authority now resolves through VM-global registry (`capi_ptr_is_owned_compat`) with context vectors used only for local pointer discovery.
  - proxy materialization now checks VM-global registry liveness first (`capi_registry_contains_live_or_pending`), reducing correctness dependence on pointer-probability heuristics after initial pointer registration.
  - owned-pointer free transitions are now centralized in context helpers (`capi_owned_ptr_prepare_for_free`, `capi_owned_ptr_mark_freed`) and used by teardown + frame-release paths.
  - owned-pointer checks are now provenance-specific (`OwnedCompat` only), so externally pinned proxy pointers are no longer misclassified as owned pointers in iterator paths.
  - NumPy proxy iterability + repr parity recovered:
    - `iter(np.arange(...))` works in direct mode again.
    - `repr(np.arange(0, 10, 0.5))` now returns arrayprint output rather than `<numpy.ndarray object at ...>`.
  - stress regressions are stable again:
    - `numpy_repeated_axis_sum_remains_stable_across_calls` passes repeatedly,
    - `numpy_axis_sum_and_repr_stress_stays_stable` passes.
    - `numpy_float_ndarray_repr_does_not_fall_back_to_instance_placeholder` now passes.
    - `numpy_ndarray_proxy_iterability_is_preserved` passes.
    - `numpy_arrayprint_array_repr_works_without_placeholder_fallback` passes.
- Scientific-stack checkpoint (2026-02-21, latest):
  - `import numpy.random` now succeeds in direct mode.
  - direct `numpy.random.MT19937()` construction now succeeds.
  - `numpy.random.default_rng()` construction now succeeds again in direct mode.
  - active blocker root-cause lane:
    - CPython 3.14 fast-thread-state exception-indicator synchronization (`PyThreadState.current_exception` parity across `PyErr_*` flows).
  - root-cause closures landed:
    - `PyType_Ready` now installs a class-level `__init__` slot wrapper for extension types when `tp_init` exists and `__init__` is absent from `tp_dict`.
    - method-descriptor `tp_call` now handles unbound `METH_METHOD` receiver shapes where the defining class is passed before the explicit instance argument.
    - transient proxy-attribute soft-miss paths now clear temporary C-API error state before context teardown (`load_cpython_proxy_attr_for_value`), preventing stale nested-context exception pointers from surviving into later `PyErr_*` checks.
    - nested-context error propagation now rematerializes into parent-owned pointers only (no raw child-context pointer passthrough), preserving exception type where possible and preventing cross-context stale-pointer reuse.
    - null-result-with-active-exception path now publishes typed exception state (`set_error_state`) instead of collapsing to message-only `RuntimeError` in descriptor call paths.
  - new regression coverage:
    - `tests/vm.rs::numpy_random_mt19937_initializer_runs_without_seedsequence_failures`.
    - `tests/vm.rs::numpy_axis_error_does_not_poison_followup_top_level_execute` now re-validated as passing with typed `AxisError` behavior preserved.
  - remaining follow-up:
    - continue generator/method long-tail closure after lifetime model hardening.
- NumPy import perf + pyc checkpoint (2026-02-21, latest round):
  - import default now matches CPython policy for source+bytecode modules:
    - prefer validated source-bound `.pyc` by default;
    - can be overridden via `PYRS_IMPORT_PREFER_PYC=0`.
  - fixed CPython bytecode translation parity bug for `StoreFastStoreFast` operand order (`src/vm/vm_execution.rs`), restoring pyc-path correctness on affected modules.
  - pyc import resolution now supports cache-only `__pycache__/...cpython-314.pyc` module/package imports without source files (`src/vm/vm_bootstrap_import.rs`), with passing regressions:
    - `imports_module_from_cached_pyc_without_source_file`
    - `imports_package_from_cached_pyc_without_source_file`
  - `PyList_GetItemRef` now has an owned-list raw-buffer fast path with correct CPython index contract and new-ref behavior (`src/vm/vm_extensions/cpython_list_api.rs`), reducing direct NumPy import overhead.
  - C-API sync/conversion hot paths were tightened:
    - `sync_value_from_cpython_storage` now avoids full list/tuple cloning unless fallback slots are actually needed (`src/vm/vm_extensions.rs`),
    - interned-unicode pointer checks now avoid unnecessary string clone lookups in rich-compare pointer gating (`src/vm/vm_extensions/cpython_object_item_compare_api.rs`).
  - VM env-flag checks in `vm_execution` now use a cached lookup helper (`env_var_present_cached`) instead of repeated raw environment probes on hot paths (`src/vm/mod.rs`, `src/vm/vm_execution.rs`).
  - measured release import baseline improved from ~`1.65s` to ~`0.62-0.65s` user-time for:
    - `target/release/pyrs -S -c "import sys; sys.path.insert(0, './.venv-ext314/lib/python3.14/site-packages'); import numpy as np"`
  - pyc-first status:
    - the previously observed `AttributeError: str has no attribute 'value'` repro was caused by a stale local cache-only `__pycache__/enum.cpython-314.pyc` shadowing stdlib enum in the workspace root.
    - with that stale local cache removed, the minimal enum pyc repro now follows CPython behavior; remaining pyc work is long-tail parity/perf closure.
    - source-bound pyc imports now preserve CPython-style metadata:
      - `__file__` / `__spec__.origin` resolve to source `.py` when available,
      - `__cached__` / `__spec__.cached` resolve to the loaded `.pyc` path.
    - vm regression `tests/vm.rs::cpython_enum_path_supports_member_value_and_name` now runs import/compile execution on a large-stack worker thread to avoid Rust test-harness thread stack overflow during deep stdlib import recursion.
  - pyc compatibility closure (latest):
    - marshal loader now accepts `TYPE_ELLIPSIS ('.')`, `TYPE_STOPITER ('S')`, and arbitrary-size `TYPE_LONG ('l')` constants (decoded to `BigInt` when needed).
    - pyc constant translation now supports bytes constants.
    - translation now maps `DELETE_ATTR`, `LOAD_FROM_DICT_OR_DEREF`, `CALL_INTRINSIC_2`, `MATCH_CLASS`, `MATCH_KEYS`, `MATCH_MAPPING`, `MATCH_SEQUENCE`, `GET_LEN`, `BUILD_TEMPLATE`, and `BUILD_INTERPOLATION`.
    - runtime now handles:
      - `CALL_INTRINSIC_1`: `3/7/8/9/10/11` (in addition to existing `2/5/6`)
      - `CALL_INTRINSIC_2`: `1/2/3/5` (in addition to existing `4`)
    - new regression: `tests/pyc_exec.rs::executes_cpython_pyc_with_bytes_bigint_ellipsis_and_delete_attr`.
    - new regression: `tests/pyc_translate.rs::translates_call_intrinsic_2`.
    - new regression: `tests/pyc_exec.rs::executes_cpython_pyc_with_match_class_mapping_and_sequence`.
    - new regression: `tests/pyc_translate.rs::translates_get_len_and_build_template`.
    - coroutine materialization parity fix landed for translated pyc function calls:
      - coroutine/async-generator code objects now allocate generator/coroutine state on call (not only `code.is_generator`),
      - fixes `_collections_abc` pyc path failure (`_coro.close()` on `None`),
      - new regression: `tests/pyc_exec.rs::executes_cpython_pyc_async_def_returns_coroutine_object`.
    - `BUILD_SLICE` parity fix landed for translated pyc execution:
      - runtime now honors operand arity (`arg=2` vs `arg=3`) and no longer pops a bogus step value on two-operand slices,
      - fixes pyc failure on `del x[-2:]`/slice-delete paths used by `re` imports,
      - new regressions:
        - `tests/pyc_exec.rs::executes_cpython_pyc_build_slice_with_two_and_three_operands`
        - `tests/pyc_translate.rs::translates_build_slice_with_two_operands`
    - pyc-only import check for `typing` + `annotationlib` now succeeds (manual direct run from cache-only path).
    - NumPy import graph counters improved to:
      - `source_compiles=1`, `pyc_attempts=111`, `pyc_fallbacks=0` (was `30/111/29`).
- VM error-model closure checkpoint (2026-02-20, latest):
  - removed VM-control-flow string classification in `src/vm/mod.rs`:
    - `runtime_error_matches_exception(...)` is typed/subclass-only,
    - `runtime_error_to_exception_object(...)` no longer re-classifies freeform text.
  - compatibility classification is now centralized at error-construction time in `RuntimeError::new(...)` (`src/runtime/mod.rs`), including:
    - traceback/prefix exception extraction,
    - legacy message-to-type inference for compatibility,
    - import (`msg`/`name`/`path`) and OS (`errno`/`strerror`) default attrs.
  - sqlite `kind: message` payloads now become typed exceptions again through constructor-time extraction without VM-level string matching.
  - `_io` module now publishes private CPython alias classes (`_IOBase`, `_RawIOBase`, `_BufferedIOBase`, `_TextIOBase`) during IO hierarchy wiring.
  - coverage checkpoints in this slice:
    - `cargo test -q --test vm typed` passes,
    - key regressions revalidated (`builtin_function_names_are_stable_and_pickle_roundtrips_functions`, `from_import_missing_name_raises_importerror`, `sqlite3_connection_call_on_closed_db_raises_programming_error`),
    - full `--test vm` fail count reduced to `54` in this tree vs baseline `120` on detached `HEAD` snapshot (used as regression baseline during refactor).
- VM error-model refactor checkpoint (2026-02-20):
  - `RuntimeError` now carries optional typed exception payload (`exception: Option<Box<ExceptionObject>>`).
  - `RuntimeError::new(...)` now auto-extracts exception type/message from prefixed and traceback-tail messages.
  - `runtime_error_matches_exception(...)` now matches typed payloads first, with legacy message fallback.
  - VM runtime-error conversion is centralized via `runtime_error_to_exception_object(...)` + `ensure_exception_default_attrs(...)`.
  - `runtime_error_from_active_exception(...)` now preserves the original `ExceptionObject` while keeping traceback text compatibility.
  - explicit `raise ... from ...` now preserves `__context__` in addition to `__cause__` + `__suppress_context__`.
  - prefixed `RuntimeError::new("XError: ...")` callsites were bulk-migrated to typed constructors (136 sites); fallback classifier is now a compatibility lane, not the primary path.
  - `runtime_error_matches_exception(...)` no longer uses classifier fallback; matching is now typed/structured, and attribute lookup surfaces were updated to raise typed `AttributeError` instead of untyped message-only errors in key VM paths.
  - len/generator parity updates: non-int `__len__` now raises typed `TypeError`, negative `__len__` raises typed `ValueError`, and generator re-entrancy now raises typed `ValueError` (`generator already executing`) with typed `TypeError` for non-generator/invalid initial send paths.
  - await/yield-from protocol paths now emit typed exceptions in `vm_native_dispatch` (`TypeError` for non-awaitable/non-iterable/non-iterator contract failures; typed `RuntimeError` for `generator ignored GeneratorExit`).
  - new regression tests landed for these protocol error contracts (`yield from` non-iterable, non-awaitable `await`, and non-iterator `__await__` return paths) in `tests/vm.rs`.
  - additional high-frequency message-only error producers were migrated to typed constructors across core runtime/ops/dispatch paths (`TypeError`/`IndexError`/`ValueError`/`OverflowError`/`StopIteration`), reducing classifier dependence in hot VM flows.
  - added focused typed-exception regressions for constructor contract failures, membership/index contract errors, and bytes/bytearray range validation.
  - codec unknown-error-handler paths now raise typed `LookupError` with CPython-style message (`unknown error handler name '<name>'`) across normalize/encode/decode flows.
  - bad-fd paths now use typed `OSError` with structured `errno`/`strerror` attrs via `RuntimeError::os_error_with_errno(...)` and `RuntimeError::bad_file_descriptor()`.
  - additional typed-constructor closure landed for core contract errors:
    - `memoryview()` bytes-like contract, `type()` base-shape/base-class validation, and `set()` arity now raise typed `TypeError`.
    - IO/iterator method arity (`__iter__`/`__next__`/`close`/`write`) now raises typed `TypeError`.
    - closed-file IO paths now raise typed `ValueError` (`I/O operation on closed file.`).
  - new regressions cover these contracts (`constructor_contract_errors_for_memoryview_type_and_set_are_typed`, `io_method_arity_and_closed_file_contracts_are_typed`).
  - `memoryview.cast(...)` argument-clinic parity improved: duplicate positional+keyword arg errors now use CPython-style TypeError text (`argument for cast() given by name ... and position ...`) and over-arity now reports dynamic counts.
  - json/parse conversion paths now emit typed `ValueError` for invalid number/unicode/utf8 payloads; range duplicate-arg/unpack/file-object contract failures now emit typed `TypeError`; unsupported encoding normalization now emits typed `LookupError`.
  - csv unknown-dialect paths now raise explicit typed `Error` exception objects instead of message-only classifier dependency.
  - additional regressions landed for this wave:
    - `range_duplicate_argument_error_is_typed`
    - `unpack_non_iterable_error_is_typed`
    - `float_invalid_literal_error_is_typed_value_error`
    - `csv_unknown_dialect_error_is_typed_error`
  - remaining message-only hotspots are now concentrated in VM-internal diagnostics and specialized memoryview-format long-tail buckets (tracked for subsequent conversion slices).
  - follow-up contract conversions now typed `TypeError` for:
    - `type()` first-argument contract,
    - enumerate iterable contract,
    - path/pattern/name str-bytes contracts,
    - `setstate()` arity,
    - `tuple.count/index` receiver+arity contracts,
    - `__mro_entries__` non-tuple return contract.
  - new regressions landed: `regex_pattern_type_contract_errors_are_typed` and `mro_entries_non_tuple_contract_error_is_typed`.
  - memoryview unsupported-format paths now raise typed `NotImplementedError`:
    - format-spec decode path (`memoryview: format <fmt> not supported`),
    - `tolist()` unsupported-format path (`memoryview: unsupported format`).
  - regression landed: `memoryview_tolist_unsupported_format_raises_not_implemented`.
  - latest typed-contract closure wave (2026-02-20, later):
    - `range(...)` zero-arg/keyword contracts now raise typed `TypeError` with CPython-style messages where applicable; step-zero remains typed `ValueError`.
    - VM Python-function argument binding now raises typed `TypeError` for unexpected keywords and duplicate argument binding (`got multiple values for argument '<name>'`) instead of generic runtime errors.
    - fallback `_random` `randrange(start, stop=None, step=1)` binding now follows CPython parameter semantics (duplicate/missing-arg/step-zero typed behavior).
    - regressions landed: `range_error_contracts_are_typed`, `randrange_duplicate_and_empty_range_contracts_are_typed`.
  - random contract typing closure (2026-02-20, latest):
    - `seed/random/randint/getrandbits/choice/choices/shuffle` signature/argument/domain errors now emit typed exceptions (TypeError/ValueError/IndexError) instead of message-only runtime errors.
    - random keyword-argument contract failures now include offending keyword names in TypeError text.
    - regressions landed: `random_empty_population_contracts_are_typed`, `random_argument_contracts_are_typed`.
  - typed core-contract cleanup (2026-02-20, latest):
    - `ord`, `dict`, `all/any`, `namedtuple._make`, and `divmod` contract/domain failures now emit typed exceptions (`TypeError`/`ValueError`/`ZeroDivisionError`) instead of untyped message-only runtime errors.
    - regression landed: `core_contract_errors_are_typed_for_ord_dict_all_divmod_and_namedtuple_make`.
  - IO contract typing cleanup (2026-02-20, latest):
    - `io.open(...)` and `FileIO.__init__(...)` argument/keyword/type contracts now emit typed `TypeError`/`ValueError` and bad-fd paths now emit typed `OSError` (`bad file descriptor`).
    - opener-callback failure path in `io.open(..., opener=...)` now preserves active exception objects instead of replacing with generic message-only runtime errors.
    - `IncrementalNewlineDecoder.__init__/decode` argument and decoder-contract errors now emit typed `TypeError` (including unexpected keyword + duplicate arg paths).
    - regressions landed: `io_open_contract_errors_are_typed`, `io_fileio_contract_errors_are_typed`, `_io_incremental_newline_decoder_contract_errors_are_typed`.
- Scientific-stack closure checkpoint (2026-02-19):
  - import-state root-cause fix:
    - source/pyc module execution now sets an internal module-initializing marker and clears it on successful frame completion.
    - module frames that unwind with unhandled exceptions now clear the marker and remove failed source/sourceless modules from `sys.modules`.
    - this closes the prior NumPy-side corruption where `ctypes` remained partially initialized after a failed import path.
  - current scientific-stack blocker shift:
    - SciPy is no longer blocked by partial `ctypes`; direct-mode failure now surfaces in `_ccallback_c` with `type 'type' has no attribute 'register'`.
  - landed substrate fixes this round:
    - builtin-attribute write/delete support for `Value::Builtin` targets (`StoreAttr`/`setattr`/`delattr` paths),
    - namedtuple runtime parity improvements (defaults + subclass `super().__new__(cls, ...)` path),
    - `set.remove` method surface + `KeyError` miss behavior,
    - `property()` keyword-argument support (`fget`/`fset`/`fdel`/`doc`),
    - `inspect.getdoc` bootstrap surface,
    - slice-assignment fallback to `__setitem__` for instance/proxy targets,
    - `PyBytes_Resize`/`_PyBytes_Resize` and `PyGen_Type` symbol/export closure.
  - current active blocker:
    - `import matplotlib` still exits with `SIGSEGV (139)` in direct mode.
    - tracing points to repeated `_multiarray_umath.add_docstring` traffic before failure; keep focus on Lane B root-cause closure (no shim patching).
- Scientific-stack closure checkpoint (2026-02-20):
  - removed bootstrap `abc` module shim (`ABCMeta = type`) so stdlib now resolves through CPython `Lib/abc.py` with native `_abc` substrate.
  - fixed metaclass descriptor binding in `tp_getattro` metatype paths so calls like `Sequence.register(...)` bind `cls` correctly.
  - relaxed proxy pointer gating for transient Cython metatype pointers (`ob_refcnt == 0`) only when their metatype chain still validates as a real `type` lineage.
  - SciPy blocker moved forward from `type 'type' has no attribute 'register'` to `_ctypes` substrate absence:
    - `ModuleNotFoundError: module '_ctypes' not found` during `scipy._lib._ccallback_c` init.
- Scientific-stack closure checkpoint (2026-02-20, later round):
  - `PyFrame_New` no longer materializes a generic module-layout object; it now allocates a frame-compatible C layout (`CpythonFrameCompatObject`) with valid `f_lineno` offset used by Cython traceback paths.
  - frame-handle refcount zero path now explicitly releases frame-owned refs (`f_code`/`f_globals`/`f_locals`) and frees frame allocations eagerly, preventing prior allocator-corruption/trap behavior in deep extension init.
  - direct `import pandas._libs._cyutility` no longer aborts with allocator trap; blocker shifted to deterministic `numpy.random.mtrand` `Py_mod_exec` failure:
    - `expected a sequence of integers or a single integer, got '<class 'numpy.uint32'>'`.
  - `PyNumber_Long` now raises typed `TypeError` (not generic runtime error) on non-int-compatible inputs in slot fallback paths; this preserves CPython exception-class semantics during extension error handling.
- NumPy import warning cleanup (2026-02-17):
  - fixed `PyInterpreterState_Main`/`PyThreadState_Get()->interp` baseline so NumPy no longer reports the sub-interpreter warning during import.
  - narrowed `types.MethodType` detection to Python-bound methods (not native bound methods), removing the prior `add_newdoc` warning flood for extension callables.
  - `_contextvars.ContextVar` now uses dict-backed marker storage compatible with native `get`/`set`/`reset` dispatch, unblocking NumPy print-options contextvar reads.
- REPL responsiveness (2026-02-17):
  - completion-state refresh now skips single-expression submissions, reducing post-expression prompt latency after large imports (e.g. NumPy).
  - completion graph building now skips CPython proxy class/instance expansion, avoiding deep recursive symbol-walk overhead after scientific-stack imports.
- Scientific-stack native-extension bring-up (2026-02-18):
  - `PyType_Ready` method-table population now uses descriptor construction (`PyDescr_NewMethod` / `PyDescr_NewClassMethod`) instead of short-lived cfunction wrappers.
  - thread-state compat now initializes a CPython-style exception-stack chain at the offset used by `PyThreadState_GetUnchecked` Cython call-sites (`tstate + 0x78`), removing prior `_cyutility` crash paths.
  - extension loader now reconciles module-instance mismatch returns from `PyInit_*` by syncing module globals/registry instead of failing with `returned unexpected module instance`.
  - capsule validity/name handling now includes external-capsule fallback for raw CPython capsule objects (`PyCapsule_GetName` / `PyCapsule_IsValid`) so Cython capsule-signature checks no longer fail with `got (null)`.
  - `include/pyrs_cpython_compat.h` now declares `PyLong_FromSsize_t` and `PyLong_AsSsize_t` explicitly to avoid implicit-declaration ABI drift in extension builds.
  - direct NumPy baseline is now green in direct mode (`perf/numpy_gate_direct_latest.json`):
    - `numpy_import`: `PASS`
    - `numpy_ndarray_sum`: `PASS`
    - `numpy_numerictypes_core`: `PASS`
    - `np.arange(0, 10, 0.5)` now renders as `array([...])` instead of proxy placeholders.
  - latest optional scientific-stack blockers are now:
    - `scipy_import`: `AttributeError: module 'ctypes' has no attribute 'CFUNCTYPE'`.
    - `pandas_import` / `pandas_series_sum`: `numpy.random.mtrand` `Py_mod_exec` runtime error
      (`index 4 is out of bounds for axis 0 with size 4`).
    - `matplotlib_import` / `matplotlib_pyplot_smoke`: missing symbol `PyInstanceMethod_Type`.
  - additional NumPy P0 blocker:
    - `import numpy.random` now fails deterministically with
      `TypeError: ... PyInit_mtrand ... attempted to call non-function` (process exit `2`),
      no longer an allocator-trap abort.
    - traced failure point is `_pickle.py` line 7 (`ImportNameCpython`) while importing
      `numpy.random.mtrand`; active root-cause direction is `ImportNameCpython` +
      pending-import semantics in extension-init recursion.
  - scientific-stack update (2026-02-19):
    - CPython active-context switching now uses RAII (`ActiveCpythonContextGuard`) across
      proxy/callable/loader/object-call paths, with nested-context error-message propagation.
    - this removed the prior `SystemError: NULL result without error in generate_state()` mask;
      traced `numpy.random.mtrand` init now reports the underlying error:
      `index 4 is out of bounds for axis 0 with size 4`.
    - current gate blockers:
      - `scipy_import`: `AttributeError: module 'ctypes' has no attribute 'CFUNCTYPE'`.
      - `pandas_import` / `pandas_series_sum`: `numpy.random.mtrand` `Py_mod_exec` runtime
        error (`index 4 is out of bounds for axis 0 with size 4`).
      - `matplotlib_import` / `matplotlib_pyplot_smoke`: missing symbol `PyInstanceMethod_Type`.
  - root-cause closures landed in this slice:
    - `PyUnicode_AsUTF8` now returns stable UTF-8 pointers from a process registry
      (instead of per-call scratch storage), closing a concrete NumPy `arr_add_docstring`
      use-after-free path.
    - type-object attr lookup now treats metatype-backed type objects as type objects (not metatype-only), unblocking `numpy.dtype` class attrs like `alignment`.
    - proxy type-object rich-compare dunder fallback (`__lt__/__le__/__eq__/__ne__/__gt__/__ge__`) now materializes callable wrappers from `tp_richcompare`, unblocking `numpy.dtype.__ge__` class-attr probes.
    - ndarray pretty-print path now runs only for ndarray instances (not proxy classes), fixing `print(type(np.arange(...)))` runtime failures.
    - `PyObject_GetBuffer` now recovers mapped handles for proxy-backed pointers and falls back to external `tp_as_buffer.bf_getbuffer` slots when internal pyrs buffer storage is not applicable.
    - `PyBuffer_Release` no longer assumes `Py_buffer.internal` is pyrs-owned; foreign exporter internals are left untouched, and pyrs-owned internals are tracked/released explicitly via context-owned pointer bookkeeping.
  - closure landed this round:
    - `PyType_FromSpec*` base-resolution now correctly handles `bases` tuples containing `Builtin(Type)` so Cython metatype construction no longer defaults to `object`.
    - this removed the earlier random-stack gate `PyDescr_NewMethod expected type object` / shared-Cython-type `PyType_Check` failure.
    - foreign `PyLong` compact payload decoding now matches CPython 3.14 (`Include/cpython/longintrepr.h`) and no longer mis-decodes compact integer payloads:
      - `np.dtype('int64').itemsize` now resolves to `8` (was regressed to `0`),
      - `np.iinfo(np.int64).bits` now resolves to `64` (was regressed to `0`).
- Top-stdlib common-usecase gate: `26/26` import, `26/26` smoke.
- Extended stdlib probe: `50/50` import, `50/50` smoke (`perf/stdlib_compat_extended_latest.json`).
- Extension scaffolding checkpoint:
  - extension manifest parser + suffix baseline (`.pyrs-ext`, ABI `pyrs314`) is landed (`src/extensions/mod.rs`).
  - import loader now recognizes extension manifests and direct shared objects (`.so`/`.dylib`/`.pyd`), including tagged CPython-style filenames (`module.cpython-314-*.so`), and executes them through `pyrs.ExtensionFileLoader`.
  - direct shared-object import now emits explicit unsupported diagnostics when only CPython-style `PyInit_*` symbols are present (no silent/ambiguous symbol-miss failures).
  - extension submodule fallback now propagates nested import failures (instead of silently collapsing to generic `cannot import name` in `from pkg import submodule` flows), so NumPy gate failures surface explicit ABI-mode mismatch diagnostics.
  - loaded native modules now publish symbol diagnostics metadata (`__pyrs_extension_expected_symbol__`, `__pyrs_extension_symbol_family__`) for ABI-mode visibility.
  - v1 extension C-API header slice is landed (`include/pyrs_capi.h`; contract in `docs/EXTENSION_CAPI_V1.md`) and now includes module setters, native callable registration (`module_add_function`, `module_add_function_kw`), init-scoped object handles + type/getter introspection (`object_new_*`, `module_set_object`, `object_incref/decref`, `object_type`, `object_get_*`), and import-time error state (`error_set/clear/occurred`).
  - C-API v1 callback surface is now isolated in `src/vm/vm_extensions/capi_v1.rs` (instead of being embedded in the main `vm_extensions.rs` monolith) to keep extension-callable API review and ownership bounded.
  - CPython proxy runtime surface is now isolated in `src/vm/vm_extensions/proxy_runtime.rs` (`call`, attr lookup, iter/getitem/setitem, numeric proxy ops) to reduce `vm_extensions.rs` size and improve reviewability.
  - Extension callable registration + invocation runtime is now isolated in `src/vm/vm_extensions/callable_runtime.rs` (`register_extension_callable`, `call_extension_callable`) to keep callback dispatch ownership bounded.
  - Extension loader execution runtime is now isolated in `src/vm/vm_extensions/loader_runtime.rs` (`exec_extension_module`, dynamic `PyInit_*`/slot execution, extension metadata publication) so loader/ABI init paths are reviewable outside the main monolith.
  - Module C-API context state/capsule lifecycle helpers are now isolated in `src/vm/vm_extensions/module_context_state.rs` (`module_set_state`, `module_get_state`, `module_set_finalize`, `module_set_attr`/`del_attr`/`has_attr`, capsule export sync) to keep module-state ownership out of the main monolith.
  - CPython active-context/pointer bridge helpers are now isolated in `src/vm/vm_extensions/cpython_context_runtime.rs` (`with_active_cpython_context_mut`, `cpython_set_active_context`, `cpython_value_from_ptr*`, `cpython_set_error`, builtin C-function varargs shim callback) to keep cross-cutting CPython context glue out of the main monolith.
  - CPython contextvar C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_contextvar_api.rs` (`PyContextVar_New`, `PyContextVar_Get`, `PyContextVar_Set`), with shared behavior delegated to active-context and mapping helpers.
  - CPython eval C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_eval_api.rs` (`PyEval_GetBuiltins`, `PyEval_GetFrame*`, `PyEval_GetGlobals/Locals`, `PyEval_GetFuncName/Desc`), with shared behavior delegated to active-context/runtime helpers.
  - CPython eval/system/marshal C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_eval_os_marshal_api.rs` (`PyErr_CheckSignals`, `PyGILState_*`, `PyEval_*` eval-entrys, `PyOS_*`, `PyMarshal_*`), with marshal encode/decode and libc parse bridges delegated out of the main monolith.
  - CPython iter C-API entrypoints + iterator compatibility helpers are now isolated in `src/vm/vm_extensions/cpython_iter_api.rs` (`PyIter_Check`, `PyIter_NextItem`, `PyIter_Send`, `PyIter_Next`, active-exception iterator helpers), with shared behavior delegated to active-context/runtime helpers.
  - CPython capsule C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_capsule_api.rs` (`PyCapsule_*` create/get/set/validate/import plus external-capsule pointer compatibility helper), with shared behavior delegated to active-context/runtime helpers.
  - CPython list C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_list_api.rs` (`PyList_*` create/access/mutation/slice/sort helpers), with shared list-storage synchronization behavior delegated to active-context/runtime helpers.
  - CPython long/float numeric C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_long_float_api.rs` (`PyLong_*` constructors/parse/native-bytes helpers plus `PyBool_FromLong` and `PyFloat_*`), with shared bigint conversion/error parity delegated to active-context/runtime helpers.
  - CPython memory allocator C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_mem_api.rs` (`PyMem_Raw*`, `PyMem_*`), with shared allocator forwarding and CPython-allocation ownership-guard behavior delegated to active-context/runtime helpers.
  - CPython object-core call/vectorcall C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_object_call_api.rs` (`PyObject_IsTrue/Not/Str/Repr/ASCII`, `PyObject_GetIter/GetAIter`, `PyObject_Call*`, `PyObject_Vectorcall*`, managed-dict/finalizer helpers, `PyMethod_New`, `PyCode_New*`), with shared vectorcall decode/materialization delegated to active-context/runtime helpers.
  - CPython object item/hash/compare C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_object_item_compare_api.rs` (`PyObject_Get/Set/DelItem`, `PyObject_Size/Length/LengthHint`, `PyObject_Hash*`, `PyObject_RichCompare*`, `PyObject_IsInstance/IsSubclass`, `PyObject_GetOptionalAttr`), with shared compare-slot fallback/debug/type-name helpers delegated to active-context/runtime helpers.
  - CPython object buffer/memoryview/print C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_object_buffer_api.rs` (`PyObject_Check*Buffer`, `PyObject_As*Buffer`, `PyObject_GetBuffer`, `PyObject_CopyData`, `PyObject_Print`, `PyBuffer_*`, `PyMemoryView_*`), with shared contiguous-layout helpers and CPython buffer-struct wiring delegated to active-context/runtime helpers.
  - CPython object lifecycle C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_object_lifecycle_api.rs` (`PyObject_Init*`, `_PyObject_New*`, `_PyObject_GC_New`, `_Py_Dealloc`), with shared raw-header initialization and CPython pointer-handle dealloc bridging delegated to active-context/runtime helpers.
  - CPython weakref C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_weakref_api.rs` (`PyWeakref_NewRef`, `PyWeakref_NewProxy`, `PyWeakref_GetRef`, `PyWeakref_GetObject`, `PyObject_ClearWeakRefs`), with shared weakref-target extraction and callback-callable validation delegated to active-context/runtime helpers.
  - CPython runtime/misc C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_runtime_misc_api.rs` (`Py_Repr*`, pending-call/atexit APIs, version/build/platform getters, path-config wide-char APIs, `Py_{Initialize,Finalize,Main,BytesMain,CompileString,Exit}`, fatal-error APIs, `_PyErr_BadInternalCall`, `_Py_HashDouble`, `_PyUnicode_Is*`), with shared pending-call queue, path-config storage, and compile/CLI bridge behavior delegated to runtime helpers.
  - CPython refcount/internal-GC C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_refcount_api.rs` (`Py_{IncRef,DecRef,XIncRef,XDecRef}`, `_Py_{IncRef,DecRef,SetRefcnt,NegativeRefcount,CheckRecursiveCall}`, `_PyObject_GC_{NewVar,Resize}`), with shared header-refcount mutation and active-context handle sync delegated to runtime helpers.
  - CPython type-object C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_type_api.rs` (`PyType_*`, `_PyType_Lookup`, and generic `type` call/new/alloc helpers), with shared heap-type registry + from-spec slot application behavior delegated to runtime helpers.
  - CPython thread/interpreter/state C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_thread_interp_api.rs` (`PyThreadState_*`, `PyInterpreterState_*`, `PyState_*`, `PyTraceMalloc_*`, recursion/init probes, constant/identity helpers), with shared interpreter/thread token lifecycle and module-def state registry behavior delegated to runtime helpers.
  - CPython exception/file/traceback C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_exception_file_api.rs` (`PyFile_*`, `PyTraceBack_*`, `PyException_*`), with shared exception-instance validation helpers and file-write/traceback formatting bridge behavior delegated to runtime helpers.
  - CPython object allocator/GC C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_gc_alloc_api.rs` (`PyObject_Malloc/Calloc/Realloc/Free`, `PyObject_GC_*`, `PyGC_*`), with shared VM GC-state/collect wiring delegated to active-context/runtime helpers.
  - CPython tuple C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_tuple_api.rs` (`PyTuple_*` create/access/mutation/slice helpers), with shared tuple-storage synchronization behavior delegated to active-context/runtime helpers.
  - CPython dict C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_dict_api.rs` (`PyDict_*`, `_PyDict_*`, and `PyDictProxy_New` mapping helpers), with shared dict-mutation, mapping-slot fallback, and error-state behavior delegated to active-context/runtime helpers.
  - CPython set C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_set_api.rs` (`PySet_*` and `PyFrozenSet_New` helpers), with shared set/frozenset mutation/query behavior delegated to native set runtime dispatch.
  - CPython object attribute/introspection C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_object_attr_api.rs` (`PyObject_*` attr/get/set/del/type/has-attr helpers), with shared slot-fallback and missing-attribute semantics delegated to active-context/runtime helpers.
  - CPython bytes/bytearray C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_bytes_api.rs` (`PyBytes_*`, `_PyBytes_Join`, `PyByteArray_*` construction/concat/size/data/resize helpers), with shared bytes-layout compatibility and buffer release semantics delegated to active-context/runtime helpers.
  - CPython tuple/dict argument conversion helpers are now isolated in `src/vm/vm_extensions/cpython_args_runtime.rs` (`cpython_positional_args_from_tuple_object`, `cpython_keyword_args_from_dict_object`) to keep ABI call-entry argument normalization out of the main monolith.
  - CPython module-def/state helpers are now isolated in `src/vm/vm_extensions/cpython_module_runtime.rs` (`cpython_bind_module_def`, `cpython_new_module_data`, module-state free callback) to keep module creation/state allocation logic out of the main monolith.
  - CPython module C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_module_api.rs` (exported `PyModule_*` create/from-spec/exec/add/get helpers), with shared behavior delegated to module-runtime/module-name/context helpers.
  - CPython import helper substrate is now isolated in `src/vm/vm_extensions/cpython_import_runtime.rs` (`cpython_import_add_module_by_name`, inittab registry/lookup, `cpython_import_exec_code_in_module`) to keep `PyImport_*` shared runtime logic out of the main monolith.
  - CPython import C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_import_api.rs` (exported `PyImport_*` functions including inittab/frozen/import-exec/reload paths), with shared behavior delegated to import-runtime/module-name/context helpers.
  - CPython module-name/value helpers are now isolated in `src/vm/vm_extensions/cpython_module_name_runtime.rs` (`cpython_module_name_from_object`, `cpython_optional_value_from_ptr`, `cpython_module_add_type_name`) to keep import/module name normalization logic out of the main monolith.
  - CPython exception-name parsing helpers are now isolated in `src/vm/vm_extensions/cpython_exception_name_runtime.rs` (`cpython_exception_name_from_runtime_message`, `cpython_exception_name_parts`) to keep error-name normalization logic out of the main monolith.
  - CPython active-context call helpers are now isolated in `src/vm/vm_extensions/cpython_call_runtime.rs` (`cpython_call_internal_in_context`, `cpython_getattr_in_context`) to keep shared call/attr dispatch glue out of the main monolith.
  - CPython codec helper substrate is now isolated in `src/vm/vm_extensions/cpython_codec_runtime.rs` (`cpython_codec_*` lookup/call/error helpers + built-in codec error handler method defs) to keep `PyCodec_*` shared runtime logic out of the main monolith.
  - CPython codec C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_codec_api.rs` (exported `PyCodec_*` functions), with shared behavior delegated to codec/call helper modules.
  - CPython unicode-error helper substrate is now isolated in `src/vm/vm_extensions/cpython_unicode_error_runtime.rs` (`cpython_unicode_error_*`, `CpythonUnicodeErrorFlavor`, `cpython_exception_value_attr`) to keep `PyUnicode*Error_*` shared runtime logic out of the main monolith.
  - CPython unicode-error C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_unicode_error_api.rs` (`PyUnicodeDecodeError_Create`, `PyUnicode*Error_Get*`, `PyUnicode*Error_Set*`), with shared behavior delegated to unicode-error runtime/context helpers.
  - CPython numeric-op helper substrate is now isolated in `src/vm/vm_extensions/cpython_numeric_runtime.rs` (`cpython_unary_numeric_op`, `cpython_binary_numeric_op`, `cpython_binary_numeric_op_with_heap`) to keep `PyNumber_*` pointer/value conversion glue out of the main monolith.
  - CPython numeric C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_numeric_api.rs` (exported `PyNumber_*` functions including in-place and conversion helpers), with shared behavior delegated to numeric runtime/context helpers.
  - CPython sequence/mapping C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_sequence_mapping_api.rs` (`PySequence_*`, `PyMapping_*`, `PySeqIter_New`, `PyCallIter_New`), with shared slice-index normalization helpers delegated out of the main monolith.
  - CPython descriptor/method/slice C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_descriptor_method_api.rs` (`PyDescr_*`, `PyCFunction_*`, `PyCMethod_New`, `PyWrapper_New`, `PySlice_*`), with C-function descriptor invoke/bind and member/getset conversion logic delegated out of the main monolith.
  - CPython legacy numeric + error C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_error_numeric_api.rs` (`PyFloat_AsDouble`, `PyLong_As*`, `PyComplex_*`, `PyStructSequence_*`, `PyErr_*`), with exception matching/normalization and error-state bridge helpers delegated out of the main monolith.
  - CPython unicode C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_unicode_api.rs` (`PyUnicode_*`, `PyBuffer_Release`, `PyCallable_Check`, `PyIndex_Check`), with unicode codec/indexing/path conversion behavior delegated out of the main monolith.
  - CPython sys/thread C-API entrypoints are now isolated in `src/vm/vm_extensions/cpython_sys_thread_api.rs` (`PySys_*`, `PyThread_*`), with sys warn/xoption helpers and thread lock/TLS/TSS registries delegated out of the main monolith.
  - CPython bigint conversion helpers are now isolated in `src/vm/vm_extensions/cpython_bigint_runtime.rs` (`cpython_bigint_*`, endian resolution), keeping two's-complement/byte-width math out of `vm_extensions.rs`.
  - CPython slot/call fallback helpers are now isolated in `src/vm/vm_extensions/cpython_slot_runtime.rs` (`cpython_*slot*`, richcompare/number slot fallback, `cpython_call_object`, unicode codec-call helpers), reducing cross-domain free-function coupling in `vm_extensions.rs`.
  - CPython value/type/debug helpers are now isolated in `src/vm/vm_extensions/cpython_value_runtime.rs` (`value -> PyType*`, builtin class/type pointer mapping, ufunc debug summaries), keeping type-dispatch/debug plumbing out of the monolith.
  - CPython thread/interpreter registry helpers are now isolated in `src/vm/vm_extensions/cpython_thread_runtime.rs` (`ThreadState` compat init, interned-unicode registry, TLS/TSS registries, pending-call/atexit queues, wide-path storage helpers), reducing global-runtime helper density in `vm_extensions.rs`.
  - CPython C-string/wide-string conversion helpers are now isolated in `src/vm/vm_extensions/cpython_string_runtime.rs` (`c_name_to_string`, `cpython_wide_ptr_to_string`, wide-unit encode/decode), centralizing string/wchar conversion logic shared by unicode/runtime-misc/sys surfaces.
  - CPython keepalive symbol-retention references (`KEEP*` static ABI/linker guards) are now isolated in `src/vm/vm_extensions/cpython_keepalive_exports.rs`, removing ~2.9k lines of non-runtime export scaffolding from `vm_extensions.rs`.
  - CPython marshal conversion helpers are now isolated in `src/vm/vm_extensions/cpython_marshal_runtime.rs` (`value_to_cpython_marshal_object`, `cpython_marshal_object_to_value`), keeping marshal translation logic out of the extension monolith.
  - CPython exported type/static-layout declarations are now isolated in `src/vm/vm_extensions/cpython_type_exports.rs` (`Py*_Type`, `_PyWeakref_*`, `PY_LONG_NUMBER_METHODS`) to separate ABI-export statics from runtime dispatch.
  - CPython shared type-layout structs/constants are now isolated in `src/vm/vm_extensions/cpython_type_layout.rs` (`CpythonTypeObject`, numeric/sequence/mapping structs, member/type-slot constants), keeping C-struct ABI layouts out of the extension dispatch file.
  - datetime capsule bootstrap/static glue (`datetime.datetime_CAPI`) is now isolated in `src/vm/vm_extensions/cpython_datetime_runtime.rs`, keeping capsule constructor stubs out of the extension core.
  - VM test harness stability update: stack-heavy stdlib tests (`pickle proto5 frameless`, `urllib.urlparse` smoke, `xml.etree` smoke, and prior dataclass/ftplib/fractions/json/pickle proto0 probes) now execute via `run_with_large_stack(...)` to avoid debug-thread stack overflows while preserving semantic assertions.
  - VM `LOAD_ATTR` instance cache now prioritizes builtin/function descriptors over plain value cache entries; this fixes repeated-loop `io.StringIO.tell/seek` call arity regressions (covered by `tests/vm.rs::io_stringio_tell_works_across_repeated_loop_iterations`).
  - C-API v1 now includes `module_get_object(...)` for handle-based reads of module globals during extension init/call paths.
  - C-API v1 now includes `module_import(...)` for native-side module loading during extension init/call paths.
  - C-API v1 now includes `module_get_attr(...)` for module-handle attribute extraction with explicit module-type checks.
  - C-API v1 now includes module-state lifecycle hooks (`module_set_state`, `module_get_state`, `module_set_finalize`) with replacement/clear/stale-prune/VM-drop finalize+free semantics.
  - C-API v1 now includes module-handle attribute mutation helpers (`module_set_attr`, `module_del_attr`, `module_has_attr`) for native-side module mutation/probe flows without Python-level glue.
  - C-API handle constructors/getters now include `None`/`float`/`bytes`/`bytearray`/`memoryview-from-handle`/`tuple`/`list`/`dict` in addition to `bool`/`int`/`str`, with smoke coverage through native handle and sequence+mapping round-trip.
  - C-API v1 now includes native iterator helpers (`object_get_iter`, `object_iter_next`) for extension-side iteration without Python-level dispatch glue.
  - C-API v1 now includes generic object helpers (`object_len`, `object_get_item`) for native-side length/subscript paths.
  - C-API v1 generic item helpers now include mutation (`object_set_item`, `object_del_item`) with dict/list/bytearray direct semantics and special-method fallback for custom containers.
  - C-API v1 now includes generic membership and dict-view helpers (`object_contains`, `object_dict_keys`, `object_dict_items`) for native-side probe/inspection paths.
  - C-API v1 now includes bytes-like buffer helpers (`object_get_buffer`, `object_release_buffer`) exposing pointer/len/readonly buffer views for `bytes`/`bytearray`/`memoryview` handles.
  - C-API v1 now includes writable buffer helper (`object_get_writable_buffer`) for mutable `bytearray` and writable `memoryview` handles, with explicit read-only rejection for `bytes`/readonly-memoryview sources and explicit rejection for non-contiguous memoryview layouts.
  - C-API v1 now includes buffer metadata helper (`object_get_buffer_info`) exposing scalar metadata (`itemsize`, `ndim`, `shape0`, `stride0`, `format`, `contiguous`) for `bytes`/`bytearray`/`memoryview`, including shaped memoryview casts (`ndim > 1`).
  - C-API v1 now includes buffer metadata pointer helper (`object_get_buffer_info_v2`) exposing `shape[]`/`strides[]` arrays for extension consumers that expect pointer-based descriptors.
  - C-API buffer acquisitions on mutable sources now pin runtime buffer exports and block bytearray resize paths until `object_release_buffer` is called.
  - module C-API context drop now cleans leaked mutable buffer pins so extension-init leaks do not leave stale resize blocks.
  - C-API v1 now includes capsule baseline helpers (`capsule_new`, `capsule_get_pointer`, `capsule_set_pointer`, `capsule_get_name`, `capsule_set_context`, `capsule_get_context`, `capsule_set_destructor`, `capsule_get_destructor`, `capsule_set_name`, `capsule_is_valid`, `capsule_export`, `capsule_import`) for opaque native-pointer interop in extension handle space.
  - C-API v1 now includes list/dict mutation helpers (`object_list_append`, `object_list_set_item`, `object_dict_contains`, `object_dict_del_item`) with positive/negative-path smoke coverage.
  - C-API v1 now includes handle-based object attribute access (`object_get_attr`, `object_set_attr`, `object_del_attr`) with native extension smoke coverage.
  - C-API v1 attribute helpers now include presence checks (`object_has_attr`) for exception-free attr probes in native code.
  - C-API v1 now includes type relation checks (`object_is_instance`, `object_is_subclass`) for native-side runtime typing paths.
  - C-API v1 now includes handle-based callable invocation (`object_call`) so native callbacks can call Python callables with positional/keyword payloads.
  - C-API v1 now includes call fast paths (`object_call_noargs`, `object_call_onearg`) for common native callback dispatch forms.
  - C-API v1 error state now exposes message retrieval (`error_get_message`) in addition to set/clear/occurred hooks.
  - C-API baseline now includes runtime feature probing via `api_has_capability(...)` (covered in extension smoke).
  - builtin `_sysconfigdata__*` now provides extension-build baseline keys (`SOABI`, `EXT_SUFFIX`, `CC`, `LDSHARED`, include/lib dir hints) for source-build toolchains.
  - `_sysconfigdata__*` build vars now include broader toolchain/linker baselines (`AR`, `ARFLAGS`, `CCSHARED`, `BLDSHARED`, `CPPFLAGS`, `LDFLAGS`, `LIBPL`, `INCLUDEDIR`, `Py_ENABLE_SHARED`) alongside prior extension keys.
  - extension smoke includes compile+import validation driven by `_sysconfigdata__*` build vars (`sysconfig_build_vars_can_compile_and_import_extension`).
  - extension smoke path now includes compiled native fixtures for manifest dynamic load, direct shared-object import, tagged filename resolution, object-handle flow, positional+keyword callable registration/invocation, and error-state propagation (`tests/extension_smoke.rs`).
  - extension smoke now includes a mixed-surface cross-API fixture exercising module import/attr-load, type checks, list+dict mutation, callable invocation, and module-global round-trips in one extension init path.
  - extension smoke now includes an invalid-handle resilience fixture to enforce consistent error-message + error-clear behavior across C-API calls.
  - extension smoke now includes a module/item helper fixture covering module attr set/del/has and generic item set/del paths (`dynamic_extension_can_set_module_attrs_and_items`).
  - extension smoke now includes a special-method fallback fixture proving `object_set_item`/`object_del_item` dispatch on custom `__setitem__`/`__delitem__` containers (`dynamic_extension_item_mutation_falls_back_to_special_methods`).
  - extension smoke now includes membership + dict-view fixture coverage for `object_contains` + `object_dict_keys`/`object_dict_items` APIs (`dynamic_extension_can_use_contains_and_dict_view_apis`).
  - extension smoke now includes buffer API coverage for pointer/len/readonly views and release semantics (`dynamic_extension_can_use_buffer_apis`).
  - extension smoke now includes writable buffer API coverage (`dynamic_extension_can_use_writable_buffer_apis`) validating in-place mutation through `bytearray` and writable `memoryview` handles plus read-only rejection paths.
  - extension smoke now includes buffer metadata coverage (`dynamic_extension_can_read_buffer_info_metadata`) across scalar (`_info`) and pointer-array (`_info_v2`) surfaces.
  - extension smoke now includes non-contiguous metadata coverage (`dynamic_extension_buffer_info_marks_noncontiguous_slice_views`) for stepped memoryview slice behavior.
  - extension smoke now includes memoryview-cast metadata coverage (`dynamic_extension_buffer_info_reflects_memoryview_cast_itemsize`) for format/itemsize propagation (`cast('I')`) and shaped cast layout propagation (`cast('B', [2, 4])`).
  - extension smoke now includes `object_get_buffer_info_v2` negative-path coverage (`dynamic_extension_buffer_info_v2_reports_invalid_and_null_output_errors`) for invalid handle + null-output pointer errors.
  - memoryview cast now accepts `shape` through positional or keyword call forms (`cast("B", shape=[...])`, `cast(format="B", shape=[...])`), and memoryview layout attrs now expose shaped metadata (`ndim`, `shape`, `strides`, `format`, `c_contiguous`, `f_contiguous`).
  - memoryview cast/tolist now supports expanded native format set (`B`, `b`, `c`, `H`, `h`, `I`, `i`, `L`, `l`, `Q`, `q`, `f`, `d`) with platform-native `long` width and parity tests in VM suite.
  - memoryview scalar indexing/stores now honor cast format semantics (`b` signed reads/writes, typed integer widths, `f`/`d` float writes, `c` bytes-only writes with CPython-style invalid-type/value errors) and scalar multi-dimensional indexing now raises `NotImplementedError` parity messages.
  - memoryview multi-dimensional first-axis slicing now preserves source-backed shape/stride metadata and nested `tolist()` output (e.g. `view[0:1]`, `view[::2]`) instead of flattening/copying.
  - memoryview byte export + iteration now honor strided layout and format decoding (`bytes(view[::2])` / `bytes(view[::-1])` flatten by stride; typed 1-D iteration decodes by format; multi-dimensional iteration raises `NotImplementedError` parity), and zero-length multidim slices now report CPython contiguity flags (`contiguous`, `c_contiguous`, `f_contiguous` all true).
  - stepped memoryview slicing now keeps source-backed strided views (no forced bytes copy), including negative-stride views (`[::-1]`) and in-place write propagation back to underlying mutable buffers.
  - extension smoke now includes resize-blocking export-pin coverage (`dynamic_extension_buffer_pin_blocks_bytearray_resize_until_release`).
  - extension smoke now includes leaked-pin cleanup coverage (`dynamic_extension_unreleased_buffer_pin_is_cleared_on_context_drop`) for context-drop unpin behavior.
  - extension smoke now includes memoryview-slice + release failure-path coverage for buffer APIs (`dynamic_extension_buffer_api_handles_memoryview_slices_and_release`).
  - extension smoke now includes capsule API coverage for create/name/pointer/context/destructor/refcount paths (`dynamic_extension_can_use_capsule_apis`).
  - capsule destructor callbacks now run on final handle decref and on module C-API context drop, with dedicated smoke coverage (`dynamic_extension_runs_capsule_destructor_on_context_drop`).
  - extension smoke now includes cross-extension named capsule export/import coverage (`dynamic_extension_can_import_exported_capsule_by_name`).
  - `capsule_import` now performs CPython-style module/attribute traversal fallback for diagnostics on non-registry names before invalid-capsule failure.
  - extension smoke now includes module-state lifecycle coverage (`dynamic_extension_can_manage_module_state_lifecycle`).
  - extension smoke now includes module-state drop ordering coverage (`dynamic_extension_module_state_drop_runs_finalize_before_free`), asserting finalize callbacks run before free callbacks on VM teardown.
  - extension smoke now includes finalize-disable coverage (`dynamic_extension_can_disable_module_state_finalize_callback`), asserting `module_set_finalize(..., NULL)` suppresses finalize callbacks while preserving free callbacks.
  - extension smoke now includes null-context guard coverage (`dynamic_extension_module_state_apis_guard_null_module_ctx`), asserting module-state C-API calls handle `module_ctx == NULL` without crashing.
  - module-state registry now prunes stale module entries during `sys.modules` churn and executes associated finalize+free callbacks (covered by module-state smoke reimport path).
  - extension smoke now includes buffer/capsule interop bridge coverage (`dynamic_extension_can_bridge_buffer_pointer_through_capsule`).
  - keyword-callable smoke now asserts negative keyword/error paths (`unknown keyword`, invalid keyword value type, and positional-only callable rejecting kwargs) to harden C-API call semantics.
  - CI has a dedicated extension smoke lane (`cargo test -q --test extension_smoke`).
  - NumPy bring-up import + source-build probes are landed (`scripts/probe_numpy_gate.py`, `docs/NUMPY_BRINGUP_GATE.md`, artifacts `perf/numpy_gate_direct_latest.json` and `perf/numpy_gate_source_build_latest.json`).
  - C-API execution strategy is now split into Lane A (CPython Stable ABI / abi3 closure) and Lane B (NumPy/scientific-stack non-abi3 closure), tracked in `docs/CAPI_PLAN.md`.
  - ABI coverage baseline is generated from CPython 3.14 `Misc/stable_abi.toml` via `scripts/generate_abi3_manifest.py`; current snapshot is `functions 782/782`, `data 143/143` in `perf/abi3_manifest_latest.json`.
  - abi3 manifest generation now normalizes Mach-O private-symbol prefixes (`__Py_*` -> `_Py_*`) to avoid undercounting private Stable-ABI exports on macOS.
  - latest abi3 closure slices landed: `PyDict_{Clear,Update,Keys,Values,Items,MergeFromSeq2}`, `PyCapsule_{GetName,SetPointer,GetDestructor,SetDestructor}`, `PyByteArray_*` constructor/access/resize/concat APIs, `PyBuffer_Release` pin/ref-state release semantics, C-function constructor/introspection APIs (`PyCFunction_New`, `PyCFunction_NewEx`, `PyCMethod_New`, `PyCFunction_{Call,GetFunction,GetSelf,GetFlags}`), descriptor constructor APIs (`PyDescr_NewMethod`, `PyDescr_NewClassMethod`, `PyDescr_NewMember`, `PyDescr_NewGetSet`) plus descriptor type export (`PyClassMethodDescr_Type`), parse/keyword validation APIs (`PyArg_Parse`, `PyArg_VaParse`, `PyArg_ValidateKeywordArguments`), eval-thread/call APIs (`PyEval_{AcquireLock,ReleaseLock,AcquireThread,ReleaseThread,InitThreads,ThreadsInitialized,CallObjectWithKeywords,CallFunction,CallMethod}`), bytes formatting/repr/escape APIs (`PyBytes_{FromFormat,FromFormatV,Repr,DecodeEscape}`), import APIs (`PyImport_GetModuleDict`, `PyImport_{AddModuleRef,AddModuleObject,AddModule,GetModule}`, `PyImport_ImportModuleNoBlock`, `PyImport_{ImportModuleLevelObject,ImportModuleLevel}`, `PyImport_ReloadModule`), import-magic APIs (`PyImport_GetMagicNumber`, `PyImport_GetMagicTag`), error/file APIs (`PyErr_{GetRaisedException,SetRaisedException,GetHandledException,SetHandledException,GetExcInfo,SetExcInfo}`, `PyFile_{GetLine,WriteObject,WriteString}`, `PyExc_EOFError`), full long-api slices (`PyLong_{FromSize_t,FromInt32,FromUInt32,FromInt64,FromUInt64,FromString,GetInfo,AsInt,AsInt32,AsUInt32,AsInt64,AsUInt64,AsSize_t,AsDouble,AsUnsignedLongMask,AsUnsignedLongLongMask,AsNativeBytes,FromNativeBytes,FromUnsignedNativeBytes}`), buffer helper APIs (`PyBuffer_{FillContiguousStrides,FillInfo,IsContiguous,GetPointer,SizeFromFormat,FromContiguous,ToContiguous}`), sequence APIs (`PySequence_{Length,GetSlice,SetItem,DelItem,SetSlice,DelSlice,List,Count,Index,In}`) plus `PyObject_DelItem`, slice index APIs (`PySlice_{GetIndices,GetIndicesEx}`), iterator/memoryview APIs (`PyIter_{NextItem,Send}`, `PyMemoryView_{FromMemory,FromBuffer,GetContiguous}` with header declarations for `PyIter_Check`, `PyIter_Next`, and `PySeqIter_New`), object convenience APIs (`PyObject_{CallNoArgs,CallMethodObjArgs,DelAttr,DelAttrString,DelItemString,Dir,GetOptionalAttrString,HasAttr,HasAttrWithError,HasAttrStringWithError,Length,Repr,SetAttr}`), legacy object/buffer helpers (`PyObject_{ASCII,Calloc,CheckReadBuffer,AsReadBuffer,AsWriteBuffer,AsCharBuffer,CopyData}`), object/GC helper APIs (`PyObject_{GetAIter,GetTypeData,HashNotImplemented}`, `PyObject_GC_{IsTracked,IsFinalized}`), mapping/async-check APIs (`PyAIter_Check`, `PyMapping_{Check,Size,Length,GetItemString,Keys,Items,Values,GetOptionalItem,GetOptionalItemString,SetItemString,HasKeyWithError,HasKeyStringWithError,HasKey,HasKeyString}`), module helper APIs (`PyModule_{NewObject,New,GetNameObject,GetName,GetFilenameObject,GetFilename,SetDocString,Add}`), module registration helpers (`PyModule_{AddFunctions,AddType}`), exception factory helpers (`PyErr_{NewException,NewExceptionWithDoc}`, `PyExceptionClass_Name`), numeric helper APIs (`PyNumber_MatrixMultiply`, `PyNumber_InPlace*`, `PyNumber_ToBase`), error helper APIs (`PyErr_SetFromErrnoWithFilename*`, `PyErr_SetExcFromWindowsErr*`, `PyErr_SetFromWindowsErr*`, `PyErr_SetInterrupt*`, `PyErr_SyntaxLocation*`, `PyErr_ProgramText`), import-error helpers (`PyErr_SetImportError`, `PyErr_SetImportErrorSubclass`), warning helper APIs (`PyErr_WarnExplicit`, `PyErr_ResourceWarning`), and exported warning category symbol (`PyExc_ResourceWarning`).
  - additional abi3 closure slices landed: `PyCallIter_New` (with callable/sentinel iterator substrate), `PyGILState_GetThisThreadState` (with non-null/stable thread-state baseline), `PyEval_{GetGlobals,GetLocals}` (frame mapping accessors with no-frame null-return baseline), state-introspection APIs `PyInterpreterState_{Get,GetID,GetDict}` + `PyThreadState_{GetInterpreter,GetID,GetDict}` (stable per-context dict pointers + ID/pointer baseline semantics), interpreter-state lifecycle APIs `PyInterpreterState_{New,Clear,Delete}` (opaque allocation lifecycle + stable main-interpreter token semantics), module-state registry APIs `PyState_{AddModule,FindModule,RemoveModule}` (single-phase module registration/find/remove substrate), member-struct APIs `PyMember_{GetOne,SetOne}` (scalar/object load-store + readonly/relative guard baseline), struct-sequence APIs `PyStructSequence_{NewType,New,SetItem,GetItem}` + `PyStructSequence_UnnamedField` (baseline type/new/set/get substrate), `PyOS_{BeforeFork,AfterFork_Parent,AfterFork_Child,AfterFork,CheckStack,FSPath,InterruptOccurred,double_to_string,getsig,setsig,mystricmp,mystrnicmp,vsnprintf}` (baseline `PyOS` substrate), full `PyExc_*` data-symbol export closure from the current abi3 manifest (including `EnvironmentError`/`IOError`/`WindowsError` aliasing to `PyExc_OSError`), remaining data-symbol closure for iterator/view/type globals + weakref globals + `PyOS_InputHook` + filesystem/version globals (`Py_FileSystemDefault*`, `Py_Version`, `_Py_RefTotal`, `_Py_SwappedOp`) with `data 143/143` coverage, full `PyThread_*` batch53 baseline (`lock`, `stacksize`, deprecated TLS, and TSS APIs), `PyType_*` batch54 baseline (`PyType_{FromMetaclass,FromModuleAndSpec,FromSpecWithBases,FromSpec,GetName,GetQualName,GetModuleName,GetFullyQualifiedName,GetSlot,GetModule,GetModuleState,GetModuleByDef,GetTypeDataSize,GetBaseByToken,ClearCache,Modified,Freeze}`) with heap-type/module/token/slot smoke coverage, and traceback batch55 baseline (`PyTraceBack_{Here,Print}`) with null-frame guard + render/write smoke coverage. Import-exec/importer APIs `PyImport_{ExecCodeModule,ExecCodeModuleEx,ExecCodeModuleObject,ExecCodeModuleWithPathnames,GetImporter}` (module exec + `__file__` propagation + failure cleanup), eval/frame helper APIs `PyEval_{GetFrame,GetFrameBuiltins,GetFrameGlobals,GetFrameLocals,GetFuncName,GetFuncDesc}`, eval-code APIs `PyEval_{EvalCode,EvalCodeEx}` (`EvalCodeEx` simple no-args/no-closure baseline), frozen/inittab import APIs `PyImport_{AppendInittab,ImportFrozenModule,ImportFrozenModuleObject}`, frame-inspection APIs `PyThreadState_GetFrame` + `PyFrame_{GetCode,GetLineNumber}`, file-from-fd API `PyFile_FromFd`, module-definition APIs `PyModule_{FromDefAndSpec2,ExecDef,GetDef,GetState}`, sys APIs `PySys_{SetObject,GetXOptions,AddXOption,HasWarnOptions,ResetWarnOptions,AddWarnOption,AddWarnOptionUnicode,WriteStdout,WriteStderr,FormatStdout,FormatStderr,Audit,AuditTuple,SetArgv,SetArgvEx,SetPath}`, eval-frame exports `PyEval_{EvalFrame,EvalFrameEx}` (current behavior: null/current-frame guard path; full frame-evaluation semantics remain open), thread-state lifecycle APIs `PyThreadState_{New,Swap,Clear,Delete,DeleteCurrent,SetAsyncExc}`, marshal APIs `PyMarshal_{ReadObjectFromString,WriteObjectToString}` with tuple/str/int round-trip and invalid-payload rejection coverage, and codec-registry APIs `PyCodec_{Register,Unregister,KnownEncoding,Encode,Decode,Encoder,Decoder,IncrementalEncoder,IncrementalDecoder,StreamReader,StreamWriter,RegisterError,LookupError,StrictErrors,IgnoreErrors,ReplaceErrors,XMLCharRefReplaceErrors,BackslashReplaceErrors,NameReplaceErrors}` with batch45 symbol + smoke coverage.
  - extension smoke coverage includes CPython-compat API probes:
    - `tests/extension_smoke.rs::cpython_compat_dict_capsule_and_bytearray_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_list_set_exception_gc_and_float_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_bytes_error_and_cfunction_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_import_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_error_state_and_file_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_long_abi_batch6_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_long_abi_batch7_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_long_abi_batch8_native_bytes_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_buffer_abi_batch9_helpers_work`
    - `tests/extension_smoke.rs::cpython_compat_sequence_abi_batch10_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_slice_abi_batch11_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_iter_and_memoryview_abi_batch12_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_object_abi_batch13_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_object_buffer_abi_batch14_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_object_gc_and_async_abi_batch15_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_mapping_abi_batch16_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_module_abi_batch17_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_module_helpers_abi_batch18_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_exception_factory_abi_batch19_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_number_abi_batch20_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_error_abi_batch21_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_import_error_abi_batch22_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_warning_abi_batch23_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_import_magic_abi_batch24_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_descriptor_abi_batch25_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_parse_abi_batch26_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_eval_abi_batch27_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_bytes_abi_batch28_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_calliter_abi_batch29_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_gilstate_abi_batch30_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_eval_frame_maps_abi_batch31_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_state_introspection_abi_batch32_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_import_exec_abi_batch33_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_eval_frame_abi_batch34_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_eval_code_abi_batch35_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_import_frozen_abi_batch36_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_frame_abi_batch37_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_file_fromfd_abi_batch38_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_module_def_state_abi_batch39_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_sys_abi_batch40_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_eval_frame_abi_batch41_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_threadstate_abi_batch42_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_marshal_abi_batch43_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_sys_abi_batch44_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_codec_abi_batch45_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_interpreterstate_abi_batch46_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_pystate_abi_batch47_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_member_abi_batch48_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_structseq_abi_batch49_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_pyos_abi_batch50_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_exceptions_abi_batch51_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_data_symbols_abi_batch52_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_thread_abi_batch53_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_type_abi_batch54_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_traceback_abi_batch55_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_unicode_error_abi_batch56_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_unicode_abi_batch57_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_unicode_codec_abi_batch58_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_unicode_append_compare_abi_batch59_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_unicode_widechar_abi_batch60_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_unicode_codec_abi_batch61_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_core_ref_and_identity_abi_batch62_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_unicode_fromformat_abi_batch63_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_weakref_abi_batch64_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_runtime_misc_abi_batch65_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_pathconfig_and_locale_abi_batch66_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_lifecycle_and_pack_abi_batch67_apis_work`
    - `tests/extension_smoke.rs::cpython_compat_internal_ref_gc_abi_batch68_apis_work`
  - latest abi3 batch56 closure landed for Unicode error helper APIs (`PyUnicodeDecodeError_Create`, `PyUnicode{Encode,Decode,Translate}Error_{GetObject,GetStart,SetStart,GetEnd,SetEnd,GetReason,SetReason}` and encoding/object getters where applicable), with smoke coverage for clipping/type-guard behavior.
  - latest abi3 batch57 closure landed for Unicode utility APIs (`PyUnicode_{FromObject,FromOrdinal,GetDefaultEncoding,Equal,EqualToUTF8,EqualToUTF8AndSize,ReadChar,Find,FindChar,Count,Join,Split,RSplit,Splitlines,Partition,RPartition,IsIdentifier}`), with smoke coverage for conversion/equality/read/find/count/split/join/partition semantics.
  - latest abi3 batch58 closure landed for Unicode codec/FS helper APIs (`PyUnicode_{Decode,DecodeASCII,DecodeLatin1,DecodeUTF8,DecodeUTF8Stateful,DecodeFSDefault,DecodeFSDefaultAndSize,DecodeLocale,DecodeLocaleAndSize,EncodeFSDefault,EncodeLocale,AsDecodedObject,AsDecodedUnicode,AsEncodedObject,AsEncodedUnicode,FSConverter,FSDecoder}`), with smoke coverage for codec helper semantics, stateful decode consumed counts, and FS converter/decoder slot behavior.
  - latest abi3 batch59 closure landed for Unicode append/intern/compare helpers (`PyUnicode_{Append,AppendAndDel,GetSize,InternInPlace,InternImmortal,RichCompare}`), with smoke coverage for append pointer-update flows, removed `GetSize` runtime-error semantics, and rich-compare bool/notimplemented return baselines.
  - latest abi3 batch60 closure landed for Unicode wide-char helpers (`PyUnicode_{FromWideChar,AsWideChar,AsWideCharString,WriteChar}`), with smoke coverage for fixed-size + NUL-terminated wide decode, required-size probes, owned wide-string allocation, and in-place character mutation/index-error paths.
  - latest abi3 batch61 closure landed for Unicode codec helpers (`PyUnicode_{AsRawUnicodeEscapeString,AsUnicodeEscapeString,AsUTF16String,AsUTF32String,DecodeRawUnicodeEscape,DecodeUnicodeEscape,DecodeUTF16,DecodeUTF16Stateful,DecodeUTF32,DecodeUTF32Stateful}`), with smoke coverage for escape round-trips plus UTF-16/UTF-32 decode/stateful consumed and byteorder output paths.
  - latest abi3 batch62 closure landed for core ref/identity/vectorcall helpers (`Py_{Is,IsNone,IsTrue,IsFalse,NewRef,XNewRef,REFCNT,TYPE}` and `PyVectorcall_NARGS`), with smoke coverage for identity predicates, ref helpers, type-pointer extraction, and vectorcall nargs-flag masking.
  - latest abi3 batch63 closure landed for Unicode varargs formatting (`PyUnicode_FromFormatV`, with `PyUnicode_FromFormat` routed through the same C varargs formatter path), with smoke coverage for direct varargs + `va_list` integer/string/pointer/percent formatting paths.
  - latest abi3 batch64 closure landed for weakref helper APIs (`PyWeakref_{NewRef,NewProxy,GetRef,GetObject}`), with smoke coverage for create/get paths, invalid-ref type errors, and callback-callable guards.
  - latest abi3 batch65 closure landed for runtime misc APIs (`Py_{AddPendingCall,MakePendingCalls,AtExit,GetRecursionLimit,SetRecursionLimit,GetVersion,GetBuildInfo,GetCompiler,GetPlatform,GetCopyright,ReprEnter,ReprLeave}`), with smoke coverage for pending-call execution, recursion-limit mutation, repr recursion guard behavior, and non-empty runtime metadata strings.
  - latest abi3 batch66 closure landed for path-config + locale helper APIs (`Py_{GetArgcArgv,SetProgramName,GetProgramName,SetPythonHome,GetPythonHome,SetPath,GetPath,GetPrefix,GetExecPrefix,GetProgramFullPath,DecodeLocale,EncodeLocale}`), with smoke coverage for setter/getter round-trips, argv export pointer updates, and decode/encode locale round-trip with `PyMem_RawFree` ownership semantics.
  - latest abi3 batch67 closure landed for lifecycle/pack helper APIs (`Py_{Initialize,InitializeEx,Finalize,FinalizeEx,IsFinalizing,NewInterpreter,EndInterpreter,PACK_FULL_VERSION,PACK_VERSION}`), with smoke coverage for finalize callback execution ordering, finalizing-state reset, interpreter pointer baseline, and packed-version bit layout parity.
  - latest abi3 batch68 closure landed for internal ref/GC helper APIs (`_Py_{IncRef,DecRef,SetRefcnt,NegativeRefcount,CheckRecursiveCall,Object_GC_NewVar,Object_GC_Resize}`), with smoke coverage for refcount mutation helpers, recursion-check baseline, GC var-object allocation/resize baseline, and negative-refcount error signaling.
  - latest abi3 batch69 closure landed for runtime/frontdoor + Unicode closure APIs (`Py_{Main,BytesMain,CompileString,Exit,FatalError}`, `PyWrapper_New`, `PyUnicode_{AsCharmapString,AsMBCSString,BuildEncodingMap,DecodeCharmap,DecodeCodePageStateful,DecodeMBCS,DecodeMBCSStateful,DecodeUTF7,DecodeUTF7Stateful,EncodeCodePage,Resize,Translate}`), with symbol + smoke coverage (`tests/abi3_surface.rs::exports_abi3_batch69_symbols`, `tests/extension_smoke.rs::cpython_compat_runtime_unicode_abi_batch69_apis_work`).
  - current abi3 manifest coverage: functions `782/782`, data `143/143` (`perf/abi3_manifest_latest.json`).
  - probe script now supports optional scientific-stack cases (`--include-scientific-stack`) and module-aware local probe mode (`--probe-local-stack`), with direct-mode artifact updates in `perf/numpy_gate_direct_latest.json`.
  - NumPy probe now supports local-install detection mode (`--probe-local-numpy`, `--python-probe-bin`) to separate environment-missing (`NOT_FOUND`) from runtime ABI/substrate failures.
  - direct shared-object imports now fall back from `pyrs_extension_init_v1` to `PyInit_<module>` when CPython-style symbols are present.
  - CPython single-phase init compatibility slice is landed and smoke-covered (`tests/extension_smoke.rs::imports_direct_cpython_style_single_phase_extension`): `PyModule_Create2`, `PyModule_AddObjectRef`, `PyModule_AddIntConstant`, `PyModule_AddStringConstant`, core constructors, `PyErr_*`, and `Py_[X]IncRef/Py_[X]DecRef`.
  - direct CPython compatibility surface now exports the `_multiarray_umath` unresolved symbol set (public `Py*` plus internal `_Py*`) so NumPy reaches module-init execution instead of failing at dynamic-link resolution.
  - direct NumPy bring-up now includes `datetime.datetime_CAPI` capsule registry baseline, `math.trunc`, and expanded CPython-style varargs builders/call helpers via C shim (`build.rs`, `src/vm/capi_variadics.c`).
  - pure-stdlib preference logic now includes `typing` (in addition to `types`) when CPython `Lib` sources are available on `sys.path`.
  - direct NumPy gate checkpoint: base direct-mode probes are green (`numpy_import`, `numpy_ndarray_sum`, `numpy_numerictypes_core`) in `perf/numpy_gate_direct_latest.json`.
  - NumPy import warning cleanup checkpoint: proxy class `__flags__` now reads CPython `tp_flags` for extension-backed types, removing prior `_add_newdocs_scalars` warning spam during `import numpy`.
  - NumPy display checkpoint: `ndarray` REPL/print no longer crashes and now runs through NumPy's native `arrayprint` path (temporary `tolist()` fallback removed); long-tail display parity (spacing/precision/line-break semantics) remains open.
  - root-cause closure for NumPy float-array repr path: `PyObject_SetAttrString` now converts foreign value pointers with `cpython_value_from_ptr_or_proxy` (instead of strict owned-pointer-only conversion), unblocking `_multiarray_umath._populate_finfo_constants` and preventing proxy-placeholder repr fallback.
  - NumPy scalar-format checkpoint: core `np.float64` constructor/`str`/`repr`/`format` parity is now green in direct mode (`0.5` / `np.float64(0.5)` baseline).
  - C varargs parser now covers `PyArg_ParseTupleAndKeywords` token `U` (unicode object) in `src/vm/capi_variadics.c`; batch26 extension smoke includes regression coverage for this token.
  - `PyNumber_Long` reduction-path blocker was closed by (a) stable CPython-pointer reuse for identity-bearing runtime objects across C-API contexts and (b) `int()` fallback through CPython proxy numeric slots (`nb_int` / `nb_index`) for extension-backed scalars.
  - unary operator runtime now falls back to special methods (`__neg__`, `__pos__`, `__invert__`) and CPython proxy numeric slots, with regression coverage (`tests/vm.rs::unary_operators_fall_back_to_special_methods`).
  - lane-B symbol closure advanced with `_PyByteArray_empty_string` data export and additional SciPy loader symbols (`_PyBytes_Join`/`PyBytes_Join`, `_PyDict_Pop`/`PyDict_Pop`/`PyDict_PopString`, `_Py_FatalErrorFunc`).
  - CPython-object ABI substrate was advanced for direct-mode extension init: compat objects now carry CPython-style object/varobject headers, singleton pointers (`Py_None`/`Py_True`/`Py_False`) are returned directly, tuple pointers expose contiguous `ob_item[]` storage, list pointers expose `ob_item`/`allocated` storage, and compat allocations are pinned across init-scoped free/decref churn.
  - CPython exception globals (`PyExc_*`) are now initialized to non-null exported sentinel objects and pointer->exception-type translation is wired in C-API pointer conversion.
  - `PyObject_CallFunction` now has C-side varargs parsing coverage for core formats (`O`/`N`/`s`/`i`/`l`/`k`/`n`/`d`/`f`, plus tuple-wrapped forms) via `src/vm/capi_variadics.c`.
  - C-side varargs handling now also covers `PyArg_ParseTuple`, `PyArg_ParseTupleAndKeywords`, `PyObject_CallFunctionObjArgs`, `PyObject_CallMethod`, `PyEval_CallFunction`, `PyEval_CallMethod`, `PyBytes_FromFormat`, and `PyBytes_FromFormatV` in `src/vm/capi_variadics.c`.
  - `PyTypeObject` compat layout has been expanded through allocation/init/new/call slots, `PyType_Ready` now seeds baseline inherited slots, and `PyType_Type.tp_call` now routes through a CPython-style `tp_new`/`tp_init` call bridge.
  - extension init now caches first per-module dynamic-init failure (`extension_init_failures`) so repeated import retries report the original `Py_mod_exec` blocker instead of masking it behind reentry noise.
  - immediate NumPy priority is expansion beyond base NumPy gates into scientific-stack probes while keeping base direct-mode gates green (`perf/numpy_gate_direct_latest.json`).
  - current optional scientific-stack gate remains red (`scipy_import`, `pandas_*`, `matplotlib_*`); primary SciPy blocker has moved to runtime semantics (`KeyError ... PyDict_DelItem key not found` during `_cyutility` init) after the latest symbol-closure slice.
  - extension slot tracing is now available via `PYRS_TRACE_EXT_SLOTS=1` for `Py_mod_create` / `Py_mod_exec` debugging in direct `PyInit_*` mode.
  - CPython-ABI bridge runtime/env path has been removed; scientific-stack gating is now direct-mode only (`perf/numpy_gate_direct_latest.json`).
  - when `VIRTUAL_ENV` is set, runtime now sets `sys.prefix`/`sys.exec_prefix` to the venv root so startup `site` handling picks up venv `site-packages`.
- Newly landed parity checkpoints:
  - `math.gcd()` baseline (unblocks `fractions` common path).
  - `threading.Condition.__enter__/__exit__` baseline.
  - `datetime.date/datetime.strftime()` baseline.
  - `math.trunc()` baseline.
  - `_operator._compare_digest` baseline and `_operator` module registration.
  - `collections.deque` class surface (`__init__`, `append*`, `pop*`, `extend*`, `clear`, `__len__`, `__iter__`) wired into module bootstrap.
  - `bytes` / `bytearray` constructor VM paths now accept generator/iterator/iterable payloads and explicit `encoding`/`errors` argument forms.
  - `datetime.date/datetime` gained `toordinal`, `weekday`, `isoweekday`; `datetime.timezone` baseline symbol added for stdlib import-chain compatibility.
  - `datetime.datetime.fromtimestamp` + `datetime.astimezone` fixed-offset baseline landed, including `%z` formatting in `strftime`.
  - synthetic exception-class materialization for `Value::ExceptionType` bases now builds CPython-style exception ancestry and wires `ExceptionTypeInit` to unblock stdlib exception subclasses.
  - `_sre` pattern object gained `split`; class/instance `__doc__` fallback parity tightened for stdlib object-model paths.
  - internal-call exception propagation now treats caller `active_exception` deltas as propagated failures (prevents false-success stack pops/underflow in descriptor/property call paths).
  - `str.join` now accepts `str` subclass instances via backing-string extraction.
  - `super(...).__init__` for synthetic builtin `list`/`dict` bases now resolves to native container initializers (`list.__init__`, `dict.__init__`) instead of falling through to `object.__init__`.
  - codec normalization now accepts core CPython aliases (`us-ascii`, `iso-8859-1`, etc.) in both codec paths and `bytes(..., encoding=...)` construction.
  - regex `Match` now supports subscript group aliasing (`m[0]`, `m[1]`, ...), including module-wrapper dispatch needed by CPython email header folding paths.
  - instance subscription now delegates to tuple/str backing for synthetic builtin subclasses; `str()` now returns backing text for str-backed instances.
  - targeted CPython `email` smoke (`EmailMessage` header/content fold + `as_string`) is green locally; extended matrix artifact refresh pending.
  - numeric parity checkpoint: `int` now exposes `numerator`/`denominator`/`real`/`imag`; `sum()` now uses binary add runtime fallback; `float()` now honors `__float__`; primitive numeric instances satisfy `numbers` ABC checks used by `fractions`/`statistics`.
  - regex parity checkpoint: `_sre` now recognizes CPython `_pydecimal` parser pattern structure with named-group captures (`sign`/`int`/`frac`/`exp`/`signal`/`diag`) and matching `groupindex` mapping.
  - import binding parity: `import` / `__import__` now bind the canonical `sys.modules[name]` module object when module code replaces the entry during execution.
  - pure `decimal` preference: when CPython `Lib/decimal.py` is available on `sys.path`, builtin bootstrap `decimal` is unloaded and pure `decimal`/`_pydecimal` is used.
  - enum shim retirement: `shims/enum.py` is removed; enum import now resolves only through CPython `Lib/enum.py` path.
  - exception hierarchy parity: `LookupError`/`IndexError`/`KeyError`, `ArithmeticError` family, warning family, pickle error family, and several core parents now follow CPython ancestry.
  - exception-match resilience: `except` matching now falls back to active exception state when stack operands are polluted by import-failure edges.
  - heavy CPython-stdlib VM tests now run on dedicated 32MB stack threads for stability (`import_http_client_runs_package_init_first`, `pyio_fileio_del_namedexpr_does_not_leak_bound_method_or_pin_cycle`, `c_pickler_newobj_ex_argument_type_errors_match_cpython_protocols_2_through_5`, `pickle_newobj_generic_matrix_from_pickletester_roundtrips`, `prefers_cpython_pkgutil_and_resources_over_local_shims_when_stdlib_is_available`, `pkgutil_resolve_name_accepts_module_only_target`).
  - additional pickle-heavy VM tests now run on dedicated 32MB stack threads (`pickle_protocol4_dict_chunking_emits_multiple_setitems_for_large_dicts`, `pickle_slot_list_roundtrip_preserves_slots_and_dynamic_dict_attrs`, `with_assert_raises_handles_missing_attr_without_stack_underflow`) to avoid debug-thread stack overflows during deep stdlib import chains.
  - `tests/vm.rs::executes_pure_decimal_getcontext_and_addition` now runs on a dedicated 32MB stack thread to avoid debug-thread stack overflow during pure-stdlib decimal import/parse recursion.
  - stack-heavy pure-stdlib tests now also pin dedicated 32MB stacks where needed (`tests/vm.rs::json_import_prefers_cpython_pure_module_when_lib_path_is_added_by_default`, `tests/vm.rs::statistics_mean_supports_basic_int_dataset`, `tests/vm.rs::argparse_parse_args_accepts_explicit_positional_list`, `tests/vm.rs::csv_dictreader_list_exhaustion_stops_cleanly`, `tests/hashlib_native.rs::hashlib_module_uses_native_md5_and_sha256_backends`).
  - CPython harness import suites (`runs_cpython_language_suite`, `runs_cpython_import_suite`) now run on dedicated 32MB stack threads to avoid debug-thread stack overflows during deep import chains.
  - strict stdlib lane remains green after enum-shim retirement (`PYRS_RUN_STRICT_STDLIB=1 cargo test -q --test cpython_harness runs_cpython_strict_stdlib_suite`).
  - builtin callable/type object repr/str parity checkpoint: `type(7)`/`str(type)` render CPython-style `<class '...'>`, and builtin callables now render `<built-in function ...>` in both `repr(...)` and `str(...)`.
  - `sys.is_finalizing()` baseline is now wired (returns `False`), including VM regression coverage (`tests/vm.rs::exposes_sys_is_finalizing_helper`).
  - import-exception parity checkpoint: `ImportError` / `ModuleNotFoundError` now populate `msg`/`name`/`path` consistently across constructor and runtime-conversion paths (required by NumPy import error-handling paths).
  - `_io.open` now preserves raw bytes paths when dispatching opener callbacks (no lossy bytes->str conversion), with regression coverage in `tests/vm.rs::io_open_passes_bytes_path_to_opener_without_lossy_conversion`.
  - `_sqlite3.connect` / `Connection.__init__` now preserve raw bytes/bytearray database paths when calling `sqlite3_open_v2` (no lossy UTF-8 replacement in native handoff).
  - `_sqlite3.connect` now accepts path-like (`__fspath__`) database arguments, matching CPython DB-API expectations.
  - `_sqlite3.Row` parity advanced: equality now compares description + row payload (not identity), and `issubclass(sqlite3.Row, collections.abc.Sequence)` / `isinstance(row, Sequence)` now pass in the runtime.
  - `_sqlite3` thread-affinity guard is now tracked in native connection state and enforced on connection operations when `check_same_thread=True` (default), matching CPython policy.
  - `_sqlite3` thread-affinity checks now apply to cursor operations (`close`/`fetch*`/`set*size` and other cursor methods), matching CPython `test_dbapi.ThreadTests` expectations.
  - `_sqlite3.Connection` now exposes `set_trace_callback`, `set_authorizer`, `set_progress_handler`, `create_collation`, `create_window_function`, and `iterdump`; `iterdump()` delegates to CPython `Lib/sqlite3/dump.py::_iterdump`.
  - `_sqlite3.Connection.backup()` now uses SQLite backup APIs with CPython-like semantics for target validation, `pages`, `progress`, `name`, and `sleep`.
  - `_sqlite3.autocommit` now supports `True` / `False` / `sqlite3.LEGACY_TRANSACTION_CONTROL` with transition semantics aligned to CPython (`BEGIN`/`COMMIT`/`ROLLBACK` behavior, context-manager mode behavior, and `close()` implicit rollback for disabled mode).
  - `_sqlite3` callback surfaces (`set_trace_callback`, `set_authorizer`, `set_progress_handler`, `create_collation`) now route through native sqlite callback APIs, including callback-traceback/unraisable handling and deprecated-keyword warning behavior expected by CPython 3.14 tests.
  - `sqlite3` DB-API failfast probe (`Lib/test/test_sqlite3/test_dbapi.py`) is now green locally.
  - strict stdlib harness suite now includes `test/test_sqlite3/test_dbapi.py`, `test/test_sqlite3/test_backup.py`, `test/test_sqlite3/test_factory.py`, `test/test_sqlite3/test_dump.py`, `test/test_sqlite3/test_transactions.py`, and `test/test_sqlite3/test_hooks.py` and stays green (`PYRS_RUN_STRICT_STDLIB=1 cargo test -q --test cpython_harness runs_cpython_strict_stdlib_suite`).
  - `_sqlite3` factory parity checkpoint: `connect(factory=ConnectionSubclass)` now relays kwargs and preserves `Base Connection.__init__ not called.` behavior for defective subclasses; `Connection.cursor(factory=...)` now follows CPython callable/class dispatch semantics, including positional/keyword `factory` handling, callable return-type validation, and native `Cursor.__init__` substrate wiring used by cursor subclasses.
  - `_sqlite3.Row` parity checkpoint: `dict(Row)` mapping conversion now follows mapping-protocol behavior, ordering comparisons now raise `TypeError` (no false ordering via equality fallback), and row hash parity (`hash(description) ^ hash(data)`) is wired via native `Row.__hash__`.
  - REPL UX checkpoint: history-hint ghost text now renders in lighter italic gray, `Esc` is explicitly bound to dismiss active completion/suggestion state, `%timeit` magic (with `-n/--number` and `-r/--repeat`) is implemented, completion indexing now traverses deeper path chains (depth 6) with primitive-type method-name surfaces, and `PYRS_REPL_THEME=auto|dark|light` selects highlighter/hint palette.
  - runtime threading identity emulation now assigns per-start synthetic ids for `_thread.start_new_thread` and `threading.Thread.start` target execution; `threading.get_ident()` reports those ids inside spawned targets.
  - object-model parity checkpoint: `object.__format__` now follows CPython semantics (empty spec -> `str(self)`, non-empty spec -> `TypeError`), unblocking unittest subtest rendering paths that rely on `str.format`.
  - builtin `threading` module now exposes `_dangling` registry baseline required by CPython test/support threading helpers.
  - iterator protocol parity for native iterators now exposes `__iter__`/`__next__` where required (including `itertools.count`), unblocking `concurrent.futures` smoke path.
  - `bytes.lstrip`/`bytes.strip` native methods are now implemented (plus metadata/dispatch wiring), closing `gzip.decompress` common smoke path.
  - `threading.Semaphore`/`BoundedSemaphore` bound semantics were corrected (`Semaphore` unbounded by default; `BoundedSemaphore` enforces initial bound), removing `Semaphore released too many times` failures in threadpool shutdown flows.
  - range-iterator parity checkpoint: `iterator_next_value()` now advances `IteratorKind::RangeObject`, fixing `bytes(range(...))` / `bytearray(range(...))` constructor semantics.
  - loop-compiler parity checkpoint: `for`/`while` loop contexts no longer leak into `else` blocks; `continue` in loop-`else` now correctly targets enclosing loops (or raises compile error if no enclosing loop), and CPython `re._parser` no longer underflows at `FOR_ITER`.
  - GC control checkpoint: `gc` module now exposes `get_threshold`, `set_threshold`, and `get_count` (in addition to `collect`/`enable`/`disable`/`isenabled`); automatic cycle collection is threshold-driven after explicit threshold configuration and uses parity-safe guardrails.
  - dispatch/cache checkpoint: `CALL_FUNCTION` site quickening metadata now includes zero-arg and two-arg direct function lanes; `LOAD_ATTR` instance cache now covers plain value attributes in addition to function/descriptor forms.
  - perf checkpoint: terminal arithmetic return fusion is active for `BinaryAdd` / `BinarySub` / `BinaryMul` / `BinaryDiv` / `BinaryFloorDiv` / `BinaryMod` on simple no-cells frames, and release builds use `lto = "fat"`; local gate currently measures `fib(29)` at ~`0.13s` user (`scripts/bench_fib_gate.sh`).
  - startup/import checkpoint: import resolver state now tracks `sys.path`/`meta_path`/`path_hooks` signatures to avoid repeated default-finder scans, default CPython stdlib auto-detection now selects a single canonical fallback root (instead of loading every discovered system path), sourceless `__pycache__/*.cpython-314.pyc` module/package imports are accepted when source files are absent, and pyc fallback diagnostics report exact translate/load failure reasons when `PYRS_IMPORT_PERF_VERBOSE=1`.
  - pyc translator checkpoint: marshal reader/writer now supports set/frozenset constants (`<`/`>`); `BINARY_OP` mapping covers the full CPython 3.14 arg table (including bitwise/shift/matmul and inplace variants); f-string translation paths handle `CONVERT_VALUE`/`FORMAT_SIMPLE`/`FORMAT_WITH_SPEC` plus `BUILD_STRING`; and CPython opcode coverage now includes `DICT_MERGE`, `COPY`, `SWAP`, masked `COMPARE_OP` decoding, `LOAD_SPECIAL`, `MATCH_CLASS`/`MATCH_KEYS`/`MATCH_MAPPING`/`MATCH_SEQUENCE`, `GET_LEN`, `BUILD_TEMPLATE`/`BUILD_INTERPOLATION`, and expanded intrinsic runtime support (`CALL_INTRINSIC_1`: `2/3/5/6/7/8/9/10/11`, `CALL_INTRINSIC_2`: `1/2/3/4/5`).
  - translated-`.pyc` exception-table runtime checkpoint: VM now executes CPython 3.14 exception-table handlers (`PUSH_EXC_INFO`/`POP_EXCEPT`/`WITH_EXCEPT_START`/`RERAISE`/`CHECK_EXC_MATCH`) with table-driven unwind dispatch and `RERAISE` lasti handling; translator jump-target math is corrected for 3.14 control-flow forms (`POP_JUMP_IF_*`, `SEND`, backward jumps), and CPython `CALL` stack layout (`callable`, `self_or_null`) is aligned for translated opcode paths.
  - opcode stack-layout checkpoint: `DICT_UPDATE`/`DICT_MERGE` and list stack-target lookup now follow CPython operand layout (`dict/list, unused[oparg-1], update -- ...`) after operand pop, removing false `stack underflow` errors on stdlib import paths (including `xml.etree.ElementTree`).
  - startup pyc-preference note: exception-table execution baseline is now active for translated `.pyc`; with `PYRS_IMPORT_PREFER_PYC=1`, `import site` no longer depends on source fallback for this gap. Remaining `.pyc` work is long-tail opcode/state parity closure.
  - startup benchmark checkpoint: `scripts/bench_startup_gate.sh` now uses wall-clock timing (`python3` `perf_counter`) instead of coarse `/usr/bin/time -p` user-time quantization; latest local run (`20` iterations, warmup `1`) is `pass(site)=0.0097s`, `pass(-S)=0.0055s`, `import-bundle=0.0663s`.
  - local validation checkpoint: `cargo test -q --test pyc_translate`, `cargo test -q --test pyc_exec`, `cargo test -q --test cpython_harness runs_cpython_import_suite`, and `cargo test -q --test cpython_harness runs_cpython_language_suite` are green in this wave.
  - local shim retirement checkpoint: `shims/pyexpat.py`, `shims/pkgutil.py`, and `shims/importlib/resources.py` are removed; native runtime fallbacks now cover `pyexpat` and `pkgutil`, and `importlib.resources` resolves only through CPython stdlib when available.
  - function annotation parity checkpoint: function `__annotate__` now exposes a callable default path (instead of `None`), restoring CPython stdlib `functools.singledispatch` plain `@register` annotation flow used by `importlib.resources`.
  - anti-scaffolding enforcement checkpoint: `scripts/audit_scaffolding.py` now gates shim/allowlist/no-op drift locally and in CI integrity lane (retired shim-path reference detection in runtime/test/CI code, `_ctypes`-only shim allowlist lock, no-op inventory sync).
  - production Python-level no-op closure checkpoint: all production-facing symbols in `docs/NOOP_BUILTIN_CLASSIFICATION.md` are now real implementations (`object.__init_subclass__`, `sys.audit`, `sys.breakpointhook`/`__breakpointhook__`, `sys.unraisablehook`/`__unraisablehook__`, `sys._clear_type_descriptors`, and `sys.monitoring.{get_tool,use_tool_id,clear_tool_id,free_tool_id,register_callback,set_events,set_local_events,restart_events}`), `sys.monitoring.events` constants now follow CPython 3.14 values, and `docs/NOOP_BUILTIN_INVENTORY.txt` is now test-only (`26` entries).
  - monitoring API parity follow-up: `sys.monitoring.get_events` and `sys.monitoring.get_local_events` are now implemented with CPython-like validation/order behavior, and regression coverage now checks normalized BRANCH event expansion and `code object` error precedence for `set_local_events`.
  - audit hook parity checkpoint: `sys.addaudithook` is now implemented with CPython suppression/propagation behavior (`Exception`-derived add failures suppressed, `BaseException` propagated), `sys.audit` dispatches `(event, args_tuple)` through interpreter hooks (including dynamic hook-add behavior during active dispatch), and C-API `PySys_AuditTuple`/`PySys_Audit` now route through the same hook-dispatch path with tuple payload propagation (`tests/extension_smoke.rs::cpython_compat_sys_abi_batch44_apis_work` now asserts C-API payload visibility).
  - C-API unraisable parity checkpoint: `PyErr_WriteUnraisable` now consumes current C-API error state, routes through `sys.unraisablehook`, and clears active error state afterwards; extension smoke now validates `ValueError` hook payload + cleared post-call error indicator in batch44.
  - REPL checkpoint: no-arg CLI path now starts an interactive `reedline` REPL (`RSPYTHON` banner, Python syntax highlighting, Tab=4-space indentation, Shift-Tab/Ctrl-Space completion menu, dotted member completion, multiline, `%time` one-shot timing, `:paste`/`:timing`/`:reset` controls, optional startup script `~/.pyrsrc`/`PYRS_REPL_INIT`), while non-interactive no-arg runs consume stdin as script input (CPython-like `python < file.py` behavior).
  - REPL expression echo now routes through Python-level `repr(...)` protocol (instead of raw `format_repr` fallback), so extension-backed objects display CPython-style repr text in interactive output.
  - builtin type-repr checkpoint: `repr(type(7))`, `repr(int)`, and `repr(type)` now match CPython class-style formatting (`<class 'int'>`, `<class 'type'>`) instead of generic `<builtin>`.
  - extension crash-stability checkpoint: re-entrant C-API callback paths now pass `ModuleCapiContext*` via `std::ptr::addr_of_mut!(...)` (not `&mut ... as *mut ...`) to avoid aliasing-UB under callback re-entry; repeated local extension-smoke loops are stable.
  - error-state safety checkpoint: `error_set(...)` / `PyErr_SetString(...)` now prioritize stable message+type indicator state over synthetic string-pointer `pvalue` materialization to prevent crashy pseudo-unicode pointer paths; full CPython non-null error-value pointer semantics are tracked in `docs/STUB_ACCOUNTING.md`.
  - NumPy formatting blocker investigation checkpoint: proxied `ndarray.__repr__` currently falls through native attr lookup and maps to runtime fallback `<bound method Repr>`; `lookup_type_attr_via_tp_dict` confirms external `numpy.ndarray` `tp_dict` miss for `__repr__` and base `object` lookup with `tp_dict == NULL`, so slot-wrapper/type-ready publication closure remains required.
- Extended probe remaining red modules: none (`50/50` smoke green).

## Execution Policy
- CPython behavior is the source of truth:
  - `Modules/*.c`
  - `Objects/*.c`
  - `Lib/*.py`
- For re-entrant extension callback paths, never pass context pointers via `&mut ctx as *mut ...`; use `std::ptr::addr_of_mut!(ctx)` and raw-pointer plumbing to avoid aliasing UB.
- Sequence Milestone 13 work as native-core-first:
  1. Native/runtime substrate closure (`_io`, `_csv`, `_sre`, `_pickle`, object protocol)
  2. Pure-stdlib strict-lane expansion/closure
- Prefer official CPython pure-Python stdlib implementations where feasible.
- Keep native handlers as substrate/accelerator layers, not replacement semantics.
- When citing Python docs, always pin URLs to `https://docs.python.org/3.14/...` (do not use unversioned `.../3/...` links).
- Local shim policy:
  - CPython `Lib/enum.py` path is now the default.
  - local `enum` shim has been retired (`shims/enum.py` removed); enum behavior now always follows CPython `Lib/enum.py` when stdlib is present.
  - local shim fallback is now `_ctypes`-only; fallback is enabled by default and can be disabled with `PYRS_DISABLE_LOCAL_SHIMS=1`.
  - `pkgutil` and `pyexpat` stdlib-less fallback now use native runtime modules (no filesystem shim).
  - CPython enum probe regression: `tests/vm.rs::cpython_enum_path_supports_member_value_and_name`.
- Keep docs updated in the same checkpoint as behavior changes.
- Keep worktrees clean; commit small focused checkpoints.
- Start each user-facing progress update with a brief executive summary focused on outcomes (what was accomplished/unlocked/fixed/enabled), not just a checklist of completed steps.
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
- Optimization phase-1 checkpoint is complete.
- Functional Milestone 13 closure is active with benchmark regression protection.
- Canonical benchmark gates:
  - `scripts/bench_fib_gate.sh 5`
  - `scripts/bench_dispatch_hotpath.sh 5`
  - `scripts/bench_dict_backend.sh 5`
  - `scripts/bench_startup_gate.sh 7`
- All optimization work must update `docs/OPTIMIZATION_BACKLOG.md` in the same checkpoint.

## Canonical Documents
- Docs index and ownership map: `docs/README.md`
- Milestones and sequencing: `docs/ROADMAP.md`
- Production blockers and release criteria: `docs/PRODUCTION_READINESS.md`
- Beta release checkpoint plan: `docs/RELEASE_PLAN_BETA.md`
- Extension ecosystem architecture plan: `docs/EXTENSION_ECOSYSTEM_DESIGN.md`
- Extension capability matrix: `docs/EXTENSION_CAPABILITY_MATRIX.md`
- Extension packaging/build contract: `docs/EXTENSION_PACKAGING_CONTRACT.md`
- Extension C-API v1 slice: `docs/EXTENSION_CAPI_V1.md`
- NumPy bring-up tracker: `docs/NUMPY_BRINGUP_GATE.md`
- Partial/stub ledger: `docs/STUB_ACCOUNTING.md`
- Top stdlib common-usecase tracker: `docs/STDLIB_COMMON_USECASE_CHECKLIST.md`
- Object-model parity audit: `docs/OBJECT_MODEL_AUDIT.md`
- Pure-stdlib migration policy: `docs/STDLIB_MIGRATION_PLAN.md`
- Engineering gates: `docs/ENGINEERING_GATES.md`
- Algorithmic/semantic audit backlog: `docs/ALGO_AUDIT_BACKLOG.md`
- Compatibility matrix: `docs/COMPATIBILITY.md`
- VM architecture map: `docs/VM_ARCHITECTURE_MAP.md`
- Optimization execution plan: `docs/OPTIMIZATION_PLAN.md`
- Optimization backlog/status: `docs/OPTIMIZATION_BACKLOG.md`
- Builtin parity gate and policy: `docs/BUILTIN_PARITY.md`, `docs/BUILTIN_OPTIMIZATION_POLICY.md`
- Unicode-name table provenance: `docs/UNICODE_NAME_DATA.md`

## Reference Artifacts
- Milestone 12 closure report: `docs/MILESTONE_12_BACKLOG.md`
- Dict backend CPython mapping: `docs/DICT_BACKEND_CPYTHON_MAPPING.md`
- Dict backend benchmark snapshot: `docs/DICT_BACKEND_BENCHMARK.md`
- Clone audit artifacts: `docs/CLONE_BASELINE.txt`, `docs/CLONE_AUDIT.md`
- No-op inventory snapshot: `docs/NOOP_BUILTIN_INVENTORY.txt`
