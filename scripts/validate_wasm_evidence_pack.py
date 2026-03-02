#!/usr/bin/env python3
"""Validate a generated WASM evidence-pack directory."""

from __future__ import annotations

import argparse
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
        copied_path = Path(copied_to)
        if not source_path.is_file() and not args.allow_missing_source:
            return fail(f"required source artifact is missing: {source}")
        if not copied_path.is_file():
            return fail(f"copied artifact is missing: {copied_to}")

    missing_sources = [path for path in required if path not in copied_sources]
    if missing_sources:
        return fail(
            "manifest missing copied rows for required artifacts: "
            + ", ".join(missing_sources)
        )

    print(
        "wasm evidence pack validation passed: "
        f"{len(required)} required artifacts, pack_dir={pack_dir}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
