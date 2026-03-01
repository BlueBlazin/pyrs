#!/usr/bin/env python3
"""Summarize native link-attribute blockers for wasm VM bring-up."""

from __future__ import annotations

import argparse
import json
import re
from dataclasses import asdict, dataclass
from pathlib import Path

LINK_ATTR_RE = re.compile(r'#\[link\(name\s*=\s*"([^"]+)"\)\]')
KNOWN_STDLIB_BLOCKER_LIBS = {"bz2", "lzma", "sqlite3", "z"}


@dataclass
class LinkAttrHit:
    file: str
    line: int
    library: str


def scan_link_attributes(root: Path) -> list[LinkAttrHit]:
    hits: list[LinkAttrHit] = []
    for path in sorted(root.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for idx, raw in enumerate(text.splitlines(), start=1):
            line = raw.strip()
            match = LINK_ATTR_RE.search(line)
            if match is None:
                continue
            hits.append(
                LinkAttrHit(file=str(path), line=idx, library=match.group(1))
            )
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

    libraries = sorted({hit.library for hit in hits})
    stdlib_blockers = sorted(set(libraries) & KNOWN_STDLIB_BLOCKER_LIBS)
    rows = [asdict(hit) for hit in hits]

    report = {
        "root": str(root),
        "counts": {
            "link_attr_hits": len(rows),
            "distinct_libraries": len(libraries),
            "known_stdlib_blockers": len(stdlib_blockers),
        },
        "libraries": libraries,
        "known_stdlib_blockers": stdlib_blockers,
        "rows": rows,
    }

    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    print(
        "wasm-vm link blockers: "
        f"hits={len(rows)} distinct_libs={len(libraries)} "
        f"known_stdlib_blockers={len(stdlib_blockers)}"
    )
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
