# WASM Curated Stdlib Subset Plan (Top 30, `.py`-only)

Status: proposed (not started)  
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

## Candidate Top-30 Seed Modules

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
23. `weakref`
24. `numbers`
25. `copyreg`
26. `keyword`
27. `reprlib`
28. `difflib`
29. `datetime`
30. `argparse`

Related note:

- `itertools` is not in this `.py` pack list because CPython provides it as a C module
  (not `Lib/itertools.py`). PYRS already provides native `itertools` substrate in Rust,
  so it should remain available without adding `.py` payload bytes.

## Size Budget (Measured on local CPython 3.14.3 `Lib`)

Measured closure-based estimates for the REPL-first seed list (`.py` files only, non-test):

- Seed-20 closure (first 20 modules in this list):
  - raw: `1,005,443` bytes
  - zip(deflate): `251,637` bytes
- Seed-30 closure:
  - raw: `1,346,056` bytes
  - zip(deflate): `333,280` bytes
- Reference full non-test `.py` zip:
  - ~`3,197,528` bytes

Target budget for initial subset pack:

- `stdlib_subset_v1.zip <= 500 KB` compressed
- Keep wasm binary size growth minimal (prefer external pack asset, lazy-loaded)

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

- Load/decode the zip asset once at runtime init.
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

## M2: Virtual Stdlib Provider API

Deliverables:

- VM-side optional virtual source resolver seam,
- memory-source compile/exec support for imports.

Exit criteria:

- unit tests: virtual module + package import works without filesystem.

## M3: WASM Loader Wiring

Deliverables:

- wasm runtime loads subset pack and registers provider.
- REPL executes imports from packed modules.

Exit criteria:

- `import functools` on WASM resolves from packed source.

## M4: Parity Targets (Initial)

Deliverables:

- dedicated WASM tests for key modules:
  - `functools.cache`,
  - `functools.lru_cache`,
  - `dataclasses`,
  - `statistics`,
  - `pathlib` basic object construction.

Exit criteria:

- `functools.cache` memoization behavior matches native for representative cases.

## M5: CI + Evidence

Deliverables:

- wasm contract/test lane includes subset-pack smoke.
- artifact summary includes stdlib subset version/hash/size.

Exit criteria:

- CI demonstrates stable load+import behavior for subset modules.

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
