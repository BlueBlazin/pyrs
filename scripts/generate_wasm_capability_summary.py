#!/usr/bin/env python3
"""Generate and validate wasm capability parity summary."""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class CapabilityRow:
    key: str
    native_supported: bool
    wasm_supported: bool


def parse_fixture_rows(source: str) -> list[CapabilityRow]:
    pattern = re.compile(
        r'WasmCapabilityFixture\s*\{\s*key:\s*"([^"]+)",\s*'
        r"native_supported:\s*(true|false),\s*"
        r"wasm_supported:\s*(true|false),\s*\}",
        flags=re.DOTALL,
    )
    rows: list[CapabilityRow] = []
    for key, native_supported, wasm_supported in pattern.findall(source):
        rows.append(
            CapabilityRow(
                key=key,
                native_supported=native_supported == "true",
                wasm_supported=wasm_supported == "true",
            )
        )
    return rows


def parse_variant_to_key(host_source: str) -> dict[str, str]:
    key_fn_match = re.search(
        r"pub fn key\(self\) -> &'static str \{(.*?)\n\s*\}",
        host_source,
        flags=re.DOTALL,
    )
    if not key_fn_match:
        raise ValueError("unable to parse HostCapability::key()")
    key_body = key_fn_match.group(1)
    mapping: dict[str, str] = {}
    for variant, key in re.findall(r'Self::([A-Za-z0-9_]+)\s*=>\s*"([^"]+)"', key_body):
        mapping[variant] = key
    return mapping


def parse_source_variant_order(host_source: str) -> list[str]:
    match = re.search(
        r"pub const ALL:\s*\[HostCapability;\s*\d+\]\s*=\s*\[(.*?)\];",
        host_source,
        flags=re.DOTALL,
    )
    if not match:
        raise ValueError("unable to parse HostCapability::ALL")
    return re.findall(r"HostCapability::([A-Za-z0-9_]+)", match.group(1))


def parse_support_variants(host_source: str, host_name: str) -> tuple[list[str], bool]:
    supports_pattern = re.compile(
        rf"impl\s+VmHost\s+for\s+{re.escape(host_name)}\s*\{{[\s\S]*?"
        r"fn supports\(&self,\s*[A-Za-z_][A-Za-z0-9_]*:\s*HostCapability\)\s*->\s*bool\s*\{"
        r"([\s\S]*?)\n\s*\}",
        flags=re.DOTALL,
    )
    supports_match = supports_pattern.search(host_source)
    if supports_match is None:
        raise ValueError(f"unable to parse supports() body for {host_name}")
    supports_body = supports_match.group(1).strip()

    if supports_body == "true":
        return [], True
    if "matches!(" in supports_body:
        return re.findall(r"HostCapability::([A-Za-z0-9_]+)", supports_body), False
    raise ValueError(
        f"unsupported supports() pattern for {host_name}; expected `true` or `matches!`"
    )


def parse_doc_table_rows(doc_source: str) -> list[CapabilityRow]:
    rows: list[CapabilityRow] = []
    for raw_line in doc_source.splitlines():
        line = raw_line.strip()
        if not line.startswith("|"):
            continue
        parts = [part.strip() for part in line.split("|")[1:-1]]
        if len(parts) < 4:
            continue
        capability, native_raw, wasm_raw, _contract = parts[:4]
        if capability == "Capability" and native_raw == "Native Host":
            continue
        if capability.startswith("---"):
            continue
        key = capability.strip("`")

        native_supported = native_raw.lower().startswith("supported")
        wasm_supported = wasm_raw.lower().startswith("supported")
        rows.append(
            CapabilityRow(
                key=key,
                native_supported=native_supported,
                wasm_supported=wasm_supported,
            )
        )
    return rows


def parse_doc_accepted_keys(doc_source: str) -> list[str]:
    section_match = re.search(
        r"Accepted capability keys:\n(.*?)\n##\s+Error-Surface Policy",
        doc_source,
        flags=re.DOTALL,
    )
    if not section_match:
        raise ValueError("unable to locate 'Accepted capability keys' section in docs")
    section = section_match.group(1)
    return re.findall(r"-\s+`([^`]+)`", section)


def row_map(rows: list[CapabilityRow]) -> dict[str, CapabilityRow]:
    return {row.key: row for row in rows}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--fixture",
        default="tests/fixtures/wasm_capability_matrix.rs",
        help="Path to wasm capability fixture file",
    )
    parser.add_argument(
        "--host-src",
        default="src/host/mod.rs",
        help="Path to host capability source file",
    )
    parser.add_argument(
        "--doc",
        default="docs/WASM_CAPABILITY_MATRIX.md",
        help="Path to wasm capability matrix doc",
    )
    parser.add_argument(
        "--out",
        default="perf/wasm_capability_summary_latest.json",
        help="Output summary JSON path",
    )
    args = parser.parse_args()

    fixture_path = Path(args.fixture)
    fixture_source = fixture_path.read_text(encoding="utf-8")
    fixture_rows = parse_fixture_rows(fixture_source)

    host_src_path = Path(args.host_src)
    host_source = host_src_path.read_text(encoding="utf-8")
    variant_to_key = parse_variant_to_key(host_source)
    source_variant_order = parse_source_variant_order(host_source)
    source_key_order = [
        variant_to_key[variant]
        for variant in source_variant_order
        if variant in variant_to_key
    ]

    native_variants, native_all_supported = parse_support_variants(host_source, "NativeHost")
    wasm_variants, wasm_all_supported = parse_support_variants(host_source, "WasmHost")

    native_supported_keys = set(source_key_order if native_all_supported else [])
    if not native_all_supported:
        native_supported_keys = {
            variant_to_key[variant] for variant in native_variants if variant in variant_to_key
        }
    wasm_supported_keys = {
        variant_to_key[variant] for variant in wasm_variants if variant in variant_to_key
    }

    source_rows = [
        CapabilityRow(
            key=key,
            native_supported=key in native_supported_keys,
            wasm_supported=key in wasm_supported_keys,
        )
        for key in source_key_order
    ]

    doc_path = Path(args.doc)
    doc_source = doc_path.read_text(encoding="utf-8")
    doc_rows = parse_doc_table_rows(doc_source)
    doc_accepted_keys = parse_doc_accepted_keys(doc_source)

    errors: list[str] = []
    if not fixture_rows:
        errors.append("fixture capability rows must not be empty")
    if not source_rows:
        errors.append("source capability rows must not be empty")
    if not doc_rows:
        errors.append("doc capability rows must not be empty")
    if not doc_accepted_keys:
        errors.append("doc accepted capability keys must not be empty")

    fixture_keys = [row.key for row in fixture_rows]
    source_keys = [row.key for row in source_rows]
    doc_keys = [row.key for row in doc_rows]

    if len(fixture_keys) != len(set(fixture_keys)):
        errors.append("fixture capability rows contain duplicate keys")
    if len(source_keys) != len(set(source_keys)):
        errors.append("source capability rows contain duplicate keys")
    if len(doc_keys) != len(set(doc_keys)):
        errors.append("doc capability table contains duplicate keys")
    if len(doc_accepted_keys) != len(set(doc_accepted_keys)):
        errors.append("doc accepted capability list contains duplicate keys")

    fixture_set = set(fixture_keys)
    source_set = set(source_keys)
    doc_set = set(doc_keys)
    accepted_set = set(doc_accepted_keys)

    if fixture_set != source_set:
        missing_in_fixture = sorted(source_set - fixture_set)
        missing_in_source = sorted(fixture_set - source_set)
        if missing_in_fixture:
            errors.append(f"capability keys missing in fixture: {missing_in_fixture}")
        if missing_in_source:
            errors.append(f"capability keys missing in source: {missing_in_source}")
    if source_set != doc_set:
        missing_in_doc = sorted(source_set - doc_set)
        extra_in_doc = sorted(doc_set - source_set)
        if missing_in_doc:
            errors.append(f"capability keys missing in doc table: {missing_in_doc}")
        if extra_in_doc:
            errors.append(f"capability keys present in doc table but missing in source: {extra_in_doc}")
    if source_set != accepted_set:
        missing_in_accepted = sorted(source_set - accepted_set)
        extra_in_accepted = sorted(accepted_set - source_set)
        if missing_in_accepted:
            errors.append(f"capability keys missing in accepted list: {missing_in_accepted}")
        if extra_in_accepted:
            errors.append(
                f"capability keys present in accepted list but missing in source: {extra_in_accepted}"
            )

    if fixture_keys != source_keys:
        errors.append(f"fixture capability key order mismatch source order: {source_keys}")
    if doc_keys != source_keys:
        errors.append(f"doc capability table order mismatch source order: {source_keys}")
    if doc_accepted_keys != source_keys:
        errors.append(f"doc accepted key order mismatch source order: {source_keys}")

    fixture_map = row_map(fixture_rows)
    doc_map = row_map(doc_rows)
    for source_row in source_rows:
        key = source_row.key
        fixture_row = fixture_map.get(key)
        doc_row = doc_map.get(key)
        if fixture_row is None:
            continue
        if (
            fixture_row.native_supported != source_row.native_supported
            or fixture_row.wasm_supported != source_row.wasm_supported
        ):
            errors.append(
                f"fixture support mismatch for '{key}': fixture(native={fixture_row.native_supported}, wasm={fixture_row.wasm_supported}) "
                f"source(native={source_row.native_supported}, wasm={source_row.wasm_supported})"
            )
        if doc_row is None:
            continue
        if (
            doc_row.native_supported != source_row.native_supported
            or doc_row.wasm_supported != source_row.wasm_supported
        ):
            errors.append(
                f"doc support mismatch for '{key}': doc(native={doc_row.native_supported}, wasm={doc_row.wasm_supported}) "
                f"source(native={source_row.native_supported}, wasm={source_row.wasm_supported})"
            )

    if errors:
        print("wasm capability summary validation failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    summary = {
        "fixture": str(fixture_path),
        "host_source": str(host_src_path),
        "doc": str(doc_path),
        "counts": {
            "fixture_rows": len(fixture_rows),
            "source_rows": len(source_rows),
            "doc_rows": len(doc_rows),
            "doc_accepted_keys": len(doc_accepted_keys),
        },
        "rows": [
            {
                "key": row.key,
                "native_supported": row.native_supported,
                "wasm_supported": row.wasm_supported,
            }
            for row in source_rows
        ],
    }

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
