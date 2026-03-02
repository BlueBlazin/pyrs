# WASM Browser Smoke Runbook

Status: manual workflow-dispatch procedure for `codex/wasm`.
Latest verified dispatch: [22592519133](https://github.com/BlueBlazin/pyrs/actions/runs/22592519133) on commit `b3605a22`.

Purpose:
- capture reproducible browser-smoke evidence for promotion review,
- keep manual CI runs consistent across maintainers.

## Prerequisites

1. `gh` CLI authenticated to the repository.
2. Branch pushed (`codex/wasm` or descendant branch).
3. Workflow file present: `.github/workflows/wasm-track.yml`.

## Trigger Browser Smoke Dispatch

From repo root:

```bash
gh workflow run wasm-track.yml --ref codex/wasm
```

Or use the one-shot helper (dispatch + watch + download + baseline validation):

```bash
scripts/run_wasm_browser_smoke_dispatch.sh --ref codex/wasm
```

Notes:
- Manual dispatch runs both jobs:
  - `wasm-contract-gate` (mandatory branch gate),
  - `wasm-browser-smoke` (manual browser lane).
- Browser lane is fail-hard; failures must be triaged before promotion.
- Browser lane also downloads `wasm-evidence-pack` from contract gate and
  validates the manifest/copies before running browser smoke.
- vm-probe supplemental state-gate smoke now runs via `wasm-pack --node`
  (not browser harness) with required shim path:
  `scripts/wasm_node_shims/env/index.js`.
- Local desktop/sandbox runs may block webdriver launch/download
  (`Operation not permitted`); use workflow-dispatch CI artifacts as the
  canonical browser-smoke baseline source.

## Local Evidence Pack (Before or After Dispatch)

After running local branch checks, bundle required local artifacts:

```bash
python3 scripts/collect_wasm_evidence_pack.py
```

Expected output:
- `perf/wasm_evidence_pack_latest/manifest.json`
- copied local wasm contract artifacts in the same directory.

## Watch Run and Capture Run ID

List recent runs:

```bash
gh run list --workflow wasm-track.yml --branch codex/wasm --limit 5
```

Watch a specific run:

```bash
gh run watch <run-id>
```

The helper script prints both run URL and downloaded artifact path:

```bash
scripts/run_wasm_browser_smoke_dispatch.sh --run-id <run-id>
```

On successful download/validation, the helper also writes:
- `<download-dir>/wasm-artifact-hashes.json`
- `<download-dir>/wasm-artifact-hashes.md`
- `<download-dir>/workflow-run.log`
- helper dispatch selection is anchored to dispatch-time run creation and
  verifies `headSha` against your local `<ref>` commit when that ref exists
  locally, to reduce stale-run/race mis-association.

If artifact downloads are unavailable in your environment, use:

```bash
scripts/run_wasm_browser_smoke_dispatch.sh --run-id <run-id> --skip-download
```

## Download Artifacts

Download all artifacts:

```bash
gh run download <run-id> --dir perf/wasm-browser-smoke-run
```

Expected artifact bundles:
- `wasm-contract-artifacts`
- `wasm-evidence-pack`
- `wasm-browser-smoke-artifacts` (present when browser lane executes)
  - includes `perf/wasm_browser_smoke_baseline_latest.json` with browser,
    fallback, vm-probe-state-gate flag/runner metadata, and git commit metadata.
  - includes `perf/wasm_evidence_pack_latest/manifest.json` when core smoke
    generated a local evidence pack.
- `wasm-browser-smoke-evidence-pack` (present when browser lane emits an
  evidence pack)

Optional local validation after download:

```bash
python3 scripts/validate_wasm_browser_smoke_baseline.py \
  --summary perf/wasm-browser-smoke-run/wasm-browser-smoke-artifacts/wasm_browser_smoke_baseline_latest.json
```

Optional artifact-hash extraction (for promotion notes):

```bash
python3 scripts/extract_wasm_ci_artifact_hashes.py \
  --run-id <run-id> \
  --format markdown
```

## Promotion Evidence Checklist

Before marking browser-smoke baseline captured:

1. Run status is green or explicitly triaged/waived.
2. `wasm-browser-smoke-artifacts` downloaded and archived.
3. `perf/wasm_browser_smoke_baseline_latest.json` captured from that run.
4. Link run URL and artifact location in promotion notes.
5. Record any flakes or browser-specific failures with root-cause notes.
