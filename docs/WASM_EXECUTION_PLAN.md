# WASM Execution Plan (Isolated Branch)

Status: in progress on isolated spike branch only.

Branch policy:
- All WASM work lives on `codex/wasm` (or descendant branches from it).
- `master` remains untouched until explicit go/no-go approval.
- It is acceptable for this track to remain unmerged indefinitely.

## Why This Is Isolated

Current runtime architecture is strongly native-host oriented (filesystem/process/network/terminal/dynamic-loader assumptions). Direct in-place changes on `master` would create unacceptable regression risk for Linux/macOS release targets.

This plan enforces an opt-in, non-invasive track with hard safeguards.

## Non-Negotiable Safety Rules

1. No direct WASM commits on `master`.
2. Every WASM milestone must preserve native behavior and keep native tests green.
3. WASM codepaths must be behind explicit compile-time gating (target/features).
4. No hidden behavior changes in existing native codepaths.
5. If any milestone causes native instability, stop and revert that milestone on `codex/wasm`.

## Scope

In scope:
- Browser-hosted PYRS execution prototype via WebAssembly.
- Website route integration for a web REPL/playground.
- Strict capability matrix for browser mode.

Out of scope (initial track):
- CPython extension loading in browser (`dlopen`, `PyInit_*`).
- Full stdlib parity in web mode.
- Any regression in native Linux/macOS interpreter behavior.

## Core Design Direction

## 1. Two-Lane Architecture

- Lane N (Native): existing behavior for CLI/runtime/extension subsystem.
- Lane W (Web): sandboxed execution mode with explicit capability restrictions.

No lane-crossing behavior changes without explicit gate review.

## 2. Host Abstraction Layer

Introduce a host boundary between VM core and platform services:
- filesystem access
- environment variables
- process/subprocess
- networking/sockets
- terminal/TTY streams
- clock/timer/sleep
- dynamic library loading

Native host implementation preserves current behavior.
Web host implementation provides sandboxed/unsupported semantics as documented.

## 3. Capability Matrix

Each subsystem is mapped to capabilities:
- `fs.read`, `fs.write`, `env.read`, `process.spawn`, `net.socket`, `terminal.interactive`, `native_ext.load`, etc.

In web mode, unsupported capabilities must fail explicitly and deterministically (CPython-shaped error surface where practical), never silently.

## Milestone Plan

## Milestone W0: Safety Scaffolding (No Behavior Change)

Deliverables:
- Add this plan and branch policy.
- Add a WASM track checklist doc section to release/engineering gates.
- Define initial capability matrix document (`docs/WASM_CAPABILITY_MATRIX.md`).

Exit criteria:
- Native test baseline unchanged.
- No runtime behavior differences.

## Milestone W1: Compile/Dependency Isolation

Current state: mostly complete.

Deliverables:
- Target-gate native-only dependencies and build steps:
  - terminal REPL crates,
  - C build glue (`build.rs` C compilation),
  - native extension dynamic loader path.
- Ensure crate can compile for `wasm32-unknown-unknown` in principle.

Exit criteria:
- Native targets compile as before.
- WASM target reaches at least compile-check stage without native-only linkage paths.

## Milestone W2: VM Host Interface Extraction

Current state: in progress (initial host seam landed).

Deliverables:
- Introduce host traits/interfaces and a `NativeHost` implementation.
- `Vm::new()` preserves current behavior via native host defaults.
- Add `Vm::new_with_host(...)` for alternate hosts.

Exit criteria:
- No native semantic drift in touched test lanes.
- Host-coupled APIs route through the interface layer.

## Milestone W3: Web Host Baseline

Current state: pending.

Deliverables:
- `WebHost` implementation with explicit unsupported-capability errors.
- Browser-mode import/runtime policy for unavailable modules/features.
- In-memory source execution path suitable for browser input.

Exit criteria:
- Deterministic behavior for supported snippets.
- Deterministic error behavior for unsupported operations.

## Milestone W4: WASM Binding Surface

Current state: in progress (syntax-check bridge + session scaffold landed).

Deliverables:
- Expose a minimal host API for browser integration:
  - init runtime,
  - execute code,
  - reset session,
  - collect stdout/stderr,
  - execution status/errors.
- Keep API versioned and small.
- Track contract details in `docs/WASM_API_CONTRACT.md`.

Exit criteria:
- Programmatic execution works from JS harness tests.

## Milestone W5: Worker Runtime + Interruption Model

Current state: pending.

Deliverables:
- Run interpreter in a Web Worker.
- Define timeout/reset semantics (worker recycle for hard-stop).
- Prevent main-thread blocking.

Exit criteria:
- Long-running snippets do not freeze site UI.
- Worker contract doc exists and is kept in sync (`docs/WASM_WORKER_RUNTIME_CONTRACT.md`).

## Milestone W6: Website Integration

Current state: pending.

Deliverables:
- Add `/playground` route in Astro.
- Lazy-load WASM bundle only on playground routes.
- Add clear compatibility notes on page.

Exit criteria:
- Website remains static-first for non-playground routes.
- Playground UX stable for supported examples.

## Milestone W7: CI and Promotion Decision

Current state: pending.

Deliverables:
- Add WASM-specific CI lanes (compile + smoke + browser harness where feasible).
- Keep native lanes mandatory and unchanged.
- Add explicit go/no-go rubric for merge candidacy.

Exit criteria:
- Native reliability unaffected.
- WASM lane reliability and maintenance cost understood.

## Validation Gates Per Milestone

Required after each meaningful checkpoint:

1. Targeted native tests for touched surfaces via `cargo nextest run`.
2. Native build check on Linux/macOS targets remains clean.
3. No unexpected changes in extension/native loader behavior.
4. WASM compile/smoke checks for new interfaces.
5. Commit checkpoint with doc updates in same commit.

Branch helper:
- `scripts/check_wasm_branch.sh` runs the current minimum checkpoint validation set.
- `scripts/run_wasm_contract_smoke.sh` runs local wasm contract smoke checks
  (compile-only by default; set `PYRS_WASM_RUN_BROWSER_SMOKE=1` for optional
  browser run via `wasm-pack`).
- wasm harness note: use targeted wasm contract compile lane
  (`cargo check --target wasm32-unknown-unknown --test wasm_contract`)
  instead of all-tests wasm compile.
- wasm lib-unit harness note: compile wasm-target lib tests (no-run) via
  `cargo test --target wasm32-unknown-unknown --lib --no-run` so
  `src/wasm/mod.rs` unit-contract checks stay gate-covered.
- host seam audit helper:
  `python3 scripts/audit_wasm_host_seam.py --out perf/wasm_host_seam_audit_latest.json`
  for tracking remaining direct `std::env` usage under `src/vm`.
- execute-contract summary helper:
  `python3 scripts/generate_wasm_execute_contract_summary.py --out perf/wasm_execute_contract_summary_latest.json`
  for fixture-driven execute phase/blocker consistency.
- worker-contract summary helper:
  `python3 scripts/generate_wasm_worker_contract_summary.py --out perf/wasm_worker_contract_summary_latest.json`
  for fixture + source parity on worker key sets/prefixes/blockers.
- module-policy summary helper:
  `python3 scripts/generate_wasm_module_policy_summary.py --out perf/wasm_module_policy_summary_latest.json`
  for fixture + source + docs parity on module blocker mappings.

## Merge Decision Rubric

WASM branch may be considered for merge only if all are true:
- Native behavior remains stable across required gates.
- WASM codepaths are fully gated and do not alter default native execution.
- Browser capability limitations are clearly documented.
- CI cost/flake profile is acceptable.

If any are not true:
- keep `codex/wasm` as an experimental long-lived branch,
- do not merge to `master`.

## Immediate Execution Order

1. Implement W1 compile/dependency isolation first.
2. Implement W2 host interface extraction with strict no-drift policy.
3. Add W3 web host + capability errors.
4. Add W4 bindings and local JS harness.
5. Add W5 worker runtime and timeout/reset behavior.
6. Integrate W6 website playground only after runtime stability.

## Progress Checkpoints (codex/wasm)

Completed on this branch:
- `afeed21`: initial isolated execution plan + branch-policy guardrails.
- `7c92917`: wasm dependency lane + target-gated build/dependency isolation.
- `302fa6b`: `VmHost`/`NativeHost`/`WasmHost` baseline + `Vm::new_with_host`.
- `04ca7a1`: startup trace flags routed through host seam.
- `42fbc8f`: `sys` bootstrap process metadata routed via host seam.
- `89a2d32`: UUID host-name probe routed via host seam.
- `26cbc4e`: wasm-target extension import warning cleanup.
- `271b5f9`: wasm runtime init + parser syntax-check bridge.
- `8f0e5a4`: structured wasm syntax diagnostics surface.
- `165052c`: stateful wasm session scaffold for syntax checks.
- `23afc18`: wasm capability-report bridge export.
- `344098e`: wasm API contract version export.
- `7fbef01`: structured wasm execute-result contract (syntax/unsupported phases).
- `e03263e`: capability-key metadata + unsupported-capability helper.
- `d11b634`: additional VM method-level trace probes routed through host seam.
- `c32b066`: wasm-target contract smoke harness + targeted wasm test compile lane.
- `5c1664c`: `vm_execution` env/debug probes routed through host seam.
- `5b75de5`: `vm_native_dispatch` env/debug probes routed through host seam.
- `05a3bb8`: `vm_runtime_methods` env/debug probes routed through host seam.
- `3fe94f6`: import pending trace probes routed through host seam.
- `e3473a7`: locale/hostname env lookups routed through host seam.
- `30d9c45`: `vm/ops` trace probes switched to shared env-probe helper.
- `b79aac8`: `vm/mod` trace/debug checks consolidated through env-probe helper.
- `a5819d6`: `cpython_object_item_compare_api` probes switched to cached env helper.
- `fe6de19`: `vm_bootstrap_import` env/config reads routed through host seam.
- `8866944`: `vm_builtin_metadata` trace probes switched to cached env helper.
- `66c29a3`: `builtins_core` env probes routed through host seam (`self.host`).
- `b515e6e`: `cpython_type_api` probes switched to cached env helper.
- `803bde5`: `cpython_dict_api` probes switched to cached env helper.
- `10042e6`: `cpython_error_numeric_api` probes switched to cached env helper.
- `64d8d1b`: `cpython_object_attr_api` probes switched to cached env helper.
- `7fe9c23`: `cpython_capsule_api` probes switched to cached env helper.
- `a4466ad`: `cpython_descriptor_method_api` probes switched to cached env helper.
- `7ec5577`: `proxy_runtime` probes switched to cached env helper.
- `b6d8e16`: `cpython_object_call_api` probes switched to cached env helper.
- `abea877`: `cpython_slot_runtime` probes switched to cached env helper.
- `a8f4e44`: `cpython_import_api` probes switched to cached env helper.
- `db0a43b`: `loader_runtime` probes switched to cached env helper.
- `27e1b4d`: `builtins_os` env reads routed through host seam.
- `d734644`: `vm_extensions` probes switched to cached env helper.
- `2accc6e`: `cpython_numeric_api` probes switched to cached env helper.
- `50c5fac`: `cpython_context_runtime` probes switched to cached env helper.
- `d6201a6`: `cpython_list_api` probes switched to cached env helper.
- `c8310f7`: `cpython_contextvar_api`/`cpython_module_api`/`cpython_tuple_api` probes switched to cached env helper.
- `cf08545`: multi-file host-seam cleanup across stdlib/import/C-API runtime helpers.
- `5811498`: host-routed depth-limit config in `vm_builtin_metadata` and host-adapter profile env reads in `pickle`.
- `8ecb0f5`: audit script now reports actionable-vs-allowlisted hits for central seam internals.
- `c86b20c`: wasm execution blocker contract exports (`keys + error`) and execute-path error wiring.
- `508317e`: wasm parse+compile contract for `execute()`/`check_compile(_result)` with explicit
  `compile_error` phase while runtime execution remains unwired.
- `f64f85b`: structured execution-blocker export (`wasm_execution_blockers`) now includes
  backend + unsupported capability blockers as key/message entries.
- `16dee07`: `WasmSession` now exposes `check_compile()` for stateful parse+compile validation.
- `656d0fd`: wasm contract tests now enforce blocker-key parity with the capability matrix.
- `e2c41fc`: wasm module-level capability preflight API (`wasm_module_support`) exports
  structured blocker mapping for known unsupported module families.
- `18abe6e`: module blocker policy is exported as canonical structured rows
  (`wasm_module_policy_entries`) for docs/UI parity.
- `9161b33`: module preflight rationale is documented in
  `docs/WASM_MODULE_SUPPORT_POLICY.md` and linked from capability docs.
- `eaacf67`: `WasmRuntimeInfo` now exposes parse+compile support and blocker-count metadata.
- `f412608`: `WasmExecutionResult` now carries structured `line`/`column` diagnostics.
- `f1abaed`: snippet preflight APIs landed (`wasm_snippet_support`/`wasm_snippet_blockers`) with
  structured import-capability blocker reporting.
- `438d249`: fixture-driven wasm contract snippet snapshots are tracked under
  `tests/fixtures/wasm_contract_snippets.rs`.
- `9db8e10`: initial worker runtime contract introspection landed
  (`wasm_worker_info`, `wasm_worker_blocker_keys`, `wasm_worker_blocker_error`).
- `0affb9c`: local wasm contract smoke script added at
  `scripts/run_wasm_contract_smoke.sh`.
- `07f2519`: worker runtime contract details are documented in
  `docs/WASM_WORKER_RUNTIME_CONTRACT.md`.
- `d5691f0`: worker lifecycle stubs (`wasm_worker_start`/`wasm_worker_terminate`) with
  fixture-backed contract tests were added.
- `df03f8d`: canonical browser call order is documented in
  `docs/WASM_CLIENT_INTEGRATION_FLOW.md`.
- `5c4514e`: `WasmWorkerSession` stateful wrapper API landed for lifecycle orchestration.
- `7988f5e`: worker state/lifecycle enum key exports were added for stable client branching.
- `c9fc0c1`: worker contract strings are now centralized through internal enum-backed sources.
- latest: worker enum contract tests now assert full set equality from fixture snapshots.
- latest: worker execute contract now exports canonical execute phases and
  `wasm_worker_execute(source)`; `WasmWorkerSession` now tracks `executes_requested`.
- latest: worker blocker contract now exports structured rows via
  `wasm_worker_blockers()` for key/message UI integration.
- latest: worker timeout/recycle policy is now exported via
  `wasm_worker_timeout_policy()` with stable unwired enforcement metadata.
- latest: worker lifecycle now includes explicit recycle stub contract
  (`wasm_worker_recycle`) and `WasmWorkerSession` tracks `recycles_requested`.
- latest: `WasmHost` now provides explicit per-capability unsupported messages
  (locked by host-level contract tests) instead of generic fallback text.
- latest: `scripts/check_wasm_branch.sh` now runs
  `wasm_host_unsupported_messages_are_stable` as a required local guard.
- latest: worker timeout updates now have explicit contract APIs
  (`wasm_worker_set_timeout`, `wasm_worker_timeout_phase_keys`) with session telemetry.
- latest: `scripts/run_wasm_contract_smoke.sh` now includes targeted host
  capability/message nextest checks in the compile-only smoke path.
- latest: execution blockers now include explicit `vm_runtime_unavailable`
  root-cause signaling alongside `execution_backend_unwired`.
- latest: snippet preflight now exports canonical import roots via
  `wasm_snippet_import_roots(source)` for deterministic client dependency UI.
- latest: worker lifecycle/timeout results now include monotonic operation IDs,
  and `WasmWorkerSession` tracks `last_operation_id` for telemetry correlation.
- latest: worker execute path now has operation-aware API
  (`wasm_worker_execute_with_operation`) while preserving `wasm_worker_execute`.
- latest: session contract tests now cover `execute_with_operation` telemetry
  (`executes_requested`, `last_operation_id`, `last_phase`) end-to-end.
- latest: client-flow pseudocode now demonstrates timeout policy/update,
  snippet import-root preflight, and operation-aware worker execute calls.
- latest: worker operation-id shape/uniqueness is now explicitly covered across
  lifecycle and timeout APIs in wasm contract tests.
- latest: host contract checks now include a capability-matrix consistency gate
  (`wasm_host_unsupported_message_matrix_matches_supports`) in branch/smoke scripts.
- latest: worker operation-id prefix expectations are now fixture-driven across
  lifecycle/timeout/execute wasm contract tests.
- latest: operation-id docs now explicitly scope guarantees to prefix shape +
  per-process uniqueness (no cross-run ordering contract).
- latest: worker contract summary snapshots now validate fixture rows against
  `src/wasm/mod.rs` source key/prefix/blocker contracts via
  `scripts/generate_wasm_worker_contract_summary.py`
  (`perf/wasm_worker_contract_summary_latest.json`) and are enforced in
  branch/smoke gate scripts.
- latest: `WasmExecutionResult`/`WasmWorkerExecutionResult` now expose
  `blocker_key` for deterministic unsupported-execution branching
  (`execution_backend_unwired` / `worker_runtime_unwired`) without
  message parsing.
- latest: top-level execute phase enums are now exported via
  `wasm_execution_phase_keys()` so clients can branch without hardcoded literals.
- latest: branch/smoke scripts now compile wasm-target lib unit tests with
  `cargo test --target wasm32-unknown-unknown --lib --no-run` to keep
  `src/wasm/mod.rs` contract unit checks in the local gate path.
- latest: execute-contract fixture summaries are now generated via
  `scripts/generate_wasm_execute_contract_summary.py` and enforced in
  branch/smoke scripts (`perf/wasm_execute_contract_summary_latest.json`).
- latest: module-policy fixture summaries are now generated via
  `scripts/generate_wasm_module_policy_summary.py` and enforced in
  branch/smoke scripts (`perf/wasm_module_policy_summary_latest.json`) with
  source+fixture+docs row-set validation.
- latest: `WasmRuntimeInfo` now includes explicit `execution_backend`
  (`\"unwired\"`) so clients can branch on backend readiness without message parsing.
- latest: `WasmWorkerInfo` now includes explicit `backend`
  (`\"unwired\"`) for worker-readiness branching without inferring from state text.

Latest host seam audit (local branch run):
- `python3 scripts/audit_wasm_host_seam.py` => `total_hits=0` (`allowlisted_hits=3`).
- allowlisted entries are intentionally scoped to:
  - `src/vm/mod.rs` cached env-probe core (`env_var_present_cached` internals).

Remaining near-term focus:
1. W3: expand `WasmHost` capability stubs and error contracts for unsupported features.
2. W4: evolve wasm API from syntax-only to controlled in-memory execution API contract.
3. Add wasm smoke harness (JS/wasm-bindgen-test or equivalent) without touching native CI gates yet.

## Risk Register

1. Hidden native coupling discovered late.
   - Mitigation: compile isolation and host-interface extraction before feature work.
2. Native regressions from shared refactor.
   - Mitigation: mandatory targeted native test gates per checkpoint.
3. Browser-mode expectation mismatch.
   - Mitigation: explicit capability matrix and hard unsupported errors.
4. Build/CI complexity growth.
   - Mitigation: staged CI enablement and strict ownership.
