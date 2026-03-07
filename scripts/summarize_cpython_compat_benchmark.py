#!/usr/bin/env python3
"""Derive grouped summaries from a CPython compatibility benchmark directory."""

from __future__ import annotations

import argparse
import json
import pathlib
import time
from collections import Counter
from typing import Any


SCHEMA_VERSION = "v1"
MAX_SIGNATURE_LEN = 240


def read_json(path: pathlib.Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: pathlib.Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


def signature_from_detail(detail: str | None, fallback: str) -> str:
    if not detail:
        return fallback
    lines = [line.strip() for line in detail.splitlines() if line.strip()]
    if not lines:
        return fallback
    signature = lines[-1]
    if len(signature) > MAX_SIGNATURE_LEN:
        signature = f"{signature[:MAX_SIGNATURE_LEN]}..."
    return signature


def ranked_counter(counter: Counter[str], top: int) -> list[dict[str, Any]]:
    rows = [
        {"name": name, "count": count}
        for name, count in counter.most_common(top)
    ]
    return rows


def ranked_records(
    records: list[dict[str, Any]],
    *,
    key: str,
    top: int,
) -> list[dict[str, Any]]:
    scored = [record for record in records if isinstance(record.get(key), (int, float))]
    scored.sort(key=lambda item: float(item[key]), reverse=True)
    return scored[:top]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Derive grouped summaries from a CPython compatibility benchmark directory")
    parser.add_argument("--benchmark-dir", required=True, help="Benchmark output directory containing summary.json")
    parser.add_argument("--out", default=None, help="Optional output path (default: <benchmark-dir>/derived_summary.json)")
    parser.add_argument("--top", type=int, default=20, help="Number of ranked rows to keep per section")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    benchmark_dir = pathlib.Path(args.benchmark_dir)
    summary_path = benchmark_dir / "summary.json"
    if not summary_path.is_file():
        raise SystemExit(f"summary.json not found under {benchmark_dir}")
    summary = read_json(summary_path)

    module_nonpass = Counter()
    case_signatures = Counter()
    subtest_signatures = Counter()
    fixture_signatures = Counter()
    slow_cases: list[dict[str, Any]] = []
    slow_subtests: list[dict[str, Any]] = []

    for entry in summary.get("entries", []):
        shard_rel = entry.get("result_shard")
        if not shard_rel:
            continue
        shard_path = benchmark_dir / shard_rel
        if not shard_path.is_file():
            continue
        shard = read_json(shard_path)
        results = shard.get("results", {})
        module_name = shard.get("module", entry.get("module", "<unknown>"))

        case_records = results.get("case_records", [])
        subtest_records = results.get("subtest_records", [])
        fixture_records = results.get("fixture_records", [])

        nonpass_count = 0
        for record in case_records:
            outcome = record.get("outcome")
            if outcome != "passed":
                nonpass_count += 1
            if outcome not in {"passed", None}:
                case_signatures[signature_from_detail(record.get("detail"), str(outcome))] += 1
        for record in subtest_records:
            outcome = record.get("outcome")
            if outcome not in {"passed", None}:
                subtest_signatures[signature_from_detail(record.get("detail"), str(outcome))] += 1
        for record in fixture_records:
            outcome = record.get("outcome")
            if outcome not in {"passed", None}:
                fixture_signatures[signature_from_detail(record.get("detail"), str(outcome))] += 1
                nonpass_count += 1
        if nonpass_count:
            module_nonpass[module_name] += nonpass_count

        for record in case_records:
            if isinstance(record.get("duration_secs"), (int, float)):
                slow_cases.append(
                    {
                        "module": module_name,
                        "id": record.get("id"),
                        "outcome": record.get("outcome"),
                        "duration_secs": record.get("duration_secs"),
                    }
                )
        for record in subtest_records:
            if isinstance(record.get("duration_secs"), (int, float)):
                slow_subtests.append(
                    {
                        "module": module_name,
                        "id": record.get("id"),
                        "parent_id": record.get("parent_id"),
                        "outcome": record.get("outcome"),
                        "duration_secs": record.get("duration_secs"),
                    }
                )

    payload = {
        "schema_version": SCHEMA_VERSION,
        "benchmark": "cpython_compat",
        "generated_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "source": {
            "benchmark_dir": str(benchmark_dir),
            "summary": str(summary_path),
        },
        "run_state": summary.get("run_state"),
        "totals": {
            "module_statuses": summary.get("results", {}).get("module_statuses", {}),
            "case_outcomes": summary.get("results", {}).get("case_outcomes", {}),
            "subtest_outcomes": summary.get("results", {}).get("subtest_outcomes", {}),
            "fixture_outcomes": summary.get("results", {}).get("fixture_outcomes", {}),
            "discoverable_case_count": summary.get("inventory", {}).get("discoverable_case_count", 0),
            "executed_case_count": summary.get("results", {}).get("executed_case_count", 0),
            "executed_subtest_count": summary.get("results", {}).get("executed_subtest_count", 0),
        },
        "top_modules_by_nonpass": ranked_counter(module_nonpass, args.top),
        "failure_signatures": {
            "case": ranked_counter(case_signatures, args.top),
            "subtest": ranked_counter(subtest_signatures, args.top),
            "fixture": ranked_counter(fixture_signatures, args.top),
        },
        "slowest_cases": ranked_records(slow_cases, key="duration_secs", top=args.top),
        "slowest_subtests": ranked_records(slow_subtests, key="duration_secs", top=args.top),
    }

    out_path = pathlib.Path(args.out) if args.out else benchmark_dir / "derived_summary.json"
    write_json(out_path, payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
