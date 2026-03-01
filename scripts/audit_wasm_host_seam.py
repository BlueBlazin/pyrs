#!/usr/bin/env python3
"""Audit remaining direct std::env usage under src/vm for wasm host-seam tracking."""

from __future__ import annotations

import argparse
import json
import re
from dataclasses import asdict, dataclass
from pathlib import Path


ENV_PATTERNS = (
    re.compile(r"\bstd::env::var_os\("),
    re.compile(r"\bstd::env::var\("),
    re.compile(r"\bstd::env::args\("),
    re.compile(r"\bstd::env::current_exe\("),
    re.compile(r"\bstd::env::consts::OS\b"),
)


@dataclass
class Hit:
    file: str
    line: int
    snippet: str


def allowlisted_reason(path: Path, snippet: str) -> str | None:
    path_text = str(path)
    if path_text.endswith("src/vm/mod.rs"):
        if snippet == "*slot.get_or_init(|| std::env::var_os(name).is_some())":
            return "central env-probe cache bootstrap"
        if snippet == ".any(|probe| std::env::var_os(probe).is_some())":
            return "central env-probe any-enabled fast-path"
        if snippet == "_ => std::env::var_os(name).is_some(),":
            return "central env-probe unknown-name fallback"
    return None


def iter_hits(root: Path) -> tuple[list[Hit], list[dict[str, str | int]]]:
    hits: list[Hit] = []
    allowlisted: list[dict[str, str | int]] = []
    for path in sorted(root.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for idx, raw in enumerate(text.splitlines(), start=1):
            line = raw.strip()
            if not line:
                continue
            if any(pattern.search(line) for pattern in ENV_PATTERNS):
                reason = allowlisted_reason(path, line)
                if reason is not None:
                    allowlisted.append(
                        {
                            "file": str(path),
                            "line": idx,
                            "snippet": line,
                            "reason": reason,
                        }
                    )
                    continue
                hits.append(Hit(file=str(path), line=idx, snippet=line))
    return hits, allowlisted


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default="src/vm")
    parser.add_argument("--out", default="perf/wasm_host_seam_audit_latest.json")
    args = parser.parse_args()

    root = Path(args.root)
    out = Path(args.out)

    hits, allowlisted = iter_hits(root)
    report = {
        "root": str(root),
        "total_hits": len(hits),
        "allowlisted_hits": len(allowlisted),
        "hits": [asdict(hit) for hit in hits],
        "allowlisted": allowlisted,
    }

    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    print(
        "wasm host seam audit: total_hits="
        f"{len(hits)} (allowlisted_hits={len(allowlisted)})"
    )
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
