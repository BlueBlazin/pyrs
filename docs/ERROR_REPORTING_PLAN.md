# Error Reporting Parity Plan (PEP 626 + PEP 657)

Status: in progress (started 2026-02-22).

## Progress Checkpoint (2026-02-23)

- Phase 1 complete:
  - exception-constructor kwargs parity landed for `AttributeError`, `NameError`,
    `ImportError`, and `ModuleNotFoundError`.
  - invalid kwargs now raise typed `TypeError` with CPython-style messages.
- Phase 2 mostly complete:
  - source text cache wired for file, `-c`, REPL, import, `exec`, `eval`, and `compile`.
  - traceback output now includes CPython-style file/line/function rows plus source and caret.
  - exception chaining now renders separate traceback blocks for `__context__` / `__cause__`
    instead of flattening into message-only chain text.
  - caret fallback now infers identifier spans when end-columns are unavailable
    (e.g. `NameError` on `foo` highlights `^^^`), and suppresses statement-keyword carets.
  - CLI/REPL parse-error output now emits `SyntaxError`-style diagnostics with file/line/source/caret
    formatting instead of raw parser offset messages.
  - parser diagnostics now normalize high-noise parser-internal wording to CPython-style
    user-facing classes/messages where applicable:
    - `expected ...`/unexpected-token parser diagnostics -> `SyntaxError: invalid syntax`,
    - indent/dedent parser diagnostics -> `IndentationError` variants,
    - unmatched/mismatched delimiter detection -> CPython-style messages
      (`unmatched ']'`, `closing parenthesis ... does not match ...`),
    - unclosed delimiter detection -> `SyntaxError: '<delimiter>' was never closed`,
    - colon-in-unclosed `(`/`[` headers now resolve to CPython-style
      `SyntaxError: invalid syntax` with colon-position caret (e.g. `class A(:`, `def f(:`, `[1:`),
    - unterminated triple-quoted string detection -> CPython-style
      `unterminated triple-quoted string literal (detected at line N)`.
- Phase 3 complete:
  - location metadata upgraded to `start+end` ranges in bytecode location model.
  - default source-compiler locations now mark end columns unknown until explicit ranges are
    available, avoiding misleading single-character highlights.
- Phase 4 complete:
  - CPython 3.14 `co_linetable` decoding added for translated `.pyc` bytecode.
- Phase 5 in progress:
  - regression coverage added for constructor kwargs parity, linetable-range decoding, and
    traceback no-rewrap behavior.
  - differential traceback-shape gates now compare pyrs vs CPython for both
    `__context__` and `__cause__` chain formatting, normalized to ignore source/caret
    rendering differences while preserving traceback-block and delimiter parity.
  - differential syntax-error gate now checks CPython-shape parity for compile-time failures
    (`File "<string>", line ...`, source line, caret row, `SyntaxError:` prefix).
  - differential syntax gates now also cover invalid-syntax span parity, unclosed-delimiter shape,
    indentation-error shape, unmatched/mismatched closing delimiters, and unterminated
    triple-quoted strings against CPython output.
  - differential `.pyc` runtime traceback gates now compare pyrs vs CPython for:
    - NameError identifier-caret span parity from compiled bytecode execution.
    - `__context__` chain traceback-shape parity from compiled bytecode execution.
    - `raise ... from None` suppressed-context traceback-shape parity.
    - direct-cause (`raise ... from exc`) traceback-shape parity from compiled bytecode.
  - differential traceback gates now cover mixed nested chains (`__cause__` inside `__context__`)
    for both source execution and `.pyc` execution.
  - traceback capture now uses `reraise_lasti_override` when available so reraised exceptions
    in exception-table cleanup paths preserve the original fault line (avoids line-0 fallback
    in `.pyc` context-chain traces).
  - reraised-exception traceback preservation is now enforced in VM unwind paths:
    - bare `raise` and `RERAISE` now preserve existing traceback frame stacks instead of
      re-rooting at synthetic cleanup/handler lines.
    - compiler-generated rethrow paths in `with`/`try-finally` cleanup now emit `RERAISE`
      instead of `Raise 1`, aligning source-compiled cleanup behavior with CPython.
  - final exception-line rendering now follows CPython `KeyError` display semantics when args are
    available (single-arg `KeyError` displays `repr(arg)` in traceback footer).
  - runtime `str(KeyError(<arg>))` now follows CPython single-arg display semantics (`repr(arg)`).
  - compile-time semantic syntax checks now raise CPython-style `SyntaxError` diagnostics (instead
    of generic compile errors) for:
    - `'return' outside function`,
    - `'break' outside loop`,
    - `'continue' not properly in loop`,
    - `'await' outside function`,
    - `'yield' outside function`,
    - `'yield from' outside function`,
    - `'return' with value in async generator`,
    - `global`/`nonlocal` declaration-order and scope diagnostics
      (`used prior`, `assigned before`, module-level `nonlocal`, missing nonlocal binding,
      parameter/global conflict, parameter/nonlocal conflict, nonlocal/global conflict).
  - CLI/REPL compile diagnostics now render in `SyntaxError` shape; `-c` mode follows CPython by
    omitting source+caret for semantic compile errors, while file/stdin paths still include line
    source and caret when span data is available.
  - syntax-error source rendering now mirrors CPython indentation presentation:
    - leading indentation is normalized in displayed source line,
    - caret offsets are adjusted accordingly,
    - top-level `unexpected indent` continues to omit caret.
  - differential gates added for semantic compile-error parity against CPython:
    - return/break/continue outside valid scope,
    - await/yield/yield-from outside function scope,
    - async-generator return-with-value,
    - global/nonlocal declaration-order and scope errors.
  - indentation diagnostics now include CPython-style parity for:
    - top-level `unexpected indent` (no caret line),
    - `unindent does not match any outer indentation level` with end-of-line caret.
  - differential traceback gates now also cover reraised traceback line-fidelity parity for both
    source and `.pyc` execution paths:
    - `differential_traceback_reraise_preserves_original_fault_line`
    - `differential_pyc_traceback_reraise_preserves_original_fault_line`.
  - explicit `raise exc` (non-bare rethrow) now preserves existing traceback chains while adding
    the current raise site, matching CPython ordering for source and `.pyc` execution:
    - `differential_traceback_raise_exc_keeps_original_traceback_chain`
    - `differential_pyc_traceback_raise_exc_keeps_original_traceback_chain`.
  - exception `__traceback__` is now a real runtime object (instead of always `None`) with a
    CPython-compatible surface needed by stdlib traceback formatting paths:
    - linked `tb_next` chain,
    - `tb_lineno` / `tb_lasti` / `tb_frame` fields,
    - `tb_frame.f_code` + basic frame metadata for traceback walkers.
  - code-object location iterators are now exposed:
    - `code.co_positions()` returns per-instruction `(start_line, end_line, start_col, end_col)`
      tuples,
    - `code.co_lines()` returns per-instruction `(start_offset, end_offset, line)` tuples.
  - traceback instruction-index propagation was tightened:
    - exception traceback frames now retain `lasti` values,
    - traceback objects publish per-frame `tb_lasti` from captured instruction offsets,
    - synthetic traceback-frame code objects now allocate enough location rows to keep
      `co_positions` lookup at `tb_lasti // 2` in-range.
  - `compile(..., flags=_ast.PyCF_ONLY_AST)` now returns `_ast` node objects for `exec` and
    `eval` modes in the core traceback-heurstic shapes (`Module`/`Assign`/`Return`/`Expr` and
    `Call`/`Name` expression surfaces with location attrs).
  - `_ast` bootstrap class surface now includes missing statement roots used in those flows
    (`Module`, `Assign`, `Return`, `Expr`, `Pass`).
  - `_ast` metadata parity pass landed:
    - AST classes now expose `_fields`, `__match_args__`, and `_attributes` for positional
      class-pattern matching compatibility.
    - AST conversion now maps key expression forms used by traceback anchor extraction:
      `BinOp`, `Compare`, `UnaryOp`, `BoolOp`, `IfExp`, `NamedExpr`, `Slice`.
    - added required operator/comparator classes in `_ast` bootstrap inventory:
      `And`, `Or`, `Not`, `Is`, `IsNot`, `In`, `NotIn` (plus `BoolOp`, `IfExp`, `NamedExpr`).
  - `_ast` class-hierarchy wiring now maps core abstract and concrete node families to CPython-like
    inheritance roots (e.g. `expr`, `stmt`, `operator`, `cmpop`) so `isinstance(...)` and
    positional pattern matching semantics align with stdlib traceback/ast consumers.
  - AST compile conversion now includes a broader statement subset used by stdlib error/traceback
    paths and AST consumers: `Delete`, `Raise`, `Assert`, `If`, `While`, `For`/`AsyncFor`,
    `With`/`AsyncWith`, `Try`/`TryStar`, `Import`/`ImportFrom`, `Global`/`Nonlocal`, and
    loop controls (`Break`/`Continue`), with helper-node materialization for
    `alias`, `withitem`, and `ExceptHandler`.
  - `compile(..., PyCF_ONLY_AST)` conversion now includes function/class definition surfaces:
    - `FunctionDef` / `AsyncFunctionDef` / `ClassDef`,
    - `arguments` / `arg`,
    - `type_param` / `TypeVar` / `ParamSpec` / `TypeVarTuple`,
    - decorator-list propagation through `StmtKind::Decorated`.
  - `compile(..., PyCF_ONLY_AST)` statement conversion now includes assignment variants:
    - `AugAssign` with operator node materialization from augmented-op enums,
    - `AnnAssign` with CPython-shaped `simple` field behavior
      (`1` for name targets, `0` otherwise).
  - `compile(..., PyCF_ONLY_AST)` now includes structural pattern-matching AST conversion:
    - statement node: `Match` + `match_case`,
    - pattern nodes: `pattern`, `MatchValue`, `MatchSingleton`, `MatchSequence`,
      `MatchMapping`, `MatchClass`, `MatchStar`, `MatchAs`, `MatchOr`,
    - conversion covers wildcard/capture/value/constant/sequence/mapping/class/or/as/star
      pattern families from parser AST.
  - `compile(..., PyCF_ONLY_AST)` expression conversion now covers previously-fallback families:
    - `Lambda`,
    - `Await`,
    - `ListComp` / `DictComp` / `GeneratorExp`,
    - `Yield` / `YieldFrom`,
    - helper-node `comprehension` materialization for generator clauses.
  - location-attribute propagation was tightened for AST helper nodes that expose location attrs:
    - `alias`, `keyword`, and `ExceptHandler` node conversion now materializes location fields
      instead of leaving `_attributes` unbound.
  - `_ast` metadata/hierarchy parity was extended for those nodes:
    - class metadata now includes CPython-shaped `_fields`/`_attributes` for
      `FunctionDef`, `AsyncFunctionDef`, `ClassDef`, `arguments`, `arg`,
      `type_param`, `TypeVar`, `ParamSpec`, and `TypeVarTuple`,
    - hierarchy wiring now maps `FunctionDef`/`AsyncFunctionDef`/`ClassDef -> stmt`,
      `arguments`/`arg`/`type_param -> AST`, and
      `TypeVar`/`ParamSpec`/`TypeVarTuple -> type_param`,
    - corrected `withitem._attributes` to CPython parity (empty tuple).
    - hierarchy now also maps `Match -> stmt`, `match_case -> AST`,
      `pattern -> AST`, and concrete `Match*` pattern classes -> `pattern`.
  - native codec keyword-path parity was tightened for traceback stdlib flows:
    - `str.encode`, `str.decode`, and `bytes.decode` now accept `encoding=`/`errors=` kwargs and
      enforce duplicate/unexpected-keyword checks.
  - `BaseException.with_traceback(...)` and direct `__traceback__` assignment now parse/apply
    traceback chains with CPython type contracts (`traceback` or `None`).
  - differential traceback gates now cover `with_traceback(...)` parity for source and `.pyc`:
    - `differential_traceback_with_traceback_restores_supplied_chain`
    - `differential_pyc_traceback_with_traceback_restores_supplied_chain`.
  - VM regressions added for new location API surface:
    - `code_object_co_positions_and_co_lines_iterators_have_expected_shape`
    - `traceback_helpers_can_read_exception_traceback_attr`.
    - `traceback_tb_lasti_maps_into_code_positions`.
  - VM regression added for AST-compile surface:
    - `compile_only_ast_returns_assign_and_call_shape`.
  - VM regressions added for AST class-pattern and expression-shape coverage:
    - `compile_only_ast_supports_positional_match_patterns`
    - `compile_only_ast_covers_binop_compare_and_slice_shapes`.
    - `compile_only_ast_honors_core_ast_hierarchy`
    - `compile_only_ast_honors_operator_hierarchy`.
    - `compile_only_ast_covers_common_statement_nodes`.
  - differential CPython gates now verify key `PyCF_ONLY_AST` parity surfaces:
    - assign-node `_fields` / `__match_args__` and abstract-base membership parity,
    - operator/comparator/unary abstract-family membership parity.
    - function/class/type-param node-shape and hierarchy parity
      (`differential_compile_only_ast_function_class_and_type_param_parity`).
    - augmented/annotated assignment node parity
      (`differential_compile_only_ast_augassign_and_annassign_parity`).
    - match/pattern node-shape and hierarchy parity
      (`differential_compile_only_ast_match_and_pattern_parity`).
  - VM regressions added for expanded AST-conversion coverage:
    - `compile_only_ast_covers_function_class_and_type_param_nodes`.
    - `compile_only_ast_covers_augassign_and_annassign_nodes`.
    - `compile_only_ast_covers_match_and_pattern_nodes`.
    - `compile_only_ast_sets_location_attrs_on_alias_keyword_and_excepthandler`.
    - `compile_only_ast_covers_lambda_await_comprehension_and_yield_nodes`.
  - next gate: close `tb_lasti`/`co_positions` precision parity (currently compatibility-safe
    fallback with `tb_lasti = -1` for runtime traceback objects) and extend AST-conversion
    coverage beyond current traceback-focused node set.

## Scope

Bring uncaught exception reporting and traceback location fidelity to CPython 3.14 semantics, with explicit alignment to:

- PEP 626: precise line numbers (`co_lines`, `f_lineno`, executed-lines fidelity)
- PEP 657: fine-grained locations (start/end line+column + caret ranges)

Local reference copies:

- `docs/references/pep-0626.rst`
- `docs/references/pep-0657.rst`

CPython source references (3.14.3):

- `Objects/codeobject.c` (`PyCode_Addr2Location`, linetable decoding)
- `Include/cpython/code.h` (`_PyCodeLocationInfoKind`)
- traceback rendering behavior in Python runtime/error-display path.

## Parity Requirements

1. Exception constructors accept CPython kwargs contracts:
   - `AttributeError(..., name=?, obj=?)`
   - `NameError(..., name=?)`
   - `ImportError(..., name=?, path=?)`
   - invalid kwargs raise typed `TypeError` with CPython-style wording.
2. Traceback frames show CPython-style file/line/function format.
3. Where source + ranges are available, traceback includes source line + caret range.
4. Line/column data is instruction-accurate for source-compiled and `.pyc`-translated code.
5. Error formatting must not degrade scientific-stack behavior.

## Execution Phases

### Phase 1: Exception Constructor Semantics (P0)

- Fix `instantiate_exception_type` kwargs contracts.
- Add VM tests for constructor behavior parity.
- Validate `np.float(0.5)` raises `AttributeError` (not `RuntimeError` fallback).

### Phase 2: Traceback Rendering Substrate (P0)

- Add VM source cache for loaded/compiled source text.
- Render traceback lines in CPython shape.
- Add caret rendering path from instruction location ranges.

### Phase 3: Location Data Model Upgrade (P0)

- Upgrade internal location metadata to include start+end line/column.
- Preserve backward compatibility while propagating ranges through compiler.

### Phase 4: `.pyc` Location Decoding (P0)

- Decode `co_linetable` using CPython 3.14 format and map per instruction.
- Fill range metadata for translated bytecode.

### Phase 5: Regression Gates (P0)

- Add tests for traceback + caret formatting and constructor kwargs semantics.
- Re-run targeted scientific-stack probes to ensure no regressions.

## Non-Goals (for this slice)

- Full debugger/tracing API parity (`sys.settrace` event model closure) in one batch.
- Rich colorized traceback output parity.

## Completion Criteria

- `np.float(0.5)` shape matches CPython exception class/message semantics.
- Tracebacks include CPython-style frames + caret location in representative cases.
- New tests pass and are committed with no dirty workspace.
