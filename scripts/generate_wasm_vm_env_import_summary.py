#!/usr/bin/env python3
"""Summarize raw wasm `env` imports for vm-probe browser bring-up tracking."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

from generate_wasm_env_shim import parse_env_imports

ZLIB_IMPORTS = {
    "zlibVersion",
    "deflateInit2_",
    "deflate",
    "deflateEnd",
    "inflateInit2_",
    "inflate",
    "inflateEnd",
    "crc32",
}

LIBC_ALLOC_IMPORTS = {"malloc", "calloc", "realloc", "free"}
LIBC_STDLIB_IMPORTS = {"snprintf", "strtod", "strtol", "strtoul"}


def classify_import(name: str) -> str:
    if name.startswith("sqlite3_"):
        return "sqlite3"
    if name.startswith("BZ2_"):
        return "bz2"
    if name.startswith("lzma_"):
        return "lzma"
    if name in ZLIB_IMPORTS:
        return "zlib"
    if name.startswith("_Py") or name.startswith("Py"):
        return "cpython_capi"
    if name in LIBC_ALLOC_IMPORTS:
        return "libc_alloc"
    if name in LIBC_STDLIB_IMPORTS:
        return "libc_stdlib"
    return "other"


def parse_exported_names(env_shim_path: Path) -> set[str]:
    if not env_shim_path.is_file():
        return set()
    source = env_shim_path.read_text(encoding="utf-8")
    exported_functions = set(
        re.findall(r"export function ([A-Za-z_][A-Za-z0-9_]*)\(", source)
    )
    exported_consts = set(re.findall(r"export const ([A-Za-z_][A-Za-z0-9_]*)\s*=", source))
    return exported_functions | exported_consts


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--wasm",
        default="target/wasm32-unknown-unknown/release-wasm/pyrs.wasm",
        help="Path to vm-probe wasm artifact.",
    )
    parser.add_argument(
        "--env-shim",
        default="scripts/wasm_node_shims/env/index.js",
        help="Path to node env shim file for export-coverage checks.",
    )
    parser.add_argument(
        "--out",
        default="perf/wasm_vm_env_import_summary_latest.json",
        help="Output summary artifact path.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    wasm_path = Path(args.wasm)
    if not wasm_path.is_file():
        print(f"wasm env-import summary failed: missing wasm artifact: {wasm_path}")
        return 1

    imports = parse_env_imports(wasm_path)
    env_funcs = imports.get("funcs", [])
    grouped: dict[str, list[str]] = {}
    for name in env_funcs:
        bucket = classify_import(name)
        grouped.setdefault(bucket, []).append(name)

    for names in grouped.values():
        names.sort()

    shim_path = Path(args.env_shim)
    shim_exports = parse_exported_names(shim_path)
    shim_missing = sorted(name for name in env_funcs if name not in shim_exports)

    summary = {
        "wasm": str(wasm_path),
        "env_shim": str(shim_path),
        "counts": {
            "env_function_imports": len(env_funcs),
            "env_globals": len(imports.get("globals", [])),
            "env_memories": len(imports.get("memories", [])),
            "env_tables": len(imports.get("tables", [])),
            "shim_exported_symbols": len(shim_exports),
            "shim_missing_symbols": len(shim_missing),
        },
        "groups": {
            group: {"count": len(names), "symbols": names}
            for group, names in sorted(grouped.items())
        },
        "env_function_symbols": env_funcs,
        "shim_missing_symbols": shim_missing,
    }

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")

    print(
        "wasm vm env-import summary: "
        f"funcs={summary['counts']['env_function_imports']} "
        f"shim_missing={summary['counts']['shim_missing_symbols']}"
    )
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
