#!/usr/bin/env python3
"""Validate wasm session contract fixture expectations and emit summary artifact."""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass
class TopLevelFixtureRow:
    name: str
    expected_execute_phase: str
    expected_execute_blocker_key: str | None
    expected_vm_probe_execute_phase: str | None
    expected_vm_probe_execute_blocker_key: str | None
    has_vm_probe_execute_blocker_override: bool
    expected_support_phase: str


@dataclass
class WorkerFixtureRow:
    name: str
    expected_phase: str
    expected_blocker_key: str | None
    expected_vm_probe_phase: str | None
    expected_vm_probe_blocker_key: str | None
    has_vm_probe_blocker_override: bool
    expect_error: bool
    expected_vm_probe_expect_error: bool | None
    expected_success: bool
    expected_vm_probe_success: bool | None
    expect_line_column: bool
    expected_vm_probe_expect_line_column: bool | None


@dataclass
class WorkerLifecycleFixtureRow:
    name: str
    action: str
    expected_state: str
    expected_vm_probe_state: str | None


@dataclass
class WorkerTimeoutFixtureRow:
    name: str
    timeout_ms: int
    expected_phase: str
    expected_state: str
    expected_success: bool
    expected_blocker_key: str | None
    expected_vm_probe_phase: str | None
    expected_vm_probe_state: str | None
    expected_vm_probe_success: bool | None
    expected_vm_probe_blocker_key: str | None
    has_vm_probe_blocker_override: bool


def parse_const_body(source: str, const_name: str) -> str:
    match = re.search(
        rf"pub const {re.escape(const_name)}:[^=]*=\s*&\[(.*?)\];",
        source,
        flags=re.DOTALL,
    )
    if not match:
        raise ValueError(f"missing fixture constant '{const_name}'")
    return match.group(1)


def parse_required_string(body: str, field: str) -> str:
    match = re.search(rf"{re.escape(field)}:\s*\"([^\"]+)\"", body)
    if not match:
        raise ValueError(f"missing required string field '{field}'")
    return match.group(1)


def parse_optional_string(body: str, field: str) -> str | None:
    some_match = re.search(rf"{re.escape(field)}:\s*Some\(\"([^\"]+)\"\)", body)
    if some_match:
        return some_match.group(1)
    none_match = re.search(rf"{re.escape(field)}:\s*None", body)
    if none_match:
        return None
    raise ValueError(f"missing optional string field '{field}' with Some(...) or None")


def parse_optional_optional_string(body: str, field: str) -> tuple[bool, str | None]:
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
        f"missing optional optional string field '{field}' with Some(Some(...)) / Some(None) / None"
    )


def parse_required_bool(body: str, field: str) -> bool:
    if re.search(rf"{re.escape(field)}:\s*true", body):
        return True
    if re.search(rf"{re.escape(field)}:\s*false", body):
        return False
    raise ValueError(f"missing required bool field '{field}'")


def parse_optional_bool(body: str, field: str) -> bool | None:
    if re.search(rf"{re.escape(field)}:\s*Some\(\s*true\s*\)", body):
        return True
    if re.search(rf"{re.escape(field)}:\s*Some\(\s*false\s*\)", body):
        return False
    if re.search(rf"{re.escape(field)}:\s*None", body):
        return None
    raise ValueError(f"missing optional bool field '{field}' with Some(bool) or None")


def parse_required_u32(body: str, field: str) -> int:
    match = re.search(rf"{re.escape(field)}:\s*([0-9_]+)", body)
    if not match:
        raise ValueError(f"missing required u32 field '{field}'")
    return int(match.group(1).replace("_", ""))


def parse_top_level_rows(source: str) -> list[TopLevelFixtureRow]:
    body = parse_const_body(source, "WASM_CONTRACT_SNIPPET_FIXTURES")
    pattern = re.compile(r"WasmContractSnippetFixture\s*\{(.*?)\n\s*\},", re.DOTALL)
    rows: list[TopLevelFixtureRow] = []
    for match in pattern.finditer(body):
        row = match.group(1)
        (
            has_vm_probe_execute_blocker_override,
            expected_vm_probe_execute_blocker_key,
        ) = parse_optional_optional_string(row, "expected_vm_probe_execute_blocker_key")
        rows.append(
            TopLevelFixtureRow(
                name=parse_required_string(row, "name"),
                expected_execute_phase=parse_required_string(row, "expected_execute_phase"),
                expected_execute_blocker_key=parse_optional_string(
                    row, "expected_execute_blocker_key"
                ),
                expected_vm_probe_execute_phase=parse_optional_string(
                    row, "expected_vm_probe_execute_phase"
                ),
                expected_vm_probe_execute_blocker_key=expected_vm_probe_execute_blocker_key,
                has_vm_probe_execute_blocker_override=has_vm_probe_execute_blocker_override,
                expected_support_phase=parse_required_string(row, "expected_support_phase"),
            )
        )
    return rows


def parse_worker_rows(source: str) -> list[WorkerFixtureRow]:
    body = parse_const_body(source, "WASM_WORKER_EXECUTE_FIXTURES")
    pattern = re.compile(r"WasmWorkerExecuteFixture\s*\{(.*?)\n\s*\},?", re.DOTALL)
    rows: list[WorkerFixtureRow] = []
    for match in pattern.finditer(body):
        row = match.group(1)
        (
            has_vm_probe_blocker_override,
            expected_vm_probe_blocker_key,
        ) = parse_optional_optional_string(row, "expected_vm_probe_blocker_key")
        rows.append(
            WorkerFixtureRow(
                name=parse_required_string(row, "name"),
                expected_phase=parse_required_string(row, "expected_phase"),
                expected_blocker_key=parse_optional_string(row, "expected_blocker_key"),
                expected_vm_probe_phase=parse_optional_string(row, "expected_vm_probe_phase"),
                expected_vm_probe_blocker_key=expected_vm_probe_blocker_key,
                has_vm_probe_blocker_override=has_vm_probe_blocker_override,
                expect_error=parse_required_bool(row, "expect_error"),
                expected_vm_probe_expect_error=parse_optional_bool(
                    row, "expected_vm_probe_expect_error"
                ),
                expected_success=parse_required_bool(row, "expected_success"),
                expected_vm_probe_success=parse_optional_bool(row, "expected_vm_probe_success"),
                expect_line_column=parse_required_bool(row, "expect_line_column"),
                expected_vm_probe_expect_line_column=parse_optional_bool(
                    row, "expected_vm_probe_expect_line_column"
                ),
            )
        )
    return rows


def parse_worker_lifecycle_rows(source: str) -> list[WorkerLifecycleFixtureRow]:
    body = parse_const_body(source, "WASM_WORKER_LIFECYCLE_FIXTURES")
    pattern = re.compile(r"WasmWorkerLifecycleFixture\s*\{(.*?)\n\s*\},?", re.DOTALL)
    rows: list[WorkerLifecycleFixtureRow] = []
    for match in pattern.finditer(body):
        row = match.group(1)
        rows.append(
            WorkerLifecycleFixtureRow(
                name=parse_required_string(row, "name"),
                action=parse_required_string(row, "action"),
                expected_state=parse_required_string(row, "expected_state"),
                expected_vm_probe_state=parse_optional_string(row, "expected_vm_probe_state"),
            )
        )
    return rows


def parse_worker_timeout_rows(source: str) -> list[WorkerTimeoutFixtureRow]:
    body = parse_const_body(source, "WASM_WORKER_TIMEOUT_FIXTURES")
    pattern = re.compile(r"WasmWorkerTimeoutFixture\s*\{(.*?)\n\s*\},?", re.DOTALL)
    rows: list[WorkerTimeoutFixtureRow] = []
    for match in pattern.finditer(body):
        row = match.group(1)
        (
            has_vm_probe_blocker_override,
            expected_vm_probe_blocker_key,
        ) = parse_optional_optional_string(row, "expected_vm_probe_blocker_key")
        rows.append(
            WorkerTimeoutFixtureRow(
                name=parse_required_string(row, "name"),
                timeout_ms=parse_required_u32(row, "timeout_ms"),
                expected_phase=parse_required_string(row, "expected_phase"),
                expected_state=parse_required_string(row, "expected_state"),
                expected_success=parse_required_bool(row, "expected_success"),
                expected_blocker_key=parse_optional_string(row, "expected_blocker_key"),
                expected_vm_probe_phase=parse_optional_string(row, "expected_vm_probe_phase"),
                expected_vm_probe_state=parse_optional_string(row, "expected_vm_probe_state"),
                expected_vm_probe_success=parse_optional_bool(row, "expected_vm_probe_success"),
                expected_vm_probe_blocker_key=expected_vm_probe_blocker_key,
                has_vm_probe_blocker_override=has_vm_probe_blocker_override,
            )
        )
    return rows


def find_row(rows: list[WorkerFixtureRow], name: str) -> WorkerFixtureRow | None:
    for row in rows:
        if row.name == name:
            return row
    return None


def find_lifecycle_row(
    rows: list[WorkerLifecycleFixtureRow], action: str
) -> WorkerLifecycleFixtureRow | None:
    for row in rows:
        if row.action == action:
            return row
    return None


def find_timeout_rows(
    rows: list[WorkerTimeoutFixtureRow], *, expected_phase: str
) -> list[WorkerTimeoutFixtureRow]:
    return [row for row in rows if row.expected_phase == expected_phase]


def validate(
    top_level_rows: list[TopLevelFixtureRow],
    worker_rows: list[WorkerFixtureRow],
    worker_lifecycle_rows: list[WorkerLifecycleFixtureRow],
    worker_timeout_rows: list[WorkerTimeoutFixtureRow],
) -> list[str]:
    errors: list[str] = []
    if not top_level_rows:
        errors.append("top-level fixture rows are empty")
    if not worker_rows:
        errors.append("worker execute fixture rows are empty")
    if not worker_lifecycle_rows:
        errors.append("worker lifecycle fixture rows are empty")
    if not worker_timeout_rows:
        errors.append("worker timeout fixture rows are empty")
    if errors:
        return errors

    top_ok_rows = [
        row
        for row in top_level_rows
        if row.expected_support_phase == "supported"
        and row.expected_execute_phase == "unsupported_execution"
        and row.expected_execute_blocker_key == "execution_backend_unwired"
        and row.expected_vm_probe_execute_phase == "ok"
        and row.has_vm_probe_execute_blocker_override
        and row.expected_vm_probe_execute_blocker_key is None
    ]
    if not top_ok_rows:
        errors.append(
            "missing top-level supported baseline fixture mapping default unsupported -> vm-probe ok"
        )

    top_runtime_error_rows = [
        row
        for row in top_level_rows
        if row.expected_support_phase == "supported"
        and row.expected_execute_phase == "unsupported_execution"
        and row.expected_execute_blocker_key == "execution_backend_unwired"
        and row.expected_vm_probe_execute_phase == "runtime_error"
        and row.has_vm_probe_execute_blocker_override
        and row.expected_vm_probe_execute_blocker_key is None
    ]
    if not top_runtime_error_rows:
        errors.append(
            "missing top-level supported runtime-error fixture mapping default unsupported -> vm-probe runtime_error"
        )

    worker_ok = find_row(worker_rows, "worker_execute_unwired")
    if worker_ok is None:
        errors.append("missing worker fixture row 'worker_execute_unwired'")
    else:
        if worker_ok.expected_phase != "unsupported_worker_execution":
            errors.append("worker_execute_unwired expected_phase must be unsupported_worker_execution")
        if worker_ok.expected_blocker_key != "worker_runtime_unwired":
            errors.append("worker_execute_unwired expected_blocker_key must be worker_runtime_unwired")
        if worker_ok.expected_vm_probe_phase != "ok":
            errors.append("worker_execute_unwired expected_vm_probe_phase must be ok")
        if not worker_ok.has_vm_probe_blocker_override or worker_ok.expected_vm_probe_blocker_key is not None:
            errors.append(
                "worker_execute_unwired expected_vm_probe_blocker_key must be Some(None)"
            )
        if worker_ok.expected_success:
            errors.append("worker_execute_unwired expected_success must be false")
        if worker_ok.expected_vm_probe_success is not True:
            errors.append("worker_execute_unwired expected_vm_probe_success must be Some(true)")
        if not worker_ok.expect_error:
            errors.append("worker_execute_unwired expect_error must be true")
        if worker_ok.expected_vm_probe_expect_error is not False:
            errors.append("worker_execute_unwired expected_vm_probe_expect_error must be Some(false)")
        if worker_ok.expect_line_column:
            errors.append("worker_execute_unwired expect_line_column must be false")
        if worker_ok.expected_vm_probe_expect_line_column is not False:
            errors.append(
                "worker_execute_unwired expected_vm_probe_expect_line_column must be Some(false)"
            )

    worker_runtime = find_row(worker_rows, "worker_execute_runtime_error_zero_division")
    if worker_runtime is None:
        errors.append("missing worker fixture row 'worker_execute_runtime_error_zero_division'")
    else:
        if worker_runtime.expected_phase != "unsupported_worker_execution":
            errors.append(
                "worker_execute_runtime_error_zero_division expected_phase must be unsupported_worker_execution"
            )
        if worker_runtime.expected_blocker_key != "worker_runtime_unwired":
            errors.append(
                "worker_execute_runtime_error_zero_division expected_blocker_key must be worker_runtime_unwired"
            )
        if worker_runtime.expected_vm_probe_phase != "runtime_error":
            errors.append(
                "worker_execute_runtime_error_zero_division expected_vm_probe_phase must be runtime_error"
            )
        if (
            not worker_runtime.has_vm_probe_blocker_override
            or worker_runtime.expected_vm_probe_blocker_key is not None
        ):
            errors.append(
                "worker_execute_runtime_error_zero_division expected_vm_probe_blocker_key must be Some(None)"
            )
        if worker_runtime.expected_success:
            errors.append(
                "worker_execute_runtime_error_zero_division expected_success must be false"
            )
        if worker_runtime.expected_vm_probe_success is not False:
            errors.append(
                "worker_execute_runtime_error_zero_division expected_vm_probe_success must be Some(false)"
            )
        if not worker_runtime.expect_error:
            errors.append(
                "worker_execute_runtime_error_zero_division expect_error must be true"
            )
        if worker_runtime.expected_vm_probe_expect_error is not True:
            errors.append(
                "worker_execute_runtime_error_zero_division expected_vm_probe_expect_error must be Some(true)"
            )
        if worker_runtime.expect_line_column:
            errors.append(
                "worker_execute_runtime_error_zero_division expect_line_column must be false"
            )
        if worker_runtime.expected_vm_probe_expect_line_column is not True:
            errors.append(
                "worker_execute_runtime_error_zero_division expected_vm_probe_expect_line_column must be Some(true)"
            )

    recycle_row = find_lifecycle_row(worker_lifecycle_rows, "recycle")
    if recycle_row is None:
        errors.append("missing worker lifecycle fixture row for action 'recycle'")
    else:
        if recycle_row.expected_state != "unwired":
            errors.append(
                "worker lifecycle recycle default expected_state must be 'unwired'"
            )
        if recycle_row.expected_vm_probe_state != "ready":
            errors.append(
                "worker lifecycle recycle expected_vm_probe_state must be Some(\"ready\")"
            )

    terminate_row = find_lifecycle_row(worker_lifecycle_rows, "terminate")
    if terminate_row is None:
        errors.append("missing worker lifecycle fixture row for action 'terminate'")
    else:
        if terminate_row.expected_vm_probe_state != "unwired":
            errors.append(
                "worker lifecycle terminate expected_vm_probe_state must be Some(\"unwired\")"
            )

    timeout_invalid_rows = find_timeout_rows(
        worker_timeout_rows, expected_phase="invalid_worker_timeout"
    )
    if not timeout_invalid_rows:
        errors.append("missing worker timeout invalid-phase fixture rows")
    for row in timeout_invalid_rows:
        if row.expected_success:
            errors.append(f"{row.name}: invalid timeout expected_success must be false")
        if row.expected_blocker_key is not None:
            errors.append(f"{row.name}: invalid timeout expected_blocker_key must be None")
        if row.expected_vm_probe_phase is not None:
            errors.append(f"{row.name}: invalid timeout expected_vm_probe_phase must be None")
        if row.expected_vm_probe_state is not None:
            errors.append(f"{row.name}: invalid timeout expected_vm_probe_state must be None")
        if row.expected_vm_probe_success is not None:
            errors.append(f"{row.name}: invalid timeout expected_vm_probe_success must be None")
        if row.has_vm_probe_blocker_override:
            errors.append(
                f"{row.name}: invalid timeout expected_vm_probe_blocker_key must be None"
            )

    timeout_unwired_rows = find_timeout_rows(
        worker_timeout_rows, expected_phase="unsupported_worker_timeout_enforcement"
    )
    if not timeout_unwired_rows:
        errors.append("missing worker timeout unsupported-enforcement fixture rows")
    for row in timeout_unwired_rows:
        if row.expected_success:
            errors.append(
                f"{row.name}: unsupported timeout expected_success must be false"
            )
        if row.expected_blocker_key != "worker_runtime_unwired":
            errors.append(
                f"{row.name}: unsupported timeout expected_blocker_key must be worker_runtime_unwired"
            )
        if row.expected_vm_probe_phase != "worker_timeout_configured":
            errors.append(
                f"{row.name}: unsupported timeout expected_vm_probe_phase must be worker_timeout_configured"
            )
        if row.expected_vm_probe_state != "unwired":
            errors.append(
                f"{row.name}: unsupported timeout expected_vm_probe_state must be Some(\"unwired\")"
            )
        if row.expected_vm_probe_success is not True:
            errors.append(
                f"{row.name}: unsupported timeout expected_vm_probe_success must be Some(true)"
            )
        if (
            not row.has_vm_probe_blocker_override
            or row.expected_vm_probe_blocker_key is not None
        ):
            errors.append(
                f"{row.name}: unsupported timeout expected_vm_probe_blocker_key must be Some(None)"
            )

    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--top-fixture",
        default="tests/fixtures/wasm_contract_snippets.rs",
        help="Path to top-level wasm contract fixture file",
    )
    parser.add_argument(
        "--worker-fixture",
        default="tests/fixtures/wasm_worker_contract.rs",
        help="Path to worker wasm contract fixture file",
    )
    parser.add_argument(
        "--out",
        default="perf/wasm_session_contract_summary_latest.json",
        help="Output summary JSON path",
    )
    args = parser.parse_args()

    top_fixture_path = Path(args.top_fixture)
    worker_fixture_path = Path(args.worker_fixture)
    top_source = top_fixture_path.read_text(encoding="utf-8")
    worker_source = worker_fixture_path.read_text(encoding="utf-8")
    top_level_rows = parse_top_level_rows(top_source)
    worker_rows = parse_worker_rows(worker_source)
    worker_lifecycle_rows = parse_worker_lifecycle_rows(worker_source)
    worker_timeout_rows = parse_worker_timeout_rows(worker_source)

    errors = validate(
        top_level_rows, worker_rows, worker_lifecycle_rows, worker_timeout_rows
    )
    if errors:
        print("wasm session contract summary validation failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    summary = {
        "top_fixture": str(top_fixture_path),
        "worker_fixture": str(worker_fixture_path),
        "counts": {
            "top_level_rows": len(top_level_rows),
            "worker_rows": len(worker_rows),
            "worker_lifecycle_rows": len(worker_lifecycle_rows),
            "worker_timeout_rows": len(worker_timeout_rows),
            "worker_timeout_invalid_rows": len(
                [
                    row
                    for row in worker_timeout_rows
                    if row.expected_phase == "invalid_worker_timeout"
                ]
            ),
            "worker_timeout_unwired_rows": len(
                [
                    row
                    for row in worker_timeout_rows
                    if row.expected_phase == "unsupported_worker_timeout_enforcement"
                ]
            ),
            "top_level_supported_vm_probe_ok_rows": len(
                [
                    row
                    for row in top_level_rows
                    if row.expected_support_phase == "supported"
                    and row.expected_vm_probe_execute_phase == "ok"
                ]
            ),
            "top_level_supported_vm_probe_runtime_error_rows": len(
                [
                    row
                    for row in top_level_rows
                    if row.expected_support_phase == "supported"
                    and row.expected_vm_probe_execute_phase == "runtime_error"
                ]
            ),
        },
        "worker_session_rows": {
            "worker_execute_unwired": next(
                (
                    {
                        "name": row.name,
                        "expected_phase": row.expected_phase,
                        "expected_blocker_key": row.expected_blocker_key,
                        "expected_vm_probe_phase": row.expected_vm_probe_phase,
                        "expected_vm_probe_blocker_key": (
                            row.expected_vm_probe_blocker_key
                            if row.has_vm_probe_blocker_override
                            else None
                        ),
                        "expect_error": row.expect_error,
                        "expected_vm_probe_expect_error": row.expected_vm_probe_expect_error,
                        "expected_success": row.expected_success,
                        "expected_vm_probe_success": row.expected_vm_probe_success,
                        "expect_line_column": row.expect_line_column,
                        "expected_vm_probe_expect_line_column": row.expected_vm_probe_expect_line_column,
                    }
                    for row in worker_rows
                    if row.name == "worker_execute_unwired"
                ),
                None,
            ),
            "worker_execute_runtime_error_zero_division": next(
                (
                    {
                        "name": row.name,
                        "expected_phase": row.expected_phase,
                        "expected_blocker_key": row.expected_blocker_key,
                        "expected_vm_probe_phase": row.expected_vm_probe_phase,
                        "expected_vm_probe_blocker_key": (
                            row.expected_vm_probe_blocker_key
                            if row.has_vm_probe_blocker_override
                            else None
                        ),
                        "expect_error": row.expect_error,
                        "expected_vm_probe_expect_error": row.expected_vm_probe_expect_error,
                        "expected_success": row.expected_success,
                        "expected_vm_probe_success": row.expected_vm_probe_success,
                        "expect_line_column": row.expect_line_column,
                        "expected_vm_probe_expect_line_column": row.expected_vm_probe_expect_line_column,
                    }
                    for row in worker_rows
                    if row.name == "worker_execute_runtime_error_zero_division"
                ),
                None,
            ),
        },
        "worker_lifecycle_state_rows": [
            {
                "name": row.name,
                "action": row.action,
                "expected_state": row.expected_state,
                "expected_vm_probe_state": row.expected_vm_probe_state,
            }
            for row in worker_lifecycle_rows
        ],
        "worker_timeout_rows": [
            {
                "name": row.name,
                "timeout_ms": row.timeout_ms,
                "expected_phase": row.expected_phase,
                "expected_state": row.expected_state,
                "expected_success": row.expected_success,
                "expected_blocker_key": row.expected_blocker_key,
                "expected_vm_probe_phase": row.expected_vm_probe_phase,
                "expected_vm_probe_state": row.expected_vm_probe_state,
                "expected_vm_probe_success": row.expected_vm_probe_success,
                "expected_vm_probe_blocker_key": (
                    row.expected_vm_probe_blocker_key
                    if row.has_vm_probe_blocker_override
                    else None
                ),
            }
            for row in worker_timeout_rows
        ],
    }

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
