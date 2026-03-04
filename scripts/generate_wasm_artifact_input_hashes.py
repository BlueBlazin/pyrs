#!/usr/bin/env python3
"""Generate deterministic SHA256 summary for local wasm evidence artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

INPUT_ARTIFACTS = [
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
    "perf/wasm_stdlib_subset_summary_latest.json",
    "perf/wasm_vm_link_blockers_latest.json",
    "perf/wasm_vm_env_import_summary_latest.json",
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--out",
        default="perf/wasm_artifact_input_hashes_latest.json",
        help="Output JSON path.",
    )
    return parser.parse_args()


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        while True:
            chunk = fh.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    args = parse_args()
    output_path = Path(args.out)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    entries: list[dict[str, object]] = []
    missing: list[str] = []
    for rel_path in INPUT_ARTIFACTS:
        path = Path(rel_path)
        if not path.is_file():
            missing.append(rel_path)
            continue
        entries.append(
            {
                "path": rel_path,
                "size_bytes": path.stat().st_size,
                "sha256": sha256_of(path),
            }
        )

    if missing:
        print("wasm artifact input hash summary failed: missing required artifacts:")
        for rel_path in missing:
            print(f"- {rel_path}")
        print("run: scripts/check_wasm_branch.sh")
        return 1

    payload = {
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "artifact_count": len(entries),
        "artifacts": entries,
    }
    output_path.write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(f"wrote {output_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
