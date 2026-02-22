#!/usr/bin/env python3
"""Audit for obsolete scaffolding surfaces.

This check enforces a narrow set of invariants so temporary scaffolding does
not silently persist after migration work lands.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

ALLOWED_SHIM_FILES = {"_ctypes.py"}
RETIRED_SHIM_PATHS = {
    "shims/enum.py",
    "shims/pkgutil.py",
    "shims/pyexpat.py",
    "shims/importlib/resources.py",
}

SCAN_SUFFIXES = {
    ".rs",
    ".md",
    ".txt",
    ".py",
    ".toml",
    ".yml",
    ".yaml",
    ".sh",
}


def run(cmd: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=REPO_ROOT,
        check=False,
        text=True,
        capture_output=True,
    )


def collect_repo_file_hits(
    needle: str, *, roots: tuple[str, ...] = ("src", "tests", ".github")
) -> list[str]:
    hits: list[str] = []
    for rel_root in roots:
        base = REPO_ROOT / rel_root
        if not base.exists():
            continue
        for root, dirs, files in os.walk(base):
            root_path = Path(root)
            if ".git" in dirs:
                dirs.remove(".git")
            if root_path.name == "target":
                dirs.clear()
                continue
            for filename in files:
                path = root_path / filename
                if path.suffix.lower() not in SCAN_SUFFIXES:
                    continue
                rel = path.relative_to(REPO_ROOT)
                try:
                    text = path.read_text(encoding="utf-8")
                except UnicodeDecodeError:
                    continue
                if needle in text:
                    hits.append(str(rel))
    return sorted(set(hits))


def check_shim_directory(failures: list[str]) -> None:
    shim_root = REPO_ROOT / "shims"
    if not shim_root.exists():
        failures.append("shims/ directory is missing")
        return
    files = {
        str(path.relative_to(shim_root))
        for path in shim_root.rglob("*")
        if path.is_file()
    }
    if files != ALLOWED_SHIM_FILES:
        failures.append(
            f"unexpected shim files: found={sorted(files)} expected={sorted(ALLOWED_SHIM_FILES)}"
        )


def check_retired_shim_references(failures: list[str]) -> None:
    for retired in sorted(RETIRED_SHIM_PATHS):
        hits = collect_repo_file_hits(retired)
        if hits:
            failures.append(
                f"stale reference to retired shim path '{retired}' in: {', '.join(hits)}"
            )


def check_local_shim_allowlist(failures: list[str]) -> None:
    vm_mod = (REPO_ROOT / "src/vm/mod.rs").read_text(encoding="utf-8")
    expected = 'const LOCAL_SHIM_MODULES: &[&str] = &["_ctypes"];'
    if expected not in vm_mod:
        failures.append("LOCAL_SHIM_MODULES is not locked to _ctypes-only")


def check_obsolete_toggle_api(failures: list[str]) -> None:
    hits = collect_repo_file_hits(
        "enable_local_shim_fallback(", roots=("src", "tests")
    )
    if hits:
        failures.append(
            "obsolete local shim toggle API still present/referenced in: " + ", ".join(hits)
        )


def check_noop_inventory_sync(failures: list[str]) -> None:
    generated = run(["cargo", "run", "--quiet", "--bin", "print_noop_inventory"])
    if generated.returncode != 0:
        failures.append(
            "failed to generate no-op inventory via print_noop_inventory: "
            + generated.stderr.strip()
        )
        return
    doc_path = REPO_ROOT / "docs/NOOP_BUILTIN_INVENTORY.txt"
    documented = doc_path.read_text(encoding="utf-8")
    if generated.stdout != documented:
        failures.append(
            "docs/NOOP_BUILTIN_INVENTORY.txt is out of sync with print_noop_inventory output"
        )


def check_capi_noop_inventory(failures: list[str]) -> None:
    result = run(
        [
            sys.executable,
            "scripts/check_capi_noop_inventory.py",
            "--manifest",
            "perf/capi_noop_inventory.json",
        ]
    )
    if result.returncode != 0:
        failures.append(
            "C-API no-op inventory drift detected:\n"
            + (result.stderr.strip() or result.stdout.strip())
        )


def main() -> int:
    failures: list[str] = []
    check_shim_directory(failures)
    check_retired_shim_references(failures)
    check_local_shim_allowlist(failures)
    check_obsolete_toggle_api(failures)
    check_noop_inventory_sync(failures)
    check_capi_noop_inventory(failures)

    if failures:
        print("scaffolding audit failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print(
        "scaffolding audit passed: shims/allowlist/no-op inventories are consistent and no retired scaffolding references were found."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
