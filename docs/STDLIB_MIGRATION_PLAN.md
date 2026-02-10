# Stdlib Migration Plan (Pure-Python First)

## Policy
Use CPython's official pure-Python stdlib modules as the primary behavior source whenever feasible.
Rust-native stdlib code should provide runtime primitives and accelerator-equivalent surfaces.

## Ownership Map

| Area | CPython source-of-truth | Rust-native responsibility | Current state | Exit criteria |
|---|---|---|---|---|
| `json` | `Lib/json/*` + `_json` accelerator | `_json` accelerator compatibility surface (`scanstring`, `make_scanner`, `encode_basestring`, `encode_basestring_ascii`, `make_encoder`) | Pure `json` is preferred when stdlib is available; native high-level fallback still exists for stdlib-less mode | Remove high-level fallback dependency for parity paths; strict and differential gates green |
| `csv` | `Lib/csv.py` + `_csv` accelerator | `_csv` accelerator-compatible parsing/writing behavior | Native substrate exists; long-tail parity remains | Full `test_csv` parity and robustness/perf proof |
| `pickle` | `Lib/pickle.py`, `Lib/pickletools.py`, `Lib/copyreg.py` + `_pickle` accelerator | `_pickle` + runtime object protocol hooks | Deferred strict pickle lane remains open | Re-enable and close deferred strict pickle suite with perf proof |
| `re` | `Lib/re/*` + `_sre` accelerator | `_sre` accelerator-compatible regex core | `_sre` core in place; pure `re` default still blocked by long-tail parity | Switch pure `Lib/re/*` to default path with strict/curated gates green |

## Sequencing (Milestone 13)
1. Native core first (`_io`, `_csv`, `_sre`, `_pickle`, object protocol hooks).
2. Pure-stdlib handoff as default behavior.
3. Strict unittest expansion and closure.

## Change Rules
1. Every core-surface change must cite CPython source reference (`Modules/*.c`, `Objects/*.c`, `Lib/*.py`) in the commit/PR notes.
2. Net-new native feature additions in these areas require explicit entry in `docs/STUB_ACCOUNTING.md`.
3. Do not add new compatibility shims for these areas unless explicitly temporary and tracked with exit criteria.
