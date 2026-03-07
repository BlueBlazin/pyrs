# Documentation Map

Use this file to find canonical ownership quickly.

## Local CPython Reference Root
- Preferred local CPython 3.14.3 checkout path: `.local/Python-3.14.3` (untracked, git-ignored).
- Preferred stdlib path for probes/runtime wiring: `.local/Python-3.14.3/Lib`.

## Planning and Release
- `docs/ROADMAP.md`: milestone order, execution strategy, and exit criteria.
- `docs/PRODUCTION_READINESS.md`: release blockers and completion requirements.
- `docs/COMPATIBILITY.md`: subsystem-level compatibility summary.
- `docs/RELEASE_PLAN_BETA.md`: staged beta release plan (tracked, not yet active execution).
- `docs/EXTENSION_ECOSYSTEM_DESIGN.md`: architecture and quality gates for NumPy/SciPy/Pandas/Matplotlib support.
- `docs/EXTENSION_CAPABILITY_MATRIX.md`: source-of-truth status table for extension API/loader surfaces.
- `docs/EXTENSION_PACKAGING_CONTRACT.md`: extension build/package contract (`pyrs314` mode first).
- `docs/EXTENSION_CAPI_V1.md`: first shipped C-API header/symbol slice for compiled-extension bring-up.
- `docs/CAPI_PLAN.md`: two-lane C-API execution plan (Stable ABI closure + NumPy-required non-abi3 closure).
- `docs/CAPI_LIFETIME_MODEL.md`: P0 ownership/lifetime architecture for CPython-compat pointers and UAF closure plan.
- `docs/WEBSITE_DOCS_DESIGN_SYSTEM_PLAN.md`: multi-milestone website/docs design-system and IA execution plan.

## WASM Track (Isolated Branch)
- `docs/WASM_EXECUTION_PLAN.md`: canonical wasm milestone tracker and current branch execution status.
- `docs/WASM_PROMOTION_GATE.md`: explicit go/no-go criteria and evidence requirements before any merge candidacy.
- `docs/WASM_API_CONTRACT.md`: top-level wasm bridge export and contract surface.
- `docs/WASM_WORKER_RUNTIME_CONTRACT.md`: worker lifecycle/execute/timeout contract semantics.
- `docs/WASM_CLIENT_INTEGRATION_FLOW.md`: browser/client integration order and worker RPC flow.
- `docs/WASM_BROWSER_SMOKE_RUNBOOK.md`: workflow-dispatch browser smoke runbook and artifact capture process.
- `scripts/run_wasm_browser_smoke_dispatch.sh`: one-shot dispatch/watch/download helper for browser-smoke evidence capture.
- `scripts/extract_wasm_ci_artifact_hashes.py`: extract artifact IDs + SHA256 digests from a workflow run log for promotion notes.
- `scripts/check_wasm_artifact_hash_extractor_contract.py`: fixture-backed contract check for artifact-hash extraction behavior.
- `scripts/update_wasm_dispatch_snapshot.py`: update runbook/promotion/execution snapshot references to a specific workflow-dispatch run.
- `scripts/check_wasm_dispatch_snapshot_contract.py`: validate snapshot-updater assumptions against current docs formatting.
- `scripts/generate_wasm_artifact_input_hashes.py`: generate deterministic local SHA256 summary for evidence-pack source artifacts.
- `scripts/generate_wasm_stdlib_subset_summary.py`: validate curated stdlib subset pack manifest/sha and emit version+size evidence summary.

## Gap Tracking
- `docs/STUB_ACCOUNTING.md`: partial/stub implementation ledger.
- `docs/LANGUAGE_FEATURE_INVENTORY.md`: source-derived CPython 3.14 language-feature inventory baseline and regeneration flow.
- `docs/LANGUAGE_FEATURE_INVENTORY.json`: machine-readable full inventory extracted from CPython grammar/tokens/reference docs.
- `docs/LANGUAGE_FEATURE_PROBE_MAP.json`: mapping from current manifest probes to inventory rows for pass/fail/unprobed accounting.
- `docs/CAPI_NOOP_INVENTORY.md`: intentional C-API no-op/placeholder inventory with closure criteria.
- `docs/NOOP_BUILTIN_CLASSIFICATION.md`: split current no-op builtin inventory into production-facing vs test-only symbols.
- `docs/ALGO_AUDIT_BACKLOG.md`: algorithmic/semantic risk backlog.
- `docs/STDLIB_COMMON_USECASE_CHECKLIST.md`: top-stdlib baseline closure tracker.
- `docs/STDLIB_EXTENDED_COMMON_USECASE_CHECKLIST.md`: expanded stdlib smoke matrix and blocker grouping.
- `docs/OBJECT_MODEL_AUDIT.md`: object-model parity audit status.

## Runtime and Architecture
- `docs/VM_ARCHITECTURE_MAP.md`: VM module ownership and placement rules.
- `docs/VM_ERROR_MODEL_REFACTOR.md`: plan to replace string-based runtime error classification with typed exception transport.
- `docs/REPL_SHARED_CORE_DESIGN.md`: proposed shared REPL semantic core architecture for native CLI and wasm adapters.
- `docs/STDLIB_MIGRATION_PLAN.md`: pure-stdlib-first migration policy.
- `docs/ENGINEERING_GATES.md`: mandatory process and quality gates.
- `docs/COVERAGE_GATE_POLICY.json`: policy source for coverage-gate floors, ignores, and targeted test bins.

## Performance and Optimization
- `docs/OPTIMIZATION_PLAN.md`: optimization execution policy/workstreams.
- `docs/OPTIMIZATION_BACKLOG.md`: optimization item ledger with statuses.
- `docs/BUILTIN_OPTIMIZATION_POLICY.md`: builtin-specific optimization policy.

## Validation and Artifacts
- `docs/BUILTIN_PARITY.md`: builtin parity gate definition and closure rules.
- `docs/CPYTHON_TEST_SUITE_TRIAGE.md`: current CPython test-suite fix loop, blocker classes, and benchmark-driven priority order.
- `docs/NUMPY_BRINGUP_GATE.md`: NumPy import/ndarray bring-up probe and current status.
- `perf/language_feature_manifest_latest.json`: latest CPython differential probe run for source-language manifest.
- `perf/language_feature_coverage_latest.json`: inventory-level pass/fail/unprobed accounting from probe map.
- `docs/UNICODE_NAME_DATA.md`: Unicode-name data provenance/regeneration.
- `docs/DICT_BACKEND_CPYTHON_MAPPING.md`: dict backend design mapping.
- `docs/DICT_BACKEND_BENCHMARK.md`: dict backend benchmark snapshot artifact.
- `docs/CLONE_AUDIT.md`, `docs/CLONE_BASELINE.txt`: clone pressure inventory.
- `docs/MILESTONE_12_BACKLOG.md`: historical milestone-12 closure report.
- `docs/DEVELOPER_TOOLING.md`: optional local dev tooling install and sanitizer runbook.

## Testing Guidance Consolidation
- Canonical local test-runner guidance (including `cargo nextest` defaults): `docs/DEVELOPER_TOOLING.md`.
- Canonical CI/process gate definitions: `docs/ENGINEERING_GATES.md`.
- Canonical coverage-gate policy inputs: `docs/COVERAGE_GATE_POLICY.json`.
