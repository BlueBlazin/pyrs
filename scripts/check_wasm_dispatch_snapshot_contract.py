#!/usr/bin/env python3
"""Contract-check wasm dispatch snapshot updater against current docs state."""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RUNBOOK_PATH = REPO_ROOT / "docs" / "WASM_BROWSER_SMOKE_RUNBOOK.md"
PROMOTION_PATH = REPO_ROOT / "docs" / "WASM_PROMOTION_GATE.md"
UPDATE_SCRIPT = REPO_ROOT / "scripts" / "update_wasm_dispatch_snapshot.py"


def fail(message: str) -> int:
    print(f"wasm dispatch snapshot contract failed: {message}")
    return 1


def main() -> int:
    runbook_text = RUNBOOK_PATH.read_text(encoding="utf-8")
    promotion_text = PROMOTION_PATH.read_text(encoding="utf-8")

    run_id_match = re.search(
        r"Latest verified dispatch: \[(?P<run_id>\d+)\]\(https://github\.com/BlueBlazin/pyrs/actions/runs/\d+\) on commit `(?P<short_sha>[0-9a-f]+)`\.",
        runbook_text,
    )
    if not run_id_match:
        return fail("could not parse latest verified dispatch row in runbook")
    run_id = run_id_match.group("run_id")

    head_sha_match = re.search(r"- head commit: `(?P<head_sha>[0-9a-f]{40})`", promotion_text)
    if not head_sha_match:
        return fail("could not parse head commit row in promotion gate")
    head_sha = head_sha_match.group("head_sha")

    update_run = subprocess.run(
        [
            sys.executable,
            str(UPDATE_SCRIPT),
            "--run-id",
            run_id,
            "--head-sha",
            head_sha,
            "--dry-run",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if update_run.returncode != 0:
        details = update_run.stderr.strip() or update_run.stdout.strip() or "unknown updater failure"
        return fail(details)

    print(
        "wasm dispatch snapshot contract passed: "
        f"run_id={run_id}, head_sha={head_sha}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
