#!/usr/bin/env python3
"""Validate wasm browser-smoke baseline summary artifact."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def fail(message: str) -> int:
    print(f"wasm browser-smoke baseline validation failed: {message}")
    return 1


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--summary",
        default="perf/wasm_browser_smoke_baseline_latest.json",
        help="Browser-smoke baseline summary JSON path.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    summary_path = Path(args.summary)
    if not summary_path.is_file():
        return fail(f"missing summary: {summary_path}")

    try:
        payload = json.loads(summary_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return fail(f"invalid JSON: {exc}")

    generated_at_utc = payload.get("generated_at_utc")
    if not isinstance(generated_at_utc, str) or not generated_at_utc:
        return fail("generated_at_utc must be a non-empty string")

    browser = payload.get("browser")
    if browser not in {"chrome", "firefox"}:
        return fail("browser must be 'chrome' or 'firefox'")

    fallback_from = payload.get("fallback_from")
    if fallback_from is not None and fallback_from != "chrome":
        return fail("fallback_from must be null or 'chrome'")

    vm_probe_state_gate_smoke_enabled = payload.get("vm_probe_state_gate_smoke_enabled")
    if not isinstance(vm_probe_state_gate_smoke_enabled, bool):
        return fail("vm_probe_state_gate_smoke_enabled must be a boolean")

    vm_probe_state_gate_runner = payload.get("vm_probe_state_gate_runner")
    if vm_probe_state_gate_smoke_enabled:
        if vm_probe_state_gate_runner not in {"node", "browser"}:
            return fail(
                "vm_probe_state_gate_runner must be 'node' or 'browser' "
                "when vm_probe_state_gate_smoke_enabled is true"
            )
    elif vm_probe_state_gate_runner is not None:
        return fail(
            "vm_probe_state_gate_runner must be null when "
            "vm_probe_state_gate_smoke_enabled is false"
        )

    git = payload.get("git")
    if not isinstance(git, dict):
        return fail("git must be an object")
    branch = git.get("branch")
    commit = git.get("commit")
    if not isinstance(branch, str) or not branch:
        return fail("git.branch must be a non-empty string")
    if not isinstance(commit, str) or not commit:
        return fail("git.commit must be a non-empty string")

    print(f"wasm browser-smoke baseline validation passed: summary={summary_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
