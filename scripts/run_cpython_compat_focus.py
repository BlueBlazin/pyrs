#!/usr/bin/env python3
"""Run focused CPython compatibility benchmark slices."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile
from collections.abc import Iterable


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]
BENCHMARK = REPO_ROOT / "scripts" / "run_cpython_compat_benchmark.py"
DEFAULT_OUT_ROOT = REPO_ROOT / "perf" / "cpython_compat_focus"


SUITES: dict[str, dict[str, object]] = {
    "smoke": {
        "description": "Tiny benchmark slice for command-path verification before larger focused runs.",
        "entries": (
            "test.test_augassign",
            "test.test_bool",
            "test.test_import",
            "test.test_json",
        ),
    },
    "high-leverage": {
        "description": "Largest current headline movers from the checked-in benchmark snapshot.",
        "entries": (
            "test.test_email",
            "test.test_pathlib",
            "test.test_importlib",
            "test.test_datetime",
            "test.test_asyncio.test_tasks",
            "test.test_socket",
            "test.test_mailbox",
            "test.test_os",
            "test.test_configparser",
            "test.test_codecs",
            "test.test_enum",
            "test.test_call",
            "test.test_traceback",
            "test.test_functools",
            "test.test_pickle",
        ),
    },
    "import-bootstrap": {
        "description": "Import/bootstrap-heavy modules that currently block large stdlib surfaces from loading.",
        "entries": (
            "test.test___all__",
            "test.test_compile",
            "test.test_dis",
            "test.test_email",
            "test.test_import",
            "test.test_importlib",
            "test.test_pathlib",
            "test.test_pkg",
            "test.test_pkgutil",
            "test.test_zipfile",
        ),
    },
    "os-fs-socket": {
        "description": "OS, filesystem, socket, selectors, subprocess, and asyncio transport coverage.",
        "entries": (
            "test.test_asyncio.test_base_events",
            "test.test_asyncio.test_events",
            "test.test_asyncio.test_futures",
            "test.test_asyncio.test_selector_events",
            "test.test_asyncio.test_subprocess",
            "test.test_asyncio.test_taskgroups",
            "test.test_asyncio.test_tasks",
            "test.test_imaplib",
            "test.test_mailbox",
            "test.test_os",
            "test.test_pathlib",
            "test.test_selectors",
            "test.test_shutil",
            "test.test_socket",
            "test.test_subprocess",
        ),
    },
    "object-model-call": {
        "description": "Object model, call protocol, repr/format, AST/code object, and traceback parity.",
        "entries": (
            "test.test_argparse",
            "test.test_ast",
            "test.test_call",
            "test.test_compile",
            "test.test_configparser",
            "test.test_dataclasses",
            "test.test_datetime",
            "test.test_dis",
            "test.test_email",
            "test.test_enum",
            "test.test_functools",
            "test.test_inspect.test_inspect",
            "test.test_memoryview",
            "test.test_ordered_dict",
            "test.test_traceback",
        ),
    },
    "text-codecs-xml": {
        "description": "Encoding/decoding and XML parser/serializer parity.",
        "entries": (
            "test.test_codeccallbacks",
            "test.test_codecs",
            "test.test_codecencodings_cn",
            "test.test_codecencodings_iso2022",
            "test.test_codecencodings_jp",
            "test.test_codecencodings_kr",
            "test.test_sax",
            "test.test_xml_etree",
            "test.test_xml_etree_c",
        ),
    },
    "timeouts-crashes": {
        "description": "Known process-error and timeout hot spots for stability-first closure.",
        "entries": (
            "test.test_bytes",
            "test.test_copy",
            "test.test_io",
            "test.test_logging",
            "test.test_pdb",
            "test.test_pickle",
            "test.test_queue",
            "test.test_re",
            "test.test_set",
            "test.test_sqlite3",
            "test.test_statistics",
            "test.test_sys_settrace",
            "test.test_tarfile",
            "test.test_threading",
            "test.test_unittest",
        ),
    },
}


def normalize_entry(raw: str) -> str | None:
    value = raw.strip()
    if not value or value.startswith("#"):
        return None
    if value.startswith("Lib/"):
        value = value[4:]
    value = value.replace("\\", "/").strip("/")
    if value.endswith("/__init__.py"):
        value = value[: -len("/__init__.py")]
    elif value.endswith(".py"):
        value = value[:-3]
    value = value.replace("/", ".")
    if value.endswith(".__init__"):
        value = value[: -len(".__init__")]
    return value or None


def read_entry_file(path: pathlib.Path) -> list[str]:
    entries: list[str] = []
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        entry = normalize_entry(raw_line)
        if entry is not None:
            entries.append(entry)
    return entries


def unique_preserving_order(values: Iterable[str]) -> list[str]:
    seen: set[str] = set()
    ordered: list[str] = []
    for value in values:
        if value in seen:
            continue
        seen.add(value)
        ordered.append(value)
    return ordered


def suite_entries(name: str) -> list[str]:
    suite = SUITES[name]
    return list(suite["entries"])  # type: ignore[index]


def selection_hash(entries: list[str]) -> str:
    payload = "\n".join(entries).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()[:10]


def default_out_dir(selected_suites: list[str], custom_selection: bool, entries: list[str]) -> pathlib.Path:
    if len(selected_suites) == 1 and not custom_selection:
        slug = selected_suites[0]
    elif selected_suites and not custom_selection:
        slug = f"multi-{selection_hash(entries)}"
    else:
        slug = f"custom-{selection_hash(entries)}"
    return DEFAULT_OUT_ROOT / slug


def focus_request(args: argparse.Namespace, entries: list[str]) -> dict[str, object]:
    return {
        "suites": list(args.suite),
        "entries": entries,
        "inventory_timeout": args.inventory_timeout,
        "run_timeout": args.run_timeout,
        "max_entries": args.max_entries if args.max_entries > 0 else None,
        "inventory_only": args.inventory_only,
        "allow_missing_entries": args.allow_missing_entries,
    }


def write_focus_markers(out_dir: pathlib.Path, request: dict[str, object]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    entries = request["entries"]
    assert isinstance(entries, list)
    selection_path = out_dir / "selected_entries.txt"
    selection_path.write_text("\n".join(entries) + "\n", encoding="utf-8")
    (out_dir / "focus_request.json").write_text(json.dumps(request, indent=2) + "\n", encoding="utf-8")


def ensure_safe_out_dir(out_dir: pathlib.Path, request: dict[str, object], force: bool) -> None:
    selection_path = out_dir / "selected_entries.txt"
    request_path = out_dir / "focus_request.json"
    expected_request = json.dumps(request, indent=2) + "\n"
    if out_dir.exists() and not force:
        if selection_path.is_file() and request_path.is_file():
            existing = request_path.read_text(encoding="utf-8")
            if existing != expected_request:
                raise SystemExit(
                    f"refusing to reuse {out_dir}: focus_request.json differs; choose a different --out-dir or pass --force"
                )
        elif any(out_dir.iterdir()):
            raise SystemExit(
                f"refusing to reuse {out_dir}: directory already exists without focused-run markers; choose a different --out-dir or pass --force"
            )
    write_focus_markers(out_dir, request)


def staging_selection_file(entries: list[str]) -> pathlib.Path:
    handle = tempfile.NamedTemporaryFile(
        mode="w",
        encoding="utf-8",
        prefix="pyrs_cpython_compat_focus_",
        suffix=".txt",
        delete=False,
    )
    with handle:
        handle.write("\n".join(entries) + "\n")
    return pathlib.Path(handle.name)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run focused CPython compatibility benchmark suites")
    parser.add_argument("--suite", action="append", default=[], choices=sorted(SUITES), help="Named focused suite to run (repeatable)")
    parser.add_argument("--entry", action="append", default=[], help="Additional entry/module name or Lib/test-relative path (repeatable)")
    parser.add_argument("--entry-file", action="append", default=[], help="Read extra entries from a newline-delimited file (repeatable)")
    parser.add_argument("--list-suites", action="store_true", help="List available suite presets and exit")
    parser.add_argument("--print-selection", action="store_true", help="Print the normalized entry selection and exit")
    parser.add_argument("--runner-bin", default="target/release/pyrs", help="Interpreter used for execution runs")
    parser.add_argument("--cpython-bin", default=None, help="CPython 3.14 binary used for inventory/discovery")
    parser.add_argument("--cpython-lib", default=None, help="CPython 3.14 Lib root")
    parser.add_argument("--out-dir", default=None, help="Output directory (default: perf/cpython_compat_focus/<suite>)")
    parser.add_argument("--inventory-timeout", type=int, default=60, help="Per-entry timeout for inventory collection")
    parser.add_argument("--run-timeout", type=int, default=180, help="Per-entry timeout for execution runs")
    parser.add_argument("--jobs", type=int, default=0, help="Parallel workers (0 = benchmark default)")
    parser.add_argument("--max-entries", type=int, default=0, help="Optional entry cap for smoke runs")
    parser.add_argument("--allow-missing-entries", action="store_true", help="Continue when explicit entries are not discoverable on this host")
    parser.add_argument("--inventory-only", action="store_true", help="Discover entries and inventory, but do not execute them")
    parser.add_argument("--force", action="store_true", help="Recompute the out-dir even if it already exists")
    return parser.parse_args()


def print_suites() -> None:
    for name in sorted(SUITES):
        entries = suite_entries(name)
        description = SUITES[name]["description"]
        print(f"{name:17} {len(entries):2d} modules  {description}")


def build_selection(args: argparse.Namespace) -> list[str]:
    selected: list[str] = []
    for suite_name in args.suite:
        selected.extend(suite_entries(suite_name))
    for raw_entry in args.entry:
        entry = normalize_entry(raw_entry)
        if entry is not None:
            selected.append(entry)
    for entry_file in args.entry_file:
        selected.extend(read_entry_file(pathlib.Path(entry_file)))
    return unique_preserving_order(selected)


def benchmark_argv(args: argparse.Namespace, selection_path: pathlib.Path, out_dir: pathlib.Path) -> list[str]:
    argv = [
        sys.executable,
        str(BENCHMARK),
        "--runner-bin",
        args.runner_bin,
        "--entry-file",
        str(selection_path),
        "--out-dir",
        str(out_dir),
        "--inventory-timeout",
        str(args.inventory_timeout),
        "--run-timeout",
        str(args.run_timeout),
    ]
    if args.cpython_bin:
        argv.extend(["--cpython-bin", args.cpython_bin])
    if args.cpython_lib:
        argv.extend(["--cpython-lib", args.cpython_lib])
    if args.jobs > 0:
        argv.extend(["--jobs", str(args.jobs)])
    if args.max_entries > 0:
        argv.extend(["--max-entries", str(args.max_entries)])
    if args.allow_missing_entries:
        argv.append("--allow-missing-entries")
    if args.inventory_only:
        argv.append("--inventory-only")
    if args.force:
        argv.append("--force")
    return argv


def main() -> int:
    args = parse_args()
    if args.list_suites:
        print_suites()
        return 0

    entries = build_selection(args)
    if not entries:
        raise SystemExit("no entries selected; pass --suite, --entry, or --entry-file")

    if args.print_selection:
        print("\n".join(entries))
        return 0

    custom_selection = bool(args.entry or args.entry_file)
    out_dir = pathlib.Path(args.out_dir) if args.out_dir else default_out_dir(args.suite, custom_selection, entries)
    request = focus_request(args, entries)
    ensure_safe_out_dir(out_dir, request, args.force)
    selection_path = staging_selection_file(entries)
    try:
        completed = subprocess.run(benchmark_argv(args, selection_path, out_dir), check=False)
    finally:
        selection_path.unlink(missing_ok=True)
        if out_dir.exists():
            write_focus_markers(out_dir, request)
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
