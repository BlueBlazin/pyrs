#!/usr/bin/env python3
"""Build a pyrs stdlib bundle from the official CPython source tarball."""

from __future__ import annotations

import argparse
import hashlib
import shutil
import tarfile
import tempfile
import urllib.request
from pathlib import Path


DEFAULT_CPYTHON_VERSION = "3.14.3"
DEFAULT_RELEASE_PAGE_URL = "https://www.python.org/downloads/release/python-3143/"
DEFAULT_SOURCE_URL = "https://www.python.org/ftp/python/3.14.3/Python-3.14.3.tgz"
DEFAULT_SOURCE_SHA256 = (
    "d7fe130d0501ae047ca318fa92aa642603ab6f217901015a1df6ce650d5470cd"
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Build a redistributable CPython stdlib pack for pyrs from official CPython sources."
        )
    )
    parser.add_argument(
        "--cpython-version",
        default=DEFAULT_CPYTHON_VERSION,
        help=f"CPython release version (default: {DEFAULT_CPYTHON_VERSION})",
    )
    parser.add_argument(
        "--release-page-url",
        default=DEFAULT_RELEASE_PAGE_URL,
        help=f"Official CPython release page URL (default: {DEFAULT_RELEASE_PAGE_URL})",
    )
    parser.add_argument(
        "--source-url",
        default=DEFAULT_SOURCE_URL,
        help=f"Official CPython source archive URL (default: {DEFAULT_SOURCE_URL})",
    )
    parser.add_argument(
        "--source-sha256",
        default=DEFAULT_SOURCE_SHA256,
        help="Expected SHA256 of the CPython source archive",
    )
    parser.add_argument(
        "--out",
        required=True,
        help="Output tar.gz path for pyrs stdlib pack",
    )
    return parser.parse_args()


def sha256_of_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def download_file(url: str, out_path: Path) -> None:
    with urllib.request.urlopen(url) as response, out_path.open("wb") as out_file:
        shutil.copyfileobj(response, out_file)


def write_metadata_file(
    metadata_path: Path, cpython_version: str, release_page_url: str, source_url: str
) -> None:
    metadata = "\n".join(
        [
            "pyrs CPython stdlib bundle metadata",
            f"cpython_version={cpython_version}",
            f"cpython_release_page={release_page_url}",
            f"cpython_source_url={source_url}",
            "",
        ]
    )
    metadata_path.write_text(metadata, encoding="utf-8")


def build_stdlib_pack(
    *,
    cpython_version: str,
    release_page_url: str,
    source_url: str,
    source_sha256: str,
    out_path: Path,
) -> None:
    with tempfile.TemporaryDirectory(prefix="pyrs-stdlib-pack-") as tmp_raw:
        tmp = Path(tmp_raw)
        source_archive = tmp / f"Python-{cpython_version}.tgz"
        download_file(source_url, source_archive)
        actual_sha = sha256_of_file(source_archive)
        if actual_sha != source_sha256.lower():
            raise RuntimeError(
                "CPython source SHA256 mismatch: "
                f"expected {source_sha256}, got {actual_sha}"
            )

        source_root_name = f"Python-{cpython_version}"
        extracted_root = tmp / source_root_name
        with tarfile.open(source_archive, "r:gz") as archive:
            archive.extractall(path=tmp)

        lib_dir = extracted_root / "Lib"
        license_file = extracted_root / "LICENSE"
        if not lib_dir.is_dir() or not (lib_dir / "site.py").is_file():
            raise RuntimeError(
                f"extracted source is missing stdlib Lib/site.py under {lib_dir}"
            )
        if not license_file.is_file():
            raise RuntimeError("extracted source is missing LICENSE file")

        pack_root_name = f"pyrs-stdlib-cpython-{cpython_version}"
        pack_root = tmp / pack_root_name
        shutil.copytree(lib_dir, pack_root / "Lib")
        shutil.copy2(license_file, pack_root / "LICENSE")
        write_metadata_file(
            pack_root / "METADATA.txt",
            cpython_version=cpython_version,
            release_page_url=release_page_url,
            source_url=source_url,
        )

        out_path.parent.mkdir(parents=True, exist_ok=True)
        with tarfile.open(out_path, "w:gz") as archive:
            archive.add(pack_root, arcname=pack_root_name)


def main() -> int:
    args = parse_args()
    out_path = Path(args.out).resolve()
    build_stdlib_pack(
        cpython_version=args.cpython_version,
        release_page_url=args.release_page_url,
        source_url=args.source_url,
        source_sha256=args.source_sha256,
        out_path=out_path,
    )
    print(f"wrote stdlib pack: {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
