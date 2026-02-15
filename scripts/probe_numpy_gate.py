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

CaseSpec = tuple[str, str, tuple[str, ...]]

BASE_CASES: list[CaseSpec] = [
    ("numpy_import", "import numpy as np", ("numpy",)),
    (
        "numpy_ndarray_sum",
        "import numpy as np\na = np.array([1, 2, 3])\nassert int(a.sum()) == 6",
        ("numpy",),
    ),
]

SCIENTIFIC_STACK_CASES: list[CaseSpec] = [
    ("scipy_import", "import scipy as sp\nassert sp.__name__ == 'scipy'", ("scipy",)),
    (
        "pandas_import",
        "import pandas as pd\nassert pd.__name__ == 'pandas'",
        ("pandas",),
    ),
    (
        "pandas_series_sum",
        "import pandas as pd\ns = pd.Series([1, 2, 3])\nassert int(s.sum()) == 6",
        ("pandas",),
    ),
    (
        "matplotlib_import",
        "import matplotlib\nassert matplotlib.__name__ == 'matplotlib'",
        ("matplotlib",),
    ),
    (
        "matplotlib_pyplot_smoke",
        "import matplotlib\nmatplotlib.use('Agg')\nimport matplotlib.pyplot as plt\nfig, ax = plt.subplots()\nax.plot([1, 2], [3, 4])\nfig.canvas.draw()\nplt.close(fig)",
        ("matplotlib",),
    ),
]


def classify_failure(stderr: str) -> dict[str, str]:
    diagnostics: dict[str, str] = {}
    abi_mode_match = re.search(
        r"expected '([^']+)'.*CPython-style extension symbols such as '([^']+)'",
        stderr,
    )
    if abi_mode_match:
        diagnostics["expected_symbol"] = abi_mode_match.group(1)
        diagnostics["found_symbol_style"] = abi_mode_match.group(2)
        diagnostics["kind"] = "abi-mode-mismatch"
        return diagnostics
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
    module_not_found_match = re.search(
        r"ModuleNotFoundError:\s+module '([^']+)' not found",
        stderr,
    )
    if module_not_found_match:
        diagnostics["kind"] = "module-not-found"
        diagnostics["missing_module"] = module_not_found_match.group(1)
        return diagnostics
    no_module_named_match = re.search(r"No module named '([^']+)'", stderr)
    if no_module_named_match:
        diagnostics["kind"] = "module-not-found"
        diagnostics["missing_module"] = no_module_named_match.group(1)
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


def probe_local_modules(
    python_probe_bin: str,
    modules: list[str],
    timeout_secs: int,
) -> tuple[dict[str, Any], list[pathlib.Path]]:
    if not modules:
        return (
            {
                "status": "SKIP",
                "ok": False,
                "reason": "no modules requested",
                "modules": {},
            },
            [],
        )
    snippet = (
        "import importlib.util, json, pathlib, sys\n"
        f"modules = {modules!r}\n"
        "report = {}\n"
        "roots = []\n"
        "for name in modules:\n"
        "    spec = importlib.util.find_spec(name)\n"
        "    if spec is None or not spec.origin:\n"
        "        report[name] = {'status': 'NOT_FOUND', 'ok': False}\n"
        "        continue\n"
        "    origin = str(pathlib.Path(spec.origin).resolve())\n"
        "    path_root = str(pathlib.Path(origin).parent.parent)\n"
        "    report[name] = {'status': 'FOUND', 'ok': True, 'origin': origin, 'path_root': path_root}\n"
        "    roots.append(path_root)\n"
        "print(json.dumps({'status': 'OK', 'ok': True, 'modules': report, 'path_roots': roots}))\n"
    )
    cmd = [python_probe_bin, "-c", snippet]
    try:
        completed = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout_secs,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return (
            {
                "status": "ERROR",
                "ok": False,
                "reason": f"timeout after {timeout_secs}s",
                "cmd": cmd,
            },
            [],
        )
    if completed.returncode != 0:
        return (
            {
                "status": "ERROR",
                "ok": False,
                "reason": "python probe command failed",
                "cmd": cmd,
                "returncode": completed.returncode,
                "stdout": completed.stdout.strip(),
                "stderr": completed.stderr.strip(),
            },
            [],
        )
    payload_text = completed.stdout.strip()
    if not payload_text:
        return (
            {
                "status": "ERROR",
                "ok": False,
                "reason": "python probe produced empty output",
                "cmd": cmd,
            },
            [],
        )
    try:
        payload = json.loads(payload_text)
    except json.JSONDecodeError:
        return (
            {
                "status": "ERROR",
                "ok": False,
                "reason": "python probe produced non-JSON output",
                "cmd": cmd,
                "stdout": payload_text,
            },
            [],
        )
    roots: list[pathlib.Path] = []
    for root in payload.get("path_roots", []):
        if isinstance(root, str) and root:
            roots.append(pathlib.Path(root))
    return (payload, roots)


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
        "--probe-local-numpy",
        action="store_true",
        help="Probe local Python for an installed NumPy and add its site-packages root to PYTHONPATH for gate runs.",
    )
    parser.add_argument(
        "--probe-local-stack",
        action="store_true",
        help="Probe local Python for NumPy/SciPy/Pandas/Matplotlib and add discovered site-packages roots to PYTHONPATH.",
    )
    parser.add_argument(
        "--include-scientific-stack",
        action="store_true",
        help="Run additional scipy/pandas/matplotlib bridge probes.",
    )
    parser.add_argument(
        "--python-probe-bin",
        default=os.environ.get("PYRS_NUMPY_PROBE_PYTHON", "python3"),
        help="Python executable used for --probe-local-numpy (default: python3)",
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
    active_cases = list(BASE_CASES)
    if args.include_scientific_stack:
        active_cases.extend(SCIENTIFIC_STACK_CASES)

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
            "total": len(active_cases),
            "passed": 0,
            "failed": 0,
            "skipped": 0,
        },
        "local_numpy_probe": None,
        "local_module_probe": None,
    }

    if not pyrs_bin.is_file():
        for case_name, _source, _required_modules in active_cases:
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

    probed_modules = ["numpy"]
    if args.probe_local_stack or args.include_scientific_stack:
        probed_modules = ["numpy", "scipy", "pandas", "matplotlib"]

    installed_module_status: dict[str, str] = {}
    if args.probe_local_numpy or args.probe_local_stack:
        local_module_report, local_module_paths = probe_local_modules(
            args.python_probe_bin,
            probed_modules,
            args.timeout,
        )
        report["local_module_probe"] = {
            "python_probe_bin": args.python_probe_bin,
            "requested_modules": probed_modules,
            **local_module_report,
        }
        module_payload = local_module_report.get("modules", {})
        if isinstance(module_payload, dict):
            for name, payload in module_payload.items():
                if isinstance(name, str) and isinstance(payload, dict):
                    status = payload.get("status")
                    if isinstance(status, str):
                        installed_module_status[name] = status
            numpy_entry = module_payload.get("numpy")
            if isinstance(numpy_entry, dict):
                report["local_numpy_probe"] = {
                    "python_probe_bin": args.python_probe_bin,
                    **numpy_entry,
                }
        if report["local_numpy_probe"] is None:
            report["local_numpy_probe"] = {
                "status": "ERROR",
                "ok": False,
                "reason": "numpy probe result missing from local module probe payload",
                "python_probe_bin": args.python_probe_bin,
            }
        for root in local_module_paths:
            python_paths.append(root)
        print(
            f"[numpy-gate] local module probe: {local_module_report.get('status', 'ERROR')}"
        )
    else:
        report["local_numpy_probe"] = {
            "status": "SKIP",
            "ok": False,
            "reason": "--probe-local-numpy not provided",
        }
        report["local_module_probe"] = {
            "status": "SKIP",
            "ok": False,
            "reason": "--probe-local-numpy/--probe-local-stack not provided",
        }

    dedup_paths: list[pathlib.Path] = []
    seen_path_text: set[str] = set()
    for path in python_paths:
        path_text = str(path)
        if path_text in seen_path_text:
            continue
        seen_path_text.add(path_text)
        dedup_paths.append(path)
    python_paths = dedup_paths

    for case_name, source, required_modules in active_cases:
        missing_modules = [
            module
            for module in required_modules
            if installed_module_status.get(module) == "NOT_FOUND"
        ]
        if missing_modules:
            report["cases"].append(
                {
                    "name": case_name,
                    "status": "SKIP",
                    "reason": (
                        "required modules not found in local probe: "
                        + ", ".join(missing_modules)
                    ),
                    "required_modules": list(required_modules),
                }
            )
            report["summary"]["skipped"] += 1
            print(f"[numpy-gate] {case_name}: SKIP ({', '.join(missing_modules)} missing)")
            continue

        result = run_case(pyrs_bin, source, args.timeout, cpython_lib, python_paths)
        status = "PASS" if result["ok"] else "FAIL"
        if status == "PASS":
            report["summary"]["passed"] += 1
        else:
            report["summary"]["failed"] += 1
            diagnostics = classify_failure(result.get("stderr", ""))
            if diagnostics:
                result["diagnostics"] = diagnostics
        report["cases"].append(
            {
                "name": case_name,
                "status": status,
                "required_modules": list(required_modules),
                **result,
            }
        )
        print(f"[numpy-gate] {case_name}: {status}")

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(f"[numpy-gate] wrote report to {out_path}")

    if args.strict and report["summary"]["failed"] > 0:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
