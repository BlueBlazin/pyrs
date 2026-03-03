#!/usr/bin/env python3
"""Validate a generated WASM evidence-pack directory."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--pack-dir",
        default="perf/wasm_evidence_pack_latest",
        help="Evidence-pack directory containing manifest.json and copied artifacts.",
    )
    parser.add_argument(
        "--allow-missing-source",
        action="store_true",
        help="Allow manifest source paths to be absent in current workspace "
        "(useful when validating a downloaded artifact bundle in CI).",
    )
    return parser.parse_args()


def fail(message: str) -> int:
    print(f"wasm evidence pack validation failed: {message}")
    return 1


def sha256_of_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        while True:
            chunk = fh.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def resolve_copied_path(copied_to: str, pack_dir: Path) -> Path:
    """Resolve copied artifact path for both local and downloaded pack validation.

    Manifest rows currently store the original local copy target
    (`perf/wasm_evidence_pack_latest/<file>`). When the bundle is downloaded as a
    CI artifact, files may be unpacked under a different directory. In that case,
    validate against `<pack-dir>/<basename>` as the relocated target.
    """
    declared = Path(copied_to)
    if declared.is_file():
        return declared
    relocated = pack_dir / declared.name
    return relocated


def find_copied_artifact(
    copied_rows: list[dict], source_path: str, pack_dir: Path
) -> Path | None:
    for row in copied_rows:
        if row.get("source") == source_path:
            copied_to = row.get("copied_to")
            if isinstance(copied_to, str) and copied_to:
                return resolve_copied_path(copied_to, pack_dir)
    return None


def main() -> int:
    args = parse_args()
    pack_dir = Path(args.pack_dir)
    manifest_path = pack_dir / "manifest.json"
    if not manifest_path.is_file():
        return fail(f"missing manifest: {manifest_path}")

    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return fail(f"invalid manifest JSON: {exc}")

    required = manifest.get("required_artifacts")
    copied = manifest.get("copied_artifacts")
    if not isinstance(required, list) or not required:
        return fail("manifest.required_artifacts must be a non-empty array")
    if not isinstance(copied, list):
        return fail("manifest.copied_artifacts must be an array")

    copied_sources: set[str] = set()
    for row in copied:
        if not isinstance(row, dict):
            return fail("manifest.copied_artifacts rows must be objects")
        source = row.get("source")
        copied_to = row.get("copied_to")
        if not isinstance(source, str) or not source:
            return fail("copied artifact row missing non-empty 'source'")
        if not isinstance(copied_to, str) or not copied_to:
            return fail("copied artifact row missing non-empty 'copied_to'")
        copied_sources.add(source)

        source_path = Path(source)
        copied_path = resolve_copied_path(copied_to, pack_dir)
        if not source_path.is_file() and not args.allow_missing_source:
            return fail(f"required source artifact is missing: {source}")
        if not copied_path.is_file():
            return fail(
                "copied artifact is missing: "
                f"declared={copied_to}, checked={copied_path}"
            )

    missing_sources = [path for path in required if path not in copied_sources]
    if missing_sources:
        return fail(
            "manifest missing copied rows for required artifacts: "
            + ", ".join(missing_sources)
        )

    env_import_summary = find_copied_artifact(
        copied,
        "perf/wasm_vm_env_import_summary_latest.json",
        pack_dir,
    )
    if env_import_summary is None:
        return fail(
            "manifest missing copied row for perf/wasm_vm_env_import_summary_latest.json"
        )
    if not env_import_summary.is_file():
        return fail(f"missing env-import summary artifact: {env_import_summary}")
    try:
        env_payload = json.loads(env_import_summary.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return fail(f"invalid env-import summary JSON ({env_import_summary}): {exc}")

    counts = env_payload.get("counts")
    if not isinstance(counts, dict):
        return fail("env-import summary missing object field 'counts'")
    env_function_imports = counts.get("env_function_imports")
    if env_function_imports != 0:
        return fail(
            "env-import gate failed: expected counts.env_function_imports == 0, "
            f"got {env_function_imports}"
        )

    shim_missing_symbols = env_payload.get("shim_missing_symbols")
    if shim_missing_symbols not in ([], None):
        return fail(
            "env-import shim gate failed: expected shim_missing_symbols to be empty, "
            f"got {shim_missing_symbols}"
        )

    playground_worker_contract = find_copied_artifact(
        copied,
        "perf/wasm_playground_worker_contract_latest.json",
        pack_dir,
    )
    if playground_worker_contract is None:
        return fail(
            "manifest missing copied row for "
            "perf/wasm_playground_worker_contract_latest.json"
        )
    if not playground_worker_contract.is_file():
        return fail(
            "missing playground worker contract artifact: "
            f"{playground_worker_contract}"
        )
    try:
        playground_payload = json.loads(
            playground_worker_contract.read_text(encoding="utf-8")
        )
    except json.JSONDecodeError as exc:
        return fail(
            "invalid playground worker contract JSON "
            f"({playground_worker_contract}): {exc}"
        )

    if playground_payload.get("ok") is not True:
        return fail(
            "playground worker contract gate failed: expected ok=true, "
            f"got {playground_payload.get('ok')}"
        )
    failure_count = playground_payload.get("failure_count")
    if failure_count != 0:
        return fail(
            "playground worker contract gate failed: expected failure_count=0, "
            f"got {failure_count}"
        )

    artifact_hash_summary = find_copied_artifact(
        copied,
        "perf/wasm_artifact_input_hashes_latest.json",
        pack_dir,
    )
    if artifact_hash_summary is None:
        return fail(
            "manifest missing copied row for perf/wasm_artifact_input_hashes_latest.json"
        )
    if not artifact_hash_summary.is_file():
        return fail(f"missing artifact input hash summary: {artifact_hash_summary}")
    try:
        hash_payload = json.loads(artifact_hash_summary.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return fail(
            f"invalid artifact input hash summary JSON ({artifact_hash_summary}): {exc}"
        )

    hash_rows = hash_payload.get("artifacts")
    if not isinstance(hash_rows, list) or not hash_rows:
        return fail("artifact input hash summary missing non-empty 'artifacts' array")

    hash_by_path: dict[str, dict] = {}
    for row in hash_rows:
        if not isinstance(row, dict):
            return fail("artifact input hash row must be an object")
        row_path = row.get("path")
        row_sha = row.get("sha256")
        row_size = row.get("size_bytes")
        if not isinstance(row_path, str) or not row_path:
            return fail("artifact input hash row missing non-empty 'path'")
        if not isinstance(row_sha, str) or not row_sha:
            return fail(f"artifact input hash row missing sha256 for {row_path}")
        if not isinstance(row_size, int) or row_size < 0:
            return fail(f"artifact input hash row has invalid size_bytes for {row_path}")
        if len(row_sha) != 64:
            return fail(f"artifact input hash row has malformed sha256 for {row_path}")
        hash_by_path[row_path] = row

    expected_hash_paths = [
        source for source in required if source != "perf/wasm_artifact_input_hashes_latest.json"
    ]
    for expected_path in expected_hash_paths:
        if expected_path not in hash_by_path:
            return fail(
                "artifact input hash summary missing required row for "
                f"{expected_path}"
            )

        copied_artifact = find_copied_artifact(copied, expected_path, pack_dir)
        if copied_artifact is None or not copied_artifact.is_file():
            return fail(
                "artifact input hash summary references missing copied artifact: "
                f"{expected_path}"
            )
        observed_sha = sha256_of_file(copied_artifact)
        expected_sha = hash_by_path[expected_path]["sha256"]
        if observed_sha != expected_sha:
            return fail(
                "artifact input hash mismatch for "
                f"{expected_path}: expected {expected_sha}, observed {observed_sha}"
            )

    print(
        "wasm evidence pack validation passed: "
        f"{len(required)} required artifacts, pack_dir={pack_dir}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
