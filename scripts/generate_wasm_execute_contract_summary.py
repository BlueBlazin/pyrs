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
    expected_vm_probe_execute_phase: str | None
    expected_vm_probe_execute_blocker_key: str | None
    has_vm_probe_execute_blocker_override: bool
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
    "expected_vm_probe_execute_phase",
    "expected_first_blocker_key",
)


def parse_string_array(source: str, const_name: str) -> list[str]:
    match = re.search(
        rf"pub const {re.escape(const_name)}:[^=]*=\s*&\[(.*?)\];",
        source,
        flags=re.DOTALL,
    )
    if not match:
        raise ValueError(f"missing fixture constant '{const_name}'")
    return re.findall(r'"([^"]+)"', match.group(1))


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
        f"missing optional field '{field}' with Some(Some(...)) / Some(None) / None"
    )


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
        (
            has_vm_probe_execute_blocker_override,
            expected_vm_probe_execute_blocker_key,
        ) = parse_optional_optional_string(body, "expected_vm_probe_execute_blocker_key")
        fixtures.append(
            SnippetFixture(
                name=values["name"] or "",
                expected_compile_phase=values["expected_compile_phase"] or "",
                expected_execute_phase=values["expected_execute_phase"] or "",
                expected_execute_blocker_key=values["expected_execute_blocker_key"],
                expected_vm_probe_execute_phase=values["expected_vm_probe_execute_phase"],
                expected_vm_probe_execute_blocker_key=expected_vm_probe_execute_blocker_key,
                has_vm_probe_execute_blocker_override=has_vm_probe_execute_blocker_override,
                expected_support_phase=values["expected_support_phase"] or "",
                expected_first_blocker_key=values["expected_first_blocker_key"],
            )
        )
    return fixtures


def unique(values: list[str]) -> list[str]:
    return sorted(set(values))


def ordered_unique(values: list[str]) -> list[str]:
    seen: set[str] = set()
    ordered: list[str] = []
    for value in values:
        if value not in seen:
            seen.add(value)
            ordered.append(value)
    return ordered


def parse_source_const_string(wasm_source: str, const_name: str) -> str:
    match = re.search(
        rf'const\s+{re.escape(const_name)}:\s*&str\s*=\s*"([^"]+)";',
        wasm_source,
    )
    if not match:
        raise ValueError(f"unable to parse {const_name} from wasm source")
    return match.group(1)


def parse_source_execution_phase_keys(wasm_source: str, vm_probe_enabled: bool) -> list[str]:
    keys = ordered_unique(
        re.findall(r'WasmExecutionPhase::[A-Za-z]+\s*=>\s*"([^"]+)"', wasm_source)
    )
    if vm_probe_enabled:
        keys.append(parse_source_const_string(wasm_source, "WASM_EXECUTION_PHASE_OK"))
        keys.append(parse_source_const_string(wasm_source, "WASM_EXECUTION_PHASE_RUNTIME_ERROR"))
    return keys


def effective_fixture_execute_expectation(
    fixture: SnippetFixture, vm_probe_enabled: bool
) -> tuple[str, str | None]:
    phase = fixture.expected_execute_phase
    blocker_key = fixture.expected_execute_blocker_key
    if vm_probe_enabled:
        if fixture.expected_vm_probe_execute_phase is not None:
            phase = fixture.expected_vm_probe_execute_phase
        if fixture.has_vm_probe_execute_blocker_override:
            blocker_key = fixture.expected_vm_probe_execute_blocker_key
    return phase, blocker_key


def parse_source_backend_blocker_key(wasm_source: str) -> str:
    match = re.search(
        r'const\s+WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED:\s*&str\s*=\s*"([^"]+)";',
        wasm_source,
    )
    if not match:
        raise ValueError("unable to parse WASM_EXECUTION_BLOCKER_BACKEND_UNWIRED from wasm source")
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
    return unique(re.findall(r'\(\s*"[^"]+"\s*,\s*"([^"]+)"\s*\)', body))


def validate(
    fixtures: list[SnippetFixture],
    fixture_execute_phase_keys: list[str],
    fixture_support_phase_keys: list[str],
    source_phase_keys: list[str],
    source_backend_blocker_key: str,
    source_module_policy_blocker_keys: list[str],
    vm_probe_enabled: bool,
) -> list[str]:
    errors: list[str] = []
    if not fixtures:
        errors.append("no snippet fixtures parsed")
        return errors
    if not source_phase_keys:
        errors.append("no execute phase keys parsed from wasm source")
    if not source_backend_blocker_key:
        errors.append("empty backend blocker key parsed from wasm source")
    if not source_module_policy_blocker_keys:
        errors.append("no module-policy blocker keys parsed from wasm source")
    if not fixture_execute_phase_keys:
        errors.append("fixture execute phase keys must not be empty")
    if not fixture_support_phase_keys:
        errors.append("fixture support phase keys must not be empty")

    allowed_compile_phases = {"ok", "syntax_error", "compile_error"}
    allowed_execute_phases = {"syntax_error", "compile_error", "unsupported_execution"}
    expected_fixture_execute_phase_keys = list(fixture_execute_phase_keys)
    if vm_probe_enabled:
        allowed_execute_phases.add("ok")
        allowed_execute_phases.add("runtime_error")
        expected_fixture_execute_phase_keys.extend(["ok", "runtime_error"])
    allowed_support_phases = {"supported", "blocked_capability", "syntax_error", "compile_error"}

    if set(source_phase_keys) != allowed_execute_phases:
        errors.append(
            "source execute phase keys must equal canonical set "
            f"{sorted(allowed_execute_phases)}; got {source_phase_keys}"
        )
    if set(expected_fixture_execute_phase_keys) != allowed_execute_phases:
        errors.append(
            "fixture execute phase keys must equal canonical set "
            f"{sorted(allowed_execute_phases)}; got {expected_fixture_execute_phase_keys}"
        )
    if set(fixture_support_phase_keys) != allowed_support_phases:
        errors.append(
            "fixture support phase keys must equal canonical set "
            f"{sorted(allowed_support_phases)}; got {fixture_support_phase_keys}"
        )
    if expected_fixture_execute_phase_keys != source_phase_keys:
        errors.append(
            "fixture execute phase key order must match source order; "
            f"fixture={expected_fixture_execute_phase_keys}, source={source_phase_keys}"
        )

    seen_names: set[str] = set()
    for fixture in fixtures:
        if fixture.name in seen_names:
            errors.append(f"duplicate fixture name: {fixture.name}")
        seen_names.add(fixture.name)

        if fixture.expected_compile_phase not in allowed_compile_phases:
            errors.append(
                f"{fixture.name}: invalid expected_compile_phase '{fixture.expected_compile_phase}'"
            )
        if fixture.expected_support_phase not in allowed_support_phases:
            errors.append(
                f"{fixture.name}: invalid expected_support_phase '{fixture.expected_support_phase}'"
            )

        effective_execute_phase, effective_execute_blocker_key = effective_fixture_execute_expectation(
            fixture, vm_probe_enabled
        )
        if effective_execute_phase not in allowed_execute_phases:
            errors.append(
                f"{fixture.name}: invalid effective expected_execute_phase '{effective_execute_phase}'"
            )

        if effective_execute_phase == "unsupported_execution":
            if fixture.expected_support_phase == "blocked_capability":
                if effective_execute_blocker_key != fixture.expected_first_blocker_key:
                    errors.append(
                        f"{fixture.name}: blocked_capability unsupported_execution must align "
                        "expected_execute_blocker_key with expected_first_blocker_key"
                    )
                elif effective_execute_blocker_key not in source_module_policy_blocker_keys:
                    errors.append(
                        f"{fixture.name}: blocked_capability blocker key must be in source "
                        f"module policy keys {source_module_policy_blocker_keys}"
                    )
            elif effective_execute_blocker_key != source_backend_blocker_key:
                errors.append(
                    f"{fixture.name}: unsupported_execution must use expected_execute_blocker_key="
                    f"'{source_backend_blocker_key}' when support phase is not blocked_capability"
                )
        elif effective_execute_blocker_key is not None:
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

    fixture_execute_phases = unique(
        [
            effective_fixture_execute_expectation(fixture, vm_probe_enabled)[0]
            for fixture in fixtures
        ]
    )
    if set(fixture_execute_phases) != set(source_phase_keys):
        errors.append(
            "fixture execute phase set must match source execute phase keys; "
            f"fixtures={fixture_execute_phases}, source={source_phase_keys}"
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
    parser.add_argument(
        "--wasm-src",
        default="src/wasm/mod.rs",
        help="Path to wasm source file for execute-phase/blocker parity checks",
    )
    parser.add_argument(
        "--vm-probe",
        action="store_true",
        help="Validate execute contract for wasm-vm-probe mode",
    )
    args = parser.parse_args()

    fixture_path = Path(args.fixture)
    fixture_source = fixture_path.read_text(encoding="utf-8")
    fixtures = parse_fixtures(fixture_source)
    fixture_execute_phase_keys = parse_string_array(
        fixture_source, "WASM_EXECUTION_PHASE_KEYS"
    )
    fixture_support_phase_keys = parse_string_array(
        fixture_source, "WASM_SUPPORT_PHASE_KEYS"
    )
    wasm_source_path = Path(args.wasm_src)
    wasm_source = wasm_source_path.read_text(encoding="utf-8")
    source_phase_keys = parse_source_execution_phase_keys(wasm_source, args.vm_probe)
    source_backend_blocker_key = parse_source_backend_blocker_key(wasm_source)
    source_module_policy_blocker_keys = parse_source_module_policy_blocker_keys(wasm_source)

    errors = validate(
        fixtures,
        fixture_execute_phase_keys,
        fixture_support_phase_keys,
        source_phase_keys,
        source_backend_blocker_key,
        source_module_policy_blocker_keys,
        args.vm_probe,
    )
    if errors:
        print("wasm execute contract summary validation failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    effective_rows = [
        (
            fixture,
            *effective_fixture_execute_expectation(fixture, args.vm_probe),
        )
        for fixture in fixtures
    ]
    fixture_execute_phase_keys_effective = list(fixture_execute_phase_keys)
    if args.vm_probe:
        fixture_execute_phase_keys_effective.extend(["ok", "runtime_error"])
    execute_blocker_keys = [
        effective_execute_blocker_key
        for _, _, effective_execute_blocker_key in effective_rows
        if effective_execute_blocker_key is not None
    ]

    summary = {
        "fixture": str(fixture_path),
        "wasm_source": str(wasm_source_path),
        "mode": "vm_probe" if args.vm_probe else "default",
        "counts": {
            "fixtures": len(fixtures),
            "execute_blocker_rows": len(execute_blocker_keys),
        },
        "phases": {
            "compile": unique([fixture.expected_compile_phase for fixture in fixtures]),
            "execute": unique(
                [effective_execute_phase for _, effective_execute_phase, _ in effective_rows]
            ),
            "support": unique([fixture.expected_support_phase for fixture in fixtures]),
            "fixture_execute": fixture_execute_phase_keys,
            "fixture_execute_effective": fixture_execute_phase_keys_effective,
            "fixture_support": fixture_support_phase_keys,
            "source_execute": source_phase_keys,
        },
        "source_backend_blocker_key": source_backend_blocker_key,
        "source_module_policy_blocker_keys": source_module_policy_blocker_keys,
        "execute_blocker_keys": unique(execute_blocker_keys),
        "rows": [
            {
                "name": fixture.name,
                "expected_compile_phase": fixture.expected_compile_phase,
                "expected_execute_phase": fixture.expected_execute_phase,
                "expected_execute_blocker_key": fixture.expected_execute_blocker_key,
                "expected_vm_probe_execute_phase": fixture.expected_vm_probe_execute_phase,
                "expected_vm_probe_execute_blocker_key": (
                    fixture.expected_vm_probe_execute_blocker_key
                    if fixture.has_vm_probe_execute_blocker_override
                    else None
                ),
                "has_vm_probe_execute_blocker_override": (
                    fixture.has_vm_probe_execute_blocker_override
                ),
                "effective_execute_phase": effective_execute_phase,
                "effective_execute_blocker_key": effective_execute_blocker_key,
                "expected_support_phase": fixture.expected_support_phase,
                "expected_first_blocker_key": fixture.expected_first_blocker_key,
            }
            for fixture, effective_execute_phase, effective_execute_blocker_key in effective_rows
        ],
    }

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
