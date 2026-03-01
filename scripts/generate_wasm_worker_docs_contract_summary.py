#!/usr/bin/env python3
"""Validate wasm worker runtime docs against source/runtime key contracts."""

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


def parse_source_const_u32(wasm_source: str, const_name: str) -> int:
    match = re.search(
        rf"const\s+{re.escape(const_name)}:\s*u32\s*=\s*([0-9_]+)\s*;",
        wasm_source,
    )
    if not match:
        raise ValueError(f"unable to parse u32 const {const_name}")
    return int(match.group(1).replace("_", ""))


def parse_source_const_string(wasm_source: str, const_name: str) -> str:
    match = re.search(
        rf'const\s+{re.escape(const_name)}:\s*&str\s*=\s*"([^"]+)";',
        wasm_source,
    )
    if not match:
        raise ValueError(f"unable to parse string const {const_name}")
    return match.group(1)


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


def parse_source_module_policy_blocker_keys(wasm_source: str) -> list[str]:
    match = re.search(
        r"const\s+WASM_MODULE_BLOCKER_POLICY:[^=]*=\s*\[(.*?)\];",
        wasm_source,
        flags=re.DOTALL,
    )
    if not match:
        raise ValueError("unable to parse WASM_MODULE_BLOCKER_POLICY")
    rows = re.findall(r'\(\s*"[^"]+"\s*,\s*"([^"]+)"\s*\)', match.group(1))
    return ordered_unique(rows)


def parse_source_worker_operation_prefixes(wasm_source: str) -> list[str]:
    actions = re.findall(r'next_worker_operation_id\("([^"]+)"\)', wasm_source)
    prefixes = [f"worker_{action}_" for action in ordered_unique(actions)]
    return ordered_unique(prefixes)


def validate_contains_all(
    docs_source: str, values: list[str], label: str, errors: list[str]
) -> None:
    for value in values:
        if value not in docs_source:
            errors.append(f"docs missing {label} '{value}'")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--docs",
        default="docs/WASM_WORKER_RUNTIME_CONTRACT.md",
        help="Path to worker runtime contract docs",
    )
    parser.add_argument(
        "--wasm-src",
        default="src/wasm/mod.rs",
        help="Path to wasm source",
    )
    parser.add_argument(
        "--out",
        default="perf/wasm_worker_docs_contract_summary_latest.json",
        help="Output summary path",
    )
    args = parser.parse_args()

    docs_path = Path(args.docs)
    wasm_src_path = Path(args.wasm_src)
    docs_source = docs_path.read_text(encoding="utf-8")
    wasm_source = wasm_src_path.read_text(encoding="utf-8")

    const_map = parse_source_const_string_map(wasm_source)
    worker_state_keys = parse_source_enum_keys(wasm_source, "WasmWorkerState", const_map)
    worker_lifecycle_phase_keys = parse_source_enum_keys(
        wasm_source, "WasmWorkerLifecyclePhase", const_map
    )
    worker_execute_phases = parse_source_enum_keys(
        wasm_source, "WasmWorkerExecutePhase", const_map
    )
    worker_timeout_phases = parse_source_enum_keys(
        wasm_source, "WasmWorkerTimeoutPhase", const_map
    )
    worker_unwired_blocker = parse_source_const_string(
        wasm_source, "WASM_WORKER_BLOCKER_RUNTIME_UNWIRED"
    )
    worker_interruption_model = parse_source_const_string(
        wasm_source, "WASM_WORKER_INTERRUPT_MODEL_RECYCLE"
    )
    vm_probe_ok_phase = parse_source_const_string(wasm_source, "WASM_EXECUTION_PHASE_OK")
    vm_probe_runtime_error_phase = parse_source_const_string(
        wasm_source, "WASM_EXECUTION_PHASE_RUNTIME_ERROR"
    )
    worker_backend_default = parse_source_const_string(
        wasm_source, "WASM_WORKER_BACKEND_UNWIRED"
    )
    worker_backend_vm_probe = const_map.get(
        "WASM_WORKER_BACKEND_VM_PROBE",
        worker_backend_default,
    )
    timeout_default_ms = parse_source_const_u32(wasm_source, "WASM_WORKER_TIMEOUT_DEFAULT_MS")
    timeout_min_ms = parse_source_const_u32(wasm_source, "WASM_WORKER_TIMEOUT_MIN_MS")
    timeout_max_ms = parse_source_const_u32(wasm_source, "WASM_WORKER_TIMEOUT_MAX_MS")
    module_policy_blocker_keys = parse_source_module_policy_blocker_keys(wasm_source)
    worker_operation_prefixes = parse_source_worker_operation_prefixes(wasm_source)
    worker_default_state = next(
        (key for key in worker_state_keys if key == "unwired"),
        worker_state_keys[0] if worker_state_keys else "unwired",
    )

    errors: list[str] = []
    validate_contains_all(docs_source, worker_state_keys, "worker state key", errors)
    validate_contains_all(
        docs_source, worker_lifecycle_phase_keys, "worker lifecycle phase key", errors
    )
    validate_contains_all(docs_source, worker_execute_phases, "worker execute phase key", errors)
    validate_contains_all(docs_source, worker_timeout_phases, "worker timeout phase key", errors)
    validate_contains_all(
        docs_source, module_policy_blocker_keys, "worker module-policy blocker key", errors
    )
    validate_contains_all(
        docs_source, worker_operation_prefixes, "worker operation-id prefix", errors
    )

    if worker_unwired_blocker not in docs_source:
        errors.append(f"docs missing worker unwired blocker key '{worker_unwired_blocker}'")
    if worker_interruption_model not in docs_source:
        errors.append(
            f"docs missing worker interruption model key '{worker_interruption_model}'"
        )
    if str(timeout_default_ms) not in docs_source:
        errors.append(f"docs missing timeout default value '{timeout_default_ms}'")
    if str(timeout_min_ms) not in docs_source:
        errors.append(f"docs missing timeout min value '{timeout_min_ms}'")
    if str(timeout_max_ms) not in docs_source:
        errors.append(f"docs missing timeout max value '{timeout_max_ms}'")
    if worker_backend_default not in docs_source:
        errors.append(
            f"docs missing worker backend default key '{worker_backend_default}'"
        )
    if worker_backend_vm_probe not in docs_source:
        errors.append(
            f"docs missing worker backend vm-probe key '{worker_backend_vm_probe}'"
        )
    if not re.search(r"execution_probe_enabled\s*=\s*false", docs_source):
        errors.append(
            "docs missing default worker info execution_probe_enabled=false shape"
        )
    if not re.search(r"execution_probe_enabled[^\n]*true", docs_source):
        errors.append(
            "docs missing vm-probe worker info execution_probe_enabled=true shape"
        )
    if not re.search(r"execute_supported\s*=\s*false", docs_source):
        errors.append(
            "docs missing default worker info execute_supported=false shape"
        )
    if not re.search(r"execute_supported[^\n]*true", docs_source):
        errors.append(
            "docs missing vm-probe worker info execute_supported=true shape"
        )
    if f'state = "{worker_default_state}"' not in docs_source:
        errors.append(
            "docs missing worker execute-with-operation/default state shape "
            f"'state = \"{worker_default_state}\"'"
        )
    if "wasm-vm-probe" not in docs_source:
        errors.append("docs missing wasm-vm-probe mention for worker execute contract")
    if vm_probe_ok_phase not in docs_source:
        errors.append(f"docs missing vm-probe worker phase '{vm_probe_ok_phase}'")
    if vm_probe_runtime_error_phase not in docs_source:
        errors.append(
            "docs missing vm-probe worker phase "
            f"'{vm_probe_runtime_error_phase}'"
        )

    if errors:
        print("wasm worker docs contract validation failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    summary = {
        "docs": str(docs_path),
        "wasm_source": str(wasm_src_path),
        "worker_state_keys": worker_state_keys,
        "worker_lifecycle_phase_keys": worker_lifecycle_phase_keys,
        "worker_execute_phases": worker_execute_phases,
        "worker_timeout_phases": worker_timeout_phases,
        "worker_unwired_blocker_key": worker_unwired_blocker,
        "worker_interruption_model": worker_interruption_model,
        "worker_backend": {
            "default": worker_backend_default,
            "vm_probe": worker_backend_vm_probe,
        },
        "worker_module_policy_blocker_keys": module_policy_blocker_keys,
        "worker_operation_prefixes": worker_operation_prefixes,
        "worker_default_state": worker_default_state,
        "worker_timeout_ms": {
            "default": timeout_default_ms,
            "min": timeout_min_ms,
            "max": timeout_max_ms,
        },
        "vm_probe_worker_phases_documented": {
            "ok": vm_probe_ok_phase in docs_source,
            "runtime_error": vm_probe_runtime_error_phase in docs_source,
        },
    }

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
