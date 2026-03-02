# WASM Promotion Gate (Go/No-Go)

Status: active for `codex/wasm` only.

Purpose:
- Define explicit promotion criteria before any WASM work can be considered for merge.
- Keep native Linux/macOS release behavior protected while WASM remains experimental.

## Branch Safety Contract

1. WASM development remains on `codex/wasm` (or child branches).
2. `master` is protected from direct WASM behavior changes until explicit approval.
3. Native defaults must remain unchanged when WASM feature flags are not enabled.

## Required CI Signals

All of the following must be green for the candidate commit:

1. `wasm-track / wasm-contract-gate` (branch workflow):
   - runs `scripts/check_wasm_branch.sh`,
   - verifies native check + wasm target check + vm-probe wasm lane,
   - verifies fixture/docs/source contract summaries and host-seam audit.
2. When browser smoke is explicitly requested (`workflow_dispatch`):
   - `wasm-track / wasm-browser-smoke` must be reviewed for pass/fail and logs,
   - browser smoke now hard-fails on Cargo wasm output-collision warnings
     (`output filename collision`) from `wasm-pack` substeps,
   - vm-probe browser state-gate smoke (`wasm_vm_probe_browser_smoke`) must pass
     in browser mode (node fallback only if browser vm-probe target fails),
   - failures block promotion unless they are triaged and explicitly waived.
3. Existing native mandatory lanes (`parity-gate`, release/nightly lanes) remain green.
4. No new CI flakes attributable to WASM codepaths.

## Technical Go/No-Go Checklist

All items below are required for a "go":

1. Compile-time isolation:
   - no native-only link paths leak into `wasm32-unknown-unknown` default builds.
2. Host seam integrity:
   - no direct host access drift in `src/vm` outside approved seam boundaries.
3. Runtime contract stability:
   - wasm execution/worker/session APIs remain fixture- and summary-validated.
4. Worker behavior determinism:
   - lifecycle state and execution gating remain explicit and test-backed.
5. Website isolation:
   - `/playground` lazily loads WASM bundle; non-playground routes remain static-first.
6. Capability transparency:
   - unsupported browser capabilities are explicit (no silent fallback behavior).

## Release Risk Checklist (Linux/macOS Priority)

1. Native tests impacted by touched files pass in local targeted runs.
2. Native CLI/REPL entry paths are unchanged by default.
3. Native extension-loading behavior remains unchanged in non-WASM targets.
4. No platform-specific regressions on required release targets:
   - Linux: `x86_64-unknown-linux-gnu`
   - macOS: `x86_64-apple-darwin`, `aarch64-apple-darwin`

## Evidence Pack for Review

Promotion review must include:

1. Latest `wasm-track` artifact bundle from CI.
   - include `wasm-evidence-pack` artifact (manifest + copied summaries).
2. Latest local `scripts/check_wasm_branch.sh` output.
3. Latest local consolidated evidence pack:
   - generated automatically by `scripts/check_wasm_branch.sh`,
   - or run `python3 scripts/collect_wasm_evidence_pack.py` directly,
   - validate with
     `python3 scripts/validate_wasm_evidence_pack.py --pack-dir perf/wasm_evidence_pack_latest`,
   - attach `perf/wasm_evidence_pack_latest/manifest.json` and copied artifacts.
4. Documented capability limitations and known gaps for browser mode.
5. Rollback plan (commit range and revert strategy) if post-merge regressions appear.
6. Manual browser-smoke dispatch evidence using
   [`docs/WASM_BROWSER_SMOKE_RUNBOOK.md`](./WASM_BROWSER_SMOKE_RUNBOOK.md),
   including `perf/wasm_browser_smoke_baseline_latest.json` from the
   `wasm-browser-smoke-artifacts` bundle.
7. Artifact ID + SHA256 snapshot for the reviewed run:
   - `python3 scripts/extract_wasm_ci_artifact_hashes.py --run-id <run-id> --format markdown`

### Latest Recorded Evidence Snapshot (2026-03-03 UTC)

- workflow-dispatch run: [22593346662](https://github.com/BlueBlazin/pyrs/actions/runs/22593346662)
- head commit: `dc1dfd99b6d98cae5ad06c585b7b46e4bbc85713`
- artifact hashes (from CI upload logs):
  - `wasm-contract-artifacts`:
    - artifact id: `5728581632`
    - sha256: `d1368d8750be5c72a5aa2a236bdb09f2850ba19e89db068e9b4d3f82472b1589`
  - `wasm-evidence-pack`:
    - artifact id: `5728581772`
    - sha256: `70be1641461ef910a5ba05c733218301c36848b3ff86bf98a78e8889912b48c0`
  - `wasm-browser-smoke-artifacts`:
    - artifact id: `5728603377`
    - sha256: `79da7481ac49b4259e9e7592a0fcb1c34de0a8f7f35c604671ac64585f2882df`
  - `wasm-browser-smoke-evidence-pack`:
    - artifact id: `5728603616`
    - sha256: `ecacf12b92028800a1caaa3fb4cb508cbacfb9fe9108513852471b4c789688bd`

## Decision Rule

- `GO`: all required signals/checklists above are satisfied and approved by maintainers.
- `NO-GO`: any required signal/checklist fails, or release-risk assessment is inconclusive.

Default decision is `NO-GO` unless all go criteria are met.
