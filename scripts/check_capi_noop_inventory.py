#!/usr/bin/env python3
"""Ensure C-API no-op symbols are tracked in docs/CAPI_NOOP_INVENTORY.md.

This gate is intentionally conservative:
- It auto-detects obvious no-op exports in Rust C-API modules:
  - empty-body functions (`{}`)
  - trivial constant-return placeholders (`0`, `1`, `NULL`)
- It then requires those symbols to be present in the C-API no-op inventory doc.
"""

from __future__ import annotations

import re
import json
import sys
import argparse
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
DOC_PATH = REPO_ROOT / "docs" / "CAPI_NOOP_INVENTORY.md"
VM_EXT_DIR = REPO_ROOT / "src" / "vm" / "vm_extensions"
VARARGS_C_PATH = REPO_ROOT / "src" / "vm" / "capi_variadics.c"


RUST_EXTERN_EXPORT_RE = re.compile(r'pub\s+unsafe\s+extern\s+"C"\s+fn\s+([A-Za-z0-9_]+)\s*\(')

RUST_EMPTY_BODY_RE = re.compile(
    r'pub\s+unsafe\s+extern\s+"C"\s+fn\s+([A-Za-z0-9_]+)\s*\([^)]*\)\s*(?:->\s*[^\{]+)?\s*\{\s*\}',
    re.MULTILINE,
)

RUST_TRIVIAL_ZERO_RE = re.compile(
    r'pub\s+unsafe\s+extern\s+"C"\s+fn\s+([A-Za-z0-9_]+)\s*\([^)]*\)\s*(?:->\s*[^\{]+)?\s*\{\s*(?:return\s+)?0\s*;?\s*\}',
    re.MULTILINE,
)

RUST_TRIVIAL_ONE_RE = re.compile(
    r'pub\s+unsafe\s+extern\s+"C"\s+fn\s+([A-Za-z0-9_]+)\s*\([^)]*\)\s*(?:->\s*[^\{]+)?\s*\{\s*(?:return\s+)?1\s*;?\s*\}',
    re.MULTILINE,
)

RUST_TRIVIAL_NULL_RE = re.compile(
    r'pub\s+unsafe\s+extern\s+"C"\s+fn\s+([A-Za-z0-9_]+)\s*\([^)]*\)\s*(?:->\s*[^\{]+)?\s*\{\s*(?:return\s+)?std::ptr::null(?:_mut)?\(\)\s*;?\s*\}',
    re.MULTILINE,
)

# Best-effort C export definition detector (for stale-doc checking).
C_EXPORT_DEF_RE = re.compile(
    r"^[A-Za-z_][A-Za-z0-9_\s\*]*\s+([_A-Za-z][_A-Za-z0-9]*)\s*\([^;]*\)\s*\{",
    re.MULTILINE,
)

DOC_SYMBOL_RE = re.compile(r"`([A-Za-z_][A-Za-z0-9_]*)`")


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def collect_rust_exports() -> set[str]:
    exports: set[str] = set()
    for path in VM_EXT_DIR.rglob("*.rs"):
        text = read_text(path)
        exports.update(RUST_EXTERN_EXPORT_RE.findall(text))
    return exports


def collect_rust_detected_noops() -> set[str]:
    noops: set[str] = set()
    for path in VM_EXT_DIR.rglob("*.rs"):
        text = read_text(path)
        noops.update(RUST_EMPTY_BODY_RE.findall(text))
        noops.update(RUST_TRIVIAL_ZERO_RE.findall(text))
        noops.update(RUST_TRIVIAL_ONE_RE.findall(text))
        noops.update(RUST_TRIVIAL_NULL_RE.findall(text))
    return noops


def collect_c_exports() -> set[str]:
    if not VARARGS_C_PATH.exists():
        return set()
    text = read_text(VARARGS_C_PATH)
    names = set(C_EXPORT_DEF_RE.findall(text))
    # Keep only likely API exports.
    return {name for name in names if name.startswith("Py") or name.startswith("_Py")}


def collect_doc_symbols() -> set[str]:
    text = read_text(DOC_PATH)
    return set(DOC_SYMBOL_RE.findall(text))


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate C-API no-op inventory doc against source and optionally emit a manifest."
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=None,
        help="Optional output path for machine-readable JSON manifest.",
    )
    args = parser.parse_args()

    if not DOC_PATH.exists():
        print(f"error: missing inventory file: {DOC_PATH}", file=sys.stderr)
        return 2

    rust_exports = collect_rust_exports()
    c_exports = collect_c_exports()
    all_exports = rust_exports | c_exports
    detected_noops = collect_rust_detected_noops()
    doc_symbols = collect_doc_symbols()

    manifest = {
        "doc_path": str(DOC_PATH.relative_to(REPO_ROOT)),
        "rust_export_count": len(rust_exports),
        "c_export_count": len(c_exports),
        "detected_noop_symbols": sorted(detected_noops),
        "documented_symbols": sorted(doc_symbols),
        "missing_from_doc": sorted(detected_noops - doc_symbols),
        "stale_in_doc": sorted(
            symbol
            for symbol in doc_symbols
            if symbol.startswith("Py") and symbol not in all_exports
        ),
    }
    if args.manifest is not None:
        manifest_path = args.manifest
        if not manifest_path.is_absolute():
            manifest_path = REPO_ROOT / manifest_path
        manifest_path.parent.mkdir(parents=True, exist_ok=True)
        manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    missing_from_doc = sorted(detected_noops - doc_symbols)
    if missing_from_doc:
        print(
            "C-API no-op inventory drift: symbols detected in source but missing from "
            "docs/CAPI_NOOP_INVENTORY.md:",
            file=sys.stderr,
        )
        for symbol in missing_from_doc:
            print(f"  - {symbol}", file=sys.stderr)
        return 1

    stale_in_doc = sorted(symbol for symbol in doc_symbols if symbol.startswith("Py") and symbol not in all_exports)
    if stale_in_doc:
        print(
            "C-API no-op inventory drift: documented symbols not found in current exports:",
            file=sys.stderr,
        )
        for symbol in stale_in_doc:
            print(f"  - {symbol}", file=sys.stderr)
        return 1

    print(
        f"C-API no-op inventory check passed "
        f"({len(detected_noops)} detected no-op exports, {len(doc_symbols)} documented symbols)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
