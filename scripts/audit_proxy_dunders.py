#!/usr/bin/env python3
"""Audit proxy special-method exposure against CPython for selected NumPy objects."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


TARGETS = {
    "float64_scalar": "np.float64(1.5)",
    "float32_scalar": "np.float32(1.5)",
    "int64_scalar": "np.int64(3)",
    "uint32_scalar": "np.uint32(3)",
    "bool_scalar": "np.bool_(True)",
    "ndarray_1d": "np.arange(3)",
}

ATTRS = [
    "__repr__",
    "__str__",
    "__format__",
    "__bool__",
    "__int__",
    "__float__",
    "__index__",
    "__hash__",
    "__eq__",
    "__ne__",
    "__lt__",
    "__le__",
    "__gt__",
    "__ge__",
    "__getitem__",
    "__setitem__",
    "__iter__",
    "__len__",
]


def build_probe_code(site_packages: str, cpython_lib: str | None) -> str:
    paths = [site_packages]
    if cpython_lib:
        paths.insert(0, cpython_lib)
    path_inits = "\n".join(f"sys.path.insert(0, {path!r})" for path in paths)
    targets = ",\n    ".join(f"{name!r}: {expr}" for name, expr in TARGETS.items())
    attrs = ", ".join(repr(attr) for attr in ATTRS)
    return f"""\
import json
import sys
{path_inits}
import numpy as np

targets = {{
    {targets}
}}
attrs = [{attrs}]
out = {{}}
for name, obj in targets.items():
    out[name] = {{attr: hasattr(obj, attr) for attr in attrs}}
print(json.dumps(out, sort_keys=True))
"""


def run_probe(
    executable: str, site_packages: str, cpython_lib: str | None
) -> dict[str, dict[str, bool]]:
    code = build_probe_code(site_packages, cpython_lib)
    proc = subprocess.run(
        [executable, "-S", "-c", code],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"probe failed for {executable} (exit={proc.returncode})\n"
            f"stdout:\n{proc.stdout}\n"
            f"stderr:\n{proc.stderr}"
        )
    stdout = proc.stdout.strip().splitlines()
    if not stdout:
        raise RuntimeError(f"probe produced no output for {executable}")
    return json.loads(stdout[-1])


def compare_surfaces(
    cpython: dict[str, dict[str, bool]],
    pyrs: dict[str, dict[str, bool]],
) -> tuple[list[dict[str, object]], list[dict[str, object]]]:
    missing_in_pyrs: list[dict[str, object]] = []
    extra_in_pyrs: list[dict[str, object]] = []
    for target in sorted(cpython.keys()):
        c_target = cpython.get(target, {})
        p_target = pyrs.get(target, {})
        for attr in sorted(set(c_target.keys()) | set(p_target.keys())):
            c_has = bool(c_target.get(attr, False))
            p_has = bool(p_target.get(attr, False))
            if c_has and not p_has:
                missing_in_pyrs.append({"target": target, "attr": attr})
            elif p_has and not c_has:
                extra_in_pyrs.append({"target": target, "attr": attr})
    return missing_in_pyrs, extra_in_pyrs


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pyrs", required=True, help="Path to pyrs executable")
    parser.add_argument("--cpython", required=True, help="Path to CPython executable")
    parser.add_argument(
        "--site-packages",
        required=True,
        help="Path to site-packages containing numpy",
    )
    parser.add_argument(
        "--cpython-lib",
        default=None,
        help="Optional CPython Lib path to prepend in probe runtimes",
    )
    parser.add_argument(
        "--out",
        default="perf/proxy_dunder_audit_latest.json",
        help="Output JSON report path",
    )
    args = parser.parse_args()

    cpython = run_probe(args.cpython, args.site_packages, args.cpython_lib)
    pyrs = run_probe(args.pyrs, args.site_packages, args.cpython_lib)
    missing_in_pyrs, extra_in_pyrs = compare_surfaces(cpython, pyrs)

    report = {
        "summary": {
            "targets": len(TARGETS),
            "attrs": len(ATTRS),
            "missing_in_pyrs_count": len(missing_in_pyrs),
            "extra_in_pyrs_count": len(extra_in_pyrs),
        },
        "missing_in_pyrs": missing_in_pyrs,
        "extra_in_pyrs": extra_in_pyrs,
        "cpython": cpython,
        "pyrs": pyrs,
    }
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"Wrote {out_path}")
    print(
        "missing_in_pyrs:",
        len(missing_in_pyrs),
        "extra_in_pyrs:",
        len(extra_in_pyrs),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
