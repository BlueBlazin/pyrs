#!/usr/bin/env python3
"""Summarize wasm-active native link-attribute blockers for VM bring-up."""

from __future__ import annotations

import argparse
import json
import re
from dataclasses import asdict, dataclass
from enum import Enum
from pathlib import Path

LINK_ATTR_RE = re.compile(r'#\[link\(name\s*=\s*"([^"]+)"\)\]')
TARGET_KV_RE = re.compile(r'^(target_(?:arch|os|family))\s*=\s*"([^"]+)"$')
KNOWN_STDLIB_BLOCKER_LIBS = {"bz2", "lzma", "sqlite3", "z"}


@dataclass
class LinkAttrHit:
    file: str
    line: int
    library: str
    active_on_wasm: bool


class TriState(Enum):
    TRUE = 1
    FALSE = 0
    UNKNOWN = -1


def split_top_level_commas(raw: str) -> list[str]:
    parts: list[str] = []
    depth = 0
    start = 0
    for idx, ch in enumerate(raw):
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth = max(0, depth - 1)
        elif ch == "," and depth == 0:
            parts.append(raw[start:idx].strip())
            start = idx + 1
    tail = raw[start:].strip()
    if tail:
        parts.append(tail)
    return parts


def evaluate_cfg_predicate(raw: str) -> TriState:
    expr = raw.strip()
    if expr == "unix":
        return TriState.FALSE
    if expr == "windows":
        return TriState.FALSE
    if expr == "debug_assertions":
        return TriState.UNKNOWN
    match = TARGET_KV_RE.match(expr)
    if match is None:
        return TriState.UNKNOWN
    key = match.group(1)
    value = match.group(2)
    if key == "target_arch":
        return TriState.TRUE if value == "wasm32" else TriState.FALSE
    if key == "target_os":
        return TriState.TRUE if value == "unknown" else TriState.FALSE
    if key == "target_family":
        return TriState.TRUE if value == "wasm" else TriState.FALSE
    return TriState.UNKNOWN


def invert_tristate(value: TriState) -> TriState:
    if value == TriState.TRUE:
        return TriState.FALSE
    if value == TriState.FALSE:
        return TriState.TRUE
    return TriState.UNKNOWN


def evaluate_cfg_expr(raw: str) -> TriState:
    expr = raw.strip()
    if expr.startswith("cfg(") and expr.endswith(")"):
        return evaluate_cfg_expr(expr[4:-1].strip())
    if expr.startswith("any(") and expr.endswith(")"):
        values = [evaluate_cfg_expr(part) for part in split_top_level_commas(expr[4:-1])]
        if any(value == TriState.TRUE for value in values):
            return TriState.TRUE
        if values and all(value == TriState.FALSE for value in values):
            return TriState.FALSE
        return TriState.UNKNOWN
    if expr.startswith("all(") and expr.endswith(")"):
        values = [evaluate_cfg_expr(part) for part in split_top_level_commas(expr[4:-1])]
        if any(value == TriState.FALSE for value in values):
            return TriState.FALSE
        if values and all(value == TriState.TRUE for value in values):
            return TriState.TRUE
        return TriState.UNKNOWN
    if expr.startswith("not(") and expr.endswith(")"):
        return invert_tristate(evaluate_cfg_expr(expr[4:-1]))
    return evaluate_cfg_predicate(expr)


def parse_cfg_attr_link(attr_line: str) -> tuple[str, TriState] | None:
    if not attr_line.startswith("#[cfg_attr(") or not attr_line.endswith(")]"):
        return None
    inner = attr_line[len("#[cfg_attr(") : -2].strip()
    parts = split_top_level_commas(inner)
    if len(parts) < 2:
        return None
    condition = parts[0]
    attrs = parts[1:]
    for attr in attrs:
        if not attr.startswith("link(") or not attr.endswith(")"):
            continue
        match = re.match(r'^link\(name\s*=\s*"([^"]+)"\)$', attr)
        if match is None:
            continue
        return match.group(1), evaluate_cfg_expr(condition)
    return None


def parse_cfg_attr_condition(attr_line: str) -> TriState | None:
    if not attr_line.startswith("#[cfg(") or not attr_line.endswith(")]"):
        return None
    inner = attr_line[len("#[cfg(") : -2].strip()
    return evaluate_cfg_expr(inner)


def is_extern_block_line(raw_line: str) -> bool:
    stripped = raw_line.strip()
    return stripped.startswith("unsafe extern ") or stripped.startswith("extern ")


def scan_link_attributes(root: Path) -> list[LinkAttrHit]:
    hits: list[LinkAttrHit] = []
    for path in sorted(root.rglob("*.rs")):
        lines = path.read_text(encoding="utf-8").splitlines()
        pending_attrs: list[tuple[int, str]] = []
        collecting_attr_start: int | None = None
        collecting_attr_parts: list[str] = []
        for idx, raw in enumerate(lines, start=1):
            line = raw.strip()
            if collecting_attr_start is not None:
                collecting_attr_parts.append(line)
                if line.endswith("]"):
                    pending_attrs.append(
                        (collecting_attr_start, " ".join(collecting_attr_parts))
                    )
                    collecting_attr_start = None
                    collecting_attr_parts = []
                continue
            if line.startswith("#["):
                if line.endswith("]"):
                    pending_attrs.append((idx, line))
                else:
                    collecting_attr_start = idx
                    collecting_attr_parts = [line]
                continue
            if is_extern_block_line(line):
                cfg_states = [
                    state
                    for _, attr in pending_attrs
                    if (state := parse_cfg_attr_condition(attr)) is not None
                ]
                base_active = not any(state == TriState.FALSE for state in cfg_states)
                for attr_line_no, attr in pending_attrs:
                    match = LINK_ATTR_RE.search(attr)
                    if match is not None:
                        hits.append(
                            LinkAttrHit(
                                file=str(path),
                                line=attr_line_no,
                                library=match.group(1),
                                active_on_wasm=base_active,
                            )
                        )
                        continue
                    cfg_attr_link = parse_cfg_attr_link(attr)
                    if cfg_attr_link is None:
                        continue
                    library, condition = cfg_attr_link
                    active_on_wasm = base_active and condition != TriState.FALSE
                    hits.append(
                        LinkAttrHit(
                            file=str(path),
                            line=attr_line_no,
                            library=library,
                            active_on_wasm=active_on_wasm,
                        )
                    )
                pending_attrs = []
                continue
            if line:
                pending_attrs = []
            else:
                pending_attrs = []
    return hits


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default="src/vm")
    parser.add_argument(
        "--out", default="perf/wasm_vm_link_blockers_latest.json"
    )
    args = parser.parse_args()

    root = Path(args.root)
    out = Path(args.out)
    hits = scan_link_attributes(root)
    active_hits = [hit for hit in hits if hit.active_on_wasm]
    libraries = sorted({hit.library for hit in hits})
    active_libraries = sorted({hit.library for hit in active_hits})
    stdlib_blockers = sorted(set(active_libraries) & KNOWN_STDLIB_BLOCKER_LIBS)
    rows = [asdict(hit) for hit in hits]
    active_rows = [asdict(hit) for hit in active_hits]

    report = {
        "root": str(root),
        "counts": {
            "link_attr_hits": len(rows),
            "wasm_active_link_attr_hits": len(active_rows),
            "distinct_libraries": len(libraries),
            "wasm_active_distinct_libraries": len(active_libraries),
            "known_stdlib_blockers": len(stdlib_blockers),
        },
        "libraries": libraries,
        "wasm_active_libraries": active_libraries,
        "known_stdlib_blockers": stdlib_blockers,
        "rows": rows,
        "wasm_active_rows": active_rows,
    }

    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    print(
        "wasm-vm link blockers: "
        f"hits={len(rows)} active_hits={len(active_rows)} "
        f"distinct_libs={len(libraries)} active_distinct_libs={len(active_libraries)} "
        f"known_stdlib_blockers={len(stdlib_blockers)}"
    )
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
