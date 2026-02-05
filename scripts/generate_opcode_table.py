#!/usr/bin/env python3
"""Generate opcode_table.csv from a CPython 3.14 source checkout.

Usage:
  python3 scripts/generate_opcode_table.py /path/to/Python-3.14.x
"""

from __future__ import annotations

import argparse
import csv
import sys
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cpython_root", help="Path to CPython 3.14 source checkout")
    parser.add_argument(
        "--output",
        default=None,
        help="Output CSV path (defaults to vendor/cpython-3.14/opcode/opcode_table.csv)",
    )
    args = parser.parse_args()

    cpython_root = Path(args.cpython_root).resolve()
    cases_dir = cpython_root / "Tools" / "cases_generator"
    if not cases_dir.exists():
        print(f"error: missing {cases_dir}", file=sys.stderr)
        return 2

    sys.path.insert(0, str(cases_dir))

    from analyzer import analyze_files  # type: ignore
    from stack import get_stack_effect  # type: ignore

    bytecodes = cpython_root / "Python" / "bytecodes.c"
    analysis = analyze_files([bytecodes.as_posix()])

    output = (
        Path(args.output)
        if args.output
        else Path(__file__).resolve().parents[1]
        / "vendor"
        / "cpython-3.14"
        / "opcode"
        / "opcode_table.csv"
    )

    rows: list[tuple[int, str, int, str]] = []
    for name, opcode in analysis.opmap.items():
        inst = analysis.instructions.get(name) or analysis.pseudos.get(name)
        if inst is None:
            continue
        effect = get_stack_effect(inst)
        net = effect.logical_sp.as_int()
        flags: list[str] = []
        if net is None:
            net = 0
            flags.append("DYNAMIC_STACK")
        if inst.properties.oparg:
            flags.append("ARG")
        if inst.properties.jumps:
            flags.append("JUMP")
        if inst.properties.uses_co_consts:
            flags.append("CONST")
        if inst.properties.uses_co_names:
            flags.append("NAME")
        if inst.properties.has_free:
            flags.append("FREE")
        if inst.properties.uses_locals:
            flags.append("LOCAL")
        if inst.properties.escapes:
            flags.append("ESCAPES")
        rows.append((opcode, name, net, "|".join(flags)))

    rows.sort(key=lambda row: row[0])
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(["opcode", "name", "stack_effect", "flags"])
        for row in rows:
            writer.writerow(row)

    print(f"wrote {output} ({len(rows)} opcodes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
