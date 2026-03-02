#!/usr/bin/env python3
"""Validate wasm client-flow docs against source-exported runtime contracts."""

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


def parse_source_worker_session_fields(wasm_source: str) -> list[str]:
    match = re.search(
        r"pub struct WasmWorkerSession \{(.*?)\n\}",
        wasm_source,
        flags=re.DOTALL,
    )
    if not match:
        raise ValueError("unable to parse WasmWorkerSession fields")
    body = match.group(1)
    fields = re.findall(r"^\s*([a-z_]+):", body, flags=re.MULTILINE)
    return ordered_unique(fields)


def parse_source_worker_info_fields(wasm_source: str) -> list[str]:
    match = re.search(
        r"pub struct WasmWorkerInfo \{(.*?)\n\}",
        wasm_source,
        flags=re.DOTALL,
    )
    if not match:
        raise ValueError("unable to parse WasmWorkerInfo fields")
    body = match.group(1)
    fields = re.findall(r"^\s*([a-z_]+):", body, flags=re.MULTILINE)
    return ordered_unique(fields)


def parse_source_exported_functions(wasm_source: str) -> set[str]:
    return set(re.findall(r"^\s*pub fn ([a-zA-Z0-9_]+)\(", wasm_source, flags=re.MULTILINE))


def validate_flow_order(docs_source: str, ordered_tokens: list[str], errors: list[str]) -> None:
    positions: list[int] = []
    for token in ordered_tokens:
        position = docs_source.find(f"`{token}`")
        if position == -1:
            errors.append(f"docs missing flow call token `{token}`")
            return
        positions.append(position)
    for idx in range(1, len(positions)):
        if positions[idx] <= positions[idx - 1]:
            errors.append(
                f"docs flow order mismatch between `{ordered_tokens[idx - 1]}` and `{ordered_tokens[idx]}`"
            )
            return


def validate_docs(
    docs_source: str,
    exported_functions: set[str],
    worker_lifecycle_phase_keys: list[str],
    worker_lifecycle_phase_keys_vm_probe_extra: list[str],
    worker_execute_phase_keys: list[str],
    worker_timeout_phase_keys: list[str],
    worker_unwired_blocker_key: str,
    vm_probe_ok_phase: str,
    vm_probe_runtime_error_phase: str,
    worker_info_fields: list[str],
    worker_session_fields: list[str],
) -> list[str]:
    errors: list[str] = []

    ordered_flow_tokens = [
        "init_wasm_runtime()",
        "wasm_runtime_info()",
        "wasm_worker_info()",
        "wasm_worker_timeout_policy()",
        "wasm_snippet_support(source)",
    ]
    validate_flow_order(docs_source, ordered_flow_tokens, errors)

    required_call_tokens = [
        "wasm_worker_set_timeout(timeout_ms)",
        "check_compile_result(source)",
        "execute(source)",
        "wasm_snippet_blockers(source)",
        "wasm_snippet_import_roots(source)",
        "wasm_worker_start()",
        "wasm_worker_terminate()",
        "wasm_worker_recycle()",
        "wasm_worker_execute(source)",
        "wasm_worker_execute_with_operation(source)",
        "wasm_worker_blockers()",
        "wasm_worker_state_keys()",
        "wasm_worker_lifecycle_phase_keys()",
        "wasm_worker_execute_phase_keys()",
        "wasm_worker_timeout_phase_keys()",
    ]
    for token in required_call_tokens:
        if f"`{token}`" not in docs_source:
            errors.append(f"docs missing client-flow call token `{token}`")

    for token in ordered_flow_tokens + required_call_tokens:
        fn_name = token.split("(", 1)[0]
        if fn_name not in exported_functions:
            errors.append(f"source export missing expected wasm function `{fn_name}`")

    for key in worker_lifecycle_phase_keys:
        if key not in docs_source:
            errors.append(f"docs missing worker lifecycle phase key '{key}'")
    for key in worker_lifecycle_phase_keys_vm_probe_extra:
        if key not in docs_source:
            errors.append(f"docs missing worker vm-probe lifecycle phase key '{key}'")
    for key in worker_execute_phase_keys:
        if key not in docs_source:
            errors.append(f"docs missing worker execute phase key '{key}'")
    for key in worker_timeout_phase_keys:
        if key not in docs_source:
            errors.append(f"docs missing worker timeout phase key '{key}'")

    if worker_unwired_blocker_key not in docs_source:
        errors.append(f"docs missing worker unwired blocker key '{worker_unwired_blocker_key}'")
    if "wasm-vm-probe" not in docs_source:
        errors.append("docs missing wasm-vm-probe mention")
    if vm_probe_ok_phase not in docs_source:
        errors.append(f"docs missing vm-probe phase '{vm_probe_ok_phase}'")
    if vm_probe_runtime_error_phase not in docs_source:
        errors.append(f"docs missing vm-probe phase '{vm_probe_runtime_error_phase}'")

    required_worker_info_fields = [
        field
        for field in worker_info_fields
        if field in {"supported", "backend", "state", "execution_probe_enabled", "execute_supported"}
    ]
    for field in required_worker_info_fields:
        if field not in docs_source:
            errors.append(f"docs missing WasmWorkerInfo field token '{field}'")

    for field in worker_session_fields:
        if field not in docs_source:
            errors.append(f"docs missing WasmWorkerSession telemetry field '{field}'")
    if "info().state" not in docs_source:
        errors.append("docs missing session-local info().state guidance")
    if "session-local" not in docs_source:
        errors.append("docs missing session-local wording for worker session state behavior")

    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--docs",
        default="docs/WASM_CLIENT_INTEGRATION_FLOW.md",
        help="Path to wasm client integration docs",
    )
    parser.add_argument(
        "--wasm-src",
        default="src/wasm/mod.rs",
        help="Path to wasm source",
    )
    parser.add_argument(
        "--out",
        default="perf/wasm_client_flow_summary_latest.json",
        help="Output summary path",
    )
    args = parser.parse_args()

    docs_path = Path(args.docs)
    wasm_src_path = Path(args.wasm_src)
    docs_source = docs_path.read_text(encoding="utf-8")
    wasm_source = wasm_src_path.read_text(encoding="utf-8")

    const_map = parse_source_const_string_map(wasm_source)
    exported_functions = parse_source_exported_functions(wasm_source)
    worker_state_keys = parse_source_enum_keys(wasm_source, "WasmWorkerState", const_map)
    worker_lifecycle_phase_keys_default = parse_source_enum_keys(
        wasm_source, "WasmWorkerLifecyclePhase", const_map
    )
    (
        worker_lifecycle_phase_keys,
        worker_lifecycle_phase_keys_vm_probe_extra,
    ) = parse_source_worker_lifecycle_phase_keys(
        worker_lifecycle_phase_keys_default, const_map
    )
    worker_execute_phase_keys = parse_source_enum_keys(
        wasm_source, "WasmWorkerExecutePhase", const_map
    )
    worker_timeout_phase_keys_default = parse_source_enum_keys(
        wasm_source, "WasmWorkerTimeoutPhase", const_map
    )
    (
        worker_timeout_phase_keys,
        worker_timeout_phase_keys_vm_probe_extra,
    ) = parse_source_worker_timeout_phase_keys(worker_timeout_phase_keys_default, const_map)
    worker_unwired_blocker_key = const_map["WASM_WORKER_BLOCKER_RUNTIME_UNWIRED"]
    vm_probe_ok_phase = const_map["WASM_EXECUTION_PHASE_OK"]
    vm_probe_runtime_error_phase = const_map["WASM_EXECUTION_PHASE_RUNTIME_ERROR"]
    worker_info_fields = parse_source_worker_info_fields(wasm_source)
    worker_session_fields = parse_source_worker_session_fields(wasm_source)

    errors = validate_docs(
        docs_source=docs_source,
        exported_functions=exported_functions,
        worker_lifecycle_phase_keys=worker_lifecycle_phase_keys,
        worker_lifecycle_phase_keys_vm_probe_extra=worker_lifecycle_phase_keys_vm_probe_extra,
        worker_execute_phase_keys=worker_execute_phase_keys,
        worker_timeout_phase_keys=worker_timeout_phase_keys,
        worker_unwired_blocker_key=worker_unwired_blocker_key,
        vm_probe_ok_phase=vm_probe_ok_phase,
        vm_probe_runtime_error_phase=vm_probe_runtime_error_phase,
        worker_info_fields=worker_info_fields,
        worker_session_fields=worker_session_fields,
    )
    if errors:
        print("wasm client flow docs validation failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    summary = {
        "docs": str(docs_path),
        "wasm_source": str(wasm_src_path),
        "ordered_flow_tokens": [
            "init_wasm_runtime()",
            "wasm_runtime_info()",
            "wasm_worker_info()",
            "wasm_worker_timeout_policy()",
            "wasm_snippet_support(source)",
        ],
        "required_call_tokens": [
            "wasm_worker_set_timeout(timeout_ms)",
            "check_compile_result(source)",
            "execute(source)",
            "wasm_snippet_blockers(source)",
            "wasm_snippet_import_roots(source)",
            "wasm_worker_start()",
            "wasm_worker_terminate()",
            "wasm_worker_recycle()",
            "wasm_worker_execute(source)",
            "wasm_worker_execute_with_operation(source)",
            "wasm_worker_blockers()",
            "wasm_worker_state_keys()",
            "wasm_worker_lifecycle_phase_keys()",
            "wasm_worker_execute_phase_keys()",
            "wasm_worker_timeout_phase_keys()",
        ],
        "worker_state_keys": worker_state_keys,
        "worker_lifecycle_phase_keys_default": worker_lifecycle_phase_keys_default,
        "worker_lifecycle_phase_keys_vm_probe_extra": worker_lifecycle_phase_keys_vm_probe_extra,
        "worker_lifecycle_phase_keys_effective": worker_lifecycle_phase_keys,
        "worker_execute_phase_keys": worker_execute_phase_keys,
        "worker_timeout_phase_keys_default": worker_timeout_phase_keys_default,
        "worker_timeout_phase_keys_vm_probe_extra": worker_timeout_phase_keys_vm_probe_extra,
        "worker_timeout_phase_keys_effective": worker_timeout_phase_keys,
        "worker_unwired_blocker_key": worker_unwired_blocker_key,
        "vm_probe_worker_phases": [vm_probe_ok_phase, vm_probe_runtime_error_phase],
        "worker_info_fields": worker_info_fields,
        "worker_session_fields": worker_session_fields,
    }

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
