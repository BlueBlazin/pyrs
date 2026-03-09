# CPython Dict Mapping: Rust Backend Design

This document maps CPython `Objects/dictobject.c` concepts to the Rust
`DictBackend` now used by `DictObject`.

## Goal

Implement CPython-style dict behavior and probe mechanics in Rust while keeping
the existing interpreter object model intact.

## Core Mapping

| CPython concept | Rust backend mapping |
|---|---|
| Open-addressed hash table | `DictBackend::slots: Vec<DictSlot>` |
| Empty slot | `DictSlot::Empty` |
| Dummy/tombstone slot | `DictSlot::Dummy` |
| Occupied slot | `DictSlot::Occupied { hash, entry }` |
| Probe sequence with perturb | `lookup_slot` / `lookup_vacant_slot` with `PERTURB_SHIFT=5` |
| Combined-table style key/value entries | `entries: Vec<(Value, Value)>` |
| Insertion-order iteration | `entries` order is authoritative iteration order |
| Resize on load factor pressure | `ensure_insert_capacity` (`filled / size >= 2/3`) |
| Rehash/compaction | `resize_slots` / `rebuild_slots` |

## Invariants

1. For any hashable key in `entries`, at most one occupied slot maps to that
   entry index.
2. `entries` preserves insertion order; updating an existing key does not move
   its position.
3. Deleting a key marks its slot as `Dummy` before entry compaction.
4. After compaction (`Vec::remove`), all occupied slot indices are adjusted.
5. `find/contains/insert/remove_key` are probe-based for hashable keys.
6. Unhashable key paths continue to use linear fallback behavior (safety and
   compatibility with existing internal construction paths), while normal Python
   user paths still enforce hashability before insertion.

## Deliberate Differences From CPython C Layout

1. CPython split-table optimization (`ma_keys` + optional `ma_values`) is not
   yet implemented. Current backend is a combined representation.
2. Entry compaction currently uses `Vec::remove` and index fixup on delete.
   CPython keeps dummies in the table and uses different compaction behavior.
3. Growth/shrink heuristics are approximate to CPython behavior but not yet
   fully tuned against CPython internals.

These differences are real implementation deltas, not compatibility claims.
They should be evaluated against source behavior and benchmark evidence when
dict performance or edge semantics change.

## Validation Coverage

1. Collision + delete + reinsertion probe tests:
   `src/runtime/dict_backend.rs`
2. Runtime dict regressions:
   `src/runtime/mod.rs`
3. VM-level dict semantics:
   `tests/vm.rs`
4. CPython language harness:
   `tests/cpython_harness.rs`
