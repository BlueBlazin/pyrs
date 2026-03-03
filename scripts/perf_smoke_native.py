#!/usr/bin/env python3
"""Native performance smoke gate for pyrs CLI execution paths."""

from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


def fib_value(n: int) -> int:
    a = 0
    b = 1
    for _ in range(n):
        a, b = b, a + b
    return a


def run_once(bin_path: str, code: str) -> tuple[float, str]:
    started = time.perf_counter()
    proc = subprocess.run(
        [bin_path, "-c", code],
        check=True,
        capture_output=True,
        text=True,
    )
    elapsed = time.perf_counter() - started
    return elapsed, proc.stdout.strip()


def run_samples(bin_path: str, code: str, warmup: int, iterations: int) -> list[float]:
    for _ in range(warmup):
        run_once(bin_path, code)
    samples: list[float] = []
    for _ in range(iterations):
        elapsed, _ = run_once(bin_path, code)
        samples.append(elapsed)
    return samples


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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pyrs", default="target/release/pyrs", help="Path to pyrs binary.")
    parser.add_argument("--python-bin", default="python3", help="Reference CPython binary.")
    parser.add_argument("--fib-n", type=int, default=36, help="Fibonacci N for workload.")
    parser.add_argument("--warmup", type=int, default=1, help="Warmup runs per binary.")
    parser.add_argument("--iterations", type=int, default=3, help="Measured runs per binary.")
    parser.add_argument(
        "--max-pyrs-seconds",
        type=float,
        default=12.0,
        help="Fail if pyrs mean runtime exceeds this threshold.",
    )
    parser.add_argument(
        "--max-pyrs-vs-python-ratio",
        type=float,
        default=8.0,
        help="Fail if pyrs_mean/python_mean exceeds this threshold.",
    )
    parser.add_argument(
        "--out",
        default="perf/native_perf_smoke_latest.json",
        help="JSON output path.",
    )
    args = parser.parse_args()

    if args.fib_n < 0:
        raise SystemExit("--fib-n must be >= 0")
    if args.warmup < 0:
        raise SystemExit("--warmup must be >= 0")
    if args.iterations <= 0:
        raise SystemExit("--iterations must be > 0")

    expected = str(fib_value(args.fib_n))
    code = (
        "fib = lambda n: n if n < 2 else fib(n - 1) + fib(n - 2); "
        f"print(fib({args.fib_n}))"
    )

    pyrs_samples = run_samples(args.pyrs, code, args.warmup, args.iterations)
    py_samples = run_samples(args.python_bin, code, args.warmup, args.iterations)

    pyrs_last_elapsed, pyrs_last_out = run_once(args.pyrs, code)
    py_last_elapsed, py_last_out = run_once(args.python_bin, code)
    if pyrs_last_out != expected:
        raise SystemExit(
            f"pyrs produced unexpected fib({args.fib_n}) output: {pyrs_last_out!r} != {expected!r}"
        )
    if py_last_out != expected:
        raise SystemExit(
            f"python produced unexpected fib({args.fib_n}) output: {py_last_out!r} != {expected!r}"
        )

    pyrs_summary = summarize(pyrs_samples)
    py_summary = summarize(py_samples)
    ratio = pyrs_summary["mean"] / py_summary["mean"]

    failures: list[str] = []
    if pyrs_summary["mean"] > args.max_pyrs_seconds:
        failures.append(
            "pyrs mean runtime exceeded threshold: "
            f"{pyrs_summary['mean']:.4f}s > {args.max_pyrs_seconds:.4f}s"
        )
    if ratio > args.max_pyrs_vs_python_ratio:
        failures.append(
            "pyrs/python runtime ratio exceeded threshold: "
            f"{ratio:.4f}x > {args.max_pyrs_vs_python_ratio:.4f}x"
        )

    report: dict[str, Any] = {
        "fib_n": args.fib_n,
        "warmup": args.warmup,
        "iterations": args.iterations,
        "thresholds": {
            "max_pyrs_seconds": args.max_pyrs_seconds,
            "max_pyrs_vs_python_ratio": args.max_pyrs_vs_python_ratio,
        },
        "pyrs": {
            "binary": args.pyrs,
            "samples_seconds": pyrs_samples,
            "summary_seconds": pyrs_summary,
            "verification_run_seconds": pyrs_last_elapsed,
            "verification_output": pyrs_last_out,
        },
        "python": {
            "binary": args.python_bin,
            "samples_seconds": py_samples,
            "summary_seconds": py_summary,
            "verification_run_seconds": py_last_elapsed,
            "verification_output": py_last_out,
        },
        "ratio_pyrs_vs_python": ratio,
        "status": "pass" if not failures else "fail",
        "failures": failures,
    }

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    print(
        "native perf smoke: "
        f"pyrs_mean={pyrs_summary['mean']:.4f}s "
        f"python_mean={py_summary['mean']:.4f}s "
        f"ratio={ratio:.3f}x"
    )
    print(f"report: {out_path}")

    if failures:
        for row in failures:
            print(f"FAIL: {row}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
