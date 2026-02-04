#!/usr/bin/env python3
"""Sync vendored CPython 3.14 artifacts from a local CPython checkout.

Usage:
  python3 scripts/sync_cpython.py /path/to/cpython --version 3.14.x
"""

from __future__ import annotations

import argparse
import pathlib
import shutil
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
VENDOR = ROOT / "vendor" / "cpython-3.14"

REQUIRED = {
    "grammar/python.gram": "Grammar/python.gram",
    "grammar/Tokens": "Grammar/Tokens",
    "opcode/opcode.py": "Lib/opcode.py",
    "opcode/bytecodes.c": "Python/bytecodes.c",
    "opcode/opcode.h": "Include/opcode.h",
}

OPTIONAL = {
    "LICENSE": "LICENSE",
}


def copy_file(src: pathlib.Path, dst: pathlib.Path) -> None:
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)
    print(f"copied {src} -> {dst}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cpython_dir", help="Path to a CPython 3.14 source checkout")
    parser.add_argument("--version", help="Record the CPython version string")
    args = parser.parse_args()

    cpython_dir = pathlib.Path(args.cpython_dir).resolve()
    if not cpython_dir.exists():
        print(f"error: {cpython_dir} does not exist", file=sys.stderr)
        return 2

    missing = []
    for dest, rel_src in REQUIRED.items():
        src = cpython_dir / rel_src
        if not src.exists():
            missing.append(rel_src)
            continue
        copy_file(src, VENDOR / dest)

    for dest, rel_src in OPTIONAL.items():
        src = cpython_dir / rel_src
        if src.exists():
            copy_file(src, VENDOR / dest)

    if args.version:
        version_file = VENDOR / "VERSION.txt"
        version_file.write_text(args.version + "\n", encoding="utf-8")
        print(f"wrote {version_file}")

    if missing:
        print("error: missing required files:", file=sys.stderr)
        for rel in missing:
            print(f"  - {rel}", file=sys.stderr)
        return 2

    print("sync complete")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
