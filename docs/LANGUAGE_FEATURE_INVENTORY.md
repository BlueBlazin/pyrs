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
- Probe-to-inventory mapping: `docs/LANGUAGE_FEATURE_PROBE_MAP.json`
- Inventory pass/fail/unprobed report: `perf/language_feature_coverage_latest.json`

## Current Baseline (Generated)
- Total inventory rows: `578`
- Grammar rules: `249`
- Grammar public rules: `188`
- Grammar internal rules (`invalid_*`): `61`
- Tokens: `69`
- Reference headings: `260`
- Current required parity probes in manifest (`docs/LANGUAGE_FEATURE_MANIFEST.json`): `15`
- Required-probe-to-inventory baseline: `2.6%`
- Current mapped inventory coverage:
  - `pass`: `329`
  - `fail`: `249`
  - `unprobed`: `0`
  - `coverage`: `100.0%` (`578/578`)
  - current probe run baseline: `24/26` probes passing

## Important Interpretation
- This inventory is complete as a **source-derived accounting baseline**.
- Inventory rows are now fully mapped to probe evidence (`unprobed = 0`).
- Remaining work is reducing the current failing inventory rows (`249`) by fixing probe and runtime parity gaps.

## Regeneration
```bash
python3 scripts/generate_language_feature_inventory.py \
  --cpython-root /Users/$USER/Downloads/Python-3.14.3 \
  --manifest docs/LANGUAGE_FEATURE_MANIFEST.json \
  --out-inventory docs/LANGUAGE_FEATURE_INVENTORY.json \
  --out-report perf/language_feature_inventory_report_latest.json

python3 scripts/check_language_feature_manifest.py \
  --pyrs target/debug/pyrs \
  --manifest docs/LANGUAGE_FEATURE_MANIFEST.json \
  --out perf/language_feature_manifest_latest.json

python3 scripts/check_language_feature_coverage.py \
  --inventory docs/LANGUAGE_FEATURE_INVENTORY.json \
  --probe-results perf/language_feature_manifest_latest.json \
  --probe-map docs/LANGUAGE_FEATURE_PROBE_MAP.json \
  --out perf/language_feature_coverage_latest.json
```
