#!/usr/bin/env python3
"""Generate a CPython 3.14 Stable ABI (abi3) coverage manifest for pyrs."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import subprocess
import sys
from datetime import datetime, timezone

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    print("error: Python 3.11+ required (missing tomllib)", file=sys.stderr)
    raise


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--stable-abi-toml",
        type=pathlib.Path,
        help="Path to CPython Misc/stable_abi.toml",
    )
    parser.add_argument(
        "--binary",
        type=pathlib.Path,
        default=REPO_ROOT / "target" / "debug" / "pyrs",
        help="Path to the pyrs executable (default: target/debug/pyrs)",
    )
    parser.add_argument(
        "--out",
        type=pathlib.Path,
        default=REPO_ROOT / "perf" / "abi3_manifest_latest.json",
        help="Output JSON path",
    )
    return parser.parse_args()


def resolve_stable_abi_toml(cli_path: pathlib.Path | None) -> pathlib.Path:
    if cli_path is not None:
        return cli_path

    candidates = [
        REPO_ROOT / "vendor" / "cpython-3.14" / "Misc" / "stable_abi.toml",
    ]
    cpython_src = os.environ.get("CPYTHON_SRC")
    if cpython_src:
        candidates.append(pathlib.Path(cpython_src) / "Misc" / "stable_abi.toml")

    # Local machine default used by this project.
    candidates.append(REPO_ROOT / ".local" / "Python-3.14.3" / "Misc" / "stable_abi.toml")

    for candidate in candidates:
        if candidate.exists():
            return candidate

    raise FileNotFoundError(
        "unable to locate stable_abi.toml; pass --stable-abi-toml or set CPYTHON_SRC"
    )


def load_stable_abi(path: pathlib.Path) -> dict[str, dict[str, object]]:
    with path.open("rb") as f:
        return tomllib.load(f)


def extract_names(table: dict[str, object], key: str) -> list[str]:
    section = table.get(key, {})
    if not isinstance(section, dict):
        return []
    names = [name for name in section.keys() if isinstance(name, str)]
    names.sort()
    return names


def read_exported_symbols(binary: pathlib.Path) -> set[str]:
    if not binary.exists():
        raise FileNotFoundError(f"binary not found: {binary}")

    commands = (["nm", "-gU", str(binary)], ["nm", "-g", str(binary)])
    output = None
    for cmd in commands:
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if proc.returncode == 0:
            output = proc.stdout
            break
    if output is None:
        raise RuntimeError(f"nm failed for {binary}")

    exported: set[str] = set()
    for line in output.splitlines():
        line = line.strip()
        if not line:
            continue
        parts = line.split()
        symbol = parts[-1]
        # macOS `nm` prefixes symbols with `_`. For CPython private globals that
        # already start with `_` (for example `_Py_NoneStruct`), this becomes a
        # double underscore (`__Py_NoneStruct`). Normalize both forms.
        if symbol.startswith("__") and len(symbol) > 2 and symbol[2].isalpha():
            symbol = symbol[1:]
        elif symbol.startswith("_") and len(symbol) > 1 and symbol[1].isalpha():
            symbol = symbol[1:]
        exported.add(symbol)
    return exported


def build_manifest(
    stable_abi_toml: pathlib.Path, binary: pathlib.Path
) -> dict[str, object]:
    stable_abi = load_stable_abi(stable_abi_toml)
    exported = read_exported_symbols(binary)

    functions = extract_names(stable_abi, "function")
    data_symbols = extract_names(stable_abi, "data")
    consts = extract_names(stable_abi, "const")
    macros = extract_names(stable_abi, "macro")
    typedefs = extract_names(stable_abi, "typedef")
    structs = extract_names(stable_abi, "struct")

    implemented_functions = sorted(name for name in functions if name in exported)
    missing_functions = sorted(name for name in functions if name not in exported)
    implemented_data = sorted(name for name in data_symbols if name in exported)
    missing_data = sorted(name for name in data_symbols if name not in exported)

    return {
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "inputs": {
            "stable_abi_toml": str(stable_abi_toml),
            "binary": str(binary),
        },
        "summary": {
            "stable_abi": {
                "function_count": len(functions),
                "data_count": len(data_symbols),
                "const_count": len(consts),
                "macro_count": len(macros),
                "typedef_count": len(typedefs),
                "struct_count": len(structs),
            },
            "pyrs_export_coverage": {
                "function": {
                    "implemented": len(implemented_functions),
                    "missing": len(missing_functions),
                },
                "data": {
                    "implemented": len(implemented_data),
                    "missing": len(missing_data),
                },
            },
        },
        "coverage": {
            "function": {
                "implemented": implemented_functions,
                "missing": missing_functions,
            },
            "data": {
                "implemented": implemented_data,
                "missing": missing_data,
            },
        },
        "catalog": {
            "const": consts,
            "macro": macros,
            "typedef": typedefs,
            "struct": structs,
        },
    }


def main() -> int:
    args = parse_args()
    stable_abi_toml = resolve_stable_abi_toml(args.stable_abi_toml)
    manifest = build_manifest(stable_abi_toml=stable_abi_toml, binary=args.binary)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2, sort_keys=True)
        f.write("\n")
    summary = manifest["summary"]["pyrs_export_coverage"]
    print(
        "abi3 coverage:",
        f"functions {summary['function']['implemented']}/{summary['function']['implemented'] + summary['function']['missing']},",
        f"data {summary['data']['implemented']}/{summary['data']['implemented'] + summary['data']['missing']}",
    )
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
