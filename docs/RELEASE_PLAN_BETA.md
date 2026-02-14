# Beta Release Plan (Draft)

Status: draft checkpoint (tracked now, not ready to execute yet).

This document captures the current release-first plan so we can continue implementation work without losing release decisions.

## Release Intent

Ship a first public beta while long-tail compatibility/performance work continues in later milestones.

## Locked Decisions

- Channel: beta (`v0.1.0-beta.1`)
- Distribution: GitHub Releases + crates.io
- Platform matrix (required): Linux `x86_64-unknown-linux-gnu`, macOS `x86_64-apple-darwin`, macOS `aarch64-apple-darwin`
- Integrity artifacts: `SHA256SUMS` + Sigstore cosign keyless signatures
- Quality gate: all required suites green (no partial release with known failing gates)
- Strict gate policy: strict stdlib + deferred pickle lanes both required green
- License model: custom restrictive beta license (`license-file`)
- License terms direction: source-visible, noncommercial
- Support policy: no maintenance promise/SLA for beta line

## Current Release Blocker Snapshot

The plan is intentionally parked until core blockers are reduced. Key blockers include:

- open Milestone 13 P0 items in `docs/PRODUCTION_READINESS.md`
- open P0 ledger rows in `docs/STUB_ACCOUNTING.md`
- release-hardening milestones (`14+`) not complete

## Release Execution Plan (When Unblocked)

## 1. Stabilization Gate

Required green commands:

1. `cargo test -q`
2. `./scripts/run_parity_gate.sh`
3. `PYRS_RUN_STRICT_STDLIB=1 cargo test -q --test cpython_harness runs_cpython_strict_stdlib_suite`
4. `PYRS_RUN_DEFERRED_PICKLE=1 cargo test -q --test cpython_harness runs_cpython_deferred_pickle_suite`
5. `PYRS_COVERAGE_ENFORCE=1 PYRS_COVERAGE_MIN_REGIONS=70 PYRS_COVERAGE_MIN_FUNCTIONS=65 PYRS_COVERAGE_MIN_LINES=70 ./scripts/run_coverage_gate.sh`

## 2. Packaging and Legal

1. set package prerelease version (`0.1.0-beta.1`) in `Cargo.toml`
2. add/validate `license-file` for beta terms
3. complete crate metadata required for publishability
4. pass `cargo package` and `cargo publish --dry-run` in CI

## 3. Release Automation

1. add release-gate workflow (blocking quality checks)
2. add release workflow (build matrix, checksum generation, signing, publish)
3. publish signed binaries + checksum manifest to GitHub Releases
4. publish crate to crates.io from release tag

## 4. Release Documentation

1. beta release notes with explicit known limitations
2. install/verify instructions for checksums/signatures
3. `KNOWN_LIMITATIONS` and security model docs aligned to tag snapshot
4. readiness and compatibility docs updated in same release commit

## 5. Tag and Verification

1. cut release branch + freeze unrelated merges
2. tag `v0.1.0-beta.1`
3. verify artifacts on each target (`--version`, source run, `.pyc` smoke)
4. verify checksum + cosign signature flows
5. verify `cargo install --locked pyrs --version 0.1.0-beta.1`

## Non-Goals for This Beta

- full CPython 3.14 parity
- production SLA/support guarantees
- C-extension ecosystem compatibility
- full release certification targets from Milestone 16

## Notes

- This plan is a release checkpoint, not a signal that release should happen immediately.
- Keep this file updated when release decisions change.
