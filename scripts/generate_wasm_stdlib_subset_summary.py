#!/usr/bin/env python3
"""Generate deterministic summary for wasm curated stdlib subset artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from datetime import datetime, timezone
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--manifest",
        default="website/public/wasm/stdlib_subset_manifest_v1.json",
        help="Input stdlib subset manifest path.",
    )
    parser.add_argument(
        "--out",
        default="perf/wasm_stdlib_subset_summary_latest.json",
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


def fail(message: str) -> int:
    print(f"wasm stdlib subset summary failed: {message}")
    return 1


def main() -> int:
    args = parse_args()
    manifest_path = Path(args.manifest)
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    if not manifest_path.is_file():
        return fail(f"missing manifest: {manifest_path}")

    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return fail(f"invalid manifest JSON ({manifest_path}): {exc}")

    source_root = manifest.get("source_lib_root")
    if not isinstance(source_root, str) or not source_root:
        return fail("manifest missing non-empty source_lib_root")

    totals = manifest.get("totals")
    if not isinstance(totals, dict):
        return fail("manifest missing totals object")

    zip_bytes = totals.get("zip_bytes")
    pack_bytes = totals.get("pack_bytes")
    zip_sha_expected = totals.get("zip_sha256")
    pack_sha_expected = totals.get("pack_sha256")
    if not isinstance(zip_bytes, int) or zip_bytes < 0:
        return fail("manifest totals.zip_bytes must be non-negative int")
    if not isinstance(pack_bytes, int) or pack_bytes < 0:
        return fail("manifest totals.pack_bytes must be non-negative int")
    if not isinstance(zip_sha_expected, str) or len(zip_sha_expected) != 64:
        return fail("manifest totals.zip_sha256 must be sha256 hex string")
    if not isinstance(pack_sha_expected, str) or len(pack_sha_expected) != 64:
        return fail("manifest totals.pack_sha256 must be sha256 hex string")

    zip_path = manifest_path.parent / "stdlib_subset_v1.zip"
    pack_path = manifest_path.parent / "stdlib_subset_v1.json"
    if not zip_path.is_file():
        return fail(f"missing stdlib subset zip: {zip_path}")
    if not pack_path.is_file():
        return fail(f"missing stdlib subset source pack: {pack_path}")

    zip_sha_observed = sha256_of(zip_path)
    pack_sha_observed = sha256_of(pack_path)
    if zip_sha_observed != zip_sha_expected:
        return fail(
            "stdlib subset zip sha mismatch: "
            f"manifest={zip_sha_expected} observed={zip_sha_observed}"
        )
    if pack_sha_observed != pack_sha_expected:
        return fail(
            "stdlib subset source-pack sha mismatch: "
            f"manifest={pack_sha_expected} observed={pack_sha_observed}"
        )

    summary = {
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "manifest_path": str(manifest_path),
        "pack_version": manifest.get("pack_version"),
        "python_version_target": manifest.get("python_version_target"),
        "module_count": manifest.get("module_count"),
        "seed_modules": manifest.get("seed_modules"),
        "excluded_modules": manifest.get("excluded_modules"),
        "source_lib_root": source_root,
        "zip": {
            "path": str(zip_path),
            "size_bytes": zip_path.stat().st_size,
            "sha256": zip_sha_observed,
        },
        "source_pack": {
            "path": str(pack_path),
            "size_bytes": pack_path.stat().st_size,
            "sha256": pack_sha_observed,
        },
    }
    out_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
