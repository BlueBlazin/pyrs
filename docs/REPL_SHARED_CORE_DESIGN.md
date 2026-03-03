# REPL Shared Core Design (Native + WASM Adapters)

Status: in-progress (incremental migration on `codex/wasm`).
Branch context: `codex/wasm` (isolated from `master`).

## 1. Context and Product Positioning

PYRS remains a native-first product (Linux/macOS installable interpreter).  
WASM REPL is a lightweight website demo surface and must not drive native regressions.

The goal of this design is to share **REPL execution semantics** between native and wasm while keeping:

- native REPL feature depth (completion, hints, history, meta commands),
- wasm REPL lean (small, fast, no heavy editor/runtime extras),
- stable wasm/public contracts.

## 2. Current State Audit

## 2.1 Native REPL (CLI)

Current implementation lives in [`src/cli/repl.rs`](../src/cli/repl.rs), with one large flow:

- `run_interactive_session(...)` owns:
  - line-editor event loop (`reedline`),
  - pending multiline buffer management,
  - parse/compile/execute decisions,
  - expression echo semantics,
  - meta commands (`:help`, `:reset`, `:paste`, `:timing`) and magic commands (`%time`, `%timeit`),
  - completion/hints refresh behavior.
- Parse/incomplete detection logic is embedded in helpers in same module:
  - `repl_parse_candidate_source`
  - `repl_parse_success_requires_more_input`
  - `repl_input_is_incomplete`
  - `has_unclosed_delimiters`
- Execution logic is also in same module:
  - `execute_module_source`
  - `execute_parsed_module`
  - timing wrappers.

This is production-grade but tightly coupled to `reedline` and terminal concerns.

## 2.2 WASM REPL Runtime Surface

Current implementation in [`src/wasm/mod.rs`](../src/wasm/mod.rs):

- `WasmReplSession::execute_input(...)` performs its own:
  - parse/compile checks (`parse_and_compile_snippet`),
  - blocker preflight (`collect_import_roots` + policy),
  - expression-vs-module execution split,
  - result shaping to `WasmExecutionResult`.
- Worker/top-level wasm execution also has related execution contract logic in same file.

This partially duplicates native REPL parse/compile/execute policy and increases drift risk.

## 2.3 Website Playground Adapter

Current browser adapter spans:

- runtime worker RPC bridge: [`website/public/workers/playground-runtime-worker.js`](../website/public/workers/playground-runtime-worker.js)
- UI + transcript/input behavior: [`website/src/pages/playground.astro`](../website/src/pages/playground.astro)

The worker API is intentionally small (`load`, `execute`, `reset`) and should remain stable while internals evolve.

## 3. Problem Statement

We currently have two mostly separate REPL semantics implementations:

1. native CLI path in `src/cli/repl.rs`,
2. wasm session path in `src/wasm/mod.rs`.

This creates:

- semantic drift risk (incomplete input, expression echo, diagnostics shape),
- slower feature/fix propagation across native and wasm,
- higher maintenance complexity while keeping strict CPython parity.

## 4. Design Goals

1. One shared REPL execution core for parse/compile/execute semantics.
2. Adapter split:
   - native adapter keeps `reedline` UX features.
   - wasm adapter keeps minimal fast surface.
3. Preserve existing external contracts:
   - CLI behavior from user perspective,
   - wasm JS contract types/phases.
4. Keep wasm size/perf constraints:
   - no `reedline` or native-only concerns in wasm path.
5. Keep migration safe and incremental with strong gates.

## 5. Non-Goals

1. Forcing native completion/hints/history into wasm.
2. Introducing wasm support for native extension-heavy stacks (`numpy`, etc.).
3. Replacing website UI stack in this design.
4. Changing default product positioning (native remains primary).

## 6. Proposed Architecture

## 6.1 New Shared Core Module

Add an internal REPL core module (proposed path):

- `src/repl/core.rs` (or `src/runtime/repl_core.rs` if preferred by ownership map)

Core responsibilities:

- maintain session state (`pending` multiline source, execution counters, last error),
- decide prompt mode (`>>>` vs `...`),
- determine “need more input” using shared parser heuristics,
- execute expression/module with shared policy,
- return structured output events and status.

Core explicitly does **not** own:

- terminal input editing,
- command history persistence,
- completion/hint rendering,
- DOM/highlighting UI.

## 6.2 Shared Data Model (Internal)

Proposed internal types:

```rust
pub enum ReplProfile {
    NativeFull,
    WasmLean,
}

pub struct ReplCoreConfig {
    pub profile: ReplProfile,
    pub echo_expression_result: bool,   // true for both paths
    pub enable_meta_commands: bool,     // native=true, wasm=false
    pub enable_magic_commands: bool,    // native=true, wasm=false
}

pub enum ReplSubmitKind {
    Line,      // interactive line
    Snippet,   // full snippet/cell submission
}

pub struct ReplStepResult {
    pub outcome: ReplOutcome,
    pub outputs: Vec<ReplOutput>,       // stdout/stderr/banners/diagnostics
    pub prompt: ReplPromptKind,         // Primary or Continuation
    pub line: usize,
    pub column: usize,
}

pub enum ReplOutcome {
    Continue,           // keep collecting lines
    Executed,           // snippet ran
    ExitRequested,      // native adapter handles process exit
    Interrupted,        // Ctrl-C clear behavior
    Fatal(String),
}
```

Note: these are internal shape proposals, not public API commitments.

## 6.3 Adapter Responsibilities

## Native adapter (`src/cli/repl.rs`)

Keeps:

- `reedline` integration,
- theme selection (`PYRS_REPL_THEME` + `COLORFGBG`),
- completion/hints/history/startup script,
- native-only meta/magic command UX.

Changes:

- delegate parse/incomplete/execution decisions to shared core.
- keep completion refresh logic in adapter using core execution outcomes.

## WASM adapter (`src/wasm/mod.rs`)

Keeps:

- wasm contract types (`WasmExecutionResult`, `WasmRuntimeInfo`, worker types),
- blocker/capability preflight and wasm worker state semantics.

Changes:

- `WasmReplSession::execute_input` delegates snippet execution semantics to shared core,
  then maps core output to existing `WasmExecutionResult` fields.
- no history/completion/meta command features in wasm lean profile.

## Website worker/UI

- no required public RPC changes for phase 1 (`load`/`execute`/`reset` stays).
- worker continues to call `WasmReplSession` APIs; internal behavior becomes less duplicated.

## 6.4 Feature Profile Matrix

| Capability | NativeFull | WasmLean |
| --- | --- | --- |
| CPython-style parse/compile/execution semantics | Yes | Yes (shared core) |
| Expression echo (`>>> 1+1 -> 2`) | Yes | Yes |
| Incomplete-input detection heuristics | Yes | Yes |
| Meta commands (`:help`, `:reset`, `:paste`, `:timing`) | Yes | No |
| `%time` / `%timeit` | Yes | No |
| Completion/hints/history/startup script | Yes | No |
| Worker lifecycle/timeout gating | N/A | Yes (existing wasm contract) |

## 7. Contract and Compatibility Constraints

1. `src/wasm/mod.rs` exported wasm API contract remains stable unless explicitly versioned.
2. Worker phase/blocker/state keys remain source-of-truth contract keys.
3. Native CLI behavior remains unchanged unless intentionally documented.
4. No `native-cli` dependency leakage into wasm target.

## 8. Incremental Migration Plan

## Phase A: Extract semantic helpers into shared core

- Move parser/incomplete-detection and expression/module execution policy into shared module.
- Keep `run_interactive_session` behavior identical, but call extracted functions.

Gate:

- `cargo nextest run --lib repl_ --status-level fail --final-status-level fail` (new tests),
- existing native REPL tests in `src/cli/repl.rs` remain green.

## Phase B: Introduce `ReplCore` session API

- Add stateful core session object with `submit_line/submit_snippet/reset/interrupt`.
- Native path uses core for execution decisions but keeps reedline loop.

Gate:

- targeted native REPL tests + `cargo nextest run --test vm` spot checks for REPL-sensitive behavior.

## Phase C: Wire wasm repl session to core

- Refactor `WasmReplSession::execute_input` to call core.
- Preserve `WasmExecutionResult` shaping and existing contract keys.

Gate:

- `cargo check --target wasm32-unknown-unknown --no-default-features --features wasm-vm-probe`,
- `cargo check --target wasm32-unknown-unknown --test wasm_contract --no-default-features --features wasm-vm-probe`,
- `cargo nextest run --lib wasm_ --status-level fail --final-status-level fail`.

## Phase D: Contract + drift hardening

- Add differential tests that compare native and wasm core behavior for shared semantic cases:
  - expression echo,
  - syntax/compile/runtime error mapping,
  - multiline continuation decisions.

Gate:

- wasm contract scripts and summaries stay green:
  - `scripts/check_wasm_branch.sh`
  - wasm summary artifacts remain unchanged except intended additions.

## Phase E: Cleanup

- Remove now-duplicate execution code paths from `src/wasm/mod.rs` and `src/cli/repl.rs`.
- Keep adapters thin and focused.

## 9. Risk Analysis and Mitigations

1. Risk: Native regression in REPL UX behavior.
   - Mitigation: keep reedline adapter untouched first; migrate semantics only.
2. Risk: wasm contract drift.
   - Mitigation: keep wasm result mapping layer explicit and fixture-tested.
3. Risk: binary-size growth in wasm.
   - Mitigation: no terminal/completion dependencies in shared core; keep feature gating strict.
4. Risk: hidden behavior divergence via helper duplication during migration.
   - Mitigation: move helper ownership once; delete old copies immediately after each phase.

## 10. Acceptance Criteria

Design is complete when:

1. Native and wasm both use one shared REPL semantic core.
2. Native keeps production REPL feature set.
3. WASM remains lean and contract-stable.
4. Existing wasm contract lanes and native targeted lanes remain green.
5. No master-branch impact until explicit promotion approval.

## 11. Immediate Implementation Order (when approved)

1. Land shared semantic helper extraction with no behavior change.
2. Introduce `ReplCore` API and migrate native adapter.
3. Migrate wasm `WasmReplSession` to `ReplCore`.
4. Add cross-adapter semantic conformance tests.
5. Delete duplicated logic and refresh wasm evidence artifacts.

## 12. Implementation Checkpoints (Live)

- Shared parse/incomplete-input semantics are centralized in [`src/repl_core.rs`](../src/repl_core.rs).
- Native CLI loop now uses `ReplCoreState` for line submission and execution flow.
- wasm `WasmReplSession` now uses `ReplCoreState` for line buffering/continuation state.
- New `ReplCoreState` session API now includes:
  - `prompt_kind()` for adapter prompt selection (`>>>` vs `...` decisions),
  - `submit_line_and_execute(...)` stateful execution method.
- Website playground worker now forwards REPL prompt state (`prompt_continuation`)
  from `WasmReplSession` so UI prompt mode follows core interpreter state.
