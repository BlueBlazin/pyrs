#!/usr/bin/env python3
"""Generate and validate wasm module-policy parity summary."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


def parse_fixture_pairs(source: str) -> list[tuple[str, str]]:
    pattern = re.compile(
        r'WasmModulePolicyFixture\s*\{\s*module:\s*"([^"]+)",\s*blocker_key:\s*"([^"]+)"',
        flags=re.DOTALL,
    )
    return pattern.findall(source)


def parse_source_pairs(source: str) -> list[tuple[str, str]]:
    const_match = re.search(
        r'const\s+WASM_MODULE_BLOCKER_POLICY:\s*\[\(&str,\s*&str\);\s*\d+\]\s*=\s*\[(.*?)\];',
        source,
        flags=re.DOTALL,
    )
    if not const_match:
        raise ValueError("unable to locate WASM_MODULE_BLOCKER_POLICY in source")
    body = const_match.group(1)
    return re.findall(r'\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*\)', body)


def parse_doc_pairs(source: str) -> list[tuple[str, str]]:
    pairs: list[tuple[str, str]] = []
    for raw_line in source.splitlines():
        line = raw_line.strip()
        if not line.startswith("|"):
            continue
        parts = [part.strip() for part in line.split("|")[1:-1]]
        if len(parts) < 3:
            continue
        module = parts[0]
        blocker_key = parts[1]
        if module == "Module" and blocker_key == "Blocker key":
            continue
        if module.startswith("---") or blocker_key.startswith("---"):
            continue
        pairs.append((module.strip("`"), blocker_key.strip("`")))
    return pairs


def unique_pairs(pairs: list[tuple[str, str]]) -> list[tuple[str, str]]:
    return sorted(set(pairs))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--fixture",
        default="tests/fixtures/wasm_module_policy.rs",
        help="Path to wasm module-policy fixture file",
    )
    parser.add_argument(
        "--wasm-src",
        default="src/wasm/mod.rs",
        help="Path to wasm source file",
    )
    parser.add_argument(
        "--out",
        default="perf/wasm_module_policy_summary_latest.json",
        help="Output summary JSON path",
    )
    parser.add_argument(
        "--doc",
        default="docs/WASM_MODULE_SUPPORT_POLICY.md",
        help="Path to wasm module support policy doc table",
    )
    args = parser.parse_args()

    fixture_path = Path(args.fixture)
    fixture_source = fixture_path.read_text(encoding="utf-8")
    fixture_pairs = parse_fixture_pairs(fixture_source)

    wasm_source_path = Path(args.wasm_src)
    wasm_source = wasm_source_path.read_text(encoding="utf-8")
    source_pairs = parse_source_pairs(wasm_source)
    doc_path = Path(args.doc)
    doc_source = doc_path.read_text(encoding="utf-8")
    doc_pairs = parse_doc_pairs(doc_source)

    errors: list[str] = []
    if not fixture_pairs:
        errors.append("fixture policy rows must not be empty")
    if not source_pairs:
        errors.append("source policy rows must not be empty")
    if not doc_pairs:
        errors.append("doc policy table rows must not be empty")

    if len(fixture_pairs) != len(set(fixture_pairs)):
        errors.append("fixture policy rows contain duplicates")
    if len(source_pairs) != len(set(source_pairs)):
        errors.append("source policy rows contain duplicates")
    if len(doc_pairs) != len(set(doc_pairs)):
        errors.append("doc policy rows contain duplicates")

    fixture_set = set(fixture_pairs)
    source_set = set(source_pairs)
    doc_set = set(doc_pairs)
    if fixture_set != source_set:
        missing_in_fixture = sorted(source_set - fixture_set)
        missing_in_source = sorted(fixture_set - source_set)
        if missing_in_fixture:
            errors.append(f"rows missing in fixture: {missing_in_fixture}")
        if missing_in_source:
            errors.append(f"rows missing in source: {missing_in_source}")
    if source_set != doc_set:
        missing_in_doc = sorted(source_set - doc_set)
        missing_in_source_from_doc = sorted(doc_set - source_set)
        if missing_in_doc:
            errors.append(f"rows missing in docs: {missing_in_doc}")
        if missing_in_source_from_doc:
            errors.append(f"rows present in docs but missing in source: {missing_in_source_from_doc}")

    if errors:
        print("wasm module policy summary validation failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    summary = {
        "fixture": str(fixture_path),
        "wasm_source": str(wasm_source_path),
        "doc": str(doc_path),
        "counts": {
            "fixture_rows": len(fixture_pairs),
            "source_rows": len(source_pairs),
            "doc_rows": len(doc_pairs),
        },
        "rows": [
            {"module": module, "blocker_key": blocker}
            for module, blocker in unique_pairs(fixture_pairs)
        ],
    }

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
