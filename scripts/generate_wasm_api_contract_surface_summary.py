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


def parse_source_const_string_map(wasm_source: str) -> dict[str, str]:
    const_map: dict[str, str] = {}
    pattern = re.compile(r'const\s+([A-Z0-9_]+):\s*&str\s*=\s*"([^"]+)";')
    for match in pattern.finditer(wasm_source):
        const_map[match.group(1)] = match.group(2)
    return const_map


def parse_source_enum_keys(
    wasm_source: str, enum_name: str, const_map: dict[str, str]
) -> list[str]:
    pattern = re.compile(
        rf"impl\s+{re.escape(enum_name)}\s*\{{.*?fn key\(self\) -> &'static str \{{(.*?)\n\s*\}}\n",
        flags=re.DOTALL,
    )
    match = pattern.search(wasm_source)
    if not match:
        raise ValueError(f"unable to parse key() implementation for enum {enum_name}")
    key_body = match.group(1)
    keys: list[str] = []
    arm_pattern = re.compile(rf"{re.escape(enum_name)}::[A-Za-z]+\s*=>\s*([^,]+),")
    for arm in arm_pattern.finditer(key_body):
        raw_value = arm.group(1).strip()
        if raw_value.startswith('"') and raw_value.endswith('"'):
            keys.append(raw_value.strip('"'))
            continue
        if raw_value in const_map:
            keys.append(const_map[raw_value])
            continue
        raise ValueError(
            f"unable to resolve enum key mapping value '{raw_value}' in {enum_name}::key"
        )
    return ordered_unique(keys)


def parse_source_worker_lifecycle_phase_keys(
    default_keys: list[str], const_map: dict[str, str]
) -> tuple[list[str], list[str]]:
    vm_probe_const_names = [
        "WASM_WORKER_LIFECYCLE_PHASE_STARTED",
        "WASM_WORKER_LIFECYCLE_PHASE_TERMINATED",
        "WASM_WORKER_LIFECYCLE_PHASE_RECYCLED",
    ]
    vm_probe_keys = [
        const_map[name] for name in vm_probe_const_names if name in const_map
    ]
    effective_keys = ordered_unique(default_keys + vm_probe_keys)
    return effective_keys, vm_probe_keys


def parse_source_worker_timeout_phase_keys(
    default_keys: list[str], const_map: dict[str, str]
) -> tuple[list[str], list[str]]:
    vm_probe_const_names = ["WASM_WORKER_TIMEOUT_CONFIGURED_PHASE"]
    vm_probe_keys = [
        const_map[name] for name in vm_probe_const_names if name in const_map
    ]
    effective_keys = ordered_unique(default_keys + vm_probe_keys)
    return effective_keys, vm_probe_keys


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
    docs_source: str,
    exported_functions: list[str],
    exported_struct_fields: dict[str, list[str]],
    docs_functions: list[str],
    docs_type_sections: dict[str, str],
    worker_lifecycle_phase_keys: list[str],
    worker_lifecycle_phase_keys_vm_probe_extra: list[str],
    worker_timeout_phase_keys: list[str],
    worker_timeout_phase_keys_vm_probe_extra: list[str],
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

    for key in worker_lifecycle_phase_keys:
        if key not in docs_source:
            errors.append(f"docs missing worker lifecycle phase key '{key}'")
    for key in worker_lifecycle_phase_keys_vm_probe_extra:
        if key not in docs_source:
            errors.append(f"docs missing worker vm-probe lifecycle phase key '{key}'")
    if worker_lifecycle_phase_keys_vm_probe_extra and "wasm-vm-probe" not in docs_source:
        errors.append("docs missing wasm-vm-probe mention for lifecycle phase mode behavior")
    for key in worker_timeout_phase_keys:
        if key not in docs_source:
            errors.append(f"docs missing worker timeout phase key '{key}'")
    for key in worker_timeout_phase_keys_vm_probe_extra:
        if key not in docs_source:
            errors.append(f"docs missing worker vm-probe timeout phase key '{key}'")
    if 'when worker `state = "ready"`' not in docs_source:
        errors.append("docs missing worker state-ready gating guidance")
    if 'worker `state != "ready"`' not in docs_source:
        errors.append("docs missing worker state-not-ready gating guidance")
    if "worker_runtime_unwired" not in docs_source:
        errors.append("docs missing worker_runtime_unwired blocker key guidance")

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

    const_map = parse_source_const_string_map(wasm_source)
    exported_functions = parse_exported_top_level_functions(wasm_source)
    exported_struct_fields = parse_exported_struct_fields(wasm_source)
    worker_lifecycle_phase_keys_default = parse_source_enum_keys(
        wasm_source, "WasmWorkerLifecyclePhase", const_map
    )
    (
        worker_lifecycle_phase_keys_effective,
        worker_lifecycle_phase_keys_vm_probe_extra,
    ) = parse_source_worker_lifecycle_phase_keys(
        worker_lifecycle_phase_keys_default, const_map
    )
    worker_timeout_phase_keys_default = parse_source_enum_keys(
        wasm_source, "WasmWorkerTimeoutPhase", const_map
    )
    (
        worker_timeout_phase_keys_effective,
        worker_timeout_phase_keys_vm_probe_extra,
    ) = parse_source_worker_timeout_phase_keys(
        worker_timeout_phase_keys_default, const_map
    )
    docs_functions = parse_docs_top_level_functions(docs_source)
    docs_type_sections = parse_docs_type_sections(docs_source)

    errors = validate(
        docs_source=docs_source,
        exported_functions=exported_functions,
        exported_struct_fields=exported_struct_fields,
        docs_functions=docs_functions,
        docs_type_sections=docs_type_sections,
        worker_lifecycle_phase_keys=worker_lifecycle_phase_keys_effective,
        worker_lifecycle_phase_keys_vm_probe_extra=worker_lifecycle_phase_keys_vm_probe_extra,
        worker_timeout_phase_keys=worker_timeout_phase_keys_effective,
        worker_timeout_phase_keys_vm_probe_extra=worker_timeout_phase_keys_vm_probe_extra,
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
        "worker_lifecycle_phase_keys_default": worker_lifecycle_phase_keys_default,
        "worker_lifecycle_phase_keys_vm_probe_extra": worker_lifecycle_phase_keys_vm_probe_extra,
        "worker_lifecycle_phase_keys_effective": worker_lifecycle_phase_keys_effective,
        "worker_timeout_phase_keys_default": worker_timeout_phase_keys_default,
        "worker_timeout_phase_keys_vm_probe_extra": worker_timeout_phase_keys_vm_probe_extra,
        "worker_timeout_phase_keys_effective": worker_timeout_phase_keys_effective,
    }

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
