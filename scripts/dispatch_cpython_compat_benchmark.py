#!/usr/bin/env python3
"""Dispatch the CPython compatibility benchmark across multiple batch runs."""

from __future__ import annotations

import argparse
import json
import math
import os
import pathlib
import shutil
import subprocess
import sys
import time
from collections import Counter
from typing import Any

import run_cpython_compat_benchmark as benchmark


SCHEMA_VERSION = benchmark.SCHEMA_VERSION
REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]
ORCHESTRATOR = REPO_ROOT / "scripts" / "run_cpython_compat_benchmark.py"
SUMMARIZER = REPO_ROOT / "scripts" / "summarize_cpython_compat_benchmark.py"


def write_json(path: pathlib.Path, payload: dict[str, Any]) -> None:
    benchmark.write_json(path, payload)


def read_json(path: pathlib.Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Dispatch the product-facing CPython compatibility benchmark across multiple batch runs"
    )
    parser.add_argument("--runner-bin", default="target/release/pyrs", help="Interpreter used for execution runs")
    parser.add_argument("--cpython-bin", default=None, help="CPython 3.14 binary used for discovery/inventory")
    parser.add_argument("--cpython-lib", default=None, help="CPython 3.14 Lib root")
    parser.add_argument(
        "--out-dir",
        default="perf/cpython_compat_benchmark_latest",
        help="Output directory for top-level summary plus nested batch runs",
    )
    parser.add_argument("--inventory-timeout", type=int, default=60, help="Per-entry timeout for inventory collection")
    parser.add_argument("--run-timeout", type=int, default=300, help="Per-entry timeout for execution runs")
    parser.add_argument("--jobs", type=int, default=0, help="Parallel workers passed through to each batch run (0 = CPU count)")
    parser.add_argument("--max-entries", type=int, default=0, help="Optional global entry cap for dry runs")
    parser.add_argument("--entry", action="append", default=[], help="Only include the named entry/module (repeatable)")
    parser.add_argument("--entry-file", action="append", default=[], help="Read explicit entry/module names from a newline-delimited file (repeatable)")
    parser.add_argument("--allow-missing-entries", action="store_true", help="Continue when explicit entry names are not discoverable on this host")
    parser.add_argument("--inventory-only", action="store_true", help="Discover entries and inventory, but do not execute them")
    parser.add_argument("--force", action="store_true", help="Recompute top-level and nested batch directories even if they already exist")
    batch_group = parser.add_mutually_exclusive_group()
    batch_group.add_argument("--entries-per-batch", type=int, default=25, help="Target number of entries per batch")
    batch_group.add_argument("--batch-count", type=int, default=0, help="Split the selected entries into this many contiguous batches")
    parser.add_argument("--skip-derived-summary", action="store_true", help="Skip the top-level derived summary rollup")
    return parser.parse_args()


def partition_entries(
    entries: list[str],
    *,
    entries_per_batch: int,
    batch_count: int,
) -> list[list[str]]:
    if not entries:
        raise SystemExit("no benchmark entries selected after applying filters")
    if batch_count > 0:
        chunk_size = max(1, math.ceil(len(entries) / batch_count))
    else:
        chunk_size = max(1, entries_per_batch)
    return [
        entries[index : index + chunk_size]
        for index in range(0, len(entries), chunk_size)
    ]


def build_dispatch_metadata(
    *,
    cpython_bin: pathlib.Path,
    cpython_lib: pathlib.Path,
    runner_bin: pathlib.Path,
    jobs: int,
    inventory_timeout_secs: int,
    run_timeout_secs: int,
    inventory_only: bool,
    requested_entries: list[str],
    requested_entry_files: list[str],
    unmatched_requested_entries: list[str],
    max_entries: int,
    allow_missing_entries: bool,
    discovered_entry_count: int,
    entries: list[str],
    planned_batches: list[dict[str, Any]],
    entries_per_batch: int,
    batch_count_requested: int,
    skip_derived_summary: bool,
) -> dict[str, Any]:
    metadata = benchmark.build_metadata(
        cpython_bin=cpython_bin,
        cpython_lib=cpython_lib,
        runner_bin=runner_bin,
        jobs=jobs,
        inventory_timeout_secs=inventory_timeout_secs,
        run_timeout_secs=run_timeout_secs,
        inventory_only=inventory_only,
        requested_entries=requested_entries,
        requested_entry_files=requested_entry_files,
        unmatched_requested_entries=unmatched_requested_entries,
        max_entries=max_entries,
        allow_missing_entries=allow_missing_entries,
        discovered_entry_count=discovered_entry_count,
        entries=entries,
    )
    metadata["dispatch"] = {
        "planned_batch_count": len(planned_batches),
        "batch_count_requested": batch_count_requested if batch_count_requested > 0 else None,
        "entries_per_batch": entries_per_batch,
        "skip_derived_summary": skip_derived_summary,
    }
    return metadata


def build_plan_rows(
    *,
    out_dir: pathlib.Path,
    batches: list[list[str]],
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for index, batch_entries in enumerate(batches):
        batch_id = f"batch-{index:03d}"
        entry_file = out_dir / "entry_files" / f"{batch_id}.txt"
        batch_out_dir = out_dir / "batches" / batch_id
        rows.append(
            {
                "batch_id": batch_id,
                "entry_count": len(batch_entries),
                "entries": batch_entries,
                "entry_file": str(entry_file),
                "out_dir": str(batch_out_dir),
                "summary": f"batches/{batch_id}/summary.json",
                "derived_summary": f"batches/{batch_id}/derived_summary.json",
            }
        )
    return rows


def build_progress(
    *,
    status: str,
    phase: str,
    metadata: dict[str, Any],
    plan_rows: list[dict[str, Any]],
    batch_rows: dict[str, dict[str, Any]],
    elapsed_total: float,
    current_batch_id: str | None = None,
    error: dict[str, Any] | None = None,
) -> dict[str, Any]:
    status_counts = Counter(row.get("status", "pending") for row in batch_rows.values())
    completed_count = sum(
        1
        for row in batch_rows.values()
        if row.get("status") in {"completed", "failed", "interrupted", "process_failed"}
    )
    return {
        "schema_version": SCHEMA_VERSION,
        "benchmark": "cpython_compat",
        "status": status,
        "phase": phase,
        "entries_total": metadata["entries"]["count"],
        "batches_total": len(plan_rows),
        "batches_completed": completed_count,
        "batch_statuses": dict(sorted(status_counts.items())),
        "current_batch_id": current_batch_id,
        "elapsed_total_secs": round(elapsed_total, 6),
        "error": error,
    }


def batch_argv(
    *,
    cpython_bin: pathlib.Path,
    runner_bin: pathlib.Path,
    cpython_lib: pathlib.Path,
    entry_file: pathlib.Path,
    out_dir: pathlib.Path,
    inventory_timeout_secs: int,
    run_timeout_secs: int,
    jobs: int,
    inventory_only: bool,
    force: bool,
) -> list[str]:
    argv = [
        str(cpython_bin),
        str(ORCHESTRATOR),
        "--runner-bin",
        str(runner_bin),
        "--cpython-bin",
        str(cpython_bin),
        "--cpython-lib",
        str(cpython_lib),
        "--entry-file",
        str(entry_file),
        "--out-dir",
        str(out_dir),
        "--inventory-timeout",
        str(inventory_timeout_secs),
        "--run-timeout",
        str(run_timeout_secs),
        "--jobs",
        str(jobs),
    ]
    if inventory_only:
        argv.append("--inventory-only")
    if force:
        argv.append("--force")
    return argv


def aggregate_summary(
    *,
    out_dir: pathlib.Path,
    metadata: dict[str, Any],
    run_state: dict[str, Any],
    plan_rows: list[dict[str, Any]],
    elapsed_total: float,
) -> dict[str, Any]:
    module_statuses = Counter()
    case_outcomes = Counter()
    subtest_outcomes = Counter()
    fixture_outcomes = Counter()
    discoverable_case_total = 0
    host_skip_entries = 0
    aggregate_batch_elapsed = 0.0

    entry_rows: list[dict[str, Any]] = []
    batch_rows: list[dict[str, Any]] = []
    for plan_row in plan_rows:
        batch_id = plan_row["batch_id"]
        batch_out_dir = out_dir / "batches" / batch_id
        summary_path = batch_out_dir / "summary.json"
        if not summary_path.is_file():
            batch_rows.append(
                {
                    "batch_id": batch_id,
                    "status": "missing_summary",
                    "entry_count": plan_row["entry_count"],
                    "summary": plan_row["summary"],
                    "derived_summary": plan_row["derived_summary"],
                }
            )
            continue

        batch_summary = read_json(summary_path)
        batch_run_state = batch_summary.get("run_state", {})
        batch_rows.append(
            {
                "batch_id": batch_id,
                "status": batch_run_state.get("status"),
                "entry_count": plan_row["entry_count"],
                "summary": plan_row["summary"],
                "derived_summary": plan_row["derived_summary"],
            }
        )
        discoverable_case_total += int(batch_summary.get("inventory", {}).get("discoverable_case_count", 0) or 0)
        host_skip_entries += int(batch_summary.get("inventory", {}).get("host_skip_entries", 0) or 0)
        aggregate_batch_elapsed += float(batch_summary.get("results", {}).get("elapsed_total_secs", 0.0) or 0.0)
        module_statuses.update(batch_summary.get("results", {}).get("module_statuses", {}))
        case_outcomes.update(batch_summary.get("results", {}).get("case_outcomes", {}))
        subtest_outcomes.update(batch_summary.get("results", {}).get("subtest_outcomes", {}))
        fixture_outcomes.update(batch_summary.get("results", {}).get("fixture_outcomes", {}))

        for entry in batch_summary.get("entries", []):
            row = dict(entry)
            inventory_shard = row.get("inventory_shard")
            if inventory_shard:
                row["inventory_shard"] = f"batches/{batch_id}/{inventory_shard}"
            result_shard = row.get("result_shard")
            if result_shard:
                row["result_shard"] = f"batches/{batch_id}/{result_shard}"
            row["batch_id"] = batch_id
            entry_rows.append(row)

    summary = {
        **metadata,
        "run_state": run_state,
        "paths": {
            "out_dir": str(out_dir),
            "manifest": str(out_dir / "manifest.json"),
            "plan": str(out_dir / "plan.json"),
            "progress": str(out_dir / "progress.json"),
            "batches_dir": str(out_dir / "batches"),
            "entry_files_dir": str(out_dir / "entry_files"),
        },
        "inventory": {
            "entry_count": metadata["entries"]["count"],
            "discoverable_case_count": discoverable_case_total,
            "host_skip_entries": host_skip_entries,
        },
        "results": {
            "module_statuses": dict(sorted(module_statuses.items())),
            "case_outcomes": dict(sorted(case_outcomes.items())),
            "subtest_outcomes": dict(sorted(subtest_outcomes.items())),
            "fixture_outcomes": dict(sorted(fixture_outcomes.items())),
            "executed_entry_count": len(entry_rows),
            "executed_case_count": sum(case_outcomes.values()),
            "executed_subtest_count": sum(subtest_outcomes.values()),
            "elapsed_total_secs": round(elapsed_total, 6),
        },
        "dispatch": {
            **metadata["dispatch"],
            "aggregate_batch_elapsed_secs": round(aggregate_batch_elapsed, 6),
            "completed_batch_count": sum(1 for row in batch_rows if row.get("status") == "completed"),
        },
        "batches": batch_rows,
        "entries": entry_rows,
    }
    return summary


def finalize_dispatch(
    *,
    out_dir: pathlib.Path,
    metadata: dict[str, Any],
    plan_rows: list[dict[str, Any]],
    batch_rows: dict[str, dict[str, Any]],
    started: float,
    status: str,
    phase: str,
    error: dict[str, Any] | None,
    current_batch_id: str | None,
    cpython_bin: pathlib.Path,
    skip_derived_summary: bool,
) -> dict[str, Any]:
    completed_at_utc = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    elapsed_total = time.perf_counter() - started
    run_state = {
        "status": status,
        "phase": phase,
        "completed_at_utc": completed_at_utc,
        "elapsed_total_secs": round(elapsed_total, 6),
        "error": error,
    }
    summary = aggregate_summary(
        out_dir=out_dir,
        metadata=metadata,
        run_state=run_state,
        plan_rows=plan_rows,
        elapsed_total=elapsed_total,
    )
    write_json(out_dir / "summary.json", summary)
    manifest = {
        **metadata,
        "status": status,
        "phase": phase,
        "completed_at_utc": completed_at_utc,
        "error": error,
        "paths": {
            "out_dir": str(out_dir),
            "plan": str(out_dir / "plan.json"),
            "progress": str(out_dir / "progress.json"),
            "summary": str(out_dir / "summary.json"),
            "batches_dir": str(out_dir / "batches"),
        },
        "batches": [
            {
                "batch_id": row["batch_id"],
                "status": batch_rows.get(row["batch_id"], {}).get("status", "pending"),
                "entry_count": row["entry_count"],
                "summary": row["summary"],
            }
            for row in plan_rows
        ],
        "results": {
            "executed_entry_count": summary["results"]["executed_entry_count"],
            "discoverable_case_count": summary["inventory"]["discoverable_case_count"],
            "executed_case_count": summary["results"]["executed_case_count"],
            "executed_subtest_count": summary["results"]["executed_subtest_count"],
        },
    }
    write_json(out_dir / "manifest.json", manifest)
    write_json(
        out_dir / "progress.json",
        build_progress(
            status=status,
            phase=phase,
            metadata=metadata,
            plan_rows=plan_rows,
            batch_rows=batch_rows,
            elapsed_total=elapsed_total,
            current_batch_id=current_batch_id,
            error=error,
        ),
    )
    if not skip_derived_summary:
        completed = subprocess.run(
            [str(cpython_bin), str(SUMMARIZER), "--benchmark-dir", str(out_dir)],
            capture_output=True,
            text=True,
            check=False,
        )
        if completed.returncode != 0:
            raise SystemExit(
                "failed to derive top-level benchmark summary: "
                f"{(completed.stderr or completed.stdout).strip()}"
            )
    return summary


def main() -> int:
    args = parse_args()
    runner_bin = pathlib.Path(args.runner_bin)
    if not runner_bin.is_file():
        raise SystemExit(f"runner binary not found: {runner_bin}")
    cpython_bin = benchmark.detect_cpython_bin(args.cpython_bin)
    cpython_lib = benchmark.detect_cpython_lib(args.cpython_lib)
    if not ORCHESTRATOR.is_file():
        raise SystemExit(f"benchmark orchestrator not found: {ORCHESTRATOR}")
    if not SUMMARIZER.is_file():
        raise SystemExit(f"benchmark summarizer not found: {SUMMARIZER}")

    out_dir = pathlib.Path(args.out_dir)
    if args.force and out_dir.exists():
        shutil.rmtree(out_dir)
    (out_dir / "batches").mkdir(parents=True, exist_ok=True)
    (out_dir / "entry_files").mkdir(parents=True, exist_ok=True)

    discovered_entries = benchmark.discover_entries(cpython_bin, cpython_lib)
    requested_entry_files = [str(pathlib.Path(path)) for path in args.entry_file]
    requested_entries = list(args.entry)
    for entry_file in args.entry_file:
        requested_entries.extend(benchmark.read_entry_file(pathlib.Path(entry_file)))
    requested_entries = benchmark.unique_preserving_order(requested_entries)
    entries, unmatched_requested_entries = benchmark.resolve_selected_entries(
        discovered_entries=discovered_entries,
        requested_entries=requested_entries,
        max_entries=args.max_entries,
        allow_missing_entries=args.allow_missing_entries,
    )
    jobs = args.jobs if args.jobs > 0 else max(1, os.cpu_count() or 1)
    batches = partition_entries(
        entries,
        entries_per_batch=args.entries_per_batch,
        batch_count=args.batch_count,
    )
    plan_rows = build_plan_rows(out_dir=out_dir, batches=batches)
    metadata = build_dispatch_metadata(
        cpython_bin=cpython_bin,
        cpython_lib=cpython_lib,
        runner_bin=runner_bin,
        jobs=jobs,
        inventory_timeout_secs=args.inventory_timeout,
        run_timeout_secs=args.run_timeout,
        inventory_only=args.inventory_only,
        requested_entries=requested_entries,
        requested_entry_files=requested_entry_files,
        unmatched_requested_entries=unmatched_requested_entries,
        max_entries=args.max_entries,
        allow_missing_entries=args.allow_missing_entries,
        discovered_entry_count=len(discovered_entries),
        entries=entries,
        planned_batches=plan_rows,
        entries_per_batch=args.entries_per_batch,
        batch_count_requested=args.batch_count,
        skip_derived_summary=args.skip_derived_summary,
    )

    for plan_row in plan_rows:
        entry_file = pathlib.Path(plan_row["entry_file"])
        entry_file.write_text("\n".join(plan_row["entries"]) + "\n", encoding="utf-8")
    write_json(
        out_dir / "plan.json",
        {
            **metadata,
            "status": "planned",
            "phase": "planned",
            "paths": {
                "out_dir": str(out_dir),
                "batches_dir": str(out_dir / "batches"),
                "entry_files_dir": str(out_dir / "entry_files"),
                "summary": str(out_dir / "summary.json"),
                "progress": str(out_dir / "progress.json"),
            },
            "batches": plan_rows,
        },
    )

    started = time.perf_counter()
    batch_rows: dict[str, dict[str, Any]] = {}
    write_json(
        out_dir / "progress.json",
        build_progress(
            status="running",
            phase="dispatch",
            metadata=metadata,
            plan_rows=plan_rows,
            batch_rows=batch_rows,
            elapsed_total=0.0,
        ),
    )

    error: dict[str, Any] | None = None
    current_batch_id: str | None = None
    final_status = "completed"
    final_phase = "completed"
    try:
        for plan_row in plan_rows:
            current_batch_id = plan_row["batch_id"]
            batch_rows[current_batch_id] = {
                "status": "running",
                "entry_count": plan_row["entry_count"],
            }
            write_json(
                out_dir / "progress.json",
                build_progress(
                    status="running",
                    phase="dispatch",
                    metadata=metadata,
                    plan_rows=plan_rows,
                    batch_rows=batch_rows,
                    elapsed_total=time.perf_counter() - started,
                    current_batch_id=current_batch_id,
                ),
            )
            batch_out_dir = pathlib.Path(plan_row["out_dir"])
            completed = subprocess.run(
                batch_argv(
                    cpython_bin=cpython_bin,
                    runner_bin=runner_bin,
                    cpython_lib=cpython_lib,
                    entry_file=pathlib.Path(plan_row["entry_file"]),
                    out_dir=batch_out_dir,
                    inventory_timeout_secs=args.inventory_timeout,
                    run_timeout_secs=args.run_timeout,
                    jobs=jobs,
                    inventory_only=args.inventory_only,
                    force=args.force,
                ),
                capture_output=True,
                text=True,
                check=False,
            )
            status = "completed"
            summary_path = batch_out_dir / "summary.json"
            if summary_path.is_file():
                batch_summary = read_json(summary_path)
                status = batch_summary.get("run_state", {}).get("status", status)
            if completed.returncode != 0:
                status = status if status != "completed" else "process_failed"
                batch_rows[current_batch_id] = {
                    "status": status,
                    "entry_count": plan_row["entry_count"],
                    "returncode": completed.returncode,
                }
                error = {
                    "type": "BatchDispatchError",
                    "message": f"batch {current_batch_id} failed",
                    "detail": benchmark.truncate_detail(
                        (completed.stderr or completed.stdout or "").strip()
                    ),
                }
                final_status = "failed"
                final_phase = "dispatch"
                break
            batch_rows[current_batch_id] = {
                "status": status,
                "entry_count": plan_row["entry_count"],
                "returncode": completed.returncode,
            }
            print(f"[dispatch] {current_batch_id} -> {status}", flush=True)
    except KeyboardInterrupt as exc:
        final_status = "interrupted"
        final_phase = "dispatch"
        error = benchmark.exception_payload(exc)
    except Exception as exc:  # noqa: BLE001 - preserve top-level failure in manifest/summary
        final_status = "failed"
        final_phase = "dispatch"
        error = benchmark.exception_payload(exc)

    finalize_dispatch(
        out_dir=out_dir,
        metadata=metadata,
        plan_rows=plan_rows,
        batch_rows=batch_rows,
        started=started,
        status=final_status,
        phase=final_phase,
        error=error,
        current_batch_id=current_batch_id,
        cpython_bin=cpython_bin,
        skip_derived_summary=args.skip_derived_summary,
    )
    if final_status == "interrupted":
        return 130
    if final_status == "failed":
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
