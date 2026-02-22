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
    - unclosed delimiter detection -> `SyntaxError: '<delimiter>' was never closed`.
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
    and indentation-error shape against CPython output.
  - next gate: expand golden traceback-shape tests against CPython output for nested chains.

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
