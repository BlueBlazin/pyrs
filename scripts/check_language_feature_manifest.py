#!/usr/bin/env python3
"""Validate source-language feature accounting against CPython 3.14 probes."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


Probe = dict[str, Any]


PROBES: list[Probe] = [
    {
        "id": "template_literal_basic_type",
        "mode": "json_result",
        "source": """variety = "Stilton"
template = t"Try some {variety} cheese!"
interp = template.interpolations[0]
result = {
    "type": repr(type(template)),
    "strings": list(template.strings),
    "interp": [interp.value, interp.expression, interp.conversion, interp.format_spec],
}
""",
    },
    {
        "id": "template_literal_debug_and_concat",
        "mode": "json_result",
        "source": """x = 7
t1 = t"{x=}"
t2 = t"{x=:>4}"
t3 = t"a{1}" t"b{2}"
result = {
    "t1_strings": list(t1.strings),
    "t1_interp": [[i.expression, i.conversion, i.format_spec, i.value] for i in t1.interpolations],
    "t2_strings": list(t2.strings),
    "t2_interp": [[i.expression, i.conversion, i.format_spec, i.value] for i in t2.interpolations],
    "t3_strings": list(t3.strings),
    "t3_interp": [[i.expression, i.conversion, i.format_spec, i.value] for i in t3.interpolations],
}
""",
    },
    {
        "id": "template_literal_mixed_literal_rejected",
        "mode": "stderr_contains",
        "source": "x = 'a' t'b'\n",
        "needle": "cannot mix t-string literals with string or bytes literals",
    },
    {
        "id": "template_literal_incompatible_f_prefix_rejected",
        "mode": "stderr_contains",
        "source": "x = tf'{x}'\n",
        "needle": "'f' and 't' prefixes are incompatible",
    },
    {
        "id": "template_literal_incompatible_b_prefix_rejected",
        "mode": "stderr_contains",
        "source": "x = bt'raw'\n",
        "needle": "'b' and 't' prefixes are incompatible",
    },
    {
        "id": "template_literal_incompatible_u_prefix_rejected",
        "mode": "stderr_contains",
        "source": "x = ut'raw'\n",
        "needle": "'u' and 't' prefixes are incompatible",
    },
]


def run_cmd(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, text=True, capture_output=True, check=False)


def run_json_result(interpreter: Path, source: str) -> tuple[bool, Any]:
    wrapped = f"{source}\nimport json\nprint(json.dumps(result))\n"
    proc = run_cmd([str(interpreter), "-S", "-c", wrapped])
    if proc.returncode != 0:
        return False, proc.stderr.strip()
    payload = proc.stdout.strip()
    try:
        return True, json.loads(payload)
    except json.JSONDecodeError:
        return False, payload


def run_stderr_contains(interpreter: Path, source: str, needle: str) -> tuple[bool, str]:
    proc = run_cmd([str(interpreter), "-S", "-c", source])
    if proc.returncode == 0:
        return False, "expected failure but command succeeded"
    stderr = proc.stderr
    return needle in stderr, stderr.strip()


def load_manifest(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def validate_manifest_ids(manifest: dict[str, Any]) -> tuple[list[str], list[str]]:
    manifest_ids = {entry["id"] for entry in manifest.get("features", [])}
    probe_ids = {probe["id"] for probe in PROBES}
    missing = sorted(probe_ids - manifest_ids)
    unknown = sorted(manifest_ids - probe_ids)
    return missing, unknown


def build_manifest_index(manifest: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {entry["id"]: entry for entry in manifest.get("features", [])}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pyrs", required=True, type=Path, help="Path to pyrs binary")
    parser.add_argument(
        "--cpython-bin",
        default="python3",
        type=Path,
        help="CPython 3.14 binary",
    )
    parser.add_argument(
        "--manifest",
        required=True,
        type=Path,
        help="docs/LANGUAGE_FEATURE_MANIFEST.json path",
    )
    parser.add_argument(
        "--out",
        required=True,
        type=Path,
        help="Output JSON path (perf/language_feature_manifest_latest.json)",
    )
    args = parser.parse_args()

    manifest = load_manifest(args.manifest)
    missing, unknown = validate_manifest_ids(manifest)
    index = build_manifest_index(manifest)

    report: dict[str, Any] = {
        "schema_version": 1,
        "cpython_bin": str(args.cpython_bin),
        "pyrs_bin": str(args.pyrs),
        "manifest_path": str(args.manifest),
        "missing_manifest_ids": missing,
        "unknown_manifest_ids": unknown,
        "features": [],
        "summary": {
            "total": 0,
            "pass": 0,
            "fail": 0,
            "required_fail": 0,
        },
    }

    for probe in PROBES:
        feature_id = probe["id"]
        mode = probe["mode"]
        source = probe["source"]

        if mode == "json_result":
            cp_ok, cp_payload = run_json_result(args.cpython_bin, source)
            py_ok, py_payload = run_json_result(args.pyrs, source)
            passed = cp_ok and py_ok and cp_payload == py_payload
            details = {
                "cpython_ok": cp_ok,
                "pyrs_ok": py_ok,
                "cpython_payload": cp_payload,
                "pyrs_payload": py_payload,
            }
        elif mode == "stderr_contains":
            needle = probe["needle"]
            cp_ok, cp_payload = run_stderr_contains(args.cpython_bin, source, needle)
            py_ok, py_payload = run_stderr_contains(args.pyrs, source, needle)
            passed = cp_ok and py_ok
            details = {
                "needle": needle,
                "cpython_ok": cp_ok,
                "pyrs_ok": py_ok,
                "cpython_stderr": cp_payload,
                "pyrs_stderr": py_payload,
            }
        else:
            raise RuntimeError(f"unknown probe mode: {mode}")

        manifest_entry = index.get(feature_id, {})
        required = bool(manifest_entry.get("required", False))
        status = manifest_entry.get("status", "untracked")
        owner = manifest_entry.get("owner", "")
        closure = manifest_entry.get("closure_criteria", "")

        row = {
            "id": feature_id,
            "required": required,
            "status": status,
            "owner": owner,
            "closure_criteria": closure,
            "pass": passed,
            "details": details,
        }
        report["features"].append(row)
        report["summary"]["total"] += 1
        if passed:
            report["summary"]["pass"] += 1
        else:
            report["summary"]["fail"] += 1
            if required:
                report["summary"]["required_fail"] += 1

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")

    failed = False
    if missing or unknown:
        failed = True
    if report["summary"]["required_fail"] > 0:
        failed = True

    if failed:
        print("language feature manifest check failed", file=sys.stderr)
        print(
            json.dumps(
                {
                    "missing_manifest_ids": missing,
                    "unknown_manifest_ids": unknown,
                    "required_fail": report["summary"]["required_fail"],
                    "out": str(args.out),
                },
                indent=2,
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1

    print(
        f"language feature manifest: {report['summary']['pass']}/{report['summary']['total']} probes passed"
    )
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
