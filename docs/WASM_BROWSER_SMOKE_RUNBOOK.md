# WASM Browser Smoke Runbook

Status: manual workflow-dispatch procedure for `codex/wasm`.

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

Notes:
- Manual dispatch runs both jobs:
  - `wasm-contract-gate` (mandatory branch gate),
  - `wasm-browser-smoke` (manual browser lane).
- Browser lane is fail-hard; failures must be triaged before promotion.

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

## Download Artifacts

Download all artifacts:

```bash
gh run download <run-id> --dir perf/wasm-browser-smoke-run
```

Expected artifact bundles:
- `wasm-contract-artifacts`
- `wasm-evidence-pack`
- `wasm-browser-smoke-artifacts` (present when browser lane executes)
  - includes `perf/wasm_evidence_pack_latest/manifest.json` when core smoke
    generated a local evidence pack.
- `wasm-browser-smoke-evidence-pack` (present when browser lane emits an
  evidence pack)

## Promotion Evidence Checklist

Before marking browser-smoke baseline captured:

1. Run status is green or explicitly triaged/waived.
2. `wasm-browser-smoke-artifacts` downloaded and archived.
3. Link run URL and artifact location in promotion notes.
4. Record any flakes or browser-specific failures with root-cause notes.
