#!/usr/bin/env python3
"""Run the public website-facing microbenchmark suite."""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import math
import os
import pathlib
import platform
import re
import shutil
import statistics
import subprocess
import time
from typing import Any


SCHEMA_VERSION = "v1"
REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]
SUITE_ROOT = REPO_ROOT / "benchmarks" / "public_micro"
MANIFEST_PATH = SUITE_ROOT / "benchmarks.json"
CPYTHON_VERSION_PROBE = (
    "import json, sys\n"
    "payload = {\n"
    "    'implementation': sys.implementation.name,\n"
    "    'version': '.'.join(str(part) for part in sys.version_info[:3]),\n"
    "}\n"
    "print(json.dumps(payload))\n"
)


@dataclasses.dataclass(frozen=True)
class BenchmarkSpec:
    id: str
    name: str
    category: str
    description: str
    kind: str
    disable_site: bool
    expected_stdout: str
    inline_code: str | None = None
    script: str | None = None


@dataclasses.dataclass(frozen=True)
class InterpreterSpec:
    id: str
    display_name: str
    binary: str
    kind: str
    expected_version: str | None = None


class BenchmarkFailure(RuntimeError):
    pass


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pyrs", default="target/release/pyrs", help="Path to the pyrs binary.")
    parser.add_argument(
        "--python314-bin",
        default="/Library/Frameworks/Python.framework/Versions/3.14/bin/python3",
        help="Path to the CPython 3.14.3 binary.",
    )
    parser.add_argument(
        "--python310-bin",
        default="python3.10",
        help="Path to the CPython 3.10.8 binary.",
    )
    parser.add_argument("--warmup", type=int, default=2, help="Warmup runs per interpreter per benchmark.")
    parser.add_argument("--iterations", type=int, default=7, help="Measured runs per interpreter per benchmark.")
    parser.add_argument(
        "--benchmark",
        action="append",
        default=[],
        help="Benchmark id to run. Repeat to select a subset; defaults to the full suite.",
    )
    parser.add_argument("--list", action="store_true", help="List available benchmarks and exit.")
    parser.add_argument(
        "--out",
        default="perf/public_micro_latest.json",
        help="JSON output artifact path.",
    )
    return parser.parse_args()


def load_manifest() -> list[BenchmarkSpec]:
    payload = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    if payload.get("schema_version") != SCHEMA_VERSION:
        raise SystemExit(f"unsupported public microbench schema in {MANIFEST_PATH}")
    benchmarks = payload.get("benchmarks")
    if not isinstance(benchmarks, list):
        raise SystemExit(f"invalid benchmark list in {MANIFEST_PATH}")
    specs: list[BenchmarkSpec] = []
    for entry in benchmarks:
        if not isinstance(entry, dict):
            raise SystemExit(f"invalid benchmark entry in {MANIFEST_PATH}: {entry!r}")
        specs.append(
            BenchmarkSpec(
                id=require_nonempty_str(entry, "id"),
                name=require_nonempty_str(entry, "name"),
                category=require_nonempty_str(entry, "category"),
                description=require_nonempty_str(entry, "description"),
                kind=require_nonempty_str(entry, "kind"),
                disable_site=require_bool(entry, "disable_site"),
                expected_stdout=require_string(entry, "expected_stdout"),
                inline_code=optional_str(entry, "inline_code"),
                script=optional_str(entry, "script"),
            )
        )
    return specs


def require_nonempty_str(payload: dict[str, Any], key: str) -> str:
    value = payload.get(key)
    if not isinstance(value, str) or not value:
        raise SystemExit(f"invalid or missing string field {key!r} in {MANIFEST_PATH}")
    return value


def require_string(payload: dict[str, Any], key: str) -> str:
    value = payload.get(key)
    if not isinstance(value, str):
        raise SystemExit(f"invalid or missing string field {key!r} in {MANIFEST_PATH}")
    return value


def optional_str(payload: dict[str, Any], key: str) -> str | None:
    value = payload.get(key)
    if value is None:
        return None
    if not isinstance(value, str) or not value:
        raise SystemExit(f"invalid string field {key!r} in {MANIFEST_PATH}")
    return value


def require_bool(payload: dict[str, Any], key: str) -> bool:
    value = payload.get(key)
    if not isinstance(value, bool):
        raise SystemExit(f"invalid or missing boolean field {key!r} in {MANIFEST_PATH}")
    return value


def select_benchmarks(specs: list[BenchmarkSpec], selected_ids: list[str]) -> list[BenchmarkSpec]:
    if not selected_ids:
        return specs
    requested = set(selected_ids)
    selected = [spec for spec in specs if spec.id in requested]
    missing = requested.difference(spec.id for spec in selected)
    if missing:
        missing_str = ", ".join(sorted(missing))
        raise SystemExit(f"unknown benchmark ids: {missing_str}")
    return selected


def resolve_executable(path_or_name: str) -> pathlib.Path:
    candidate = pathlib.Path(path_or_name).expanduser()
    if candidate.is_file():
        return candidate.resolve()
    resolved = shutil.which(path_or_name)
    if resolved:
        return pathlib.Path(resolved).resolve()
    raise SystemExit(f"executable not found: {path_or_name}")


def file_fingerprint(path: pathlib.Path) -> dict[str, Any]:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    stat = path.stat()
    return {
        "path": str(path),
        "size_bytes": stat.st_size,
        "mtime_ns": stat.st_mtime_ns,
        "sha256": digest.hexdigest(),
    }


def git_head() -> str | None:
    try:
        completed = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=False,
            cwd=REPO_ROOT,
        )
    except OSError:
        return None
    if completed.returncode != 0:
        return None
    value = completed.stdout.strip()
    return value or None


def probe_cpython_info(path: pathlib.Path, expected_version: str) -> dict[str, Any]:
    completed = subprocess.run(
        [str(path), "-S", "-c", CPYTHON_VERSION_PROBE],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise SystemExit(f"failed to query interpreter metadata from {path}: {completed.stderr.strip()}")
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise SystemExit(f"invalid interpreter metadata from {path}: {exc}") from exc
    implementation = payload.get("implementation")
    version = payload.get("version")
    if implementation != "cpython":
        raise SystemExit(f"expected CPython for {path}, got {implementation!r}")
    if version != expected_version:
        raise SystemExit(f"expected CPython {expected_version} at {path}, got {version!r}")
    return {
        "kind": "cpython",
        "version": version,
        "implementation": implementation,
        "version_banner": f"Python {version}",
    }


def probe_pyrs_info(path: pathlib.Path) -> dict[str, Any]:
    completed = subprocess.run(
        [str(path), "--version"],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise SystemExit(f"failed to query pyrs version from {path}: {completed.stderr.strip()}")
    banner = completed.stdout.strip()
    match = re.fullmatch(r"pyrs\s+(.+)", banner)
    if not match:
        raise SystemExit(f"unexpected pyrs --version output from {path}: {banner!r}")
    return {
        "kind": "pyrs",
        "version": match.group(1),
        "implementation": "pyrs",
        "version_banner": banner,
    }


def build_argv(interpreter: pathlib.Path, spec: BenchmarkSpec) -> list[str]:
    argv = [str(interpreter)]
    if spec.disable_site:
        argv.append("-S")
    if spec.kind == "inline":
        if spec.inline_code is None:
            raise SystemExit(f"inline benchmark missing inline_code: {spec.id}")
        argv.extend(["-c", spec.inline_code])
        return argv
    if spec.kind == "script":
        if spec.script is None:
            raise SystemExit(f"script benchmark missing script path: {spec.id}")
        argv.append(str((SUITE_ROOT / spec.script).resolve()))
        return argv
    raise SystemExit(f"unsupported benchmark kind {spec.kind!r} for {spec.id}")


def run_once(argv: list[str], expected_stdout: str) -> tuple[float, str]:
    started = time.perf_counter()
    completed = subprocess.run(
        argv,
        capture_output=True,
        text=True,
        check=False,
    )
    elapsed = time.perf_counter() - started
    stdout = completed.stdout.strip()
    stderr = completed.stderr.strip()
    if completed.returncode != 0:
        raise BenchmarkFailure(
            f"command failed ({completed.returncode}): {' '.join(argv)}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        )
    if stdout != expected_stdout:
        raise BenchmarkFailure(
            f"unexpected stdout for {' '.join(argv)}: expected {expected_stdout!r}, got {stdout!r}"
        )
    return elapsed, stdout


def run_samples(argv: list[str], expected_stdout: str, warmup: int, iterations: int) -> tuple[list[float], str]:
    for _ in range(warmup):
        run_once(argv, expected_stdout)
    samples: list[float] = []
    verification_output = expected_stdout
    for _ in range(iterations):
        elapsed, stdout = run_once(argv, expected_stdout)
        samples.append(elapsed)
        verification_output = stdout
    return samples, verification_output


def summarize(samples: list[float]) -> dict[str, float]:
    if len(samples) == 1:
        stddev = 0.0
    else:
        stddev = statistics.pstdev(samples)
    return {
        "min": min(samples),
        "max": max(samples),
        "mean": statistics.fmean(samples),
        "median": statistics.median(samples),
        "stddev": stddev,
    }


def ratio(numerator: float, denominator: float) -> float:
    if denominator <= 0.0:
        raise BenchmarkFailure(f"benchmark ratio denominator must be positive, got {denominator}")
    return numerator / denominator


def geometric_mean(values: list[float]) -> float:
    if not values:
        raise BenchmarkFailure("cannot compute geometric mean of an empty sequence")
    if any(value <= 0.0 for value in values):
        raise BenchmarkFailure(f"geometric mean requires positive values: {values!r}")
    return math.exp(sum(math.log(value) for value in values) / len(values))


def main() -> int:
    args = parse_args()
    if args.warmup < 0:
        raise SystemExit("--warmup must be >= 0")
    if args.iterations <= 0:
        raise SystemExit("--iterations must be > 0")

    specs = load_manifest()
    if args.list:
        for spec in specs:
            print(f"{spec.id}\t{spec.name}\t{spec.description}")
        return 0
    selected = select_benchmarks(specs, args.benchmark)

    interpreter_specs = [
        InterpreterSpec(id="pyrs", display_name="pyrs", binary=args.pyrs, kind="pyrs"),
        InterpreterSpec(
            id="cpython3143",
            display_name="CPython 3.14.3",
            binary=args.python314_bin,
            kind="cpython",
            expected_version="3.14.3",
        ),
        InterpreterSpec(
            id="cpython3108",
            display_name="CPython 3.10.8",
            binary=args.python310_bin,
            kind="cpython",
            expected_version="3.10.8",
        ),
    ]

    interpreters: dict[str, dict[str, Any]] = {}
    resolved_paths: dict[str, pathlib.Path] = {}
    for spec in interpreter_specs:
        path = resolve_executable(spec.binary)
        resolved_paths[spec.id] = path
        if spec.kind == "pyrs":
            metadata = probe_pyrs_info(path)
        else:
            assert spec.expected_version is not None
            metadata = probe_cpython_info(path, spec.expected_version)
        metadata.update(
            {
                "display_name": spec.display_name,
                "binary": str(path),
                "fingerprint": file_fingerprint(path),
            }
        )
        interpreters[spec.id] = metadata

    benchmark_rows: list[dict[str, Any]] = []
    pyrs_vs_3143: list[float] = []
    pyrs_vs_3108: list[float] = []
    py3143_vs_py3108: list[float] = []

    for spec in selected:
        results: dict[str, dict[str, Any]] = {}
        medians: dict[str, float] = {}
        for interpreter_spec in interpreter_specs:
            argv = build_argv(resolved_paths[interpreter_spec.id], spec)
            samples, verification_output = run_samples(
                argv=argv,
                expected_stdout=spec.expected_stdout,
                warmup=args.warmup,
                iterations=args.iterations,
            )
            summary = summarize(samples)
            medians[interpreter_spec.id] = summary["median"]
            results[interpreter_spec.id] = {
                "argv": argv,
                "samples_seconds": samples,
                "summary_seconds": summary,
                "verification_output": verification_output,
            }

        ratios = {
            "pyrs_vs_cpython3143": ratio(medians["pyrs"], medians["cpython3143"]),
            "pyrs_vs_cpython3108": ratio(medians["pyrs"], medians["cpython3108"]),
            "cpython3143_vs_cpython3108": ratio(medians["cpython3143"], medians["cpython3108"]),
        }
        pyrs_vs_3143.append(ratios["pyrs_vs_cpython3143"])
        pyrs_vs_3108.append(ratios["pyrs_vs_cpython3108"])
        py3143_vs_py3108.append(ratios["cpython3143_vs_cpython3108"])

        winner_by_median = min(medians.items(), key=lambda item: item[1])[0]
        benchmark_rows.append(
            {
                "id": spec.id,
                "name": spec.name,
                "category": spec.category,
                "description": spec.description,
                "unit": "seconds",
                "expected_stdout": spec.expected_stdout,
                "results": results,
                "ratios_by_median": ratios,
                "winner_by_median": winner_by_median,
            }
        )

    payload = {
        "schema_version": SCHEMA_VERSION,
        "benchmark_suite": "public_micro",
        "generated_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "git": {
            "head": git_head(),
        },
        "host": {
            "platform": platform.platform(),
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "python": platform.python_version(),
            "cpu_count": os.cpu_count(),
        },
        "config": {
            "warmup": args.warmup,
            "iterations": args.iterations,
            "benchmark_ids": [spec.id for spec in selected],
        },
        "interpreters": interpreters,
        "benchmarks": benchmark_rows,
        "summary": {
            "benchmark_count": len(benchmark_rows),
            "geometric_mean_median_ratios": {
                "pyrs_vs_cpython3143": geometric_mean(pyrs_vs_3143),
                "pyrs_vs_cpython3108": geometric_mean(pyrs_vs_3108),
                "cpython3143_vs_cpython3108": geometric_mean(py3143_vs_py3108),
            },
        },
    }

    out_path = pathlib.Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    print(
        "public microbench: "
        f"{len(benchmark_rows)} benchmarks, "
        f"pyrs_vs_cpython3143_geo_mean={payload['summary']['geometric_mean_median_ratios']['pyrs_vs_cpython3143']:.4f}x"
    )
    print(f"report: {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
