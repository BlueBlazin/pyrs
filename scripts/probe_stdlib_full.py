#!/usr/bin/env python3
"""Exhaustive stdlib baseline probe for pyrs against CPython 3.14.

This script does two systematic passes:
1. Import pass:
   - Enumerates stdlib module names from CPython (`sys.stdlib_module_names`).
   - Checks import behavior in CPython (host support baseline) and in pyrs.
2. Comprehensive pass (module-mapped CPython tests):
   - Discovers `Lib/test/test_*.py` modules.
   - Maps stdlib modules to direct CPython test modules by naming convention.
   - Runs mapped tests under pyrs with per-test-module timeout.

Output artifact is JSON with per-module rows + aggregate counters.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import pathlib
import subprocess
import sys
import time
from dataclasses import dataclass
from typing import Any


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class ProcessResult:
    ok: bool
    returncode: int | None
    stdout: str
    stderr: str
    elapsed_secs: float
    timeout: bool


def ensure_text(value: str | bytes | None) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return value


def detect_cpython_bin(explicit: str | None) -> pathlib.Path:
    candidates: list[pathlib.Path] = []
    if explicit:
        candidates.append(pathlib.Path(explicit))
    env = os.environ.get("PYRS_CPYTHON_BIN")
    if env:
        candidates.append(pathlib.Path(env))
    candidates.extend(
        [
            pathlib.Path("/Library/Frameworks/Python.framework/Versions/3.14/bin/python3"),
            pathlib.Path(sys.executable),
        ]
    )
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    raise SystemExit("could not locate CPython 3.14 binary; pass --cpython-bin")


def detect_cpython_lib(explicit: str | None) -> pathlib.Path:
    candidates: list[pathlib.Path] = []
    if explicit:
        candidates.append(pathlib.Path(explicit))
    env = os.environ.get("PYRS_CPYTHON_LIB")
    if env:
        candidates.append(pathlib.Path(env))
    candidates.extend(
        [
            REPO_ROOT / ".local" / "Python-3.14.3" / "Lib",
            pathlib.Path("/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14"),
        ]
    )
    for candidate in candidates:
        if candidate.joinpath("test").is_dir():
            return candidate
    raise SystemExit("could not locate CPython Lib directory; pass --cpython-lib")


def run_process(
    argv: list[str],
    timeout_secs: int,
    env: dict[str, str],
) -> ProcessResult:
    start = time.perf_counter()
    try:
        completed = subprocess.run(
            argv,
            capture_output=True,
            text=True,
            timeout=timeout_secs,
            env=env,
            check=False,
        )
        elapsed = round(time.perf_counter() - start, 4)
        return ProcessResult(
            ok=completed.returncode == 0,
            returncode=completed.returncode,
            stdout=(completed.stdout or "").strip(),
            stderr=(completed.stderr or "").strip(),
            elapsed_secs=elapsed,
            timeout=False,
        )
    except subprocess.TimeoutExpired as exc:
        elapsed = round(time.perf_counter() - start, 4)
        return ProcessResult(
            ok=False,
            returncode=None,
            stdout=ensure_text(exc.stdout).strip(),
            stderr=ensure_text(exc.stderr).strip() or f"timeout after {timeout_secs}s",
            elapsed_secs=elapsed,
            timeout=True,
        )


def extract_last_json_line(output: str) -> dict[str, Any] | None:
    for raw in reversed(output.splitlines()):
        line = raw.strip()
        if not line:
            continue
        try:
            payload = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(payload, dict):
            return payload
    return None


def get_stdlib_modules(cpython_bin: pathlib.Path) -> list[str]:
    code = (
        "import json,sys\n"
        "mods = sorted(sys.stdlib_module_names)\n"
        "print(json.dumps({'modules': mods}))\n"
    )
    proc = run_process(
        [str(cpython_bin), "-S", "-c", code],
        timeout_secs=20,
        env=os.environ.copy(),
    )
    if not proc.ok:
        raise SystemExit(f"failed to query stdlib module names: {proc.stderr or proc.stdout}")
    payload = extract_last_json_line(proc.stdout)
    if not payload or "modules" not in payload:
        raise SystemExit("failed to parse stdlib module inventory from CPython")
    modules = payload["modules"]
    if not isinstance(modules, list) or not all(isinstance(item, str) for item in modules):
        raise SystemExit("invalid stdlib module inventory payload")
    return modules


def import_probe_snippet(module: str) -> str:
    return (
        "import importlib, json, sys\n"
        f"name = {module!r}\n"
        "try:\n"
        "    importlib.import_module(name)\n"
        "    print(json.dumps({'ok': True, 'module': name}))\n"
        "except BaseException as exc:\n"
        "    print(json.dumps({'ok': False, 'module': name, 'error_type': type(exc).__name__, 'error': str(exc)}))\n"
        "    raise\n"
    )


def run_import_probe(
    interpreter: pathlib.Path,
    module: str,
    timeout_secs: int,
    env: dict[str, str],
) -> dict[str, Any]:
    proc = run_process(
        [str(interpreter), "-S", "-c", import_probe_snippet(module)],
        timeout_secs=timeout_secs,
        env=env,
    )
    combined = "\n".join(part for part in [proc.stdout, proc.stderr] if part).strip()
    payload = extract_last_json_line(combined) or {}
    msg = combined
    if len(msg) > 2000:
        msg = msg[:2000]
    return {
        "ok": bool(payload.get("ok", proc.ok)),
        "error_type": payload.get("error_type"),
        "error": payload.get("error"),
        "returncode": proc.returncode,
        "timeout": proc.timeout,
        "elapsed_secs": proc.elapsed_secs,
        "message": msg,
    }


def discover_test_modules(cpython_lib: pathlib.Path) -> list[str]:
    test_root = cpython_lib / "test"
    discovered: set[str] = set()
    for path in sorted(test_root.glob("test_*.py")):
        discovered.add(f"test.{path.stem}")
    for path in sorted(test_root.glob("test_*")):
        if path.is_dir() and path.joinpath("__init__.py").is_file():
            discovered.add(f"test.{path.name}")
    return sorted(discovered)


def module_candidate_keys(module: str) -> set[str]:
    keys = {module.replace(".", "_")}
    top = module.split(".")[0]
    keys.add(top.replace(".", "_"))
    if module.startswith("_"):
        keys.add(module.lstrip("_").replace(".", "_"))
    if top.startswith("_"):
        keys.add(top.lstrip("_").replace(".", "_"))
    if "." in module:
        tail = module.split(".")[-1]
        keys.add(tail.replace(".", "_"))
        if tail.startswith("_"):
            keys.add(tail.lstrip("_").replace(".", "_"))
    return {key for key in keys if key}


def map_tests_for_modules(stdlib_modules: list[str], test_modules: list[str]) -> dict[str, list[str]]:
    mapped: dict[str, list[str]] = {}
    test_names = [name.split(".", 1)[1] for name in test_modules if "." in name]
    for module in stdlib_modules:
        keys = module_candidate_keys(module)
        selected: list[str] = []
        for full_name, short_name in zip(test_modules, test_names, strict=False):
            for key in keys:
                exact = f"test_{key}"
                prefix = f"test_{key}_"
                if short_name == exact or short_name.startswith(prefix):
                    selected.append(full_name)
                    break
        mapped[module] = sorted(set(selected))
    return mapped


def run_unittest_module(
    pyrs_bin: pathlib.Path,
    test_module: str,
    timeout_secs: int,
    env: dict[str, str],
) -> dict[str, Any]:
    source = (
        "import importlib, io, json, sys, unittest\n"
        "from test import support as _pyrs_test_support\n"
        "_pyrs_test_support.use_resources = {}\n"
        f"name = {test_module!r}\n"
        "stream = io.StringIO()\n"
        "try:\n"
        "    mod = importlib.import_module(name)\n"
        "except BaseException as exc:\n"
        "    print(json.dumps({'ok': False, 'phase': 'import', 'test_module': name, "
        "'error_type': type(exc).__name__, 'error': str(exc)}))\n"
        "    raise\n"
        "suite = unittest.defaultTestLoader.loadTestsFromModule(mod)\n"
        "runner = unittest.TextTestRunner(stream=stream, verbosity=0)\n"
        "result = runner.run(suite)\n"
        "payload = {\n"
        "    'ok': result.wasSuccessful(),\n"
        "    'phase': 'run',\n"
        "    'test_module': name,\n"
        "    'tests_run': result.testsRun,\n"
        "    'failures': len(result.failures),\n"
        "    'errors': len(result.errors),\n"
        "    'skipped': len(result.skipped),\n"
        "    'expected_failures': len(result.expectedFailures),\n"
        "    'unexpected_successes': len(result.unexpectedSuccesses),\n"
        "    'runner_output': stream.getvalue()[-1200:],\n"
        "}\n"
        "print(json.dumps(payload))\n"
    )
    proc = run_process(
        [str(pyrs_bin), "-S", "-c", source],
        timeout_secs=timeout_secs,
        env=env,
    )
    combined = "\n".join(part for part in [proc.stdout, proc.stderr] if part).strip()
    payload = extract_last_json_line(combined) or {}
    message = combined
    if len(message) > 2000:
        message = message[:2000]
    return {
        "test_module": test_module,
        "ok": bool(payload.get("ok", proc.ok)),
        "phase": payload.get("phase", "unknown"),
        "tests_run": int(payload.get("tests_run", 0) or 0),
        "failures": int(payload.get("failures", 0) or 0),
        "errors": int(payload.get("errors", 0) or 0),
        "skipped": int(payload.get("skipped", 0) or 0),
        "expected_failures": int(payload.get("expected_failures", 0) or 0),
        "unexpected_successes": int(payload.get("unexpected_successes", 0) or 0),
        "returncode": proc.returncode,
        "timeout": proc.timeout,
        "elapsed_secs": proc.elapsed_secs,
        "message": message,
    }


def run_import_phase_for_module(
    module: str,
    cpython_bin: pathlib.Path,
    pyrs_bin: pathlib.Path,
    import_timeout_secs: int,
    cpython_env: dict[str, str],
    pyrs_env: dict[str, str],
    mapped_tests: list[str],
) -> dict[str, Any]:
    cp_import = run_import_probe(cpython_bin, module, import_timeout_secs, cpython_env)
    pyrs_import = run_import_probe(pyrs_bin, module, import_timeout_secs, pyrs_env)
    supported_on_host = bool(cp_import["ok"])
    if not mapped_tests:
        comprehensive_status = "NO_DIRECT_TEST_MODULE"
    elif not supported_on_host:
        comprehensive_status = "SKIPPED_UNSUPPORTED_ON_HOST"
    elif not pyrs_import["ok"]:
        comprehensive_status = "SKIPPED_IMPORT_FAILED"
    else:
        comprehensive_status = "PENDING"
    return {
        "module": module,
        "supported_on_host": supported_on_host,
        "cpython_import": cp_import,
        "pyrs_import": pyrs_import,
        "mapped_test_modules": mapped_tests,
        "comprehensive_status": comprehensive_status,
        "test_results": [],
    }


def run_comprehensive_phase_for_module(
    module: str,
    mapped_tests: list[str],
    pyrs_bin: pathlib.Path,
    test_timeout_secs: int,
    pyrs_env: dict[str, str],
) -> tuple[str, str, list[dict[str, Any]]]:
    test_results: list[dict[str, Any]] = []
    for test_module in mapped_tests:
        test_results.append(
            run_unittest_module(
                pyrs_bin=pyrs_bin,
                test_module=test_module,
                timeout_secs=test_timeout_secs,
                env=pyrs_env,
            )
        )
    if any(result["timeout"] for result in test_results):
        status = "TIMEOUT"
    elif all(result["ok"] for result in test_results):
        status = "PASS"
    else:
        status = "FAIL"
    return module, status, test_results


def main() -> None:
    parser = argparse.ArgumentParser(description="Run exhaustive CPython-3.14 stdlib import/comprehensive probes against pyrs")
    parser.add_argument("--pyrs", default="target/debug/pyrs", help="Path to pyrs binary")
    parser.add_argument("--cpython-bin", default=None, help="Path to CPython 3.14 binary")
    parser.add_argument("--cpython-lib", default=None, help="Path to CPython 3.14 Lib directory")
    parser.add_argument("--out", default="perf/stdlib_full_probe_latest.json", help="JSON output artifact path")
    parser.add_argument("--import-timeout", type=int, default=20, help="Per-module import timeout in seconds")
    parser.add_argument("--test-timeout", type=int, default=120, help="Per-test-module timeout in seconds")
    parser.add_argument("--jobs", type=int, default=0, help="Parallel workers (0 = CPU core count)")
    parser.add_argument("--max-modules", type=int, default=0, help="Optional module cap for quick dry runs (0 = all)")
    args = parser.parse_args()

    pyrs_bin = pathlib.Path(args.pyrs)
    if not pyrs_bin.is_file():
        raise SystemExit(f"pyrs binary not found: {pyrs_bin}")
    cpython_bin = detect_cpython_bin(args.cpython_bin)
    cpython_lib = detect_cpython_lib(args.cpython_lib)

    stdlib_modules = get_stdlib_modules(cpython_bin)
    if args.max_modules > 0:
        stdlib_modules = stdlib_modules[: args.max_modules]
    discovered_tests = discover_test_modules(cpython_lib)
    module_test_map = map_tests_for_modules(stdlib_modules, discovered_tests)
    jobs = args.jobs if args.jobs > 0 else max(1, os.cpu_count() or 1)

    cpython_env = os.environ.copy()
    cpython_env["BROWSER"] = "true"

    pyrs_env = os.environ.copy()
    pyrs_env["PYRS_CPYTHON_LIB"] = str(cpython_lib)
    pyrs_env["BROWSER"] = "true"

    rows_by_module: dict[str, dict[str, Any]] = {}

    started = time.perf_counter()
    print(f"running import phase with jobs={jobs}", flush=True)
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as executor:
        future_to_module: dict[concurrent.futures.Future[dict[str, Any]], str] = {}
        for module in stdlib_modules:
            future = executor.submit(
                run_import_phase_for_module,
                module,
                cpython_bin,
                pyrs_bin,
                args.import_timeout,
                cpython_env,
                pyrs_env,
                module_test_map.get(module, []),
            )
            future_to_module[future] = module
        for index, future in enumerate(concurrent.futures.as_completed(future_to_module), start=1):
            module = future_to_module[future]
            try:
                row = future.result()
            except Exception as exc:  # noqa: BLE001
                row = {
                    "module": module,
                    "supported_on_host": False,
                    "cpython_import": {"ok": False, "error_type": type(exc).__name__, "error": str(exc)},
                    "pyrs_import": {"ok": False, "error_type": type(exc).__name__, "error": str(exc)},
                    "mapped_test_modules": module_test_map.get(module, []),
                    "comprehensive_status": "ERROR_IMPORT_PHASE",
                    "test_results": [],
                }
            rows_by_module[module] = row
            print(f"[import {index}/{len(stdlib_modules)}] {module}", flush=True)

    modules_for_comprehensive = [
        module
        for module in stdlib_modules
        if rows_by_module[module]["comprehensive_status"] == "PENDING"
    ]
    print(
        f"running comprehensive phase with jobs={jobs} across {len(modules_for_comprehensive)} modules",
        flush=True,
    )
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as executor:
        future_to_module: dict[concurrent.futures.Future[tuple[str, str, list[dict[str, Any]]]], str] = {}
        for module in modules_for_comprehensive:
            mapped_tests = rows_by_module[module]["mapped_test_modules"]
            future = executor.submit(
                run_comprehensive_phase_for_module,
                module,
                mapped_tests,
                pyrs_bin,
                args.test_timeout,
                pyrs_env,
            )
            future_to_module[future] = module
        for index, future in enumerate(concurrent.futures.as_completed(future_to_module), start=1):
            module = future_to_module[future]
            try:
                resolved_module, status, test_results = future.result()
            except Exception as exc:  # noqa: BLE001
                resolved_module = module
                status = "ERROR_COMPREHENSIVE_PHASE"
                test_results = [
                    {
                        "test_module": "<framework>",
                        "ok": False,
                        "phase": "framework",
                        "tests_run": 0,
                        "failures": 0,
                        "errors": 1,
                        "skipped": 0,
                        "expected_failures": 0,
                        "unexpected_successes": 0,
                        "returncode": None,
                        "timeout": False,
                        "elapsed_secs": 0.0,
                        "message": f"{type(exc).__name__}: {exc}",
                    }
                ]
            rows_by_module[resolved_module]["comprehensive_status"] = status
            rows_by_module[resolved_module]["test_results"] = test_results
            print(
                f"[tests {index}/{len(modules_for_comprehensive)}] {resolved_module} -> {status}",
                flush=True,
            )

    rows = [rows_by_module[module] for module in stdlib_modules]
    host_supported = sum(1 for row in rows if row["supported_on_host"])
    import_pass_supported = sum(
        1 for row in rows if row["supported_on_host"] and row["pyrs_import"]["ok"]
    )
    modules_with_direct_tests = sum(1 for row in rows if row["mapped_test_modules"])
    comprehensive_pass = sum(1 for row in rows if row["comprehensive_status"] == "PASS")
    comprehensive_fail = sum(1 for row in rows if row["comprehensive_status"] == "FAIL")
    comprehensive_timeout = sum(1 for row in rows if row["comprehensive_status"] == "TIMEOUT")

    elapsed_total = round(time.perf_counter() - started, 2)
    payload: dict[str, Any] = {
        "pyrs_bin": str(pyrs_bin),
        "cpython_bin": str(cpython_bin),
        "cpython_lib": str(cpython_lib),
        "total_stdlib_modules": len(stdlib_modules),
        "host_supported_modules": host_supported,
        "pyrs_import_pass_on_host_supported": import_pass_supported,
        "modules_with_direct_tests": modules_with_direct_tests,
        "comprehensive_pass": comprehensive_pass,
        "comprehensive_fail": comprehensive_fail,
        "comprehensive_timeout": comprehensive_timeout,
        "import_timeout_secs": args.import_timeout,
        "test_timeout_secs": args.test_timeout,
        "jobs": jobs,
        "elapsed_total_secs": elapsed_total,
        "rows": rows,
    }

    out_path = pathlib.Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    print(
        "stdlib baseline summary: "
        f"imports={import_pass_supported}/{host_supported} (host-supported), "
        f"comprehensive pass/fail/timeout={comprehensive_pass}/{comprehensive_fail}/{comprehensive_timeout}, "
        f"total modules={len(stdlib_modules)}, elapsed={elapsed_total}s"
    )
    print(f"wrote {out_path}")


if __name__ == "__main__":
    main()
