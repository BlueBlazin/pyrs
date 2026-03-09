# C-API No-Op Inventory

## Purpose

This file exists because `scripts/check_capi_noop_inventory.py` validates it
directly against the exported C-API source in `src/vm/vm_extensions/`.

The check is conservative. It looks for obvious placeholder exports such as:

- empty function bodies
- trivial `0` / `1` returns
- trivial null-pointer returns

## Current Checked-In Result

Latest manifest: `perf/capi_noop_inventory.json`

- detected no-op symbols: none
- missing-from-doc symbols: none
- stale documented symbols: none

That means the current checked-in source does not expose any C-API exports that
match the gate's placeholder heuristics.

## Local Validation

```bash
python3 scripts/check_capi_noop_inventory.py --manifest perf/capi_noop_inventory.json
```

## Interpretation

This inventory is intentionally narrow:

- it covers C-API exports in `src/vm/vm_extensions/*`
- it does not cover Python-level builtin no-op placeholders
- it does not prove full CPython compatibility; it only guards against obvious
  placeholder exports surviving unnoticed
