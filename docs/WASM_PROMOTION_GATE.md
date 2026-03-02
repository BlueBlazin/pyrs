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
2. Existing native mandatory lanes (`parity-gate`, release/nightly lanes) remain green.
3. No new CI flakes attributable to WASM codepaths.

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
2. Latest local `scripts/check_wasm_branch.sh` output.
3. Documented capability limitations and known gaps for browser mode.
4. Rollback plan (commit range and revert strategy) if post-merge regressions appear.

## Decision Rule

- `GO`: all required signals/checklists above are satisfied and approved by maintainers.
- `NO-GO`: any required signal/checklist fails, or release-risk assessment is inconclusive.

Default decision is `NO-GO` unless all go criteria are met.
