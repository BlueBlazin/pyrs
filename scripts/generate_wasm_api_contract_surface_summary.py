#!/usr/bin/env python3
"""Validate WASM API contract docs against source-exported surface."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


def ordered_unique(values: list[str]) -> list[str]:
    seen: set[str] = set()
    ordered: list[str] = []
    for value in values:
        if value in seen:
            continue
        seen.add(value)
        ordered.append(value)
    return ordered


def parse_exported_top_level_functions(wasm_source: str) -> list[str]:
    pattern = re.compile(
        r"^\s*#\[wasm_bindgen\]\s*\n\s*pub fn ([a-zA-Z0-9_]+)\(",
        flags=re.MULTILINE,
    )
    return ordered_unique(pattern.findall(wasm_source))


def parse_exported_struct_fields(wasm_source: str) -> dict[str, list[str]]:
    pattern = re.compile(
        r"^\s*#\[wasm_bindgen\(getter_with_clone\)\]\s*\n\s*pub struct ([A-Za-z0-9_]+)\s*\{(.*?)\n\}",
        flags=re.MULTILINE | re.DOTALL,
    )
    result: dict[str, list[str]] = {}
    for match in pattern.finditer(wasm_source):
        struct_name = match.group(1)
        body = match.group(2)
        fields = re.findall(r"^\s*([a-z_][a-z0-9_]*):", body, flags=re.MULTILINE)
        result[struct_name] = ordered_unique(fields)
    return result


def parse_docs_top_level_functions(docs_source: str) -> list[str]:
    start_anchor = "## Top-Level Functions"
    end_anchor = "## Exported Types"
    if start_anchor not in docs_source:
        raise ValueError("missing '## Top-Level Functions' section")
    if end_anchor not in docs_source:
        raise ValueError("missing '## Exported Types' section")
    section = docs_source.split(start_anchor, 1)[1].split(end_anchor, 1)[0]
    names = re.findall(r"^-\s*`([a-zA-Z0-9_]+)\(", section, flags=re.MULTILINE)
    return ordered_unique(names)


def parse_docs_type_sections(docs_source: str) -> dict[str, str]:
    pattern = re.compile(
        r"^##\s+`([A-Za-z0-9_]+)`\s*\n(.*?)(?=^##\s+`[A-Za-z0-9_]+`|\Z)",
        flags=re.MULTILINE | re.DOTALL,
    )
    sections: dict[str, str] = {}
    for match in pattern.finditer(docs_source):
        sections[match.group(1)] = match.group(2)
    return sections


def validate(
    exported_functions: list[str],
    exported_struct_fields: dict[str, list[str]],
    docs_functions: list[str],
    docs_type_sections: dict[str, str],
) -> list[str]:
    errors: list[str] = []

    source_fn_set = set(exported_functions)
    docs_fn_set = set(docs_functions)

    missing_doc_fns = sorted(source_fn_set - docs_fn_set)
    unknown_doc_fns = sorted(docs_fn_set - source_fn_set)
    if missing_doc_fns:
        errors.append(
            "docs missing top-level wasm functions: " + ", ".join(missing_doc_fns)
        )
    if unknown_doc_fns:
        errors.append(
            "docs list unknown top-level wasm functions: " + ", ".join(unknown_doc_fns)
        )

    for struct_name, fields in exported_struct_fields.items():
        section = docs_type_sections.get(struct_name)
        if section is None:
            errors.append(f"docs missing exported type section `{struct_name}`")
            continue
        for field in fields:
            if f"`{field}:" not in section:
                errors.append(f"docs missing field `{field}` in `{struct_name}` section")

    unknown_doc_type_sections = sorted(set(docs_type_sections.keys()) - set(exported_struct_fields.keys()))
    if unknown_doc_type_sections:
        errors.append(
            "docs contain unknown exported type sections: "
            + ", ".join(unknown_doc_type_sections)
        )

    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--docs",
        default="docs/WASM_API_CONTRACT.md",
        help="Path to WASM API contract docs",
    )
    parser.add_argument(
        "--wasm-src",
        default="src/wasm/mod.rs",
        help="Path to wasm source",
    )
    parser.add_argument(
        "--out",
        default="perf/wasm_api_contract_surface_summary_latest.json",
        help="Output summary path",
    )
    args = parser.parse_args()

    docs_path = Path(args.docs)
    wasm_src_path = Path(args.wasm_src)
    docs_source = docs_path.read_text(encoding="utf-8")
    wasm_source = wasm_src_path.read_text(encoding="utf-8")

    exported_functions = parse_exported_top_level_functions(wasm_source)
    exported_struct_fields = parse_exported_struct_fields(wasm_source)
    docs_functions = parse_docs_top_level_functions(docs_source)
    docs_type_sections = parse_docs_type_sections(docs_source)

    errors = validate(
        exported_functions=exported_functions,
        exported_struct_fields=exported_struct_fields,
        docs_functions=docs_functions,
        docs_type_sections=docs_type_sections,
    )
    if errors:
        print("wasm api contract surface validation failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    summary = {
        "docs": str(docs_path),
        "wasm_source": str(wasm_src_path),
        "exported_top_level_functions": exported_functions,
        "docs_top_level_functions": docs_functions,
        "exported_structs": {
            name: fields for name, fields in sorted(exported_struct_fields.items())
        },
        "docs_type_sections": sorted(docs_type_sections.keys()),
    }

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
