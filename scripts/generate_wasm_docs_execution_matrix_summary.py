#!/usr/bin/env python3
"""Validate wasm execution matrix docs against source phase/blocker contracts."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


def parse_source_phase_keys(wasm_source: str, enum_name: str) -> list[str]:
    pattern = re.compile(
        rf"impl\s+{re.escape(enum_name)}\s*\{{.*?fn key\(self\) -> &'static str \{{(.*?)\n\s*\}}\n",
        flags=re.DOTALL,
    )
    match = pattern.search(wasm_source)
    if not match:
        raise ValueError(f"unable to parse key() for enum {enum_name}")
    body = match.group(1)
    return re.findall(r'=>\s*"([^"]+)"', body)


def parse_source_const(wasm_source: str, const_name: str) -> str:
    match = re.search(
        rf'const\s+{re.escape(const_name)}:\s*&str\s*=\s*"([^"]+)";',
        wasm_source,
    )
    if not match:
        raise ValueError(f"unable to parse const {const_name}")
    return match.group(1)


def extract_execution_matrix_rows(docs_source: str) -> list[str]:
    anchor = "## Execution Mode Matrix"
    if anchor not in docs_source:
        raise ValueError("missing '## Execution Mode Matrix' section in docs")
    after = docs_source.split(anchor, 1)[1]
    rows: list[str] = []
    for raw_line in after.splitlines():
        line = raw_line.strip()
        if line.startswith("|"):
            rows.append(line)
            continue
        if rows and line == "":
            break
    return rows


def validate_rows(rows: list[str], source_contract: dict[str, str]) -> list[str]:
    errors: list[str] = []
    if len(rows) < 6:
        errors.append(
            "execution mode matrix is incomplete; expected header + separator + four data rows"
        )
        return errors

    data_rows = [row for row in rows[2:] if row.count("|") >= 4]
    if len(data_rows) != 4:
        errors.append(f"expected exactly 4 data rows in execution matrix, got {len(data_rows)}")
        return errors

    expected_patterns = [
        (
            "execute_default",
            [
                "`execute(source)`",
                "default",
                "syntax_error",
                "compile_error",
                source_contract["top_unsupported_phase"],
                source_contract["top_unwired_blocker"],
            ],
        ),
        (
            "execute_vm_probe",
            [
                "`execute(source)`",
                "`wasm-vm-probe`",
                "ok",
                "runtime_error",
                source_contract["top_unsupported_phase"],
            ],
        ),
        (
            "worker_default",
            [
                "`wasm_worker_execute(source)`",
                "default",
                "syntax_error",
                "compile_error",
                source_contract["worker_unsupported_phase"],
                source_contract["worker_unwired_blocker"],
            ],
        ),
        (
            "worker_vm_probe",
            [
                "`wasm_worker_execute(source)`",
                "`wasm-vm-probe`",
                "ok",
                "runtime_error",
                source_contract["worker_unsupported_phase"],
            ],
        ),
    ]

    for row_name, needles in expected_patterns:
        matched = False
        for row in data_rows:
            if all(needle in row for needle in needles):
                matched = True
                break
        if not matched:
            errors.append(f"missing or stale execution matrix row: {row_name}")

    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--docs",
        default="docs/WASM_API_CONTRACT.md",
        help="Path to wasm API contract docs",
    )
    parser.add_argument(
        "--wasm-src",
        default="src/wasm/mod.rs",
        help="Path to wasm source contract definitions",
    )
    parser.add_argument(
        "--out",
        default="perf/wasm_docs_execution_matrix_summary_latest.json",
        help="Output summary path",
    )
    args = parser.parse_args()

    docs_path = Path(args.docs)
    wasm_src_path = Path(args.wasm_src)
    docs_source = docs_path.read_text(encoding="utf-8")
    wasm_source = wasm_src_path.read_text(encoding="utf-8")

    top_phases = parse_source_phase_keys(wasm_source, "WasmExecutionPhase")
    worker_phases = parse_source_phase_keys(wasm_source, "WasmWorkerExecutePhase")
    top_unwired_blocker = parse_source_const(
        wasm_source, "WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED"
    )
    worker_unwired_blocker = parse_source_const(
        wasm_source, "WASM_WORKER_BLOCKER_RUNTIME_UNWIRED"
    )

    source_contract = {
        "top_unsupported_phase": next(
            (phase for phase in top_phases if phase == "unsupported_execution"), ""
        ),
        "worker_unsupported_phase": next(
            (phase for phase in worker_phases if phase == "unsupported_worker_execution"), ""
        ),
        "top_unwired_blocker": top_unwired_blocker,
        "worker_unwired_blocker": worker_unwired_blocker,
    }

    rows = extract_execution_matrix_rows(docs_source)
    errors = validate_rows(rows, source_contract)
    if not source_contract["top_unsupported_phase"]:
        errors.append("source missing top-level unsupported_execution phase")
    if not source_contract["worker_unsupported_phase"]:
        errors.append("source missing worker unsupported_worker_execution phase")

    if errors:
        print("wasm docs execution matrix validation failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    summary = {
        "docs": str(docs_path),
        "wasm_source": str(wasm_src_path),
        "source_contract": source_contract,
        "matrix_row_count": len(rows),
        "matrix_rows": rows,
    }

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
