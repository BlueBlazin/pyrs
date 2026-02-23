# CPython 3.14 Language Feature Inventory

This document is the authoritative, source-derived inventory baseline for CPython 3.14 language features.

## Canonical Sources
- `/Users/$USER/Downloads/Python-3.14.3/Grammar/python.gram`
- `/Users/$USER/Downloads/Python-3.14.3/Grammar/Tokens`
- `/Users/$USER/Downloads/Python-3.14.3/Doc/reference/lexical_analysis.rst`
- `/Users/$USER/Downloads/Python-3.14.3/Doc/reference/simple_stmts.rst`
- `/Users/$USER/Downloads/Python-3.14.3/Doc/reference/compound_stmts.rst`
- `/Users/$USER/Downloads/Python-3.14.3/Doc/reference/expressions.rst`
- `/Users/$USER/Downloads/Python-3.14.3/Doc/reference/toplevel_components.rst`
- `/Users/$USER/Downloads/Python-3.14.3/Doc/reference/import.rst`
- `/Users/$USER/Downloads/Python-3.14.3/Doc/reference/executionmodel.rst`
- `/Users/$USER/Downloads/Python-3.14.3/Doc/reference/datamodel.rst`

## Generated Artifacts
- Full machine-readable inventory: `docs/LANGUAGE_FEATURE_INVENTORY.json`
- Baseline coverage report: `perf/language_feature_inventory_report_latest.json`

## Current Baseline (Generated)
- Total inventory rows: `578`
- Grammar rules: `249`
- Grammar public rules: `188`
- Grammar internal rules (`invalid_*`): `61`
- Tokens: `69`
- Reference headings: `260`
- Current required parity probes in manifest (`docs/LANGUAGE_FEATURE_MANIFEST.json`): `15`
- Required-probe-to-inventory baseline: `2.6%`

## Important Interpretation
- This inventory is complete as a **source-derived accounting baseline**.
- It is not yet a one-to-one probe matrix.
- The low percentage above means we still need to map and probe most inventory rows.

## Regeneration
```bash
python3 scripts/generate_language_feature_inventory.py \
  --cpython-root /Users/$USER/Downloads/Python-3.14.3 \
  --manifest docs/LANGUAGE_FEATURE_MANIFEST.json \
  --out-inventory docs/LANGUAGE_FEATURE_INVENTORY.json \
  --out-report perf/language_feature_inventory_report_latest.json
```
