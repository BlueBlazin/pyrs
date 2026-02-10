# Algorithmic and Semantic Audit Backlog

This backlog tracks high-risk algorithmic or semantic-contract items that must be closed
for production readiness.

Status values:
- `OPEN`
- `IN_PROGRESS`
- `CLOSED`

## P0 Items (Milestone 13 blockers)

| ID | Area | Risk | Current state | Required closure | Status |
|---|---|---|---|---|---|
| AQ-001 | `list.sort` semantic contract | Correctness + perf | In-place pathway exists; mutation/error contract parity and clone pressure still need explicit closure proof | Differential parity tests for mutation/error ordering + benchmark/profile evidence on representative workloads | IN_PROGRESS |
| AQ-002 | `_io` core semantics | Stdlib correctness | Core open/mode/newline behavior landed, but full stream layering/edge behavior remains partial | Strict stdlib parity for remaining `_io`-dependent paths | IN_PROGRESS |
| AQ-003 | `json` robustness/perf | Correctness + reliability | Pure-module-first direction exists; fallback and edge behavior still partial | `test_json` closure + malformed-input differential + perf baseline | OPEN |
| AQ-004 | `_csv`/`csv` robustness/perf | Correctness + reliability | Substantial `_csv` work landed; long-tail parity still open | `test_csv` closure + malformed-input differential + perf baseline | OPEN |
| AQ-005 | `pickle`/`pickletools`/`copyreg` | Correctness + reliability + perf | Deferred strict pickle lane still open | Deferred lane closure + perf baseline + protocol/runtime parity | OPEN |
| AQ-006 | GC/leak regression control | Reliability | Dedicated GC regression lane exists | Keep lane green and close any new growth/hang root causes with targeted repro tests | IN_PROGRESS |

## P1 Items (Milestone 14)

| ID | Area | Risk | Current state | Required closure | Status |
|---|---|---|---|---|---|
| AQ-101 | Hash-container growth policy | Throughput stability | Dict backend moved to open addressing; tuning remains | Load-factor/growth policy tuning with adversarial tests and benchmarks | IN_PROGRESS |
| AQ-102 | Clone pressure in hot paths | Throughput + memory churn | Clone audit tooling exists; hotspots remain | Reduce avoidable full-data clones and add perf sentinels | IN_PROGRESS |
| AQ-103 | VM monolith reviewability | Defect risk | Major split landed: `src/vm/mod.rs` reduced from ~43k to ~6.5k with domain `impl Vm` files; further tightening still needed | Continue concern-based extraction with behavior-preserving tests and keep clone hotspots moving out of central dispatch paths | IN_PROGRESS |

## Audit Procedure
1. Add minimal repro test (and CPython differential test when applicable).
2. Add benchmark/profile probe for performance-sensitive items.
3. Implement fix with explicit semantic contract notes.
4. Run required parity gates.
5. Update row status only after evidence is committed.
