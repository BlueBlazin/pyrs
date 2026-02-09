# Algorithmic and Semantic Audit Backlog

This is the tracked backlog for high-risk algorithmic or semantic correctness issues.

Status values:
- `OPEN`: not fixed
- `IN_PROGRESS`: fix in progress
- `CLOSED`: fixed and verified

## P0 Items

| ID | Area | Risk | Current state | Required closure | Milestone | Status |
|---|---|---|---|---|---|---|
| AQ-001 | `list.sort` | Contract + perf | Implementation clones full list and writes back; does not raise CPython-equivalent mutation-during-sort error | Implement in-place sort semantics + mutation guard behavior; add CPython differential tests | 13 | OPEN |
| AQ-002 | `_io.open` integration | Stdlib correctness | Simplified helper semantics still block strict `test_csv`/`tempfile` paths | Implement CPython-compatible file object semantics for required options/behaviors | 13 | OPEN |
| AQ-003 | `json` stack | Robustness/perf | Partial semantics | Close `test_json` parity, malformed-input safety corpus, and baseline perf report | 13 | OPEN |
| AQ-004 | `_csv`/`csv` stack | Robustness/perf | Partial semantics despite significant progress | Close `test_csv` parity, malformed-input safety corpus, and baseline perf report | 13 | OPEN |
| AQ-005 | `pickle`/`pickletools`/`copyreg` | Robustness/perf | Partial protocol/runtime semantics | Close protocol/runtime parity + perf report | 13 | OPEN |
| AQ-006 | GC/leak regression detection | Reliability | Dedicated cycle-heavy leak regressions were added (`tests/gc_regression.rs`) and strict stdlib harness now runs per-entry subprocesses with timeout; root-cause closures for any remaining growth incidents are still pending | Keep leak regression lane green and close remaining root-cause issues surfaced by strict suites | 13 | IN_PROGRESS |

## P1 Items

| ID | Area | Risk | Current state | Required closure | Milestone | Status |
|---|---|---|---|---|---|---|
| AQ-101 | Dict/set index internals | Algorithmic scaling | Dict now uses CPython-style open-addressing probe slots and set uses compact single-or-many buckets, but delete/index-compaction costs and load-factor/growth policy tuning are still open | Complete hash-container hot-path architecture closure; benchmark and regressions | 14 | IN_PROGRESS |
| AQ-102 | Set/dict growth/load-factor policy | Perf stability | Basic hash index exists, growth strategy not fully tuned/validated | Implement and validate growth/load-factor policies with adversarial tests | 14 | OPEN |
| AQ-103 | Clone hot spots in VM/runtime | Throughput + memory churn | Clone baseline/report tooling is now in place (`scripts/clone_audit.sh`, `docs/CLONE_BASELINE.txt`, `docs/CLONE_AUDIT.md`), but many hot-path clones remain | Audit, classify, and reduce avoidable clones in hot paths; add perf sentinels | 14 | IN_PROGRESS |
| AQ-104 | VM monolith reviewability | Defect detection risk | `src/vm/mod.rs` is still large, but CSV helper algorithms were extracted into `src/vm/stdlib/csv.rs` with direct unit tests | Continue extraction by concern with behavior-preserving tests | 14 | IN_PROGRESS |

## Audit Procedure

For each item:
1. Reproduce with a minimal test in `tests/vm.rs` (and CPython differential test if applicable).
2. Add benchmark case where performance is part of closure criteria.
3. Implement fix with explicit semantics notes in code or commit message.
4. Run full parity gates (`cargo test`, curated CPython harness, differential/fuzz as applicable).
5. Mark item `CLOSED` only after tests and docs are updated.
