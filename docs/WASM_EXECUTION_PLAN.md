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

Current state: in progress.

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

Current state: in progress.

Deliverables:
- Run interpreter in a Web Worker.
- Define timeout/reset semantics (worker recycle for hard-stop).
- Prevent main-thread blocking.

Exit criteria:
- Long-running snippets do not freeze site UI.
- Worker contract doc exists and is kept in sync (`docs/WASM_WORKER_RUNTIME_CONTRACT.md`).

## Milestone W6: Website Integration

Current state: in progress.

Deliverables:
- Add `/playground` route in Astro.
- Lazy-load WASM bundle only on playground routes.
- Add clear compatibility notes on page.

Exit criteria:
- Website remains static-first for non-playground routes.
- Playground UX stable for supported examples.

## Milestone W7: CI and Promotion Decision

Current state: in progress.

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
  browser run via `wasm-pack` covering both `--test wasm_contract` and `--lib`;
  set `PYRS_WASM_SKIP_CORE_SMOKE=1` to run browser smoke without repeating
  core compile/summary/nextest checks;
  set `PYRS_WASM_RUN_VM_PROBE_BROWSER_STATE_GATE_SMOKE=1` to additionally run
  vm-probe state-gate smoke (`--test wasm_vm_probe_browser_smoke`) in the same
  browser lane first, with automatic node fallback when browser vm-probe smoke
  fails.
  The node fallback requires `scripts/wasm_node_shims/env/index.js` to exist
  and fails fast if that shim is missing).
- wasm bridge unit-contract helper:
  `cargo nextest run --lib wasm_ --status-level fail --final-status-level fail`
  for host-executed wasm bridge/runtime unit-contract coverage.
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
- api-contract surface summary helper:
  `python3 scripts/generate_wasm_api_contract_surface_summary.py --out perf/wasm_api_contract_surface_summary_latest.json`
  for source/doc parity on top-level wasm exports and exported type field coverage.
- worker-docs summary helper:
  `python3 scripts/generate_wasm_worker_docs_contract_summary.py --out perf/wasm_worker_docs_contract_summary_latest.json`
  for source/doc parity on worker state/lifecycle/execute/timeout keys, worker info backend/probe-flag shapes, interruption model, timeout bounds, and blocker keys.
- client-flow docs summary helper:
  `python3 scripts/generate_wasm_client_flow_summary.py --out perf/wasm_client_flow_summary_latest.json`
  for source/doc parity on browser call-order guidance, worker phase enums, and `WasmWorkerSession` telemetry fields.
- worker-contract summary helper:
  `python3 scripts/generate_wasm_worker_contract_summary.py --out perf/wasm_worker_contract_summary_latest.json`
  for fixture + source parity on worker key sets/prefixes/blockers and worker-info mode semantics.
- module-policy summary helper:
  `python3 scripts/generate_wasm_module_policy_summary.py --out perf/wasm_module_policy_summary_latest.json`
  for fixture + source + docs parity on module blocker mappings.
- capability summary helper:
  `python3 scripts/generate_wasm_capability_summary.py --out perf/wasm_capability_summary_latest.json`
  for fixture + source + docs parity on capability keys/support matrix.
- playground worker contract helper:
  `node scripts/check_playground_worker_contract.mjs --out perf/wasm_playground_worker_contract_latest.json`
  for source-level parity between `/playground` worker transport wiring and
  `website/public/workers/playground-runtime-worker.js` action handling.
- promotion evidence-pack helper:
  `python3 scripts/collect_wasm_evidence_pack.py`
  to bundle required local wasm artifact snapshots under
  `perf/wasm_evidence_pack_latest/` with a manifest for review handoff.
  `scripts/check_wasm_branch.sh` and `scripts/run_wasm_contract_smoke.sh`
  now emit this evidence pack automatically when core smoke is enabled.
- evidence-pack validator helper:
  `python3 scripts/validate_wasm_evidence_pack.py --pack-dir perf/wasm_evidence_pack_latest`
  to enforce manifest + copied-artifact completeness in local and CI gates.
- vm-compile probe helper (non-gating):
  `scripts/probe_wasm_vm_compile.sh`
  to surface current wasm-vm compile blockers under opt-in `wasm-vm-probe`.
- vm native-link blocker helper:
  `python3 scripts/generate_wasm_vm_link_blockers_summary.py --out perf/wasm_vm_link_blockers_latest.json`
  to inventory `#[link(name = ...)]` libraries under `src/vm` that block full
  wasm target link/test lanes.
- vm raw env-import blocker helper:
  `python3 scripts/generate_wasm_vm_env_import_summary.py --wasm target/wasm32-unknown-unknown/release-wasm/pyrs.wasm --out perf/wasm_vm_env_import_summary_latest.json`
  to track unresolved wasm `env` function imports (grouped by family) and node
  shim coverage while vm-probe browser bring-up remains in progress.
- CI lane helper:
  `.github/workflows/wasm-track.yml`
  runs branch-level wasm contract gating (`scripts/check_wasm_branch.sh`) and
  uploads wasm contract evidence artifacts plus dedicated
  `wasm-evidence-pack` artifacts. Browser-smoke lane now downloads that pack
  and validates it before smoke execution. It also includes optional
  `wasm-browser-smoke` coverage on manual `workflow_dispatch`.
- promotion decision rubric:
  `docs/WASM_PROMOTION_GATE.md`
  defines explicit go/no-go criteria before any merge candidacy decision.
- browser-smoke dispatch runbook:
  `docs/WASM_BROWSER_SMOKE_RUNBOOK.md`
  documents repeatable `gh` workflow-dispatch + artifact capture steps.
- browser-smoke dispatch helper:
  `scripts/run_wasm_browser_smoke_dispatch.sh`
  runs one-shot workflow dispatch/watch/download flow with baseline-summary
  validation so browser evidence capture is deterministic.

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
- latest: worker enum contract tests now assert fixture-ordered key parity
  (state/lifecycle/execute/timeout), not only set membership.
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
- latest: optional browser smoke path now runs both wasm integration contract
  tests (`--test wasm_contract`) and wasm lib unit tests (`--lib`) via
  `wasm-pack`.
- latest: opt-in `wasm-vm-probe` feature + `scripts/probe_wasm_vm_compile.sh`
  landed to allow explicit wasm-target vm compile probing without changing
  default native/wasm build paths.
- latest: `Py_Main`/`Py_BytesMain` now avoid hard `crate::cli` coupling under
  wasm target (returning explicit unavailable status) so vm-probe progresses to
  deeper wasm-specific blockers.
- latest: `PyMember_GetOne` bool conversion now matches `PyBool_FromLong(i64)`
  signature, removing the previous wasm-vm-probe numeric-type blocker.
- latest: unix-only fd/process trait imports are now locally gated in
  `builtins_io`/`builtins_os`, removing wasm-vm-probe unresolved-import blockers
  for `AsRawFd`/`FromRawFd`/`IntoRawFd`/`ExitStatusExt`/`UnixStream`.
- latest: pointer-threshold guards now use a shared width-safe
  `MIN_VALID_PTR_THRESHOLD` across vm extension surfaces, removing wasm32
  `usize` overflow compile blockers from `0x1_0000_0000` literals.
- latest: top-level `execute()` and `wasm_worker_execute()` now perform
  capability preflight on parse+compile-valid snippets and return
  capability-specific blocker keys for known blocked imports (falling back to
  backend/worker-unwired blocker keys when no capability blocker applies).
- latest: worker execute contract summary validation now accepts
  module-policy-derived capability blocker keys in unsupported worker execute
  fixtures (in addition to runtime-unwired key), preserving source/fixture
  parity while capability preflight expands.
- latest: worker blocker exports now include module-policy capability keys
  (`dynamic_library_load`, `network_sockets`, `process_spawn`,
  `interactive_terminal`) alongside `worker_runtime_unwired`, and
  `wasm_worker_blocker_error` resolves capability messages for those keys.
- latest: host capability contracts now include `clock_time` (supported in wasm)
  and `thread_sleep` (explicitly unsupported in wasm) with bridge/test/docs
  parity across host + wasm capability exports.
- latest: execute phase fixture constants are now source-order-validated via
  `scripts/generate_wasm_execute_contract_summary.py`, and wasm contract tests
  assert ordered parity against `WASM_EXECUTION_PHASE_KEYS`.
- latest: remaining allowlisted direct `std::env` probes in `src/vm/mod.rs`
  were removed; env probe caching now routes through host-capability-aware
  seam logic (`EnvironmentRead`) with no direct env calls left under `src/vm`.
- latest: worker summary validation and wasm contract tests now enforce ordered
  key parity for worker enum exports, reducing fixture/source drift risk.
- latest: module-policy and capability/export tests now enforce deterministic
  row/key ordering, and module-policy summary validation now checks
  fixture/source/docs order parity in addition to set parity.
- latest: `scripts/check_wasm_branch.sh` and
  `scripts/run_wasm_contract_smoke.sh` now include
  `scripts/audit_wasm_host_seam.py` snapshots so host-seam drift is guarded by
  the standard local wasm gate scripts.
- `671171f`: capability matrix fixture/source/docs summary gate landed and is
  enforced in branch/smoke scripts.
- `3d0684f`: worker blocker key order parity is enforced against source order.
- `6c9beea`: execute phase key fixture constants and source-order parity checks
  landed for top-level execute contract summaries/tests.
- `26e8e87`: vm env-probe cache no longer uses direct `std::env` in `src/vm`;
  host seam routing now drives env probe behavior.
- `a199816`: worker enum key exports are now fixture-order validated in both
  wasm contract tests and worker summary generation.
- `cec1dbb`: module-policy and capability/export contracts now enforce ordered
  parity (fixture/source/docs where applicable).
- `ca6f0af`: host seam audit snapshots were folded into default local wasm gate
  scripts.
- `276a82c`: execution blocker matrix test now enforces deterministic blocker
  ordering with uniqueness guard.
- latest: stdlib native-link `extern` blocks now use target-aware
  `cfg_attr(not(target_arch = "wasm32"), link(name = ...))` in
  `bz2`/`lzma`/`sqlite3`/`zlib`, so `wasm-vm-probe` can compile and wasm
  lib test link lanes no longer fail on missing `-lbz2/-llzma/-lsqlite3/-lz`.
- latest: both local wasm gate entrypoints (`check_wasm_branch.sh` and
  `run_wasm_contract_smoke.sh`) now execute `probe_wasm_vm_compile.sh`, so
  wasm vm-probe compile + lib-test link lanes are always exercised.
- latest: top-level `execute()` now has a feature-gated runtime probe path:
  when built with `wasm-vm-probe`, parse+compile-valid and capability-allowed
  snippets execute via `Vm::new_with_host(WasmHost)` and return
  `phase = \"ok\"` or `phase = \"runtime_error\"`; default wasm builds keep the
  existing unsupported-execution contract.
- latest: `wasm_worker_execute()` now mirrors the same feature-gated runtime
  probe behavior for capability-allowed snippets (`ok`/`runtime_error`) while
  default builds keep `unsupported_worker_execution`; worker lifecycle stubs
  (`start`/`terminate`/`recycle`) remain intentionally unwired.
- latest: vm-probe local gates now also compile-check `--test wasm_contract`
  with `wasm-vm-probe`, keeping feature-gated wasm API/test contracts in sync.
- latest: runtime metadata/blocker exports are now feature-aware for wasm
  vm-probe builds (`wasm_runtime_info.supports_execution = true`,
  `execution_backend = \"vm_probe\"`, and unwired execution blocker keys are
  omitted when the probe runtime is enabled).
- latest: execute-contract fixtures/scripts are now dual-mode aware:
  fixture rows can declare vm-probe-specific execute expectations, a dedicated
  runtime-error fixture (`1 / 0`) is tracked, execute-summary validation can
  run in `--vm-probe` mode, and both wasm gate scripts now emit
  `perf/wasm_execute_contract_summary_vm_probe_latest.json` in addition to the
  default summary.
- latest: worker execute fixtures/scripts now mirror the same dual-mode
  contract guardrail: fixture rows support vm-probe-specific execute
  expectations (`ok`/`runtime_error`), worker runtime-error coverage is tracked
  in fixture rows, worker-summary validation can run in `--vm-probe` mode, and
  both wasm gate scripts now emit
  `perf/wasm_worker_contract_summary_vm_probe_latest.json`.
- latest: wasm worker-session contract tests are now vm-probe aware for
  execute/execute-with-operation baseline snippets (`x = 1`), so probe-mode
  `ok` behavior and default-mode `unsupported_worker_execution` behavior are
  both explicitly asserted.
- latest: vm-probe execute/worker contract summaries are now emitted directly
  by `scripts/probe_wasm_vm_compile.sh`; branch/smoke wrapper scripts rely on
  that lane for vm-probe summary artifacts and keep default-summary generation
  local to wrapper scope.
- latest: top-level `execute()` and `wasm_worker_execute()` now share a single
  contract-mode execution helper in `src/wasm/mod.rs`, removing duplicated
  parse/compile/blocker/runtime-probe fallback logic and reducing future
  drift risk between top-level and worker wasm execution paths.
- latest: `wasm-vm-probe` worker execution now uses a persistent worker VM while
  `state = "ready"`; worker lifecycle controls now apply runtime resets
  deterministically (`start`/`recycle` reset VM state, `terminate` clears it).
- latest: worker non-ready gating now distinguishes `worker_runtime_unwired`
  from `worker_runtime_failed`; vm-probe worker execute paths enter `failed`
  state for internal runtime failures and require `start`/`recycle` recovery.
- latest: native wasm unit tests in `src/wasm/mod.rs` are now feature-aware for
  `wasm-vm-probe` (phase-key exports and baseline execute/worker-execute
  behavior), reducing hidden drift when vm-probe-enabled native test lanes are
  introduced later.
- latest: feature-gated native wasm unit tests now also assert vm-probe
  runtime-error contracts for top-level and worker execution (`1 / 0` =>
  `phase = "runtime_error"`, no blocker key, populated line/column).
- latest: `generate_wasm_worker_contract_summary.py` now validates effective
  worker execute contract semantics (success/error/line-column) per phase in
  both default and vm-probe modes, not just phase/blocker-key parity.
- latest: `generate_wasm_execute_contract_summary.py` now enforces vm-probe
  support-phase mapping semantics: `supported` fixtures must resolve to
  `ok/runtime_error` in vm-probe mode while `blocked_capability` fixtures
  remain `unsupported_execution`.
- latest: host seam now exposes `host::WebHost` as an explicit compatibility
  alias over `WasmHost`, aligning code terminology with the W3 “web host”
  milestone without changing runtime behavior.
- latest: `generate_wasm_session_contract_summary.py` now validates session-level
  fixture invariants for top-level and worker baseline flows (default unwired
  vs vm-probe `ok/runtime_error` overrides), and is enforced by both
  `check_wasm_branch.sh` and `run_wasm_contract_smoke.sh`.
- latest: `generate_wasm_docs_execution_matrix_summary.py` now validates the
  `WASM_API_CONTRACT.md` execution-mode matrix against source phase/blocker
  contracts, and is enforced in both local wasm gate scripts.
- latest: `generate_wasm_worker_docs_contract_summary.py` now validates
  `WASM_WORKER_RUNTIME_CONTRACT.md` against source keys/constants (worker
  state/lifecycle/execute/timeout phases, interruption model, timeout bounds,
  and worker/module-policy blocker keys), and is enforced in both local wasm
  gate scripts.
- latest: worker-docs summary validation now also enforces operation-id prefix
  coverage (`worker_<action>_`) and default worker execute-operation state
  shape (`state = "unwired"`).
- latest: `generate_wasm_client_flow_summary.py` now validates
  `WASM_CLIENT_INTEGRATION_FLOW.md` call-order guidance and function tokens
  against source-exported wasm APIs, plus worker lifecycle/execute/vm-probe
  phase and `WasmWorkerSession` telemetry-field parity.
- latest: `generate_wasm_api_contract_surface_summary.py` now validates
  `WASM_API_CONTRACT.md` top-level export listings and exported type field
  coverage against `src/wasm/mod.rs`, and is enforced in both local wasm gate
  scripts.
- latest: `WasmWorkerSession` now tracks `last_state` telemetry alongside
  operation/phase/error metadata, and wasm contract tests/docs were updated to
  keep session-state reporting explicit for UI integration.
- latest: `WasmWorkerExecutionResult` now includes a structured `state` field
  (currently `"unwired"`), and worker execute-with-operation/session contract
  tests/docs now assert state parity directly.
- latest: `WasmWorkerSession.snapshot()` and
  `WasmWorkerSessionSnapshot` were added for atomic worker telemetry reads;
  snapshot fields are parity-tested against live session getters and documented
  in the wasm API contract.
- latest: worker state emission now routes through a centralized
  `current_worker_state*` seam in `src/wasm/mod.rs` so worker info/lifecycle
  timeout/execute-with-operation surfaces stay state-consistent as backend
  wiring evolves.
- latest: `WasmWorkerInfo` now exposes `execution_probe_enabled` so browser
  clients can detect `wasm-vm-probe` runtime availability without changing
  `supported` semantics for unwired worker lifecycle APIs.
- latest: `WasmWorkerInfo.backend` is now mode-aware (`"unwired"` default,
  `"vm_probe"` with `wasm-vm-probe`) while worker lifecycle APIs remain
  explicitly unsupported.
- latest: worker contract fixtures now include explicit `WASM_WORKER_INFO_FIXTURES`,
  wasm contract tests consume those fixtures for `wasm_worker_info()`/session
  info assertions, and `generate_wasm_worker_contract_summary.py` enforces
  fixture-vs-source parity for worker-info mode semantics.
- latest: `WasmWorkerInfo` now also exports `execute_supported`
  (default `false`, `true` in `wasm-vm-probe`), and worker-info fixtures +
  summary validation enforce mode-aware parity for this field.
- latest: `generate_wasm_worker_docs_contract_summary.py` now also enforces
  worker-info docs parity for backend mode keys (`"unwired"`/`"vm_probe"`) and
  `execution_probe_enabled`/`execute_supported` default-vs-vm-probe shape coverage.
- latest: `generate_wasm_client_flow_summary.py` now requires explicit
  `WasmWorkerInfo` integration tokens (`supported`, `backend`, `state`,
  `execution_probe_enabled`, `execute_supported`) in client-flow docs.
- latest: worker lifecycle contracts are now dual-mode aware: default builds
  keep explicit unsupported lifecycle phases while `wasm-vm-probe` exposes
  deterministic lifecycle probe phases (`worker_started`,
  `worker_terminated`, `worker_recycled`) with fixture/source/test parity.
- latest: worker/client docs and summary guards now enforce vm-probe lifecycle
  phase coverage in `WASM_WORKER_RUNTIME_CONTRACT.md`,
  `WASM_CLIENT_INTEGRATION_FLOW.md`, and `WASM_API_CONTRACT.md`.
- latest: `generate_wasm_api_contract_surface_summary.py` now validates worker
  lifecycle phase coverage (default + vm-probe extra lifecycle keys) in
  `WASM_API_CONTRACT.md` in addition to top-level export/type field parity.
- latest: `WasmWorkerSession` now carries lifecycle-derived state through
  `execute_with_operation` and `set_timeout_ms` telemetry updates (not just
  direct lifecycle calls), with vm-probe `ready` state preserved after recycle.
- latest: `generate_wasm_session_contract_summary.py` now validates worker
  lifecycle fixture state prerequisites (`recycle` vm-probe `ready`,
  `terminate` vm-probe `unwired`) alongside existing execute contract rows.
- latest: `WasmWorkerSession.info()` is now session-local state aware and
  reports lifecycle-derived state after start/terminate/recycle calls.
- latest: worker/client docs summary validators now require explicit
  session-local `info().state` guidance tokens, preventing silent drift in
  session-state integration docs.
- latest: worker timeout contracts are now dual-mode aware: default in-range
  updates remain `unsupported_worker_timeout_enforcement`, while
  `wasm-vm-probe` in-range updates return `worker_timeout_configured` with
  success/no blocker and runtime timeout enforcement on worker executes.
- latest: `WasmWorkerTimeoutPolicy` now exports explicit
  `configuration_supported` mode signaling (`false` default,
  `true` in `wasm-vm-probe`) and docs/client-flow validations enforce usage of
  that capability flag alongside mode-aware `enforcement_supported`.
- latest: timeout-policy `unsupported_reason` is now mode-aware: default builds
  report worker-runtime unwired status, while `wasm-vm-probe` reports no
  unsupported reason (`None`) because enforcement is wired in probe mode.
- latest: `WasmWorkerInfo` now exports explicit `lifecycle_supported`
  mode signaling (`false` default, `true` in `wasm-vm-probe`) so clients can
  branch lifecycle controls without inferring from phase text.
- latest: `WasmWorkerInfo` now also exports timeout capability signals
  (`timeout_configuration_supported` and `timeout_enforcement_supported`
  both mode-aware), keeping worker-info summary aligned with timeout policy
  semantics.
- latest: `WasmWorkerInfo.state` is now mode-aware (`"unwired"` default,
  `"ready"` in `wasm-vm-probe`) for consistent worker-readiness signaling.
- latest: timeout phase key parity and docs guards now include vm-probe timeout
  extras across worker-contract, worker-docs, and client-flow summary scripts.
- latest: `generate_wasm_api_contract_surface_summary.py` now also validates
  worker timeout phase coverage (default + vm-probe timeout extras) in
  `WASM_API_CONTRACT.md`.
- latest: `generate_wasm_session_contract_summary.py` now validates timeout
  fixture semantics too (invalid-timeout vs unsupported-enforcement rows and
  vm-probe `worker_timeout_configured` overrides), not just execute/lifecycle rows.
- latest: top-level worker lifecycle calls now mutate shared worker state and
  `wasm_worker_info().state` reflects the current top-level lifecycle state;
  worker contract tests/docs and summary checks now guard this behavior.
- latest: worker execute and timeout-configuration paths are now state-aware:
  in `wasm-vm-probe`, capability-allowed worker execute and in-range timeout
  configuration require `state = "ready"` (post-terminate calls now return
  unwired unsupported phases until `start()`/`recycle()` restores readiness),
  with fixture/tests/docs/summary guards updated for deterministic behavior.
- latest: `WasmWorkerSession` follow-up calls (`execute_with_operation`,
  `set_timeout_ms`) now retain operation-reported shared worker state instead of
  overriding with cached session-local state, preventing stale-state telemetry
  after external top-level lifecycle mutations.
- latest: mixed top-level/session lifecycle behavior is now fixture-driven:
  `WASM_WORKER_SESSION_STATE_GATE_FIXTURES` encodes post-terminate and
  post-recycle worker execute/timeout contracts, and
  `generate_wasm_session_contract_summary.py` now validates those fixture rows
  against lifecycle state/phase invariants and vm-probe overrides.
- latest: `WASM_API_CONTRACT.md` execution matrix now requires explicit worker
  vm-probe state branches (`state = "ready"` vs `state != "ready"`), with
  `generate_wasm_docs_execution_matrix_summary.py` enforcing those row tokens.
- latest: `WasmWorkerSession::info()` now reports shared top-level worker
  state (instead of session-local override), and mixed-flow tests/docs/session
  summaries were updated so external top-level lifecycle changes are reflected
  immediately in session info state.
- latest: website W6 bootstrap landed with a `/playground` Astro route and
  lazy wasm module loading flow (`/wasm/pyrs.js`) isolated to that page; top
  navigation now exposes the playground route while non-playground pages remain
  static-first.
- latest: W7 bootstrap landed with branch-scoped CI lane
  (`.github/workflows/wasm-track.yml`) running `scripts/check_wasm_branch.sh`
  plus artifact upload, and explicit promotion go/no-go criteria are now
  codified in `docs/WASM_PROMOTION_GATE.md`.
- latest: W7 CI now includes an optional `wasm-browser-smoke` job on manual
  dispatch; it installs `wasm-pack` and executes
  `scripts/run_wasm_contract_smoke.sh` with browser and vm-probe state-gate
  smoke flags enabled (`PYRS_WASM_SKIP_CORE_SMOKE=1` in CI to avoid duplicate
  core checks), while keeping the mandatory gate lane deterministic.
- latest: manual `wasm-browser-smoke` dispatch now fails hard on smoke
  regressions (no `continue-on-error`), so promotion evidence cannot silently
  ignore browser-lane failures.
- latest: browser-smoke dispatch and artifact capture now have an explicit
  operator runbook (`docs/WASM_BROWSER_SMOKE_RUNBOOK.md`) to standardize first
  baseline collection and future promotion evidence.
- latest: successful browser-smoke runs now emit a machine-readable baseline
  summary artifact (`perf/wasm_browser_smoke_baseline_latest.json`) from
  `scripts/run_wasm_contract_smoke.sh`, and CI uploads it with
  `wasm-browser-smoke-artifacts` for promotion evidence tracking.
- latest: browser-smoke CI now validates baseline-summary shape via
  `scripts/validate_wasm_browser_smoke_baseline.py` before artifact upload, so
  missing/malformed baseline output fails the lane immediately.
- latest: native-only integration test crates are now explicitly gated with
  `#![cfg(not(target_arch = "wasm32"))]`, so wasm-target browser smoke compiles
  only wasm-relevant suites instead of failing on native `pyrs::vm` imports.
- latest: both wasm local gate entrypoints (`scripts/check_wasm_branch.sh` and
  `scripts/run_wasm_contract_smoke.sh`) now include explicit
  `cargo check --target wasm32-unknown-unknown --tests` compile gates in
  default and `wasm-vm-probe` modes, preventing future wasm-pack compile drift
  from native-only integration suites.
- latest: `run_wasm_contract_smoke.sh` browser helper now aggregates
  per-substep failures (integration + lib wasm-pack runs) before returning,
  so fallback and pass/fail decisions cannot silently ignore a failed smoke
  sub-command.
- latest: docs index now links to `/playground/` as an explicit browser-route
  entrypoint in Getting Started navigation.
- latest: browser routes are now split by product UX vs diagnostics:
  - `/playground` is the minimal product-facing wasm REPL route with automatic
    runtime load, load-status indicator, and session-oriented input/output flow,
  - `/debug` is the unlisted diagnostics route carrying runtime introspection,
    preflight, and worker-state probe controls.
- latest: wasm website artifact generation now has a dedicated size-first build
  path:
  - Cargo profile `release-wasm` (`opt-level = "z"`, `strip = "symbols"`,
    release inheritance),
  - script `scripts/build_wasm_website_bundle.sh` using
    `cargo build --profile release-wasm --features wasm-vm-probe` followed by
    direct `wasm-bindgen` emission into `website/public/wasm/`,
  keeping native release tuning speed-focused while wasm artifacts prioritize
  browser payload size.
- latest: playground worker controls now route through `WasmWorkerSession`
  when available, exposing `worker_session_info`/`worker_session_snapshot`
  telemetry in runtime inspector output and supporting session-telemetry reset
  without mutating shared worker lifecycle state.
- latest: docs now include a dedicated browser playground reference page
  (`website/src/pages/docs/playground.mdx`) and docs navigation wiring, with
  explicit interpretation guidance for preflight payloads and worker control
  phase expectations.
- latest: `run_wasm_contract_smoke.sh` now supports
  `PYRS_WASM_SKIP_CORE_SMOKE=1` so browser-smoke lanes can run wasm-pack
  checks without re-running core compile/summary/nextest gates.
- latest: wasm gate scripts now execute host wasm-bridge unit-contract tests
  (`cargo nextest run --lib wasm_`) in both default and vm-probe lanes
  (vm-probe path via `scripts/probe_wasm_vm_compile.sh`) instead of relying
  only on compile-time wasm contract harness checks.
- latest: `src/wasm/mod.rs` unit-contract coverage now includes explicit worker
  lifecycle state-gate assertions for both default and vm-probe modes
  (start/terminate/recycle phase+state, execute/timeout behavior after
  terminate/recycle), so these invariants are exercised by browser wasm lib
  test lanes and compile-validated in local branch gates.
- latest: `wasm_worker_info.supported` is now mode-aware (`false` default,
  `true` in `wasm-vm-probe`), with fixture/test/summary/docs alignment so
  worker readiness signals stay internally consistent with backend capability
  flags.
- latest: `generate_wasm_worker_docs_contract_summary.py` now enforces
  docs-level `supported = false` default and `supported = true` vm-probe
  wording for worker info contract docs, preventing future mode-signal drift.
- latest: `generate_wasm_worker_contract_summary.py` now enforces that
  `wasm_worker_info.supported` is sourced from `wasm_vm_runtime_enabled()`
  (not a stale literal), so source contract checks protect mode-aware support
  semantics directly.
- latest: worker timeout configuration now has persistent contract state:
  - `wasm_worker_current_timeout_ms()` exports the current worker timeout value,
  - `wasm_worker_set_timeout(...)` in vm-probe ready state updates that value,
  - worker lifecycle reset calls (`start` / `terminate` / `recycle`) reset it
  to the default `5000` ms,
  with wasm unit + wasm contract + docs/summary gate coverage to prevent drift.
- latest: vm-probe worker runtime errors are now explicitly contract-locked to
  preserve worker readiness:
  - `phase = "runtime_error"` leaves worker `state = "ready"`,
  - subsequent capability-allowed executes continue in the same worker VM
    session state,
  covered by both native wasm unit tests and wasm browser-contract tests.
- latest: vm-probe worker execution now applies configured timeout guards via
  VM execution deadlines:
  - worker executes run with `wasm_worker_current_timeout_ms()` when in
    `state = "ready"`,
  - timeout runtime errors are treated as recoverable (no transition to
    `failed` state),
  - timeout-triggered recycle resets worker VM state and timeout value to
    default (`5000` ms), preserving deterministic recovery semantics.
- latest: browser vm-probe smoke coverage now includes:
  - timeout-state assertions via `wasm_worker_current_timeout_ms()` (configured
    timeout visible before execute and reset to default after timeout-triggered
    recycle),
  - cross-path state-gate validation that top-level `execute()` remains
    functional while worker state is `unwired` (worker execute blocked,
    top-level execute still `phase = "ok"`).
- latest: browser vm-probe smoke now also covers `WasmWorkerSession` shared
  lifecycle parity:
  - session `info().state` reflects external top-level lifecycle transitions,
  - session execute-with-operation follows worker state-gates (`unwired` after
    terminate),
  - session snapshot telemetry (`last_phase`, `last_state`,
    `executes_requested`) remains consistent in browser lane.
- latest: `scripts/run_wasm_contract_smoke.sh` now hard-fails browser smoke
  when `wasm-pack` output contains `output filename collision`, preventing
  silent regression toward Cargo’s future hard-error behavior for wasm bin/lib
  artifact name conflicts.
- latest: `/playground` runtime execution now runs through a dedicated browser
  Web Worker transport (`website/public/workers/playground-runtime-worker.js`)
  so wasm load/execute/reset calls are off-main-thread:
  - main-thread UI uses request/response worker RPC (`load`, `execute`,
    `reset`) instead of direct `WasmReplSession`/`WasmSession` calls,
  - REPL transcript/highlighting behavior remains unchanged while execution is
    isolated from UI event-loop work,
  - reset now targets worker-side session state directly.
- latest: local wasm gates now enforce playground worker transport contract
  invariants via `node scripts/check_playground_worker_contract.mjs` in both
  `scripts/check_wasm_branch.sh` and
  `scripts/run_wasm_contract_smoke.sh`.
- latest: playground worker-contract snapshots are now included in the wasm
  evidence pack (`perf/wasm_playground_worker_contract_latest.json`) and
  validated as a required promotion artifact (`ok=true`, `failure_count=0`).
- latest: client/API docs now include explicit `/playground` worker RPC
  envelope and action contracts (`load`, `execute`, `reset`) in
  `docs/WASM_CLIENT_INTEGRATION_FLOW.md` and `docs/WASM_API_CONTRACT.md`,
  with both docs summary generators re-run to keep source/doc gates current.
- latest: browser-smoke capture now has an operator helper script
  (`scripts/run_wasm_browser_smoke_dispatch.sh`) that dispatches
  `wasm-track.yml`, waits for completion, downloads artifacts, and validates
  `wasm_browser_smoke_baseline_latest.json` from the run output.

Latest host seam audit (local branch run):
- `python3 scripts/audit_wasm_host_seam.py` => `total_hits=0` (`allowlisted_hits=0`).

Remaining near-term focus:
1. W5: increase worker runtime execution coverage in vm-probe mode (state/lifecycle edges).
2. W6: refine playground UX/error affordances from first real browser-smoke transcripts.
3. W7: capture and publish first workflow-dispatch browser-smoke baseline artifact.

### W6-native: Browser REPL UX Convergence Plan

Goal:
- make `/playground` feel like a native `pyrs` REPL first, while keeping
  wasm-loading diagnostics available outside the transcript surface.

Milestones:
1. Transcript model convergence
   - remove synthetic transcript status lines (for example
     `# loading wasm runtime`, `# runtime loaded (...)`).
   - remove command numbering/meta separators (`#1`, `#2`, etc.).
   - ensure successful statement execution without output produces no extra
     synthetic line noise.
2. Shell framing convergence
   - terminal-first layout with minimal chrome and full-width transcript/input.
   - prompt-forward interaction (`>>>` primary prompt, `...` continuation).
   - native-like startup banner shape rendered once per session bootstrap.
3. Input interaction convergence
   - enter executes, shift+enter inserts newline, arrow-up/down history recall.
   - preserve multiline replay into transcript with prompt continuity.
   - keep focus behavior deterministic after each run/reset.
4. Status/diagnostics separation
   - keep runtime loading state in dedicated UI status indicator (outside
     transcript).
   - keep detailed diagnostics and worker probes on `/debug`, not in product
     REPL transcript.
5. Visual parity pass
   - align typography, spacing, and color emphasis with native terminal feel.
   - avoid nested “box-within-box” patterns in the REPL interaction surface.

Acceptance criteria:
- first visit auto-loads runtime without transcript status spam.
- transcript contains only REPL prompts, user input, runtime stdout/stderr,
  and native-style banner text.
- no synthetic “phase/blocker/line-column” meta lines in happy path.
- clear/reset actions keep a clean REPL narrative (not debug-log narrative).

Current `wasm-vm-probe` snapshot (non-gating, latest local run):
- compile status: `scripts/probe_wasm_vm_compile.sh` now completes without
  compile errors.
- current probe status is clean in the scripted probe lane (no active compile
  blockers and no warning debt from the most recent local probe run).
- link blocker status: `perf/wasm_vm_link_blockers_latest.json` tracks
  remaining source-level native-link attributes under `src/vm` plus
  wasm-active filtering; current snapshot reports
  `active_hits=0` and `known_stdlib_blockers=0` after wasm-safe target gating
  on stdlib C-link modules.
- env-import blocker status: `perf/wasm_vm_env_import_summary_latest.json`
  tracks remaining raw wasm `env` imports (current vm-probe baseline) so
  browser-loader closure can be driven by import-family elimination rather than
  trial-and-error patching.
- gate hardening: `scripts/validate_wasm_evidence_pack.py` now enforces
  `counts.env_function_imports == 0` (and no missing shim symbols) from
  `perf/wasm_vm_env_import_summary_latest.json`, so non-zero wasm vm-probe
  `env` imports fail contract-gate/browser-smoke evidence validation.
- latest: wasm vm-probe lane now uses wasm-native allocator shims
  (`malloc`/`calloc`/`realloc`/`free`) plus wasm-native float-formatting and
  `PyOS_strto*` parsing paths (no direct wasm libc imports for
  `snprintf`/`strtol`/`strtoul`/`strtod`), wasm-safe `zlib`/`bz2`/`lzma`
  shims for stdlib compression modules, and wasm-safe `sqlite3` stubs for
  stdlib database module compilation.
- latest: wasm vm-probe lane now gates `cpython_keepalive_exports` on native
  targets and provides a wasm-native `PyOS_FSPath` bridge. This removes the
  remaining CPython C-API import family and reduces raw wasm `env` imports from
  `121` to `0` in the current probe baseline.

## Risk Register

1. Hidden native coupling discovered late.
   - Mitigation: compile isolation and host-interface extraction before feature work.
2. Native regressions from shared refactor.
   - Mitigation: mandatory targeted native test gates per checkpoint.
3. Browser-mode expectation mismatch.
   - Mitigation: explicit capability matrix and hard unsupported errors.
4. Build/CI complexity growth.
   - Mitigation: staged CI enablement and strict ownership.
