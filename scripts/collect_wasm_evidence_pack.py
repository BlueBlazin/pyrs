#!/usr/bin/env python3
"""Bundle required local wasm artifacts into a single promotion-evidence directory."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

REQUIRED_ARTIFACTS = [
    "perf/wasm_worker_contract_summary_latest.json",
    "perf/wasm_worker_contract_summary_vm_probe_latest.json",
    "perf/wasm_execute_contract_summary_latest.json",
    "perf/wasm_execute_contract_summary_vm_probe_latest.json",
    "perf/wasm_session_contract_summary_latest.json",
    "perf/wasm_docs_execution_matrix_summary_latest.json",
    "perf/wasm_api_contract_surface_summary_latest.json",
    "perf/wasm_worker_docs_contract_summary_latest.json",
    "perf/wasm_client_flow_summary_latest.json",
    "perf/wasm_module_policy_summary_latest.json",
    "perf/wasm_capability_summary_latest.json",
    "perf/wasm_playground_worker_contract_latest.json",
    "perf/wasm_host_seam_audit_latest.json",
    "perf/wasm_vm_link_blockers_latest.json",
    "perf/wasm_vm_env_import_summary_latest.json",
]


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
        "--out-dir",
        default="perf/wasm_evidence_pack_latest",
        help="Directory where copied artifacts + manifest are written.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    copied: list[dict[str, str]] = []
    missing: list[str] = []
    for rel_path in REQUIRED_ARTIFACTS:
        src = Path(rel_path)
        if not src.is_file():
            missing.append(rel_path)
            continue
        dst = out_dir / src.name
        shutil.copy2(src, dst)
        copied.append({"source": rel_path, "copied_to": str(dst)})

    if missing:
        print("wasm evidence pack failed: missing required artifacts:")
        for rel_path in missing:
            print(f"- {rel_path}")
        print("run: scripts/check_wasm_branch.sh")
        return 1

    manifest = {
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "git": {
            "branch": git_value("rev-parse", "--abbrev-ref", "HEAD"),
            "commit": git_value("rev-parse", "HEAD"),
        },
        "artifact_count": len(copied),
        "required_artifacts": REQUIRED_ARTIFACTS,
        "copied_artifacts": copied,
    }
    manifest_path = out_dir / "manifest.json"
    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(f"wrote {manifest_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
