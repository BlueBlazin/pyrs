#!/usr/bin/env python3
"""Update wasm docs snapshot references to a specific workflow-dispatch run."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RUNBOOK_PATH = REPO_ROOT / "docs" / "WASM_BROWSER_SMOKE_RUNBOOK.md"
EXEC_PLAN_PATH = REPO_ROOT / "docs" / "WASM_EXECUTION_PLAN.md"
PROMOTION_PATH = REPO_ROOT / "docs" / "WASM_PROMOTION_GATE.md"


def is_transient_gh_error(message: str) -> bool:
    lowered = message.lower()
    return any(
        token in lowered
        for token in (
            "error connecting to api.github.com",
            "tls",
            "timed out",
            "timeout",
            "connection reset",
            "temporary failure",
            "service unavailable",
        )
    )


def run_gh(args: list[str], retries: int = 3, retry_delay_seconds: float = 1.25) -> str:
    last_error = "unknown gh error"
    for attempt in range(1, retries + 1):
        proc = subprocess.run(
            args,
            check=False,
            capture_output=True,
            text=True,
        )
        if proc.returncode == 0:
            return proc.stdout
        message = proc.stderr.strip() or proc.stdout.strip() or "unknown gh error"
        last_error = message
        if attempt >= retries or not is_transient_gh_error(message):
            raise RuntimeError(last_error)
        time.sleep(retry_delay_seconds * attempt)
    raise RuntimeError(last_error)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-id", required=True, help="Workflow run id.")
    parser.add_argument(
        "--run-url",
        help="Run URL override (defaults to canonical URL from run id).",
    )
    parser.add_argument(
        "--head-sha",
        help="Head SHA override (if omitted, fetched via gh run view).",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Validate replacements but do not write files.",
    )
    return parser.parse_args()


def replace_exactly_once(
    path: Path,
    pattern: str,
    replacement: str,
    label: str,
) -> str:
    source = path.read_text(encoding="utf-8")
    updated, count = re.subn(pattern, replacement, source, flags=re.MULTILINE)
    if count != 1:
        raise RuntimeError(f"{label}: expected exactly 1 replacement in {path}, got {count}")
    return updated


def main() -> int:
    args = parse_args()
    run_id = args.run_id
    run_url = args.run_url or f"https://github.com/BlueBlazin/pyrs/actions/runs/{run_id}"

    head_sha = args.head_sha
    if not head_sha:
        head_sha = run_gh(
            ["gh", "run", "view", run_id, "--json", "headSha", "--jq", ".headSha"]
        ).strip()
    if not head_sha or not re.fullmatch(r"[0-9a-f]{40}", head_sha):
        print(f"error: invalid head sha '{head_sha}'", file=sys.stderr)
        return 1

    short_sha = head_sha[:8]

    runbook_text = replace_exactly_once(
        RUNBOOK_PATH,
        r"Latest verified dispatch: \[\d+\]\(https://github\.com/BlueBlazin/pyrs/actions/runs/\d+\) on commit `[0-9a-f]+`\.",
        f"Latest verified dispatch: [{run_id}]({run_url}) on commit `{short_sha}`.",
        "runbook latest dispatch line",
    )

    execution_text = replace_exactly_once(
        EXEC_PLAN_PATH,
        r"\(`\d+`, commit `[0-9a-f]+`\) is green for both jobs",
        f"(`{run_id}`, commit `{short_sha}`) is green for both jobs",
        "execution plan latest checkpoint line",
    )

    promotion_text = PROMOTION_PATH.read_text(encoding="utf-8")
    promotion_text, count_run = re.subn(
        r"- workflow-dispatch run: \[\d+\]\(https://github\.com/BlueBlazin/pyrs/actions/runs/\d+\)",
        f"- workflow-dispatch run: [{run_id}]({run_url})",
        promotion_text,
        count=1,
        flags=re.MULTILINE,
    )
    if count_run != 1:
        raise RuntimeError("promotion gate run line: expected exactly 1 replacement")

    promotion_text, count_head = re.subn(
        r"- head commit: `[0-9a-f]{40}`",
        f"- head commit: `{head_sha}`",
        promotion_text,
        count=1,
        flags=re.MULTILINE,
    )
    if count_head != 1:
        raise RuntimeError("promotion gate head commit line: expected exactly 1 replacement")

    promotion_text, count_hash = re.subn(
        r"refresh hashes for run `\d+` from a network-enabled shell:",
        f"refresh hashes for run `{run_id}` from a network-enabled shell:",
        promotion_text,
        count=1,
        flags=re.MULTILINE,
    )
    if count_hash != 1:
        raise RuntimeError("promotion gate hash refresh line: expected exactly 1 replacement")

    promotion_text, count_cmd = re.subn(
        r"`python3 scripts/extract_wasm_ci_artifact_hashes\.py --run-id \d+ --format markdown`\.",
        f"`python3 scripts/extract_wasm_ci_artifact_hashes.py --run-id {run_id} --format markdown`.",
        promotion_text,
        count=1,
        flags=re.MULTILINE,
    )
    if count_cmd != 1:
        raise RuntimeError("promotion gate hash command line: expected exactly 1 replacement")

    if args.dry_run:
        print(
            "dry-run ok: "
            f"run_id={run_id}, head_sha={head_sha}, run_url={run_url}"
        )
        return 0

    RUNBOOK_PATH.write_text(runbook_text, encoding="utf-8")
    EXEC_PLAN_PATH.write_text(execution_text, encoding="utf-8")
    PROMOTION_PATH.write_text(promotion_text, encoding="utf-8")
    print(f"updated docs snapshot to run_id={run_id}, head_sha={head_sha}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
