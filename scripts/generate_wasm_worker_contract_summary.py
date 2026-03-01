#!/usr/bin/env python3
"""Generate a compact wasm worker contract summary from fixture snapshots.

This script is intentionally fixture-driven so contract drift is visible in a
single JSON artifact and can be validated in local branch checks.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


ARRAY_RE_TEMPLATE = r"pub const {name}:[^=]*=\s*&\[(.*?)\];"
FIXTURE_RE_TEMPLATE = r"pub const {name}:[^=]*=\s*&\[(.*?)\];"


def parse_string_array(source: str, const_name: str) -> list[str]:
    pattern = ARRAY_RE_TEMPLATE.format(name=re.escape(const_name))
    match = re.search(pattern, source, flags=re.DOTALL)
    if not match:
        raise ValueError(f"unable to find string array constant: {const_name}")
    body = match.group(1)
    return re.findall(r'"([^"]+)"', body)


def parse_operation_prefixes(source: str, const_name: str) -> list[str]:
    pattern = FIXTURE_RE_TEMPLATE.format(name=re.escape(const_name))
    match = re.search(pattern, source, flags=re.DOTALL)
    if not match:
        raise ValueError(f"unable to find fixture constant: {const_name}")
    body = match.group(1)
    return re.findall(r'expected_operation_prefix:\s*"([^"]+)"', body)


def unique(values: list[str]) -> list[str]:
    return sorted(set(values))


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
        "--out",
        default="perf/wasm_worker_contract_summary_latest.json",
        help="Output summary JSON path",
    )
    args = parser.parse_args()

    fixture_path = Path(args.fixture)
    source = fixture_path.read_text(encoding="utf-8")

    state_keys = parse_string_array(source, "WASM_WORKER_STATE_KEYS")
    lifecycle_phase_keys = parse_string_array(source, "WASM_WORKER_LIFECYCLE_PHASE_KEYS")
    execute_phase_keys = parse_string_array(source, "WASM_WORKER_EXECUTE_PHASE_KEYS")
    timeout_phase_keys = parse_string_array(source, "WASM_WORKER_TIMEOUT_PHASE_KEYS")

    lifecycle_operation_prefixes = parse_operation_prefixes(
        source, "WASM_WORKER_LIFECYCLE_FIXTURES"
    )
    execute_operation_prefixes = parse_operation_prefixes(
        source, "WASM_WORKER_EXECUTE_FIXTURES"
    )
    timeout_operation_prefixes = parse_operation_prefixes(
        source, "WASM_WORKER_TIMEOUT_FIXTURES"
    )

    errors: list[str] = []
    validate_non_empty("WASM_WORKER_STATE_KEYS", state_keys, errors)
    validate_non_empty("WASM_WORKER_LIFECYCLE_PHASE_KEYS", lifecycle_phase_keys, errors)
    validate_non_empty("WASM_WORKER_EXECUTE_PHASE_KEYS", execute_phase_keys, errors)
    validate_non_empty("WASM_WORKER_TIMEOUT_PHASE_KEYS", timeout_phase_keys, errors)
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

    if errors:
        print("wasm worker contract summary validation failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    summary = {
        "fixture": str(fixture_path),
        "worker_state_keys": state_keys,
        "lifecycle_phase_keys": lifecycle_phase_keys,
        "execute_phase_keys": execute_phase_keys,
        "timeout_phase_keys": timeout_phase_keys,
        "operation_prefixes": {
            "lifecycle": unique(lifecycle_operation_prefixes),
            "execute": unique(execute_operation_prefixes),
            "timeout": unique(timeout_operation_prefixes),
        },
        "counts": {
            "worker_state_keys": len(state_keys),
            "lifecycle_phase_keys": len(lifecycle_phase_keys),
            "execute_phase_keys": len(execute_phase_keys),
            "timeout_phase_keys": len(timeout_phase_keys),
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

