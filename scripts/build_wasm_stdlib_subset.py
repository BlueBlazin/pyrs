#!/usr/bin/env python3
"""Build a curated CPython stdlib .py subset pack for wasm playground usage."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import sysconfig
import zipfile
from dataclasses import dataclass
from pathlib import Path

DEFAULT_SEED_MODULES = [
    "functools",
    "random",
    "collections",
    "abc",
    "types",
    "operator",
    "dataclasses",
    "typing",
    "statistics",
    "enum",
    "copy",
    "contextlib",
    "re",
    "string",
    "textwrap",
    "pprint",
    "json",
    "fractions",
    "decimal",
    "heapq",
    "bisect",
    "urllib.parse",
    "numbers",
    "difflib",
    "datetime",
    "argparse",
]


@dataclass(frozen=True)
class ModuleSource:
    name: str
    path: Path
    is_package: bool


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build wasm curated stdlib subset zip + manifest."
    )
    parser.add_argument(
        "--cpython-lib",
        default=None,
        help="Path to CPython Lib directory (default: auto-detect).",
    )
    parser.add_argument(
        "--out-zip",
        default="website/public/wasm/stdlib_subset_v1.zip",
        help="Output zip path.",
    )
    parser.add_argument(
        "--out-manifest",
        default="website/public/wasm/stdlib_subset_manifest_v1.json",
        help="Output manifest path.",
    )
    parser.add_argument(
        "--pack-version",
        default="v1",
        help="Pack version label recorded in manifest.",
    )
    parser.add_argument(
        "--max-zip-bytes",
        type=int,
        default=524_288,
        help="Fail if compressed zip exceeds this size budget.",
    )
    return parser.parse_args()


def detect_cpython_lib(explicit: str | None) -> Path:
    candidates: list[Path] = []
    if explicit:
        candidates.append(Path(explicit))

    env_path = os.environ.get("PYRS_CPYTHON_LIB")
    if env_path:
        candidates.append(Path(env_path))

    candidates.append(Path(".local/Python-3.14.3/Lib"))

    stdlib = sysconfig.get_paths().get("stdlib")
    if stdlib:
        candidates.append(Path(stdlib))

    candidates.extend(
        [
            Path("/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14"),
            Path("/opt/hostedtoolcache/Python/3.14.3/x64/lib/python3.14"),
            Path("/usr/lib/python3.14"),
            Path("/usr/local/lib/python3.14"),
        ]
    )

    for candidate in candidates:
        if candidate.is_dir():
            return candidate.resolve()

    tried = "\n".join(f"- {path}" for path in candidates)
    raise SystemExit(f"unable to locate CPython Lib directory; tried:\n{tried}")


def module_name_from_rel_path(rel_path: Path) -> tuple[str, bool]:
    if rel_path.name == "__init__.py":
        parts = rel_path.parts[:-1]
        return ".".join(parts), True
    return ".".join(rel_path.with_suffix("").parts), False


def collect_runtime_import_closure(
    lib_root: Path,
    seed_modules: list[str],
) -> dict[str, ModuleSource]:
    probe = r"""
import importlib
import json
import sys
from pathlib import Path

lib_root = Path(sys.argv[1]).resolve()
seed_modules = json.loads(sys.argv[2])

for preload in ("math", "_random", "_datetime"):
    try:
        importlib.import_module(preload)
    except Exception:
        pass

sys.path[:] = [str(lib_root)]
failed = {}
for module_name in seed_modules:
    try:
        importlib.import_module(module_name)
    except Exception as exc:
        failed[module_name] = f"{type(exc).__name__}: {exc}"

files = set()
for module in sys.modules.values():
    if module is None:
        continue
    module_file = getattr(module, "__file__", None)
    if not module_file:
        continue
    path = Path(module_file)
    try:
        resolved = path.resolve()
    except Exception:
        continue
    if (
        resolved.is_file()
        and lib_root in resolved.parents
        and resolved.suffix == ".py"
        and "test" not in resolved.parts
    ):
        files.add(str(resolved))

print(json.dumps({"files": sorted(files), "failed": failed}, sort_keys=True))
"""

    result = subprocess.run(
        [sys.executable, "-S", "-c", probe, str(lib_root), json.dumps(seed_modules)],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise SystemExit(
            "closure probe failed:\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        )

    payload = json.loads(result.stdout)
    failed: dict[str, str] = payload.get("failed", {})
    if failed:
        lines = "\n".join(f"- {name}: {message}" for name, message in sorted(failed.items()))
        raise SystemExit(f"seed module import failures:\n{lines}")

    closure: dict[str, ModuleSource] = {}
    for path_str in payload.get("files", []):
        source_path = Path(path_str).resolve()
        rel_path = source_path.relative_to(lib_root)
        module_name, is_package = module_name_from_rel_path(rel_path)
        closure[module_name] = ModuleSource(module_name, source_path, is_package=is_package)
    return closure


def write_deterministic_zip(
    out_zip: Path,
    lib_root: Path,
    closure: dict[str, ModuleSource],
) -> int:
    out_zip.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(out_zip, "w") as archive:
        for module_name in sorted(closure):
            source = closure[module_name]
            rel_path = source.path.relative_to(lib_root).as_posix()
            data = source.path.read_bytes()
            info = zipfile.ZipInfo(rel_path)
            info.date_time = (1980, 1, 1, 0, 0, 0)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            archive.writestr(info, data, compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)
    return out_zip.stat().st_size


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def write_manifest(
    out_manifest: Path,
    pack_version: str,
    lib_root: Path,
    seed_modules: list[str],
    closure: dict[str, ModuleSource],
    zip_path: Path,
    zip_bytes: int,
) -> None:
    out_manifest.parent.mkdir(parents=True, exist_ok=True)
    files = []
    raw_bytes = 0
    for module_name in sorted(closure):
        source = closure[module_name]
        size_bytes = source.path.stat().st_size
        raw_bytes += size_bytes
        files.append(
            {
                "module": module_name,
                "path": source.path.relative_to(lib_root).as_posix(),
                "is_package": source.is_package,
                "size_bytes": size_bytes,
            }
        )

    manifest = {
        "pack_version": pack_version,
        "python_version_target": "3.14",
        "source_lib_root": str(lib_root),
        "seed_modules": seed_modules,
        "module_count": len(closure),
        "files": files,
        "totals": {
            "raw_bytes": raw_bytes,
            "zip_bytes": zip_bytes,
            "zip_sha256": sha256_file(zip_path),
        },
    }
    out_manifest.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def main() -> int:
    args = parse_args()
    lib_root = detect_cpython_lib(args.cpython_lib)
    out_zip = Path(args.out_zip)
    out_manifest = Path(args.out_manifest)
    seed_modules = list(DEFAULT_SEED_MODULES)
    closure = collect_runtime_import_closure(lib_root, seed_modules)
    if not closure:
        print("stdlib subset closure is empty", file=sys.stderr)
        return 1

    zip_bytes = write_deterministic_zip(out_zip, lib_root, closure)
    if zip_bytes > args.max_zip_bytes:
        print(
            (
                f"stdlib subset zip exceeds budget: {zip_bytes} > {args.max_zip_bytes} "
                f"({out_zip})"
            ),
            file=sys.stderr,
        )
        return 1

    write_manifest(
        out_manifest,
        args.pack_version,
        lib_root,
        seed_modules,
        closure,
        out_zip,
        zip_bytes,
    )
    print(f"wrote {out_zip} ({zip_bytes} bytes)")
    print(f"wrote {out_manifest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
