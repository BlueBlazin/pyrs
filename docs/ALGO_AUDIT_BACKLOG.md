# Algorithmic and Semantic Audit Backlog

This backlog tracks high-risk algorithmic/semantic items that must close for production readiness.

Status values:
- `OPEN`
- `IN_PROGRESS`
- `CLOSED`

## P0 Items (Milestone 13 blockers)

| ID | Area | Risk | Open gap | Closure criteria | Status |
|---|---|---|---|---|---|
| AQ-001 | `list.sort` semantic contract | Correctness + perf | Mutation/error-ordering parity and clone pressure are not fully proven. | CPython differential parity for mutation/error ordering + benchmark/profile evidence on representative workloads. | IN_PROGRESS |
| AQ-002 | `_io` core semantics | Stdlib correctness | Remaining strict-lane `_io` edge behavior is incomplete. | Strict parity closure for remaining `_io`-dependent paths. | IN_PROGRESS |
| AQ-003 | `json` robustness/perf | Correctness + reliability | Long-tail malformed-input/edge semantics and perf closure are incomplete. | `test_json` closure + malformed-input differential coverage + perf baseline. | OPEN |
| AQ-004 | `_csv`/`csv` robustness/perf | Correctness + reliability | Long-tail dialect/error semantics and perf closure are incomplete. | `test_csv` closure + malformed-input differential coverage + perf baseline. | OPEN |
| AQ-005 | `pickle`/`pickletools`/`copyreg` | Correctness + reliability + perf | Deferred strict pickle lane remains open. | Deferred lane green + protocol/runtime parity closure + perf baseline. | OPEN |
| AQ-006 | GC/leak regression control | Reliability | Leak/hang regressions can still reappear under strict/harness stress. | Keep regression lane green and close any growth/hang root cause with targeted repro tests. | IN_PROGRESS |

## P1 Items (Milestone 14)

| ID | Area | Risk | Open gap | Closure criteria | Status |
|---|---|---|---|---|---|
| AQ-101 | Hash-container growth policy | Throughput stability | Load-factor/growth tuning remains open. | Adversarial-size tests + benchmark closure for growth/rehash policy. | IN_PROGRESS |
| AQ-102 | Clone pressure in hot paths | Throughput + memory churn | Avoidable hot-path full-data clones remain. | Remove avoidable clones and add perf sentinels for clone-sensitive paths. | IN_PROGRESS |
| AQ-103 | VM monolith reviewability | Defect risk | Further decomposition is needed for maintainability/reviewability. | Continue concern-based extraction with behavior-preserving tests. | IN_PROGRESS |

## Audit Procedure
1. Add a minimal repro test (and CPython differential test when applicable).
2. Add benchmark/profile probes for performance-sensitive items.
3. Implement fix with explicit semantic-contract notes.
4. Run required parity gates.
5. Record evidence in commit/docs.
6. Update item status only after evidence is committed.
