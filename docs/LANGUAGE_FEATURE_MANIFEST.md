# Language Feature Manifest (CPython 3.14)

This manifest is the source-language counterpart to ABI/opcode manifests.

- Machine-readable feature inventory: `docs/LANGUAGE_FEATURE_MANIFEST.json`
- Probe runner + validator: `scripts/check_language_feature_manifest.py`
- Latest probe artifact: `perf/language_feature_manifest_latest.json`

## Purpose

1. Ensure every tracked 3.14 source feature has an explicit manifest row.
2. Fail CI if the manifest drifts (missing or unknown feature ids).
3. Fail CI if a `required=true` feature does not match CPython probe behavior.

## Run Locally

```bash
python3 scripts/check_language_feature_manifest.py \
  --pyrs target/debug/pyrs \
  --cpython-bin /Library/Frameworks/Python.framework/Versions/3.14/bin/python3 \
  --manifest docs/LANGUAGE_FEATURE_MANIFEST.json \
  --out perf/language_feature_manifest_latest.json
```

## Process

When adding a new CPython 3.14 language feature:

1. Add a probe in `scripts/check_language_feature_manifest.py`.
2. Add a matching entry in `docs/LANGUAGE_FEATURE_MANIFEST.json`.
3. Add parser/compiler/runtime tests (`tests/parser.rs`, `tests/vm.rs`, and `tests/differential_cpython.rs` where applicable).
4. Ensure the manifest gate stays green.
