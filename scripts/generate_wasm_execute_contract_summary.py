#!/usr/bin/env python3
"""Generate and validate top-level wasm execute contract fixture summary."""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass
class SnippetFixture:
    name: str
    expected_compile_phase: str
    expected_execute_phase: str
    expected_execute_blocker_key: str | None
    expected_support_phase: str
    expected_first_blocker_key: str | None


REQUIRED_STR_FIELDS = (
    "name",
    "expected_compile_phase",
    "expected_execute_phase",
    "expected_support_phase",
)
OPTIONAL_STR_FIELDS = (
    "expected_execute_blocker_key",
    "expected_first_blocker_key",
)


def parse_required_string(body: str, field: str) -> str:
    match = re.search(rf"{re.escape(field)}:\s*\"([^\"]+)\"", body)
    if not match:
        raise ValueError(f"missing required field '{field}'")
    return match.group(1)


def parse_optional_string(body: str, field: str) -> str | None:
    some_match = re.search(rf"{re.escape(field)}:\s*Some\(\"([^\"]+)\"\)", body)
    if some_match:
        return some_match.group(1)
    none_match = re.search(rf"{re.escape(field)}:\s*None", body)
    if none_match:
        return None
    raise ValueError(f"missing optional field '{field}' with Some(...) or None")


def parse_fixtures(source: str) -> list[SnippetFixture]:
    fixtures: list[SnippetFixture] = []
    pattern = re.compile(r"WasmContractSnippetFixture\s*\{(.*?)\n\s*\},", re.DOTALL)
    for match in pattern.finditer(source):
        body = match.group(1)
        values: dict[str, str | None] = {}
        for field in REQUIRED_STR_FIELDS:
            values[field] = parse_required_string(body, field)
        for field in OPTIONAL_STR_FIELDS:
            values[field] = parse_optional_string(body, field)
        fixtures.append(
            SnippetFixture(
                name=values["name"] or "",
                expected_compile_phase=values["expected_compile_phase"] or "",
                expected_execute_phase=values["expected_execute_phase"] or "",
                expected_execute_blocker_key=values["expected_execute_blocker_key"],
                expected_support_phase=values["expected_support_phase"] or "",
                expected_first_blocker_key=values["expected_first_blocker_key"],
            )
        )
    return fixtures


def unique(values: list[str]) -> list[str]:
    return sorted(set(values))


def validate(fixtures: list[SnippetFixture]) -> list[str]:
    errors: list[str] = []
    if not fixtures:
        errors.append("no snippet fixtures parsed")
        return errors

    allowed_compile_phases = {"ok", "syntax_error", "compile_error"}
    allowed_execute_phases = {"syntax_error", "compile_error", "unsupported_execution"}
    allowed_support_phases = {"supported", "blocked_capability", "syntax_error", "compile_error"}

    seen_names: set[str] = set()
    for fixture in fixtures:
        if fixture.name in seen_names:
            errors.append(f"duplicate fixture name: {fixture.name}")
        seen_names.add(fixture.name)

        if fixture.expected_compile_phase not in allowed_compile_phases:
            errors.append(
                f"{fixture.name}: invalid expected_compile_phase '{fixture.expected_compile_phase}'"
            )
        if fixture.expected_execute_phase not in allowed_execute_phases:
            errors.append(
                f"{fixture.name}: invalid expected_execute_phase '{fixture.expected_execute_phase}'"
            )
        if fixture.expected_support_phase not in allowed_support_phases:
            errors.append(
                f"{fixture.name}: invalid expected_support_phase '{fixture.expected_support_phase}'"
            )

        if fixture.expected_execute_phase == "unsupported_execution":
            if fixture.expected_execute_blocker_key != "execution_backend_unwired":
                errors.append(
                    f"{fixture.name}: unsupported_execution must use expected_execute_blocker_key='execution_backend_unwired'"
                )
        elif fixture.expected_execute_blocker_key is not None:
            errors.append(
                f"{fixture.name}: non-unsupported execute phase must set expected_execute_blocker_key=None"
            )

        if fixture.expected_support_phase == "blocked_capability":
            if fixture.expected_first_blocker_key is None:
                errors.append(
                    f"{fixture.name}: blocked_capability must set expected_first_blocker_key"
                )
        elif fixture.expected_first_blocker_key is not None:
            errors.append(
                f"{fixture.name}: non-blocked support phase must set expected_first_blocker_key=None"
            )

    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--fixture",
        default="tests/fixtures/wasm_contract_snippets.rs",
        help="Path to top-level wasm contract fixture file",
    )
    parser.add_argument(
        "--out",
        default="perf/wasm_execute_contract_summary_latest.json",
        help="Output summary JSON path",
    )
    args = parser.parse_args()

    fixture_path = Path(args.fixture)
    source = fixture_path.read_text(encoding="utf-8")
    fixtures = parse_fixtures(source)

    errors = validate(fixtures)
    if errors:
        print("wasm execute contract summary validation failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    execute_blocker_keys = [
        fixture.expected_execute_blocker_key
        for fixture in fixtures
        if fixture.expected_execute_blocker_key is not None
    ]

    summary = {
        "fixture": str(fixture_path),
        "counts": {
            "fixtures": len(fixtures),
            "execute_blocker_rows": len(execute_blocker_keys),
        },
        "phases": {
            "compile": unique([fixture.expected_compile_phase for fixture in fixtures]),
            "execute": unique([fixture.expected_execute_phase for fixture in fixtures]),
            "support": unique([fixture.expected_support_phase for fixture in fixtures]),
        },
        "execute_blocker_keys": unique(execute_blocker_keys),
        "rows": [
            {
                "name": fixture.name,
                "expected_compile_phase": fixture.expected_compile_phase,
                "expected_execute_phase": fixture.expected_execute_phase,
                "expected_execute_blocker_key": fixture.expected_execute_blocker_key,
                "expected_support_phase": fixture.expected_support_phase,
                "expected_first_blocker_key": fixture.expected_first_blocker_key,
            }
            for fixture in fixtures
        ],
    }

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
