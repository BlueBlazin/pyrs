#!/usr/bin/env python3
"""Generate and validate wasm worker contract summary with source parity checks."""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path

CONST_RE_TEMPLATE = r"pub const {name}:[^=]*=\s*&\[(.*?)\];"


@dataclass
class WorkerExecuteFixtureRow:
    phase: str
    blocker_key: str | None
    vm_probe_phase: str | None
    vm_probe_blocker_key: str | None
    has_vm_probe_blocker_override: bool


def parse_const_body(source: str, const_name: str) -> str:
    pattern = CONST_RE_TEMPLATE.format(name=re.escape(const_name))
    match = re.search(pattern, source, flags=re.DOTALL)
    if not match:
        raise ValueError(f"unable to find fixture constant: {const_name}")
    return match.group(1)


def parse_string_array(source: str, const_name: str) -> list[str]:
    body = parse_const_body(source, const_name)
    return re.findall(r'"([^"]+)"', body)


def parse_operation_prefixes(source: str, const_name: str) -> list[str]:
    body = parse_const_body(source, const_name)
    return re.findall(r'expected_operation_prefix:\s*"([^"]+)"', body)


def parse_expected_phases(source: str, const_name: str) -> list[str]:
    body = parse_const_body(source, const_name)
    return re.findall(r'expected_phase:\s*"([^"]+)"', body)


def parse_required_blocker_keys(source: str, const_name: str) -> list[str]:
    body = parse_const_body(source, const_name)
    return re.findall(r'expected_blocker_key:\s*"([^"]+)"', body)


def parse_optional_blocker_keys(source: str, const_name: str) -> list[str | None]:
    body = parse_const_body(source, const_name)
    results: list[str | None] = []
    pattern = re.compile(r'expected_blocker_key:\s*(Some\("([^"]+)"\)|None)')
    for match in pattern.finditer(body):
        if match.group(2) is not None:
            results.append(match.group(2))
        else:
            results.append(None)
    return results


def parse_required_string_field(body: str, field: str) -> str:
    match = re.search(rf"{re.escape(field)}:\s*\"([^\"]+)\"", body)
    if not match:
        raise ValueError(f"missing required field '{field}' in worker execute fixture")
    return match.group(1)


def parse_optional_string_field(body: str, field: str) -> str | None:
    some_match = re.search(rf"{re.escape(field)}:\s*Some\(\"([^\"]+)\"\)", body)
    if some_match:
        return some_match.group(1)
    none_match = re.search(rf"{re.escape(field)}:\s*None", body)
    if none_match:
        return None
    raise ValueError(f"missing optional field '{field}' with Some(...) or None")


def parse_optional_optional_string_field(body: str, field: str) -> tuple[bool, str | None]:
    some_some_match = re.search(
        rf"{re.escape(field)}:\s*Some\(\s*Some\(\"([^\"]+)\"\)\s*\)", body
    )
    if some_some_match:
        return True, some_some_match.group(1)

    some_none_match = re.search(rf"{re.escape(field)}:\s*Some\(\s*None\s*\)", body)
    if some_none_match:
        return True, None

    none_match = re.search(rf"{re.escape(field)}:\s*None", body)
    if none_match:
        return False, None

    raise ValueError(
        f"missing optional field '{field}' with Some(Some(...)) / Some(None) / None"
    )


def parse_worker_execute_fixture_rows(source: str) -> list[WorkerExecuteFixtureRow]:
    body = parse_const_body(source, "WASM_WORKER_EXECUTE_FIXTURES")
    pattern = re.compile(r"WasmWorkerExecuteFixture\s*\{(.*?)\n\s*\},", re.DOTALL)
    rows: list[WorkerExecuteFixtureRow] = []
    for match in pattern.finditer(body):
        row_body = match.group(1)
        phase = parse_required_string_field(row_body, "expected_phase")
        blocker_key = parse_optional_string_field(row_body, "expected_blocker_key")
        vm_probe_phase = parse_optional_string_field(row_body, "expected_vm_probe_phase")
        (
            has_vm_probe_blocker_override,
            vm_probe_blocker_key,
        ) = parse_optional_optional_string_field(row_body, "expected_vm_probe_blocker_key")
        rows.append(
            WorkerExecuteFixtureRow(
                phase=phase,
                blocker_key=blocker_key,
                vm_probe_phase=vm_probe_phase,
                vm_probe_blocker_key=vm_probe_blocker_key,
                has_vm_probe_blocker_override=has_vm_probe_blocker_override,
            )
        )
    return rows


def effective_worker_execute_expectation(
    row: WorkerExecuteFixtureRow, vm_probe_enabled: bool
) -> tuple[str, str | None]:
    phase = row.phase
    blocker_key = row.blocker_key
    if vm_probe_enabled:
        if row.vm_probe_phase is not None:
            phase = row.vm_probe_phase
        if row.has_vm_probe_blocker_override:
            blocker_key = row.vm_probe_blocker_key
    return phase, blocker_key


def unique(values: list[str]) -> list[str]:
    return sorted(set(values))


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
    arm_pattern = re.compile(
        rf"{re.escape(enum_name)}::[A-Za-z]+\s*=>\s*([^,]+),"
    )
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


def parse_source_worker_execute_phase_keys(
    wasm_source: str, const_map: dict[str, str], vm_probe_enabled: bool
) -> list[str]:
    keys = parse_source_enum_keys(wasm_source, "WasmWorkerExecutePhase", const_map)
    if vm_probe_enabled:
        keys.append(const_map["WASM_EXECUTION_PHASE_OK"])
        keys.append(const_map["WASM_EXECUTION_PHASE_RUNTIME_ERROR"])
    return keys


def parse_source_lifecycle_actions(wasm_source: str) -> list[str]:
    block_match = re.search(
        r"fn worker_unwired_result\(.*?\) -> WasmWorkerLifecycleResult \{(.*?)let blocker_key",
        wasm_source,
        flags=re.DOTALL,
    )
    if not block_match:
        raise ValueError("unable to locate worker_unwired_result action mapping")
    block = block_match.group(1)
    return unique(re.findall(r'WasmWorkerLifecyclePhase::[A-Za-z]+\s*=>\s*"([^"]+)"', block))


def parse_source_operation_actions(wasm_source: str) -> list[str]:
    return unique(re.findall(r'next_worker_operation_id\("([^"]+)"\)', wasm_source))


def parse_source_worker_blocker_key(wasm_source: str) -> str:
    match = re.search(
        r'const\s+WASM_WORKER_BLOCKER_RUNTIME_UNWIRED:\s*&str\s*=\s*"([^"]+)";',
        wasm_source,
    )
    if not match:
        raise ValueError("unable to parse WASM_WORKER_BLOCKER_RUNTIME_UNWIRED from wasm source")
    return match.group(1)


def parse_source_module_policy_blocker_keys(wasm_source: str) -> list[str]:
    match = re.search(
        r"const\s+WASM_MODULE_BLOCKER_POLICY:[^=]*=\s*\[(.*?)\];",
        wasm_source,
        flags=re.DOTALL,
    )
    if not match:
        raise ValueError("unable to parse WASM_MODULE_BLOCKER_POLICY from wasm source")
    body = match.group(1)
    rows = re.findall(r'\(\s*"[^"]+"\s*,\s*"([^"]+)"\s*\)', body)
    keys: list[str] = []
    seen: set[str] = set()
    for key in rows:
        if key not in seen:
            seen.add(key)
            keys.append(key)
    return keys


def validate_non_empty(name: str, values: list[str], errors: list[str]) -> None:
    if not values:
        errors.append(f"{name} must not be empty")


def validate_unique(name: str, values: list[str], errors: list[str]) -> None:
    if len(values) != len(set(values)):
        errors.append(f"{name} contains duplicate entries")


def validate_prefix_shape(name: str, prefixes: list[str], errors: list[str]) -> None:
    for prefix in prefixes:
        if not prefix.startswith("worker_") or not prefix.endswith("_"):
            errors.append(f"{name} has invalid prefix shape: {prefix}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--fixture",
        default="tests/fixtures/wasm_worker_contract.rs",
        help="Path to wasm worker fixture file",
    )
    parser.add_argument(
        "--wasm-src",
        default="src/wasm/mod.rs",
        help="Path to wasm source file for source/fixture parity checks",
    )
    parser.add_argument(
        "--out",
        default="perf/wasm_worker_contract_summary_latest.json",
        help="Output summary JSON path",
    )
    parser.add_argument(
        "--vm-probe",
        action="store_true",
        help="Validate worker execute contract for wasm-vm-probe mode",
    )
    args = parser.parse_args()

    fixture_path = Path(args.fixture)
    fixture_source = fixture_path.read_text(encoding="utf-8")
    wasm_source_path = Path(args.wasm_src)
    wasm_source = wasm_source_path.read_text(encoding="utf-8")

    state_keys = parse_string_array(fixture_source, "WASM_WORKER_STATE_KEYS")
    lifecycle_phase_keys = parse_string_array(
        fixture_source, "WASM_WORKER_LIFECYCLE_PHASE_KEYS"
    )
    execute_phase_keys = parse_string_array(fixture_source, "WASM_WORKER_EXECUTE_PHASE_KEYS")
    timeout_phase_keys = parse_string_array(fixture_source, "WASM_WORKER_TIMEOUT_PHASE_KEYS")
    worker_blocker_keys = parse_string_array(
        fixture_source, "WASM_WORKER_BLOCKER_KEYS"
    )

    lifecycle_operation_prefixes = parse_operation_prefixes(
        fixture_source, "WASM_WORKER_LIFECYCLE_FIXTURES"
    )
    execute_operation_prefixes = parse_operation_prefixes(
        fixture_source, "WASM_WORKER_EXECUTE_FIXTURES"
    )
    timeout_operation_prefixes = parse_operation_prefixes(
        fixture_source, "WASM_WORKER_TIMEOUT_FIXTURES"
    )

    lifecycle_blocker_keys = parse_required_blocker_keys(
        fixture_source, "WASM_WORKER_LIFECYCLE_FIXTURES"
    )
    execute_rows = parse_worker_execute_fixture_rows(fixture_source)
    execute_fixture_phases = [
        effective_worker_execute_expectation(row, args.vm_probe)[0] for row in execute_rows
    ]
    execute_blocker_keys = [
        effective_worker_execute_expectation(row, args.vm_probe)[1] for row in execute_rows
    ]
    timeout_fixture_phases = parse_expected_phases(
        fixture_source, "WASM_WORKER_TIMEOUT_FIXTURES"
    )
    timeout_blocker_keys = parse_optional_blocker_keys(
        fixture_source, "WASM_WORKER_TIMEOUT_FIXTURES"
    )

    source_const_map = parse_source_const_string_map(wasm_source)
    source_state_keys = parse_source_enum_keys(
        wasm_source, "WasmWorkerState", source_const_map
    )
    source_lifecycle_phase_keys = parse_source_enum_keys(
        wasm_source, "WasmWorkerLifecyclePhase", source_const_map
    )
    source_execute_phase_keys = parse_source_worker_execute_phase_keys(
        wasm_source, source_const_map, args.vm_probe
    )
    source_timeout_phase_keys = parse_source_enum_keys(
        wasm_source, "WasmWorkerTimeoutPhase", source_const_map
    )
    source_lifecycle_actions = parse_source_lifecycle_actions(wasm_source)
    source_operation_actions = parse_source_operation_actions(wasm_source)
    source_worker_blocker_key = parse_source_worker_blocker_key(wasm_source)
    source_module_policy_blocker_keys = parse_source_module_policy_blocker_keys(wasm_source)
    source_expected_worker_blocker_keys = [
        source_worker_blocker_key,
        *source_module_policy_blocker_keys,
    ]
    allowed_execute_unsupported_blocker_keys = sorted(
        {source_worker_blocker_key, *source_module_policy_blocker_keys}
    )

    expected_lifecycle_prefixes = unique(
        [f"worker_{action}_" for action in source_lifecycle_actions]
    )
    expected_execute_prefixes = unique(
        [f"worker_{action}_" for action in source_operation_actions if action == "execute"]
    )
    expected_timeout_prefixes = unique(
        [
            f"worker_{action}_"
            for action in source_operation_actions
            if action == "set_timeout"
        ]
    )
    execute_phase_keys_effective = list(execute_phase_keys)
    if args.vm_probe:
        execute_phase_keys_effective.extend(
            [
                source_const_map["WASM_EXECUTION_PHASE_OK"],
                source_const_map["WASM_EXECUTION_PHASE_RUNTIME_ERROR"],
            ]
        )

    errors: list[str] = []
    validate_non_empty("WASM_WORKER_STATE_KEYS", state_keys, errors)
    validate_non_empty("WASM_WORKER_LIFECYCLE_PHASE_KEYS", lifecycle_phase_keys, errors)
    validate_non_empty("WASM_WORKER_EXECUTE_PHASE_KEYS", execute_phase_keys, errors)
    validate_non_empty("WASM_WORKER_TIMEOUT_PHASE_KEYS", timeout_phase_keys, errors)
    validate_non_empty("WASM_WORKER_BLOCKER_KEYS", worker_blocker_keys, errors)
    validate_non_empty(
        "WASM_WORKER_LIFECYCLE_FIXTURES.expected_operation_prefix",
        lifecycle_operation_prefixes,
        errors,
    )
    validate_non_empty(
        "WASM_WORKER_EXECUTE_FIXTURES.expected_operation_prefix",
        execute_operation_prefixes,
        errors,
    )
    validate_non_empty(
        "WASM_WORKER_TIMEOUT_FIXTURES.expected_operation_prefix",
        timeout_operation_prefixes,
        errors,
    )

    validate_unique("WASM_WORKER_STATE_KEYS", state_keys, errors)
    validate_unique("WASM_WORKER_LIFECYCLE_PHASE_KEYS", lifecycle_phase_keys, errors)
    validate_unique("WASM_WORKER_EXECUTE_PHASE_KEYS", execute_phase_keys, errors)
    validate_unique("WASM_WORKER_TIMEOUT_PHASE_KEYS", timeout_phase_keys, errors)
    validate_unique("WASM_WORKER_BLOCKER_KEYS", worker_blocker_keys, errors)

    validate_prefix_shape(
        "WASM_WORKER_LIFECYCLE_FIXTURES.expected_operation_prefix",
        lifecycle_operation_prefixes,
        errors,
    )
    validate_prefix_shape(
        "WASM_WORKER_EXECUTE_FIXTURES.expected_operation_prefix",
        execute_operation_prefixes,
        errors,
    )
    validate_prefix_shape(
        "WASM_WORKER_TIMEOUT_FIXTURES.expected_operation_prefix",
        timeout_operation_prefixes,
        errors,
    )

    if set(state_keys) != set(source_state_keys):
        errors.append(
            f"worker state key set mismatch fixtures={unique(state_keys)} source={source_state_keys}"
        )
    if state_keys != source_state_keys:
        errors.append(
            "worker state key order mismatch "
            f"fixtures={state_keys} source={source_state_keys}"
        )
    if set(lifecycle_phase_keys) != set(source_lifecycle_phase_keys):
        errors.append(
            "worker lifecycle phase key set mismatch "
            f"fixtures={unique(lifecycle_phase_keys)} source={source_lifecycle_phase_keys}"
        )
    if lifecycle_phase_keys != source_lifecycle_phase_keys:
        errors.append(
            "worker lifecycle phase key order mismatch "
            f"fixtures={lifecycle_phase_keys} source={source_lifecycle_phase_keys}"
        )
    if set(execute_phase_keys_effective) != set(source_execute_phase_keys):
        errors.append(
            "worker execute phase key set mismatch "
            f"fixtures={unique(execute_phase_keys_effective)} source={source_execute_phase_keys}"
        )
    if execute_phase_keys_effective != source_execute_phase_keys:
        errors.append(
            "worker execute phase key order mismatch "
            f"fixtures={execute_phase_keys_effective} source={source_execute_phase_keys}"
        )
    if set(timeout_phase_keys) != set(source_timeout_phase_keys):
        errors.append(
            f"worker timeout phase key set mismatch fixtures={unique(timeout_phase_keys)} source={source_timeout_phase_keys}"
        )
    if timeout_phase_keys != source_timeout_phase_keys:
        errors.append(
            "worker timeout phase key order mismatch "
            f"fixtures={timeout_phase_keys} source={source_timeout_phase_keys}"
        )
    if set(worker_blocker_keys) != set(source_expected_worker_blocker_keys):
        errors.append(
            "worker blocker key set mismatch "
            f"fixtures={unique(worker_blocker_keys)} source={source_expected_worker_blocker_keys}"
        )
    if worker_blocker_keys != source_expected_worker_blocker_keys:
        errors.append(
            "worker blocker key order mismatch "
            f"fixtures={worker_blocker_keys} source={source_expected_worker_blocker_keys}"
        )

    if unique(lifecycle_operation_prefixes) != expected_lifecycle_prefixes:
        errors.append(
            "worker lifecycle operation prefix set mismatch "
            f"fixtures={unique(lifecycle_operation_prefixes)} source={expected_lifecycle_prefixes}"
        )
    if unique(execute_operation_prefixes) != expected_execute_prefixes:
        errors.append(
            "worker execute operation prefix set mismatch "
            f"fixtures={unique(execute_operation_prefixes)} source={expected_execute_prefixes}"
        )
    if unique(timeout_operation_prefixes) != expected_timeout_prefixes:
        errors.append(
            "worker timeout operation prefix set mismatch "
            f"fixtures={unique(timeout_operation_prefixes)} source={expected_timeout_prefixes}"
        )

    if any(key != source_worker_blocker_key for key in lifecycle_blocker_keys):
        errors.append(
            "worker lifecycle fixture blocker keys must all equal source worker blocker key "
            f"'{source_worker_blocker_key}'"
        )

    if len(execute_fixture_phases) != len(execute_blocker_keys):
        errors.append("worker execute fixture phase/blocker row count mismatch")
    else:
        for phase, blocker_key in zip(execute_fixture_phases, execute_blocker_keys, strict=True):
            if phase == "unsupported_worker_execution":
                if blocker_key not in allowed_execute_unsupported_blocker_keys:
                    errors.append(
                        "worker execute unsupported phase must use an allowed blocker key "
                        f"{allowed_execute_unsupported_blocker_keys}"
                    )
            elif blocker_key is not None:
                errors.append(
                    f"worker execute phase '{phase}' must not set expected_blocker_key"
                )
    execute_fixture_phase_set = unique(execute_fixture_phases)
    if set(execute_fixture_phase_set) != set(source_execute_phase_keys):
        errors.append(
            "worker execute fixture phase set mismatch "
            f"fixtures={execute_fixture_phase_set} source={source_execute_phase_keys}"
        )

    if len(timeout_fixture_phases) != len(timeout_blocker_keys):
        errors.append("worker timeout fixture phase/blocker row count mismatch")
    else:
        for phase, blocker_key in zip(timeout_fixture_phases, timeout_blocker_keys, strict=True):
            if phase == "unsupported_worker_timeout_enforcement":
                if blocker_key != source_worker_blocker_key:
                    errors.append(
                        "worker timeout unsupported phase must use source blocker key "
                        f"'{source_worker_blocker_key}'"
                    )
            elif phase == "invalid_worker_timeout" and blocker_key is not None:
                errors.append("worker timeout invalid phase must not set expected_blocker_key")

    if errors:
        print("wasm worker contract summary validation failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    summary = {
        "fixture": str(fixture_path),
        "wasm_source": str(wasm_source_path),
        "mode": "vm_probe" if args.vm_probe else "default",
        "worker_state_keys": state_keys,
        "lifecycle_phase_keys": lifecycle_phase_keys,
        "execute_phase_keys": execute_phase_keys,
        "execute_phase_keys_effective": execute_phase_keys_effective,
        "timeout_phase_keys": timeout_phase_keys,
        "worker_blocker_keys": worker_blocker_keys,
        "execute_fixture_phases": execute_fixture_phase_set,
        "source_key_sets": {
            "state": source_state_keys,
            "lifecycle_phase": source_lifecycle_phase_keys,
            "execute_phase": source_execute_phase_keys,
            "timeout_phase": source_timeout_phase_keys,
        },
        "source_worker_blocker_key": source_worker_blocker_key,
        "source_module_policy_blocker_keys": source_module_policy_blocker_keys,
        "source_expected_worker_blocker_keys": source_expected_worker_blocker_keys,
        "allowed_execute_unsupported_blocker_keys": allowed_execute_unsupported_blocker_keys,
        "operation_prefixes": {
            "lifecycle": unique(lifecycle_operation_prefixes),
            "execute": unique(execute_operation_prefixes),
            "timeout": unique(timeout_operation_prefixes),
            "source_expected": {
                "lifecycle": expected_lifecycle_prefixes,
                "execute": expected_execute_prefixes,
                "timeout": expected_timeout_prefixes,
            },
        },
        "counts": {
            "worker_state_keys": len(state_keys),
            "lifecycle_phase_keys": len(lifecycle_phase_keys),
            "execute_phase_keys": len(execute_phase_keys_effective),
            "timeout_phase_keys": len(timeout_phase_keys),
            "worker_blocker_keys": len(worker_blocker_keys),
            "lifecycle_prefix_entries": len(lifecycle_operation_prefixes),
            "execute_prefix_entries": len(execute_operation_prefixes),
            "timeout_prefix_entries": len(timeout_operation_prefixes),
        },
    }

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
