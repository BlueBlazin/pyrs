#!/usr/bin/env python3
"""Run the product-facing CPython compatibility benchmark against pyrs."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import pathlib
import platform
import shutil
import subprocess
import sys
import time
from collections import Counter
from typing import Any


SCHEMA_VERSION = "v1"
REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]
WORKER = REPO_ROOT / "scripts" / "cpython_compat_benchmark_worker.py"


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


def run_subprocess(argv: list[str], timeout_secs: int, env: dict[str, str]) -> dict[str, Any]:
    started = time.perf_counter()
    try:
        completed = subprocess.run(
            argv,
            capture_output=True,
            text=True,
            timeout=timeout_secs,
            env=env,
            check=False,
        )
    except subprocess.TimeoutExpired as exc:
        return {
            "ok": False,
            "timeout": True,
            "returncode": None,
            "stdout": (exc.stdout or "").strip(),
            "stderr": (exc.stderr or "").strip(),
            "elapsed_secs": round(time.perf_counter() - started, 6),
        }
    return {
        "ok": completed.returncode == 0,
        "timeout": False,
        "returncode": completed.returncode,
        "stdout": (completed.stdout or "").strip(),
        "stderr": (completed.stderr or "").strip(),
        "elapsed_secs": round(time.perf_counter() - started, 6),
    }


def json_from_output(stdout: str) -> dict[str, Any] | None:
    try:
        payload = json.loads(stdout)
    except json.JSONDecodeError:
        return None
    return payload if isinstance(payload, dict) else None


def discover_entries(cpython_bin: pathlib.Path, cpython_lib: pathlib.Path) -> list[str]:
    code = (
        "import json, sys\n"
        f"sys.path.insert(0, {str(cpython_lib)!r})\n"
        "from test.libregrtest.findtests import findtests, split_test_packages\n"
        "from test.libregrtest.utils import abs_module_name\n"
        f"test_dir = {str(cpython_lib / 'test')!r}\n"
        "entries = split_test_packages(findtests(testdir=test_dir), testdir=test_dir)\n"
        "names = []\n"
        "for entry in entries:\n"
        "    name = abs_module_name(entry, test_dir)\n"
        "    if not name.startswith('test.'):\n"
        "        name = 'test.' + name\n"
        "    names.append(name)\n"
        "names.sort()\n"
        "print(json.dumps({'entries': names}))\n"
    )
    result = run_subprocess([str(cpython_bin), "-S", "-c", code], timeout_secs=60, env=os.environ.copy())
    payload = json_from_output(result["stdout"])
    if not result["ok"] or payload is None or "entries" not in payload:
        raise SystemExit(
            f"failed to discover CPython test entries: {result['stderr'] or result['stdout']}"
        )
    entries = payload["entries"]
    if not isinstance(entries, list) or not all(isinstance(item, str) for item in entries):
        raise SystemExit("invalid CPython entry inventory payload")
    return entries


def worker_argv(
    interpreter: pathlib.Path,
    module_name: str,
    mode: str,
    cpython_lib: pathlib.Path,
    out_path: pathlib.Path | None = None,
) -> list[str]:
    argv = [
        str(interpreter),
        "-S",
        str(WORKER),
        "--mode",
        mode,
        "--module",
        module_name,
        "--sys-path",
        str(cpython_lib),
    ]
    if out_path is not None:
        argv.extend(["--out", str(out_path)])
    return argv


def safe_name(module_name: str) -> str:
    return "".join(ch if ch.isalnum() or ch in "._-" else "_" for ch in module_name)


def write_json(path: pathlib.Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


def git_head() -> str | None:
    try:
        completed = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=False,
            cwd=REPO_ROOT,
        )
    except OSError:
        return None
    if completed.returncode != 0:
        return None
    value = (completed.stdout or "").strip()
    return value or None


def build_metadata(
    *,
    cpython_bin: pathlib.Path,
    cpython_lib: pathlib.Path,
    runner_bin: pathlib.Path,
    jobs: int,
    inventory_timeout_secs: int,
    run_timeout_secs: int,
    inventory_only: bool,
    entries: list[str],
) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "benchmark": "cpython_compat",
        "generated_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "git": {
            "head": git_head(),
        },
        "host": {
            "platform": platform.platform(),
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "python": sys.version.split()[0],
            "cpu_count": os.cpu_count(),
        },
        "config": {
            "cpython_bin": str(cpython_bin),
            "cpython_lib": str(cpython_lib),
            "runner_bin": str(runner_bin),
            "jobs": jobs,
            "inventory_timeout_secs": inventory_timeout_secs,
            "run_timeout_secs": run_timeout_secs,
            "inventory_only": inventory_only,
        },
        "entries": {
            "count": len(entries),
            "names": entries,
        },
    }


def build_progress(
    *,
    metadata: dict[str, Any],
    phase: str,
    entries: list[str],
    inventory_rows: dict[str, dict[str, Any]],
    run_rows: dict[str, dict[str, Any]],
    elapsed_total: float,
    last_completed_module: str | None = None,
) -> dict[str, Any]:
    inventory_statuses = Counter(
        row.get("status", "unknown")
        for row in inventory_rows.values()
    )
    run_statuses = Counter(
        row.get("status", "unknown")
        for row in run_rows.values()
    )
    runnable_entries = sum(
        1 for row in inventory_rows.values() if row.get("status") == "ok"
    )
    return {
        "schema_version": SCHEMA_VERSION,
        "benchmark": "cpython_compat",
        "generated_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "git": metadata["git"],
        "host": metadata["host"],
        "config": metadata["config"],
        "phase": phase,
        "entries_total": len(entries),
        "inventory_completed": len(inventory_rows),
        "runnable_entries": runnable_entries,
        "run_completed": len(run_rows),
        "inventory_statuses": dict(sorted(inventory_statuses.items())),
        "run_statuses": dict(sorted(run_statuses.items())),
        "elapsed_total_secs": round(elapsed_total, 6),
        "last_completed_module": last_completed_module,
    }


def inventory_entry(
    module_name: str,
    cpython_bin: pathlib.Path,
    cpython_lib: pathlib.Path,
    timeout_secs: int,
    inventory_dir: pathlib.Path,
    force: bool,
) -> dict[str, Any]:
    shard = inventory_dir / f"{safe_name(module_name)}.json"
    tmp_path = inventory_dir / f"{safe_name(module_name)}.worker.json"
    if shard.is_file() and not force:
        return json.loads(shard.read_text(encoding="utf-8"))

    result = run_subprocess(
        worker_argv(cpython_bin, module_name, "inventory", cpython_lib, tmp_path),
        timeout_secs=timeout_secs,
        env=os.environ.copy(),
    )
    payload = None
    if tmp_path.is_file():
        payload = json.loads(tmp_path.read_text(encoding="utf-8"))
        tmp_path.unlink(missing_ok=True)
    elif result["stdout"]:
        payload = json_from_output(result["stdout"])
    if result["timeout"]:
        payload = {
            "schema_version": SCHEMA_VERSION,
            "mode": "inventory",
            "module": module_name,
            "status": "inventory_timeout",
            "case_ids": [],
            "case_count": 0,
            "load_state": {
                "status": "inventory_timeout",
                "reason": f"timeout after {timeout_secs}s",
                "error": None,
            },
            "interpreter": {
                "executable": str(cpython_bin),
                "implementation": "cpython",
                "version": None,
            },
        }
    elif payload is None:
        payload = {
            "schema_version": SCHEMA_VERSION,
            "mode": "inventory",
            "module": module_name,
            "status": "inventory_process_error",
            "case_ids": [],
            "case_count": 0,
            "load_state": {
                "status": "inventory_process_error",
                "reason": "worker produced invalid JSON",
                "error": {
                    "type": "InvalidJson",
                    "message": "worker produced invalid JSON",
                    "detail": (result["stderr"] or result["stdout"])[:8000],
                },
            },
            "interpreter": {
                "executable": str(cpython_bin),
                "implementation": "cpython",
                "version": None,
            },
        }
    write_json(shard, payload)
    return payload


def run_entry(
    module_name: str,
    runner_bin: pathlib.Path,
    cpython_lib: pathlib.Path,
    timeout_secs: int,
    results_dir: pathlib.Path,
    force: bool,
) -> dict[str, Any]:
    shard = results_dir / f"{safe_name(module_name)}.json"
    tmp_path = results_dir / f"{safe_name(module_name)}.worker.json"
    if shard.is_file() and not force:
        return json.loads(shard.read_text(encoding="utf-8"))

    env = os.environ.copy()
    env["PYRS_CPYTHON_LIB"] = str(cpython_lib)
    result = run_subprocess(
        worker_argv(runner_bin, module_name, "run", cpython_lib, tmp_path),
        timeout_secs=timeout_secs,
        env=env,
    )
    payload = None
    if tmp_path.is_file():
        payload = json.loads(tmp_path.read_text(encoding="utf-8"))
        tmp_path.unlink(missing_ok=True)
    elif result["stdout"]:
        payload = json_from_output(result["stdout"])
    if result["timeout"]:
        payload = {
            "schema_version": SCHEMA_VERSION,
            "mode": "run",
            "module": module_name,
            "status": "process_timeout",
            "elapsed_secs": result["elapsed_secs"],
            "process": {
                "timeout": True,
                "returncode": None,
                "stdout": result["stdout"][:4000],
                "stderr": result["stderr"][:4000],
            },
            "results": {
                "tests_run": 0,
                "case_records": [],
                "subtest_records": [],
                "fixture_records": [],
                "case_outcomes": {},
                "subtest_outcomes": {},
                "fixture_outcomes": {},
            },
        }
    elif payload is None:
        payload = {
            "schema_version": SCHEMA_VERSION,
            "mode": "run",
            "module": module_name,
            "status": "process_error",
            "elapsed_secs": result["elapsed_secs"],
            "process": {
                "timeout": False,
                "returncode": result["returncode"],
                "stdout": result["stdout"][:4000],
                "stderr": result["stderr"][:4000],
            },
            "results": {
                "tests_run": 0,
                "case_records": [],
                "subtest_records": [],
                "fixture_records": [],
                "case_outcomes": {},
                "subtest_outcomes": {},
                "fixture_outcomes": {},
            },
        }
    else:
        payload["process"] = {
            "timeout": False,
            "returncode": result["returncode"],
            "stdout": result["stdout"][:2000],
            "stderr": result["stderr"][:2000],
        }
    write_json(shard, payload)
    return payload


def summarize(
    out_dir: pathlib.Path,
    metadata: dict[str, Any],
    entries: list[str],
    inventory_rows: dict[str, dict[str, Any]],
    run_rows: dict[str, dict[str, Any]],
    elapsed_total: float,
) -> dict[str, Any]:
    module_statuses = Counter()
    case_outcomes = Counter()
    subtest_outcomes = Counter()
    fixture_outcomes = Counter()
    discoverable_case_total = 0
    host_skip_entries = 0

    index_rows: list[dict[str, Any]] = []
    for module_name in entries:
        inventory = inventory_rows[module_name]
        run = run_rows.get(module_name)
        inventory_status = inventory.get("status", "unknown")
        case_count = int(inventory.get("case_count", 0) or 0)
        if inventory_status == "ok":
            discoverable_case_total += case_count
        elif inventory_status == "host_skip":
            host_skip_entries += 1

        run_status = None
        tests_run = 0
        subtest_count = 0
        if run is not None:
            run_status = run.get("status")
            module_statuses[run_status] += 1
            results = run.get("results", {})
            tests_run = int(results.get("tests_run", 0) or 0)
            subtest_count = len(results.get("subtest_records", []))
            case_outcomes.update(results.get("case_outcomes", {}))
            subtest_outcomes.update(results.get("subtest_outcomes", {}))
            fixture_outcomes.update(results.get("fixture_outcomes", {}))
        index_rows.append(
            {
                "module": module_name,
                "inventory_status": inventory_status,
                "inventory_case_count": case_count,
                "run_status": run_status,
                "tests_run": tests_run,
                "subtest_events": subtest_count,
                "inventory_shard": f"inventory/{safe_name(module_name)}.json",
                "result_shard": f"results/{safe_name(module_name)}.json" if run is not None else None,
            }
        )

    summary = {
        **metadata,
        "paths": {
            "out_dir": str(out_dir),
            "inventory_dir": str(out_dir / "inventory"),
            "results_dir": str(out_dir / "results"),
            "manifest": str(out_dir / "manifest.json"),
        },
        "inventory": {
            "entry_count": len(entries),
            "discoverable_case_count": discoverable_case_total,
            "host_skip_entries": host_skip_entries,
        },
        "results": {
            "module_statuses": dict(sorted(module_statuses.items())),
            "case_outcomes": dict(sorted(case_outcomes.items())),
            "subtest_outcomes": dict(sorted(subtest_outcomes.items())),
            "fixture_outcomes": dict(sorted(fixture_outcomes.items())),
            "executed_entry_count": len(run_rows),
            "executed_case_count": sum(case_outcomes.values()),
            "executed_subtest_count": sum(subtest_outcomes.values()),
            "elapsed_total_secs": round(elapsed_total, 6),
        },
        "entries": index_rows,
    }
    return summary


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the product-facing CPython compatibility benchmark")
    parser.add_argument("--runner-bin", default="target/debug/pyrs", help="Interpreter used for execution runs")
    parser.add_argument("--cpython-bin", default=None, help="CPython 3.14 binary used for inventory/discovery")
    parser.add_argument("--cpython-lib", default=None, help="CPython 3.14 Lib root")
    parser.add_argument("--out-dir", default="perf/cpython_compat_benchmark_latest", help="Output directory for shards and summary")
    parser.add_argument("--inventory-timeout", type=int, default=60, help="Per-entry timeout for inventory collection")
    parser.add_argument("--run-timeout", type=int, default=300, help="Per-entry timeout for execution runs")
    parser.add_argument("--jobs", type=int, default=0, help="Parallel workers (0 = CPU count)")
    parser.add_argument("--max-entries", type=int, default=0, help="Optional entry cap for dry runs")
    parser.add_argument("--entry", action="append", default=[], help="Only run the named entry/module (repeatable)")
    parser.add_argument("--inventory-only", action="store_true", help="Discover entries and inventory, but do not execute them")
    parser.add_argument("--force", action="store_true", help="Recompute shards even if they already exist")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    runner_bin = pathlib.Path(args.runner_bin)
    if not runner_bin.is_file():
        raise SystemExit(f"runner binary not found: {runner_bin}")
    cpython_bin = detect_cpython_bin(args.cpython_bin)
    cpython_lib = detect_cpython_lib(args.cpython_lib)
    if not WORKER.is_file():
        raise SystemExit(f"worker script not found: {WORKER}")

    out_dir = pathlib.Path(args.out_dir)
    inventory_dir = out_dir / "inventory"
    results_dir = out_dir / "results"
    if args.force and out_dir.exists():
        shutil.rmtree(out_dir)
    inventory_dir.mkdir(parents=True, exist_ok=True)
    if not args.inventory_only:
        results_dir.mkdir(parents=True, exist_ok=True)

    entries = discover_entries(cpython_bin, cpython_lib)
    if args.entry:
        selected = set(args.entry)
        entries = [entry for entry in entries if entry in selected]
    if args.max_entries > 0:
        entries = entries[: args.max_entries]
    jobs = args.jobs if args.jobs > 0 else max(1, os.cpu_count() or 1)
    metadata = build_metadata(
        cpython_bin=cpython_bin,
        cpython_lib=cpython_lib,
        runner_bin=runner_bin,
        jobs=jobs,
        inventory_timeout_secs=args.inventory_timeout,
        run_timeout_secs=args.run_timeout,
        inventory_only=args.inventory_only,
        entries=entries,
    )
    manifest = {
        **metadata,
        "status": "running",
        "paths": {
            "out_dir": str(out_dir),
            "inventory_dir": str(inventory_dir),
            "results_dir": str(results_dir),
            "summary": str(out_dir / "summary.json"),
            "progress": str(out_dir / "progress.json"),
        },
    }
    write_json(out_dir / "manifest.json", manifest)

    started = time.perf_counter()
    inventory_rows: dict[str, dict[str, Any]] = {}
    write_json(
        out_dir / "progress.json",
        build_progress(
            metadata=metadata,
            phase="inventory",
            entries=entries,
            inventory_rows=inventory_rows,
            run_rows={},
            elapsed_total=0.0,
        ),
    )
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as executor:
        future_to_module = {
            executor.submit(
                inventory_entry,
                module_name,
                cpython_bin,
                cpython_lib,
                args.inventory_timeout,
                inventory_dir,
                args.force,
            ): module_name
            for module_name in entries
        }
        for future in concurrent.futures.as_completed(future_to_module):
            module_name = future_to_module[future]
            inventory_rows[module_name] = future.result()
            write_json(
                out_dir / "progress.json",
                build_progress(
                    metadata=metadata,
                    phase="inventory",
                    entries=entries,
                    inventory_rows=inventory_rows,
                    run_rows={},
                    elapsed_total=time.perf_counter() - started,
                    last_completed_module=module_name,
                ),
            )
            print(f"[inventory] {module_name}", flush=True)

    run_rows: dict[str, dict[str, Any]] = {}
    if not args.inventory_only:
        runnable_entries = [
            module_name
            for module_name in entries
            if inventory_rows[module_name].get("status") == "ok"
        ]
        write_json(
            out_dir / "progress.json",
            build_progress(
                metadata=metadata,
                phase="run",
                entries=entries,
                inventory_rows=inventory_rows,
                run_rows=run_rows,
                elapsed_total=time.perf_counter() - started,
            ),
        )
        with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as executor:
            future_to_module = {
                executor.submit(
                    run_entry,
                    module_name,
                    runner_bin,
                    cpython_lib,
                    args.run_timeout,
                    results_dir,
                    args.force,
                ): module_name
                for module_name in runnable_entries
            }
            for future in concurrent.futures.as_completed(future_to_module):
                module_name = future_to_module[future]
                run_rows[module_name] = future.result()
                write_json(
                    out_dir / "progress.json",
                    build_progress(
                        metadata=metadata,
                        phase="run",
                        entries=entries,
                        inventory_rows=inventory_rows,
                        run_rows=run_rows,
                        elapsed_total=time.perf_counter() - started,
                        last_completed_module=module_name,
                    ),
                )
                print(f"[run] {module_name} -> {run_rows[module_name].get('status')}", flush=True)

    summary = summarize(
        out_dir=out_dir,
        metadata=metadata,
        entries=entries,
        inventory_rows=inventory_rows,
        run_rows=run_rows,
        elapsed_total=time.perf_counter() - started,
    )
    write_json(out_dir / "summary.json", summary)
    manifest["status"] = "completed"
    manifest["completed_at_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    manifest["results"] = {
        "executed_entry_count": len(run_rows),
        "discoverable_case_count": summary["inventory"]["discoverable_case_count"],
        "executed_case_count": summary["results"]["executed_case_count"],
        "executed_subtest_count": summary["results"]["executed_subtest_count"],
    }
    write_json(out_dir / "manifest.json", manifest)
    write_json(
        out_dir / "progress.json",
        build_progress(
            metadata=metadata,
            phase="completed",
            entries=entries,
            inventory_rows=inventory_rows,
            run_rows=run_rows,
            elapsed_total=time.perf_counter() - started,
        ),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
