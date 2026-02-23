#!/usr/bin/env python3
"""Compute inventory-level language coverage from probe results and probe-to-inventory mappings."""

from __future__ import annotations

import argparse
import json
import re
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def selector_matches(feature: dict[str, Any], selector: dict[str, Any]) -> bool:
    if "kind" in selector and feature.get("kind") != selector["kind"]:
        return False
    if "id" in selector and feature.get("id") != selector["id"]:
        return False
    if "id_prefix" in selector and not feature.get("id", "").startswith(selector["id_prefix"]):
        return False
    if "id_regex" in selector:
        feature_id = feature.get("id", "")
        if re.search(selector["id_regex"], feature_id) is None:
            return False
    if "chapter" in selector and feature.get("chapter") != selector["chapter"]:
        return False
    if "grammar_section" in selector and feature.get("grammar_section") != selector["grammar_section"]:
        return False
    if "title_contains" in selector:
        title = feature.get("title", "")
        needle = selector["title_contains"]
        if selector.get("case_sensitive", False):
            if needle not in title:
                return False
        elif needle.lower() not in title.lower():
            return False
    if "title_regex" in selector:
        title = feature.get("title", "")
        if re.search(selector["title_regex"], title) is None:
            return False
    if "internal" in selector and feature.get("internal") != selector["internal"]:
        return False
    return True


def build_probe_pass_table(probe_results: dict[str, Any]) -> dict[str, bool]:
    table: dict[str, bool] = {}
    for row in probe_results.get("features", []):
        feature_id = row.get("id")
        if isinstance(feature_id, str):
            table[feature_id] = bool(row.get("pass", False))
    return table


def resolve_probe_to_inventory(
    inventory_rows: list[dict[str, Any]],
    mappings: list[dict[str, Any]],
) -> tuple[dict[str, set[str]], dict[str, int]]:
    probe_to_inventory: dict[str, set[str]] = {}
    matched_counts: dict[str, int] = {}

    for mapping in mappings:
        probe_id = mapping["probe_id"]
        selectors = mapping.get("selectors", [])
        matched: set[str] = set()
        for selector in selectors:
            for row in inventory_rows:
                if selector_matches(row, selector):
                    matched.add(row["id"])
        probe_to_inventory[probe_id] = matched
        matched_counts[probe_id] = len(matched)

    return probe_to_inventory, matched_counts


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", required=True, type=Path)
    parser.add_argument("--probe-results", required=True, type=Path)
    parser.add_argument("--probe-map", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    inventory_doc = load_json(args.inventory)
    probe_results = load_json(args.probe_results)
    probe_map = load_json(args.probe_map)

    inventory_rows = inventory_doc.get("features", [])
    probe_pass = build_probe_pass_table(probe_results)
    mappings = probe_map.get("mappings", [])

    known_probe_ids = set(probe_pass.keys())
    mapped_probe_ids = {row.get("probe_id") for row in mappings if isinstance(row.get("probe_id"), str)}

    unknown_probe_ids = sorted(pid for pid in mapped_probe_ids if pid not in known_probe_ids)
    unmapped_probe_ids = sorted(pid for pid in known_probe_ids if pid not in mapped_probe_ids)

    probe_to_inventory, matched_counts = resolve_probe_to_inventory(inventory_rows, mappings)

    inventory_to_probes: dict[str, list[str]] = defaultdict(list)
    for probe_id, ids in probe_to_inventory.items():
        for inventory_id in ids:
            inventory_to_probes[inventory_id].append(probe_id)

    rows: list[dict[str, Any]] = []
    counts = Counter()
    by_kind = Counter()
    by_chapter = Counter()

    for feature in inventory_rows:
        inventory_id = feature["id"]
        probes = sorted(inventory_to_probes.get(inventory_id, []))
        if not probes:
            status = "unprobed"
        else:
            probe_states = [probe_pass.get(pid, False) for pid in probes]
            status = "pass" if all(probe_states) else "fail"

        counts[status] += 1
        by_kind[f"{feature.get('kind','unknown')}::{status}"] += 1
        chapter = feature.get("chapter", "(none)")
        by_chapter[f"{chapter}::{status}"] += 1

        rows.append(
            {
                "id": inventory_id,
                "kind": feature.get("kind"),
                "chapter": chapter,
                "title": feature.get("title"),
                "status": status,
                "covered_by_probes": probes,
            }
        )

    coverage_rows = counts["pass"] + counts["fail"]
    total_rows = len(inventory_rows)
    coverage_pct = round((coverage_rows / total_rows * 100.0) if total_rows else 0.0, 2)

    report = {
        "schema_version": 1,
        "inventory_path": str(args.inventory),
        "probe_results_path": str(args.probe_results),
        "probe_map_path": str(args.probe_map),
        "summary": {
            "inventory_total": total_rows,
            "pass": counts["pass"],
            "fail": counts["fail"],
            "unprobed": counts["unprobed"],
            "coverage_rows": coverage_rows,
            "coverage_percent": coverage_pct,
            "manifest_probe_total": len(known_probe_ids),
            "mapped_probe_total": len(mapped_probe_ids),
            "unknown_probe_ids": unknown_probe_ids,
            "unmapped_probe_ids": unmapped_probe_ids,
        },
        "probe_match_counts": dict(sorted(matched_counts.items())),
        "by_kind_status": dict(sorted(by_kind.items())),
        "by_chapter_status": dict(sorted(by_chapter.items())),
        "rows": rows,
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")

    summary = report["summary"]
    print(
        "language feature coverage: "
        f"pass={summary['pass']} fail={summary['fail']} unprobed={summary['unprobed']} "
        f"(coverage={summary['coverage_percent']}%)"
    )
    if unknown_probe_ids:
        print(f"warning: unknown probe ids in map: {unknown_probe_ids}")
    if unmapped_probe_ids:
        print(f"warning: probes without mapping rows: {unmapped_probe_ids}")
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
