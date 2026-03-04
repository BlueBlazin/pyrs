# WASM Curated Stdlib Subset Plan (Top 26, `.py`-only)

Status: in progress (M1-M3 complete, M4 in progress)  
Date: 2026-03-04  
Owner: runtime/wasm track

## Goal

Provide a curated, production-quality browser stdlib subset for the website playground so common Python workflows (including `functools.cache`) behave like native PYRS where feasible, without regressing native interpreter behavior.

## Product Constraints

- Native PYRS remains the primary product. WASM is demo/playground only.
- No native runtime regression risk is acceptable.
- Keep browser payload small and startup responsive.
- No dynamic libraries / extension loading in browser mode.
- No broad host capability expansion (filesystem, sockets, subprocess, etc.).

## Why This Is Needed

WASM currently uses bootstrap modules for many imports. For `functools`, bootstrap wiring maps `cache`/`lru_cache` to a placeholder builtin path, so memoization semantics are missing in browser mode.

## Chosen Direction (Option 1)

Ship a curated subset of CPython `Lib/*.py` modules for WASM and load them from an in-memory stdlib pack (not host filesystem). Missing modules continue to follow current WASM policy (unsupported capability errors or existing bootstrap behavior).

## Candidate Top-26 Seed Modules

This seed list is REPL-first and intentionally excludes filesystem-centric modules
(`pathlib`, `glob`) and debugging/introspection-heavy modules (`inspect`, `ast`,
`dis`, `tokenize`, `traceback`) for the initial browser payload.

1. `functools`
2. `random`
3. `collections`
4. `abc`
5. `types`
6. `operator`
7. `dataclasses`
8. `typing`
9. `statistics`
10. `enum`
11. `copy`
12. `contextlib`
13. `re`
14. `string`
15. `textwrap`
16. `pprint`
17. `json`
18. `fractions`
19. `decimal`
20. `heapq`
21. `bisect`
22. `urllib.parse`
23. `numbers`
24. `difflib`
25. `datetime`
26. `argparse`

Related note:

- `itertools` is not in this `.py` pack list because CPython provides it as a C module
  (not `Lib/itertools.py`). PYRS already provides native `itertools` substrate in Rust,
  so it should remain available without adding `.py` payload bytes.

## Size Budget (Measured on local CPython 3.14.3 `Lib`)

Measured closure-based estimates for the REPL-first seed list (`.py` files only, non-test):

- Seed-26 closure:
  - raw: `1,392,419` bytes
  - zip(deflate): `345,832` bytes
- Reference full non-test `.py` zip:
  - ~`3,197,528` bytes

Target budget for initial subset pack:

- `stdlib_subset_v1.zip <= 500 KB` compressed
- Keep wasm binary size growth minimal (prefer external pack asset, lazy-loaded)

Current generated pack snapshot (`website/public/wasm/stdlib_subset_manifest_v1.json`):

- module_count: `57`
- zip_bytes: `489,945`
- json_pack_bytes: `2,022,235`

## Architecture Plan

## 1) Build-Time Stdlib Pack Generation

Add generator script:

- `scripts/build_wasm_stdlib_subset.py`

Responsibilities:

- Read seed module list.
- Resolve import closure against local CPython `Lib` (only `.py`, exclude tests).
- Emit:
  - `website/public/wasm/stdlib_subset_v1.zip`
  - `website/public/wasm/stdlib_subset_manifest_v1.json`
- Manifest includes:
  - pack version,
  - seed modules,
  - included modules/files,
  - sha256,
  - byte counts (raw/compressed).

## 2) WASM Runtime Pack Loader

Add a wasm-only stdlib provider in `src/wasm/mod.rs` flow:

- Load JSON stdlib source pack once at runtime init.
- Build in-memory map:
  - module name -> source text,
  - package name -> `__init__.py` source.
- Keep provider wasm-scoped only (no native path changes).

## 3) VM Import Resolver Hook (Non-invasive to Native)

Add a VM resolver seam for virtual source modules:

- Before filesystem root probing, check optional virtual stdlib provider.
- If module exists in virtual provider:
  - return a virtual source spec (`module`, `is_package`, synthetic filename).
- Compile/execute source from memory (do not require `std::fs::read`).

Design rule:

- Virtual-source import path must be opt-in and only active when provider exists (WASM).
- Native import path remains unchanged.

## 4) Source Identity and Diagnostics

For traceback/debug parity, assign stable synthetic filenames:

- e.g. `<wasm-stdlib>/functools.py`
- and for packages: `<wasm-stdlib>/pkg/__init__.py`

## 5) Website Integration

Playground bootstrap:

- Auto-load wasm runtime.
- Auto-load stdlib pack (single fetch) before first import/execute needing subset.
- Show compact status in existing runtime-ready indicator only if load fails (silent success path).

## 6) Fallback Policy

- If module exists in curated pack: use packed source.
- Else: use existing WASM behavior (bootstrap import / unsupported capability policy).
- No silent module stubs added in this work.

## Milestones

## M1: Pack Builder + Manifest

Deliverables:

- subset seed list file (source of truth),
- pack generator script,
- generated zip + manifest artifacts,
- CI check for deterministic manifest output.

Exit criteria:

- pack generated reproducibly,
- size budget reported and tracked.

Status: complete (2026-03-04)
- `scripts/build_wasm_stdlib_subset.py` now emits deterministic zip + manifest.
- CI/build hook added in `scripts/build_wasm_website_bundle.sh`.
- Pack budget enforced via `--max-zip-bytes`.

## M2: Virtual Stdlib Provider API

Deliverables:

- VM-side optional virtual source resolver seam,
- memory-source compile/exec support for imports.

Exit criteria:

- unit tests: virtual module + package import works without filesystem.

Status: complete (2026-03-04)
- VM resolver now supports opt-in virtual source specs before filesystem probing.
- Source execution supports in-memory virtual module text.
- Regression coverage added in `tests/vm.rs` for module/package import and traceback filename shape.

## M3: WASM Loader Wiring

Deliverables:

- wasm runtime loads subset pack and registers provider.
- REPL executes imports from packed modules.

Exit criteria:

- `import functools` on WASM resolves from packed source.

Status: complete (2026-03-04)
- wasm runtime exports virtual-stdlib registration API:
  - `wasm_virtual_stdlib_clear`
  - `wasm_virtual_stdlib_register`
  - `wasm_virtual_stdlib_count`
- playground worker now auto-loads `wasm/stdlib_subset_v1.json` during runtime load and registers all module sources before creating `WasmReplSession`.
- Builder now also emits JSON source pack (`stdlib_subset_v1.json`) alongside zip + manifest.
- Closure probe now runs with CPython frozen modules disabled (`-X frozen_modules=off`) to avoid missing pure-Python dependencies behind frozen stdlib modules.
- WASM curated pack explicitly excludes `os` and keeps native `os` substrate ownership in browser runtime, preventing `os.py` dependency cascades (`stat`, etc.) from breaking common imports.
- Verified interactive imports in WASM REPL:
  - `import functools` (including `functools.cache` behavior path),
  - `import random` (including `collections/_collections_abc` dependency chain).

## M4: Parity Targets (Initial)

Deliverables:

- dedicated WASM tests for key modules:
  - `functools.cache`,
  - `functools.lru_cache`,
  - `dataclasses`,
  - `statistics`,
  - `random` import + runtime behavior (including reset/lifecycle stability).

Exit criteria:

- `functools.cache` memoization behavior matches native for representative cases.
- REPL session reset does not force deterministic `random` repeats from fixed seed state.

Status: in progress (advanced)
- Added wasm contract tests in `tests/wasm_contract.rs` (`wasm-vm-probe` lane):
  - `wasm_vm_probe_functools_cache_semantics_match_contract`
  - `wasm_vm_probe_random_values_change_after_repl_reset_and_worker_recycle`
  - `wasm_vm_probe_dataclasses_repl_parity_smoke`
  - `wasm_vm_probe_statistics_repl_parity_smoke`
  - `wasm_vm_probe_combined_curated_subset_repl_scenario`
  - `wasm_vm_probe_combined_curated_subset_worker_scenario`
- These tests explicitly load the curated stdlib source pack into runtime virtual-module registry before REPL/worker execution.

## M5: CI + Evidence

Deliverables:

- wasm contract/test lane includes subset-pack smoke.
- artifact summary includes stdlib subset version/hash/size.

Exit criteria:

- CI demonstrates stable load+import behavior for subset modules.

Status: in progress (CI blocker closure complete for current lane)
- `scripts/check_playground_worker_contract.mjs` now gates stdlib pack wiring:
  - verifies `stdlibPackPath` propagation from page to worker load request,
  - verifies worker stdlib load sequence appears before REPL session creation/execute flow.
- Curated pack builder now enforces exclusion guardrails so `os` cannot silently re-enter the subset (`scripts/build_wasm_stdlib_subset.py`).
- `scripts/run_wasm_contract_smoke.sh` now refreshes `perf/wasm_artifact_input_hashes_latest.json` before evidence-pack validation, keeping local/CI gate flow deterministic after contract-summary changes.
- Added explicit stdlib subset evidence artifact:
  - `scripts/generate_wasm_stdlib_subset_summary.py` emits `perf/wasm_stdlib_subset_summary_latest.json`
    with pack version, module count, and zip/source-pack size+sha checks against
    `website/public/wasm/stdlib_subset_manifest_v1.json`.
  - `scripts/check_wasm_branch.sh` now generates this summary and includes it in
    evidence-pack hash/manifest requirements.
- Latest verified workflow-dispatch checkpoint:
  - run: [22673172834](https://github.com/BlueBlazin/pyrs/actions/runs/22673172834)
  - commit: `1cddc9951faa537271d29d9400702fc35dbca6b7`
  - jobs: `wasm-contract-gate=success`, `wasm-browser-smoke=success`
- Closed CI blockers for curated-subset/browser lane:
  - browser-only smoke (`PYRS_WASM_SKIP_CORE_SMOKE=1`) now prebuilds curated
    stdlib pack before wasm test compile, so
    `website/public/wasm/stdlib_subset_v1.json` is always available,
  - `scripts/run_wasm_contract_smoke.sh` now uses `rg` or `grep` fallback for
    output-collision detection (no hard failure when `rg` is absent in runner).

## Test Plan

## Required Tests

- Rust unit/integration:
  - virtual source resolver behavior,
  - module/package import from memory provider,
  - traceback filename shape for virtual modules.
- WASM contract tests:
  - import + execute from packed modules,
  - `functools.cache` call-count regression.
- Website worker smoke:
  - first-load pack fetch + runtime init,
  - REPL command sequence using packed modules.

## Regression Tests (Specific)

- `@functools.cache` recursive fib call-count is reduced vs uncached baseline.
- `functools.cache` exposes CPython-shaped wrapper attrs expected by stdlib paths (`cache_info`, `cache_clear` where applicable via CPython `Lib/functools.py` behavior).
- `random` auto-seed path is not constant across fresh VMs (`seed(None)` / constructor default).
- Curated subset resolution keeps `collections` / `_collections_abc` available for `random` import chain.

## Risks and Mitigations

1. Import resolver complexity drift
- Mitigation: add single virtual provider seam; do not fork import architecture.

2. Size creep from dependency closure
- Mitigation: strict seed allowlist + manifest diff gate + pack size threshold.

3. Native regression risk
- Mitigation: provider is optional + wasm-only wiring; native code path untouched by default.

4. Hidden dependency on C-extension modules
- Mitigation: seed list review excludes extension-required modules for initial rollout.

## Out of Scope for This Plan

- Full stdlib in WASM.
- NumPy/scientific stack in WASM.
- Host capability expansion (sockets/process/fs writes/dynamic loading).
- UI redesign of web REPL.

## Decision Log

- Adopt Option 1 (curated subset) as initial production demo strategy.
- Keep Option 2 (larger general stdlib pack) as future fallback if product scope changes.
- For WASM demo runtime, keep native `os` substrate as policy and do not pack `os.py` by default.
- Drive closure discovery with CPython frozen modules disabled to avoid hidden-dependency omissions.
