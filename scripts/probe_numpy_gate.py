#!/usr/bin/env python3
"""Probe NumPy bring-up gates for pyrs and emit a JSON report."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import time
from typing import Any


CASES: list[tuple[str, str]] = [
    ("numpy_import", "import numpy as np"),
    (
        "numpy_ndarray_sum",
        "import numpy as np\na = np.array([1, 2, 3])\nassert int(a.sum()) == 6",
    ),
]


def classify_failure(stderr: str) -> dict[str, str]:
    diagnostics: dict[str, str] = {}
    symbol_match = re.search(r"failed to resolve symbol '([^']+)'", stderr)
    if symbol_match:
        diagnostics["missing_symbol"] = symbol_match.group(1)
        diagnostics["kind"] = "missing-symbol"
        return diagnostics
    if "unsupported extension ABI" in stderr:
        diagnostics["kind"] = "abi-mismatch"
        return diagnostics
    if "initializer" in stderr and "failed with status" in stderr:
        diagnostics["kind"] = "init-failure"
        return diagnostics
    if "ModuleNotFoundError: module 'numpy' not found" in stderr:
        diagnostics["kind"] = "module-not-found"
        return diagnostics
    return diagnostics


def run_case(
    pyrs_bin: pathlib.Path,
    source: str,
    timeout_secs: int,
    cpython_lib: pathlib.Path | None,
    python_paths: list[pathlib.Path],
) -> dict[str, Any]:
    env = os.environ.copy()
    if cpython_lib is not None:
        env["PYRS_CPYTHON_LIB"] = str(cpython_lib)
    if python_paths:
        existing = env.get("PYTHONPATH", "")
        prefix = os.pathsep.join(str(path) for path in python_paths)
        env["PYTHONPATH"] = f"{prefix}{os.pathsep}{existing}" if existing else prefix
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


def build_numpy_from_source(
    python_build_bin: str,
    numpy_src: pathlib.Path,
    timeout_secs: int,
) -> tuple[dict[str, Any], pathlib.Path | None]:
    if not numpy_src.exists():
        return (
            {
                "status": "SKIP",
                "ok": False,
                "reason": f"numpy source path '{numpy_src}' does not exist",
            },
            None,
        )

    target_dir = pathlib.Path(tempfile.mkdtemp(prefix="pyrs_numpy_target_"))
    cmd = [
        python_build_bin,
        "-m",
        "pip",
        "install",
        "--no-deps",
        "--no-build-isolation",
        "--target",
        str(target_dir),
        str(numpy_src),
    ]
    start = time.perf_counter()
    try:
        completed = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout_secs,
            check=False,
        )
    except subprocess.TimeoutExpired:
        elapsed = round(time.perf_counter() - start, 4)
        return (
            {
                "status": "FAIL",
                "ok": False,
                "elapsed_secs": elapsed,
                "cmd": cmd,
                "reason": f"timeout after {timeout_secs}s",
            },
            None,
        )

    elapsed = round(time.perf_counter() - start, 4)
    ok = completed.returncode == 0
    return (
        {
            "status": "PASS" if ok else "FAIL",
            "ok": ok,
            "elapsed_secs": elapsed,
            "cmd": cmd,
            "returncode": completed.returncode,
            "stdout": completed.stdout.strip(),
            "stderr": completed.stderr.strip(),
            "target_dir": str(target_dir),
        },
        target_dir if ok else None,
    )


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
        "--numpy-src",
        default=os.environ.get("PYRS_NUMPY_SRC", ""),
        help="Optional NumPy source tree path for source-build bring-up",
    )
    parser.add_argument(
        "--python-build-bin",
        default=os.environ.get("PYRS_NUMPY_BUILD_PYTHON", "python3"),
        help="Python executable used for source-build phase (default: python3)",
    )
    parser.add_argument(
        "--build-timeout",
        type=int,
        default=900,
        help="Source-build timeout in seconds when --numpy-src is set (default: 900)",
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
    numpy_src = pathlib.Path(args.numpy_src) if args.numpy_src else None

    report: dict[str, Any] = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "pyrs_bin": str(pyrs_bin),
        "cpython_lib": str(cpython_lib) if cpython_lib else None,
        "numpy_src": str(numpy_src) if numpy_src else None,
        "mode": "source-build" if numpy_src else "import-probe",
        "timeout_secs": args.timeout,
        "cases": [],
        "build": None,
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

    python_paths: list[pathlib.Path] = []
    if numpy_src:
        build_report, build_target = build_numpy_from_source(
            args.python_build_bin,
            numpy_src,
            args.build_timeout,
        )
        report["build"] = build_report
        if build_target is not None:
            python_paths.append(build_target)
        print(f"[numpy-gate] source-build: {build_report['status']}")
    else:
        report["build"] = {
            "status": "SKIP",
            "ok": False,
            "reason": "--numpy-src not provided",
        }

    for case_name, source in CASES:
        result = run_case(pyrs_bin, source, args.timeout, cpython_lib, python_paths)
        status = "PASS" if result["ok"] else "FAIL"
        if status == "PASS":
            report["summary"]["passed"] += 1
        else:
            report["summary"]["failed"] += 1
            diagnostics = classify_failure(result.get("stderr", ""))
            if diagnostics:
                result["diagnostics"] = diagnostics
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
