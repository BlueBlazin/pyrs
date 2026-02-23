#!/usr/bin/env python3
"""Generate a CPython 3.14 source-language inventory from canonical sources."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any

REFERENCE_FILES = [
    "Doc/reference/lexical_analysis.rst",
    "Doc/reference/simple_stmts.rst",
    "Doc/reference/compound_stmts.rst",
    "Doc/reference/expressions.rst",
    "Doc/reference/toplevel_components.rst",
    "Doc/reference/import.rst",
    "Doc/reference/executionmodel.rst",
    "Doc/reference/datamodel.rst",
]

HEADING_CHARS = "*= -^\"~+#`:".replace(" ", "")
HEADING_LEVEL = {ch: idx for idx, ch in enumerate(HEADING_CHARS)}


def slugify(text: str) -> str:
    lowered = text.lower()
    lowered = re.sub(r"[^a-z0-9]+", "-", lowered)
    lowered = re.sub(r"-+", "-", lowered).strip("-")
    return lowered or "section"


def normalize_title(title: str) -> str:
    out = title
    out = re.sub(r":\w+:`([^`]+)`", r"\1", out)
    out = out.replace("`", "")
    out = out.replace("\\", "")
    out = re.sub(r"\s+", " ", out).strip()
    return out


def parse_rst_headings(path: Path) -> list[dict[str, Any]]:
    lines = path.read_text(encoding="utf-8").splitlines()
    features: list[dict[str, Any]] = []
    stack: list[tuple[int, str]] = []

    for idx in range(len(lines) - 1):
        raw_title = lines[idx]
        underline = lines[idx + 1]
        title = raw_title.strip()
        if not title or title.startswith(".."):
            continue

        underline_stripped = underline.strip()
        if not underline_stripped:
            continue
        if len(underline_stripped) < len(title):
            continue
        if len(set(underline_stripped)) != 1:
            continue

        ch = underline_stripped[0]
        if ch not in HEADING_LEVEL:
            continue
        if not re.fullmatch(re.escape(ch) + r"+", underline_stripped):
            continue

        normalized = normalize_title(title)
        level = HEADING_LEVEL[ch]
        while stack and stack[-1][0] >= level:
            stack.pop()
        stack.append((level, normalized))

        line_no = idx + 1
        stem = path.stem
        feature_id = f"ref::{stem}::L{line_no}::{slugify(normalized)}"
        section_path = [part for _, part in stack]
        features.append(
            {
                "id": feature_id,
                "kind": "reference_heading",
                "title": normalized,
                "source": {
                    "file": str(path),
                    "line": line_no,
                },
                "chapter": stem,
                "heading_level_char": ch,
                "section_path": section_path,
            }
        )

    return features


def parse_grammar_rules(path: Path) -> list[dict[str, Any]]:
    lines = path.read_text(encoding="utf-8").splitlines()
    features: list[dict[str, Any]] = []
    for idx, line in enumerate(lines, start=1):
        match = re.match(r"^([A-Za-z_][A-Za-z0-9_]*)(?:\[[^\]]*\])?:", line)
        if not match:
            continue
        name = match.group(1)
        features.append(
            {
                "id": f"grammar::{name}",
                "kind": "grammar_rule",
                "title": name,
                "source": {
                    "file": str(path),
                    "line": idx,
                },
                "internal": name.startswith("invalid_"),
            }
        )
    return features


def parse_tokens(path: Path) -> list[dict[str, Any]]:
    lines = path.read_text(encoding="utf-8").splitlines()
    features: list[dict[str, Any]] = []
    for idx, line in enumerate(lines, start=1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        token = stripped.split()[0]
        if not re.fullmatch(r"[A-Z_][A-Z0-9_]*", token):
            continue
        features.append(
            {
                "id": f"token::{token}",
                "kind": "token",
                "title": token,
                "source": {
                    "file": str(path),
                    "line": idx,
                },
            }
        )
    return features


def summarize(features: list[dict[str, Any]], manifest: dict[str, Any]) -> dict[str, Any]:
    by_kind: dict[str, int] = {}
    for feature in features:
        by_kind[feature["kind"]] = by_kind.get(feature["kind"], 0) + 1

    grammar_public = sum(
        1
        for feature in features
        if feature["kind"] == "grammar_rule" and not feature.get("internal", False)
    )
    grammar_internal = sum(
        1
        for feature in features
        if feature["kind"] == "grammar_rule" and feature.get("internal", False)
    )

    manifest_features = manifest.get("features", [])
    manifest_required = [row for row in manifest_features if row.get("required", False)]

    return {
        "inventory_total": len(features),
        "by_kind": by_kind,
        "grammar_public_rules": grammar_public,
        "grammar_internal_rules": grammar_internal,
        "manifest_probe_total": len(manifest_features),
        "manifest_required_probe_total": len(manifest_required),
        "manifest_vs_inventory_required_ratio": round(
            (len(manifest_required) / len(features) * 100.0) if features else 0.0,
            2,
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--cpython-root",
        required=True,
        type=Path,
        help="Path to CPython source root (e.g. /path/to/Python-3.14.3)",
    )
    parser.add_argument(
        "--manifest",
        required=True,
        type=Path,
        help="Path to docs/LANGUAGE_FEATURE_MANIFEST.json",
    )
    parser.add_argument(
        "--out-inventory",
        required=True,
        type=Path,
        help="Output inventory JSON path",
    )
    parser.add_argument(
        "--out-report",
        required=True,
        type=Path,
        help="Output report JSON path",
    )
    args = parser.parse_args()

    root = args.cpython_root
    grammar_path = root / "Grammar/python.gram"
    tokens_path = root / "Grammar/Tokens"

    features: list[dict[str, Any]] = []
    features.extend(parse_grammar_rules(grammar_path))
    features.extend(parse_tokens(tokens_path))
    for rel in REFERENCE_FILES:
        features.extend(parse_rst_headings(root / rel))

    features.sort(key=lambda row: row["id"])

    inventory = {
        "schema_version": 1,
        "target": "CPython 3.14 source-language inventory",
        "cpython_root": str(root),
        "sources": {
            "grammar": str(grammar_path),
            "tokens": str(tokens_path),
            "reference_files": [str(root / rel) for rel in REFERENCE_FILES],
        },
        "features": features,
    }

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    report = {
        "schema_version": 1,
        "inventory_path": str(args.out_inventory),
        "manifest_path": str(args.manifest),
        "summary": summarize(features, manifest),
    }

    args.out_inventory.parent.mkdir(parents=True, exist_ok=True)
    args.out_report.parent.mkdir(parents=True, exist_ok=True)
    args.out_inventory.write_text(json.dumps(inventory, indent=2, sort_keys=True), encoding="utf-8")
    args.out_report.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")

    summary = report["summary"]
    print(
        "language inventory: "
        f"{summary['inventory_total']} rows "
        f"(grammar_public={summary['grammar_public_rules']}, "
        f"grammar_internal={summary['grammar_internal_rules']}, "
        f"tokens={summary['by_kind'].get('token', 0)}, "
        f"reference={summary['by_kind'].get('reference_heading', 0)})"
    )
    print(
        "manifest coverage baseline: "
        f"required_probes={summary['manifest_required_probe_total']} "
        f"({summary['manifest_vs_inventory_required_ratio']}% of inventory rows)"
    )
    print(f"wrote {args.out_inventory}")
    print(f"wrote {args.out_report}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
