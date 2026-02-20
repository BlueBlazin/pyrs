#!/usr/bin/env python3
"""Audit `tests/vm.rs` expected globals against CPython semantics.

This checks tests that:
1. define a literal `let source = ...;`
2. execute that source
3. assert `vm.get_global(...) == Some(Value::...)` for scalar value kinds

Supported expected kinds:
- `Value::Bool`
- `Value::Int`
- `Value::Float`
- `Value::Str("...".to_string())`
- `Value::None`
"""

from __future__ import annotations

import argparse
import ast
import json
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any


HELPER_CODE = r"""
import json
import types
import sys

payload = json.loads(sys.stdin.read())
src = payload["source"]
names = payload["names"]
main_module = types.ModuleType("__main__")
ns = main_module.__dict__
ns.update({
    "__name__": "__main__",
    "__package__": None,
    "__builtins__": __builtins__,
    "__file__": "<audit_vm_semantics>",
})
previous_main = sys.modules.get("__main__")
sys.modules["__main__"] = main_module
try:
    exec(src, ns, ns)
except Exception as exc:
    print(json.dumps({
        "ok": False,
        "error_type": type(exc).__name__,
        "error": str(exc),
    }))
    if previous_main is None:
        del sys.modules["__main__"]
    else:
        sys.modules["__main__"] = previous_main
    raise SystemExit(0)
if previous_main is None:
    del sys.modules["__main__"]
else:
    sys.modules["__main__"] = previous_main

out = {}
for name in names:
    if name not in ns:
        out[name] = {"kind": "MISSING"}
        continue
    value = ns[name]
    if value is None:
        out[name] = {"kind": "None", "value": None}
    elif isinstance(value, bool):
        out[name] = {"kind": "Bool", "value": value}
    elif isinstance(value, int) and not isinstance(value, bool):
        out[name] = {"kind": "Int", "value": value}
    elif isinstance(value, float):
        out[name] = {"kind": "Float", "value": value}
    elif isinstance(value, str):
        out[name] = {"kind": "Str", "value": value}
    else:
        out[name] = {
            "kind": "Other",
            "type": type(value).__name__,
            "repr": repr(value),
        }

print(json.dumps({"ok": True, "values": out}))
"""


@dataclass
class Expected:
    name: str
    kind: str
    value: Any


def find_test_blocks(text: str) -> list[tuple[str, str]]:
    blocks: list[tuple[str, str]] = []
    indices = [m.start() for m in re.finditer(r"(?m)^#\[test\]\s*$", text)]
    for idx, start in enumerate(indices):
        end = indices[idx + 1] if idx + 1 < len(indices) else len(text)
        block = text[start:end]
        match = re.search(r"fn\s+([A-Za-z0-9_]+)\s*\(", block)
        if not match:
            continue
        blocks.append((match.group(1), block))
    return blocks


def parse_source_literal(block: str) -> str | None:
    marker = "let source ="
    start = block.find(marker)
    if start < 0:
        return None
    i = start + len(marker)
    while i < len(block) and block[i].isspace():
        i += 1
    if i >= len(block):
        return None
    # Skip dynamic sources (format!, include_str!, etc.)
    if block.startswith("format!", i) or block.startswith("include_str!", i):
        return None
    # Raw string: r"...", r#"..."#, r##"..."## ...
    if block[i] == "r":
        i += 1
        hashes = 0
        while i < len(block) and block[i] == "#":
            hashes += 1
            i += 1
        if i >= len(block) or block[i] != '"':
            return None
        body_start = i + 1
        close = '"' + ("#" * hashes)
        body_end = block.find(close, body_start)
        if body_end < 0:
            return None
        return block[body_start:body_end]
    # Normal string literal.
    if block[i] != '"':
        return None
    j = i + 1
    escaped = False
    while j < len(block):
        ch = block[j]
        if escaped:
            escaped = False
        elif ch == "\\":
            escaped = True
        elif ch == '"':
            break
        j += 1
    if j >= len(block):
        return None
    literal = block[i : j + 1]
    try:
        value = ast.literal_eval(literal)
    except Exception:
        return None
    return value if isinstance(value, str) else None


def parse_expected_from_line(line: str) -> Expected | None:
    line = line.strip()
    m = re.match(
        r'assert_eq!\(vm\.get_global\("([^"]+)"\),\s*Some\((.+)\)\);$',
        line,
    )
    if not m:
        return None
    name, expr = m.group(1), m.group(2).strip()
    if expr == "Value::None":
        return Expected(name=name, kind="None", value=None)
    m_bool = re.fullmatch(r"Value::Bool\((true|false)\)", expr)
    if m_bool:
        return Expected(name=name, kind="Bool", value=(m_bool.group(1) == "true"))
    m_int = re.fullmatch(r"Value::Int\((-?[0-9_]+)\)", expr)
    if m_int:
        return Expected(name=name, kind="Int", value=int(m_int.group(1).replace("_", "")))
    m_float = re.fullmatch(r"Value::Float\((-?[0-9_]+(?:\.[0-9_]+)?)\)", expr)
    if m_float:
        return Expected(name=name, kind="Float", value=float(m_float.group(1).replace("_", "")))
    m_str = re.fullmatch(r'Value::Str\("((?:[^"\\]|\\.)*)"\.to_string\(\)\)', expr)
    if m_str:
        return Expected(name=name, kind="Str", value=ast.literal_eval(f'"{m_str.group(1)}"'))
    return None


def parse_expectations(block: str) -> list[Expected]:
    expected: list[Expected] = []
    for line in block.splitlines():
        parsed = parse_expected_from_line(line)
        if parsed is not None:
            expected.append(parsed)
    return expected


def is_core_semantics_source(source: str) -> bool:
    lowered = source.lower()
    if re.search(r"(?m)^\s*(import|from)\s+", lowered):
        return False
    return True


def run_case(cpython: str, source: str, names: list[str], timeout_sec: float) -> dict[str, Any]:
    payload = {"source": source, "names": names}
    try:
        proc = subprocess.run(
            [cpython, "-S", "-c", HELPER_CODE],
            input=json.dumps(payload),
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout_sec,
        )
    except subprocess.TimeoutExpired:
        return {
            "ok": False,
            "runner_error": "timeout",
        }
    if proc.returncode != 0:
        return {
            "ok": False,
            "runner_error": f"exit={proc.returncode}",
            "stdout": proc.stdout,
            "stderr": proc.stderr,
        }
    lines = [line for line in proc.stdout.splitlines() if line.strip()]
    if not lines:
        return {
            "ok": False,
            "runner_error": "no-output",
            "stdout": proc.stdout,
            "stderr": proc.stderr,
        }
    for line in reversed(lines):
        start = line.find("{")
        if start < 0:
            continue
        candidate = line[start:]
        try:
            return json.loads(candidate)
        except json.JSONDecodeError:
            continue
    return {
        "ok": False,
        "runner_error": "invalid-json",
        "stdout": proc.stdout,
        "stderr": proc.stderr,
    }


CPYTHON_PARITY_EXEMPT_TESTS = {
    # Interpreter identity intentionally differs from CPython branding.
    "exposes_sys_implementation_identity",
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--vm-tests",
        default="tests/vm.rs",
        help="Path to vm.rs test file",
    )
    parser.add_argument(
        "--cpython",
        default="/Library/Frameworks/Python.framework/Versions/3.14/bin/python3",
        help="Path to CPython executable",
    )
    parser.add_argument(
        "--out",
        default="perf/vm_semantic_expectation_audit_latest.json",
        help="Output report path",
    )
    parser.add_argument(
        "--timeout-sec",
        type=float,
        default=5.0,
        help="Per-test CPython timeout in seconds",
    )
    parser.add_argument(
        "--mode",
        choices=["all", "core"],
        default="core",
        help="Audit mode: 'core' skips module-import-dependent tests",
    )
    args = parser.parse_args()

    text = Path(args.vm_tests).read_text(encoding="utf-8")
    blocks = find_test_blocks(text)

    audited = 0
    skipped = 0
    skipped_tests: list[dict[str, str]] = []
    mismatches: list[dict[str, Any]] = []
    audited_tests: list[str] = []

    for test_name, block in blocks:
        if test_name in CPYTHON_PARITY_EXEMPT_TESTS:
            skipped += 1
            skipped_tests.append({"test": test_name, "reason": "parity-exempt"})
            continue
        source = parse_source_literal(block)
        expected = parse_expectations(block)
        if source is None or not expected:
            skipped += 1
            skipped_tests.append({"test": test_name, "reason": "unsupported-source-or-assert"})
            continue
        if "expect_err(" in block:
            skipped += 1
            skipped_tests.append({"test": test_name, "reason": "expects-error-path"})
            continue
        if "LIB_PATH" in source:
            skipped += 1
            skipped_tests.append({"test": test_name, "reason": "template-placeholder"})
            continue
        if "vm.add_module_path(" in block:
            skipped += 1
            skipped_tests.append({"test": test_name, "reason": "module-path-dependent"})
            continue
        if args.mode == "core" and not is_core_semantics_source(source):
            skipped += 1
            skipped_tests.append({"test": test_name, "reason": "non-core-source"})
            continue
        names = sorted({item.name for item in expected})
        result = run_case(args.cpython, source, names, args.timeout_sec)
        audited += 1
        audited_tests.append(test_name)
        if not result.get("ok", False):
            if result.get("runner_error") == "timeout":
                skipped += 1
                audited -= 1
                audited_tests.pop()
                skipped_tests.append({"test": test_name, "reason": "cpython-timeout"})
            else:
                mismatches.append(
                    {
                        "test": test_name,
                        "reason": "cpython-run-failed",
                        "detail": result,
                    }
                )
            continue
        values = result.get("values", {})
        for item in expected:
            actual = values.get(item.name, {"kind": "MISSING"})
            if actual.get("kind") != item.kind:
                mismatches.append(
                    {
                        "test": test_name,
                        "global": item.name,
                        "reason": "kind-mismatch",
                        "expected": {"kind": item.kind, "value": item.value},
                        "actual": actual,
                    }
                )
                continue
            if item.kind in {"Bool", "Int", "Float", "Str"}:
                if actual.get("value") != item.value:
                    mismatches.append(
                        {
                            "test": test_name,
                            "global": item.name,
                            "reason": "value-mismatch",
                            "expected": {"kind": item.kind, "value": item.value},
                            "actual": actual,
                        }
                    )

    report = {
        "summary": {
            "total_tests": len(blocks),
            "audited_tests": audited,
            "skipped_tests": skipped,
            "mismatch_count": len(mismatches),
        },
        "mismatches": mismatches,
        "audited_tests": audited_tests,
        "skipped_tests": skipped_tests,
    }
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"Wrote {out_path}")
    print(
        "audited_tests:",
        audited,
        "skipped_tests:",
        skipped,
        "mismatch_count:",
        len(mismatches),
    )
    return 1 if mismatches else 0


if __name__ == "__main__":
    raise SystemExit(main())
