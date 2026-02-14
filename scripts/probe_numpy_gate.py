#!/usr/bin/env python3
"""Probe NumPy bring-up gates for pyrs and emit a JSON report."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import subprocess
import sys
import time
from typing import Any


CASES: list[tuple[str, str]] = [
    ("numpy_import", "import numpy as np"),
    (
        "numpy_ndarray_sum",
        "import numpy as np\na = np.array([1, 2, 3])\nassert int(a.sum()) == 6",
    ),
]


def run_case(
    pyrs_bin: pathlib.Path,
    source: str,
    timeout_secs: int,
    cpython_lib: pathlib.Path | None,
) -> dict[str, Any]:
    env = os.environ.copy()
    if cpython_lib is not None:
        env["PYRS_CPYTHON_LIB"] = str(cpython_lib)
    start = time.perf_counter()
    try:
        completed = subprocess.run(
            [str(pyrs_bin), "-S", "-c", source],
            capture_output=True,
            text=True,
            timeout=timeout_secs,
            env=env,
            check=False,
        )
        elapsed = round(time.perf_counter() - start, 4)
        return {
            "ok": completed.returncode == 0,
            "returncode": completed.returncode,
            "elapsed_secs": elapsed,
            "stdout": completed.stdout.strip(),
            "stderr": completed.stderr.strip(),
        }
    except subprocess.TimeoutExpired as exc:
        elapsed = round(time.perf_counter() - start, 4)
        return {
            "ok": False,
            "returncode": None,
            "elapsed_secs": elapsed,
            "stdout": (exc.stdout or "").strip(),
            "stderr": f"timeout after {timeout_secs}s",
        }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--pyrs",
        default="target/debug/pyrs",
        help="Path to pyrs binary (default: target/debug/pyrs)",
    )
    parser.add_argument(
        "--cpython-lib",
        default=os.environ.get("PYRS_CPYTHON_LIB", ""),
        help="Optional PYRS_CPYTHON_LIB path to pass through to pyrs",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=20,
        help="Per-case timeout in seconds (default: 20)",
    )
    parser.add_argument(
        "--out",
        default="perf/numpy_gate_latest.json",
        help="JSON output path (default: perf/numpy_gate_latest.json)",
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help="Return non-zero if any gate fails",
    )
    args = parser.parse_args()

    pyrs_bin = pathlib.Path(args.pyrs)
    out_path = pathlib.Path(args.out)
    cpython_lib = pathlib.Path(args.cpython_lib) if args.cpython_lib else None

    report: dict[str, Any] = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "pyrs_bin": str(pyrs_bin),
        "cpython_lib": str(cpython_lib) if cpython_lib else None,
        "timeout_secs": args.timeout,
        "cases": [],
        "summary": {
            "total": len(CASES),
            "passed": 0,
            "failed": 0,
            "skipped": 0,
        },
    }

    if not pyrs_bin.is_file():
        for case_name, _source in CASES:
            report["cases"].append(
                {
                    "name": case_name,
                    "status": "SKIP",
                    "reason": f"pyrs binary not found at '{pyrs_bin}'",
                }
            )
            report["summary"]["skipped"] += 1
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
        print(
            f"[numpy-gate] skipped all cases: pyrs binary not found at {pyrs_bin}",
            file=sys.stderr,
        )
        return 0

    for case_name, source in CASES:
        result = run_case(pyrs_bin, source, args.timeout, cpython_lib)
        status = "PASS" if result["ok"] else "FAIL"
        if status == "PASS":
            report["summary"]["passed"] += 1
        else:
            report["summary"]["failed"] += 1
        report["cases"].append({"name": case_name, "status": status, **result})
        print(f"[numpy-gate] {case_name}: {status}")

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(f"[numpy-gate] wrote report to {out_path}")

    if args.strict and report["summary"]["failed"] > 0:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
