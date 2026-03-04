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

PACK_EXCLUDED_MODULES = {
    # Keep native bootstrap substrate for os in wasm runtime.
    "os",
}


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
        "--out-pack",
        default="website/public/wasm/stdlib_subset_v1.json",
        help="Output JSON source-pack path consumed by wasm runtime.",
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


def module_source_path_for_name(lib_root: Path, module_name: str) -> tuple[Path | None, bool]:
    rel_parts = module_name.split(".")
    package_init = lib_root.joinpath(*rel_parts, "__init__.py")
    if package_init.is_file():
        return package_init.resolve(), True
    module_file = lib_root.joinpath(*rel_parts).with_suffix(".py")
    if module_file.is_file():
        return module_file.resolve(), False
    return None, False


def collect_runtime_import_closure(
    lib_root: Path,
    seed_modules: list[str],
) -> dict[str, ModuleSource]:
    probe = r"""
import importlib
import json
import sys
import builtins
from pathlib import Path

lib_root = Path(sys.argv[1]).resolve()
seed_modules = json.loads(sys.argv[2])

for preload in ("math", "_random", "_datetime"):
    try:
        importlib.import_module(preload)
    except Exception:
        pass

sys.path[:] = [str(lib_root)]
baseline_modules = set(sys.modules)
tracked_import_names = set()
orig_import = builtins.__import__

def tracking_import(name, globals=None, locals=None, fromlist=(), level=0):
    module = orig_import(name, globals, locals, fromlist, level)
    if isinstance(name, str) and name:
        tracked_import_names.add(name)
    module_name = getattr(module, "__name__", None)
    if isinstance(module_name, str) and module_name:
        tracked_import_names.add(module_name)
        if fromlist:
            for item in fromlist:
                if isinstance(item, str) and item and item != "*":
                    tracked_import_names.add(f"{module_name}.{item}")
    return module

builtins.__import__ = tracking_import
failed = {}
try:
    for module_name in seed_modules:
        try:
            importlib.import_module(module_name)
        except Exception as exc:
            failed[module_name] = f"{type(exc).__name__}: {exc}"
finally:
    builtins.__import__ = orig_import

loaded_delta = set(sys.modules) - baseline_modules
names = set(seed_modules)
names.update(tracked_import_names)
names.update(loaded_delta)

expanded = set()
for module_name in names:
    if not module_name:
        continue
    parts = module_name.split(".")
    for i in range(1, len(parts) + 1):
        expanded.add(".".join(parts[:i]))

print(
    json.dumps(
        {"names": sorted(expanded), "failed": failed},
        sort_keys=True,
    )
)
"""

    result = subprocess.run(
        [
            sys.executable,
            "-X",
            "frozen_modules=off",
            "-S",
            "-c",
            probe,
            str(lib_root),
            json.dumps(seed_modules),
        ],
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
    for module_name in sorted(payload.get("names", [])):
        source_path, is_package = module_source_path_for_name(lib_root, module_name)
        if source_path is None:
            continue
        if "test" in source_path.parts:
            continue
        if module_name in PACK_EXCLUDED_MODULES:
            continue
        closure[module_name] = ModuleSource(
            module_name,
            source_path,
            is_package=is_package,
        )
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
    pack_path: Path,
    pack_bytes: int,
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
            "pack_bytes": pack_bytes,
            "pack_sha256": sha256_file(pack_path),
        },
    }
    out_manifest.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def write_source_pack(
    out_pack: Path,
    pack_version: str,
    lib_root: Path,
    closure: dict[str, ModuleSource],
) -> int:
    out_pack.parent.mkdir(parents=True, exist_ok=True)
    modules = []
    for module_name in sorted(closure):
        source = closure[module_name]
        modules.append(
            {
                "module": module_name,
                "path": source.path.relative_to(lib_root).as_posix(),
                "is_package": source.is_package,
                "source": source.path.read_text(encoding="utf-8"),
            }
        )
    payload = {
        "pack_version": pack_version,
        "python_version_target": "3.14",
        "module_count": len(modules),
        "modules": modules,
    }
    out_pack.write_text(
        json.dumps(payload, sort_keys=True, separators=(",", ":")),
        encoding="utf-8",
    )
    return out_pack.stat().st_size


def main() -> int:
    args = parse_args()
    lib_root = detect_cpython_lib(args.cpython_lib)
    out_zip = Path(args.out_zip)
    out_manifest = Path(args.out_manifest)
    out_pack = Path(args.out_pack)
    seed_modules = list(DEFAULT_SEED_MODULES)
    closure = collect_runtime_import_closure(lib_root, seed_modules)
    if not closure:
        print("stdlib subset closure is empty", file=sys.stderr)
        return 1
    missing_seed = [
        module_name
        for module_name in seed_modules
        if module_name not in closure
        and module_source_path_for_name(lib_root, module_name)[0] is not None
    ]
    if missing_seed:
        print(
            "stdlib subset closure missing seed modules with available .py source:\n"
            + "\n".join(f"- {name}" for name in missing_seed),
            file=sys.stderr,
        )
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

    pack_bytes = write_source_pack(out_pack, args.pack_version, lib_root, closure)
    write_manifest(
        out_manifest,
        args.pack_version,
        lib_root,
        seed_modules,
        closure,
        out_zip,
        zip_bytes,
        out_pack,
        pack_bytes,
    )
    print(f"wrote {out_zip} ({zip_bytes} bytes)")
    print(f"wrote {out_pack} ({pack_bytes} bytes)")
    print(f"wrote {out_manifest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
