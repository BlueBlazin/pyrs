# Milestone 12 Execution Backlog (P0)

This backlog is the ordered implementation plan for **Milestone 12: Core CPython parity closure**.
It translates milestone DoD into concrete work packages tied to:

- Current CPython harness allowlist ownership (`tests/cpython_allowlist.txt`)
- Production readiness accounting (`docs/PRODUCTION_READINESS.md`)

## Baseline Snapshot (Current)

Source: `tests/cpython_allowlist.txt`

- `parser-gap`: 51 entries
- `stdlib-gap`: 1 entry
- `runtime-gap`: 0 entries currently listed (still tracked in readiness checklist)

Owner-tag groups currently present:

- `milestone7-string-lexing-parity`: 19
- `milestone7-indentation-parity`: 15
- `milestone7-operator-token-parity`: 5
- `milestone7-tokenizer-parity`: 4
- `milestone7-expression-parity`: 3
- `milestone7-import-grammar-parity`: 2
- `milestone7-bytes-literal-parity`: 2
- `milestone7-fstring-pep701-parity`: 1
- `milestone11-importlib-support`: 1

## Ordered Work Packages

### WP12-01: Tokenizer Lexing Closure

- Priority: P0
- Readiness mapping:
- `Language & Grammar`: full tokenizer parity
- `Language & Grammar`: full grammar coverage (token-level blockers)
- Allowlist owners targeted:
- `milestone7-tokenizer-parity`
- `milestone7-string-lexing-parity`
- `milestone7-bytes-literal-parity`
- Scope:
- String/bytes literal lexing edge cases (prefixes, escapes, quoting variants)
- Numeric and operator token boundary correctness where lexer-driven
- Comment/newline/token stream parity edge cases
- Exit criteria:
- These owner-tag groups are removed from allowlist or reclassified with explicit non-goal rationale
- Targeted regression tests added for each fixed lexical family

### WP12-02: Indentation and Block-Structure Closure

- Priority: P0
- Readiness mapping:
- `Language & Grammar`: full grammar coverage
- Allowlist owners targeted:
- `milestone7-indentation-parity`
- Scope:
- INDENT/DEDENT parity behavior
- Block structure interactions with decorators, comprehensions, class/function bodies, and multiline constructs
- Exit criteria:
- `milestone7-indentation-parity` entries are closed or explicitly non-goal documented
- CPython harness entries in this owner group pass in default parity run

### WP12-03: Expression/Operator/Import Grammar Closure

- Priority: P0
- Readiness mapping:
- `Language & Grammar`: full grammar coverage
- `Semantic Analysis & Compilation`: `__future__` and compiler-feature edge semantics
- Allowlist owners targeted:
- `milestone7-operator-token-parity`
- `milestone7-expression-parity`
- `milestone7-import-grammar-parity`
- Scope:
- Expression precedence/associativity edge behavior
- Import grammar edge forms and parser acceptance/rejection parity
- Remaining grammar constructs currently blocked by parser-gap ownership tags
- Exit criteria:
- Owner-tag groups above are closed or explicitly non-goal documented
- Harness language and import suites no longer fail due parser for these classes

### WP12-04: F-String/Pattern/Exception-Group Semantic Closure

- Priority: P0
- Readiness mapping:
- `Language & Grammar`: PEP 701 and full pattern families
- `Language & Grammar`: `ExceptionGroup`/`except*` semantics
- Allowlist owners targeted:
- `milestone7-fstring-pep701-parity`
- Related parser-gap entries in language suite blocked on these semantics
- Scope:
- Full f-string parsing/lowering semantics for in-scope CPython behavior
- Pattern matching family completion beyond literal/capture/guard subset
- Exception-group splitting and propagation semantics in runtime/compiler lowering
- Exit criteria:
- f-string owner tag closed
- Language-suite pattern/exception-group tests in current allowlist move to pass or explicit non-goal

### WP12-05: Bytecode/VM Opcode and Unwind Closure

- Priority: P0
- Readiness mapping:
- `Bytecode & VM Execution`: full opcode execution parity
- `Bytecode & VM Execution`: precise exception propagation and frame unwinding
- Scope:
- Remaining opcode families required for CPython harness paths
- Exception-unwind/frame-state parity under nested control-flow and error paths
- Translation/decoder parity hardening where partial support remains
- Exit criteria:
- No harness failures attributable to unsupported in-scope opcode families
- VM/unwind regression tests added for each newly-supported opcode family

### WP12-06: Runtime Data-Model Edge Closure

- Priority: P0
- Readiness mapping:
- `Runtime Object Model & Data Model`: `__getattribute__` parity
- `Runtime Object Model & Data Model`: metaclass precedence/selection parity
- `Runtime Object Model & Data Model`: `__slots__` layout edge parity
- `Runtime Object Model & Data Model`: unicode/codecs behavior parity closure
- Scope:
- Attribute resolution precedence for data/non-data descriptors and instance/class hooks
- Metaclass selection/override and class construction precedence edge cases
- `__slots__` behavior consistency for layout/restriction edge paths
- Codecs error-mode and behavior parity where still partial
- Exit criteria:
- Residual Milestone 8/9 semantic gap set is demonstrably closed in harness/regression tests
- Any remaining deviations are explicit non-goals with owner + rationale

### WP12-07: Importlib Stdlib Gap Closure (In-Scope)

- Priority: P0
- Readiness mapping:
- `Import System`: full importlib machinery for in-scope pure-Python loaders
- `Standard Library Coverage`: import/package helpers needed by parity suites
- Allowlist owners targeted:
- `milestone11-importlib-support`
- Scope:
- Resolve current stdlib-gap failure(s) in importlib harness suite for in-scope behavior
- Exit criteria:
- `milestone11-importlib-support` allowlist entry removed, or explicitly re-scoped with rationale if out-of-scope for Milestone 12

### WP12-08: Allowlist Burn-Down + CI Gate Hardening

- Priority: P0
- Readiness mapping:
- `Testing & QA`: large Lib/test subset + CI gating
- `Security, Reliability, and Release Engineering`: parity-regression blocker policy
- Scope:
- Convert allowlist from broad parser-gap ownership to residual, explicit non-goals only
- Wire parity profile (`scripts/run_parity_gate.sh`) into merge-blocking CI lane
- Add dashboard/report artifact for allowlist delta per run
- Exit criteria:
- Allowlist contains only explicit non-goals with owner+rationale
- CI blocks regressions in parity suites by default

## Milestone 12 Exit Gate (Quantitative)

Milestone 12 is complete only when all are true:

- `parser-gap` allowlist entries are 0, except explicit non-goals documented and approved.
- In-scope `stdlib-gap` entries in language/import suites are 0.
- In-scope `runtime-gap` entries are 0.
- `scripts/run_parity_gate.sh` (or equivalent CI target) is merge-blocking and green.
- `docs/COMPATIBILITY.md` and `docs/PRODUCTION_READINESS.md` reflect reduced/closed gap status for Milestone 12 scope.

## Tracking Rules

- Every remaining allowlist row must retain `test|category|owner` with an actionable owner.
- Owner tags should be moved from `milestone7-*` legacy naming to `milestone12-*` during closure work to keep ownership current.
- No silent demotion of failing tests to allowlist without a linked rationale and scope decision.

