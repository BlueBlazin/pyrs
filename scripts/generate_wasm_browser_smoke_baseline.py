#!/usr/bin/env python3
"""Generate a browser-smoke baseline summary artifact for wasm workflow runs."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


def git_value(*args: str) -> str:
    result = subprocess.run(
        ["git", *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        return "unknown"
    return result.stdout.strip() or "unknown"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--browser",
        required=True,
        choices=("chrome", "firefox"),
        help="Browser that completed smoke successfully.",
    )
    parser.add_argument(
        "--out",
        default="perf/wasm_browser_smoke_baseline_latest.json",
        help="Path to write baseline summary JSON.",
    )
    parser.add_argument(
        "--fallback-from",
        choices=("chrome",),
        help="Primary browser that failed before fallback succeeded.",
    )
    parser.add_argument(
        "--vm-probe-state-gate",
        action="store_true",
        help="Record that vm-probe terminate/recycle state-gate browser smoke was enabled.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    payload = {
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "browser": args.browser,
        "fallback_from": args.fallback_from,
        "vm_probe_state_gate_smoke_enabled": args.vm_probe_state_gate,
        "git": {
            "branch": git_value("rev-parse", "--abbrev-ref", "HEAD"),
            "commit": git_value("rev-parse", "HEAD"),
        },
    }
    out_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
