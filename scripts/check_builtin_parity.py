#!/usr/bin/env python3
"""CPython vs pyrs builtin surface and semantic probe parity gate.

This tool compares:
1) builtin symbol inventory (names, callable flag, lightweight signature metadata)
2) per-builtin semantic probes (value/exception classification)

It emits a machine-readable JSON report and can enforce a gate in CI/local runs.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import os
import pathlib
import subprocess
import sys
from typing import Any


INVENTORY_SNIPPET = r"""
import builtins
import inspect
import json
import sys

def signature_meta(obj):
    try:
        sig = inspect.signature(obj)
    except Exception as exc:
        return {
            "available": False,
            "error_type": type(exc).__name__,
        }
    empty_marker = getattr(inspect, "_empty", object())
    parameter_type = getattr(inspect, "Parameter", None)
    positional_only = getattr(parameter_type, "POSITIONAL_ONLY", "POSITIONAL_ONLY")
    positional_or_keyword = getattr(
        parameter_type, "POSITIONAL_OR_KEYWORD", "POSITIONAL_OR_KEYWORD"
    )
    var_positional_kind = getattr(parameter_type, "VAR_POSITIONAL", "VAR_POSITIONAL")
    var_keyword_kind = getattr(parameter_type, "VAR_KEYWORD", "VAR_KEYWORD")
    keyword_only_kind = getattr(parameter_type, "KEYWORD_ONLY", "KEYWORD_ONLY")

    def kind_name(kind):
        if hasattr(kind, "name"):
            return str(kind.name)
        return str(kind)

    def kind_matches(kind, expected):
        if kind == expected:
            return True
        name = kind_name(kind).upper()
        expected_name = str(expected).split(".")[-1].upper()
        return name == expected_name or expected_name in name

    parameters = getattr(sig, "parameters", None)
    if not isinstance(parameters, dict):
        return {
            "available": False,
            "error_type": "UnsupportedSignatureShape",
            "detail": type(parameters).__name__,
        }
    min_positional = 0
    max_positional = 0
    var_positional = False
    var_keyword = False
    required_kwonly = 0
    params = []
    for param in parameters.values():
        if parameter_type is not None and not isinstance(param, parameter_type):
            return {
                "available": False,
                "error_type": "UnsupportedParameterShape",
                "detail": type(param).__name__,
            }
        param_default = getattr(param, "default", empty_marker)
        has_default = param_default is not empty_marker
        param_kind = getattr(param, "kind", "POSITIONAL_OR_KEYWORD")
        kind = kind_name(param_kind)
        if kind_matches(param_kind, positional_only) or kind_matches(
            param_kind, positional_or_keyword
        ):
            max_positional += 1
            if not has_default:
                min_positional += 1
        elif kind_matches(param_kind, var_positional_kind):
            var_positional = True
        elif kind_matches(param_kind, var_keyword_kind):
            var_keyword = True
        elif kind_matches(param_kind, keyword_only_kind) and not has_default:
            required_kwonly += 1
        params.append(
            {
                "name": str(getattr(param, "name", "?")),
                "kind": kind,
                "has_default": has_default,
            }
        )
    return {
        "available": True,
        "text": str(sig),
        "min_positional": min_positional,
        "max_positional": None if var_positional else max_positional,
        "var_positional": var_positional,
        "var_keyword": var_keyword,
        "required_kwonly": required_kwonly,
        "parameters": params,
    }

entries = {}
for name, obj in builtins.__dict__.items():
    if name.startswith("__"):
        continue
    entry = {
        "callable": callable(obj),
        "type_name": type(obj).__name__,
    }
    text_sig = getattr(obj, "__text_signature__", None)
    if isinstance(text_sig, str):
        entry["text_signature"] = text_sig
    else:
        entry["text_signature"] = None
    if callable(obj):
        entry["signature"] = signature_meta(obj)
    entries[name] = entry

payload = {
    "runtime": {
        "implementation": sys.implementation.name,
        "version": list(sys.version_info[:3]),
    },
    "names": sorted(entries.keys()),
    "entries": entries,
}
print(json.dumps(payload, sort_keys=True, separators=(",", ":")))
"""


PROBE_SNIPPET = r"""
import builtins
import json
import os

probes = json.loads(os.environ["PYRS_BUILTIN_PROBES_JSON"])
results = {}

def run_probe(probe_id):
    if probe_id == "len_tuple":
        return len((1, 2, 3))
    if probe_id == "len_typeerror":
        return len(1)
    if probe_id == "pow_three_args":
        return pow(2, 5, 7)
    if probe_id == "divmod_negative":
        return divmod(-7, 3)
    if probe_id == "format_hex_spec":
        return format(255, "02X")
    if probe_id == "round_bankers":
        return round(1.25, 1)
    if probe_id == "iter_next_default":
        return next(iter([]), "sentinel")
    if probe_id == "callable_lambda":
        return callable(lambda: 1)
    if probe_id == "input_callable":
        return callable(input)
    if probe_id == "dict_missing_hook":
        cls = type("D", (dict,), {"__missing__": lambda self, key: 42})
        return cls()["x"]
    if probe_id == "bytes_rstrip_default":
        return b"abc \n\t".rstrip()
    if probe_id == "bytes_rstrip_custom":
        return b"abcxxx".rstrip(b"x")
    if probe_id == "str_format_spec":
        return "{:02X}".format(255)
    if probe_id == "builtins_true_attr":
        return hasattr(__import__("builtins"), "True")
    if probe_id == "builtins_false_attr":
        return hasattr(__import__("builtins"), "False")
    if probe_id == "builtins_none_attr":
        return hasattr(__import__("builtins"), "None")
    if probe_id == "breakpoint_callable":
        return callable(getattr(builtins, "breakpoint", None))
    if probe_id == "eval_callable":
        return callable(getattr(builtins, "eval", None))
    if probe_id == "hash_callable":
        return callable(getattr(builtins, "hash", None))
    if probe_id == "vars_callable":
        return callable(getattr(builtins, "vars", None))
    raise KeyError("unknown probe id: " + probe_id)

for probe in probes:
    probe_id = probe["id"]
    try:
        value = run_probe(probe_id)
        results[probe_id] = {
            "ok": True,
            "type": type(value).__name__,
            "repr": repr(value),
        }
    except BaseException as exc:
        results[probe_id] = {
            "ok": False,
            "exc_type": type(exc).__name__,
            "message": str(exc),
        }

print(json.dumps(results, sort_keys=True, separators=(",", ":")))
"""


DEFAULT_PROBES = [
    {"id": "len_tuple"},
    {"id": "len_typeerror"},
    {"id": "pow_three_args"},
    {"id": "divmod_negative"},
    {"id": "format_hex_spec"},
    {"id": "round_bankers"},
    {"id": "iter_next_default"},
    {"id": "callable_lambda"},
    {"id": "input_callable"},
    {"id": "dict_missing_hook"},
    {"id": "bytes_rstrip_default"},
    {"id": "bytes_rstrip_custom"},
    {"id": "str_format_spec"},
    {"id": "builtins_true_attr"},
    {"id": "builtins_false_attr"},
    {"id": "builtins_none_attr"},
    {"id": "breakpoint_callable"},
    {"id": "eval_callable"},
    {"id": "hash_callable"},
    {"id": "vars_callable"},
]


def _read_allowlist(path: pathlib.Path) -> set[str]:
    if not path.is_file():
        return set()
    entries: set[str] = set()
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.split("#", 1)[0].strip()
        if line:
            entries.add(line)
    return entries


def _run_runtime_json(
    runtime_cmd: list[str],
    snippet: str,
    extra_env: dict[str, str] | None = None,
) -> Any:
    env = os.environ.copy()
    if extra_env:
        env.update(extra_env)
    proc = subprocess.run(
        [*runtime_cmd, "-S", "-c", snippet],
        check=False,
        capture_output=True,
        text=True,
        env=env,
    )
    if proc.returncode != 0:
        stderr = proc.stderr.strip()
        stdout = proc.stdout.strip()
        raise RuntimeError(
            f"runtime command failed: {' '.join(runtime_cmd)}\n"
            f"exit={proc.returncode}\nstdout={stdout}\nstderr={stderr}"
        )
    return json.loads(proc.stdout)


def _signature_shape(entry: dict[str, Any]) -> tuple[Any, ...] | None:
    sig = entry.get("signature")
    if not isinstance(sig, dict) or not sig.get("available"):
        return None
    return (
        sig.get("min_positional"),
        sig.get("max_positional"),
        sig.get("var_positional"),
        sig.get("var_keyword"),
        sig.get("required_kwonly"),
    )


def _build_report(
    cpython_inventory: dict[str, Any],
    pyrs_inventory: dict[str, Any],
    cpython_probes: dict[str, Any],
    pyrs_probes: dict[str, Any],
    missing_allowlist: set[str],
    probe_allowlist: set[str],
    cpython_cmd: list[str],
    pyrs_cmd: list[str],
) -> dict[str, Any]:
    cpython_names = set(cpython_inventory["names"])
    pyrs_names = set(pyrs_inventory["names"])

    missing_names = sorted(cpython_names - pyrs_names)
    extra_names = sorted(pyrs_names - cpython_names)

    callability_mismatches = []
    signature_mismatches = []
    for name in sorted(cpython_names & pyrs_names):
        cp_entry = cpython_inventory["entries"][name]
        py_entry = pyrs_inventory["entries"][name]
        if cp_entry.get("callable") != py_entry.get("callable"):
            callability_mismatches.append(
                {
                    "name": name,
                    "cpython": cp_entry.get("callable"),
                    "pyrs": py_entry.get("callable"),
                }
            )
        cp_shape = _signature_shape(cp_entry)
        py_shape = _signature_shape(py_entry)
        if cp_shape is not None and py_shape is not None and cp_shape != py_shape:
            signature_mismatches.append(
                {
                    "name": name,
                    "cpython": {
                        "min_positional": cp_shape[0],
                        "max_positional": cp_shape[1],
                        "var_positional": cp_shape[2],
                        "var_keyword": cp_shape[3],
                        "required_kwonly": cp_shape[4],
                    },
                    "pyrs": {
                        "min_positional": py_shape[0],
                        "max_positional": py_shape[1],
                        "var_positional": py_shape[2],
                        "var_keyword": py_shape[3],
                        "required_kwonly": py_shape[4],
                    },
                }
            )

    probe_mismatches = []
    for probe in DEFAULT_PROBES:
        probe_id = probe["id"]
        cp_result = cpython_probes.get(probe_id)
        py_result = pyrs_probes.get(probe_id)
        if cp_result is None or py_result is None:
            probe_mismatches.append(
                {
                    "id": probe_id,
                    "reason": "missing_result",
                    "cpython": cp_result,
                    "pyrs": py_result,
                }
            )
            continue
        if cp_result.get("ok") != py_result.get("ok"):
            probe_mismatches.append(
                {
                    "id": probe_id,
                    "reason": "ok_mismatch",
                    "cpython": cp_result,
                    "pyrs": py_result,
                }
            )
            continue
        if cp_result.get("ok"):
            if (
                cp_result.get("type") != py_result.get("type")
                or cp_result.get("repr") != py_result.get("repr")
            ):
                probe_mismatches.append(
                    {
                        "id": probe_id,
                        "reason": "value_mismatch",
                        "cpython": cp_result,
                        "pyrs": py_result,
                    }
                )
        else:
            if cp_result.get("exc_type") != py_result.get("exc_type"):
                probe_mismatches.append(
                    {
                        "id": probe_id,
                        "reason": "exception_type_mismatch",
                        "cpython": cp_result,
                        "pyrs": py_result,
                    }
                )

    missing_unexpected = sorted(name for name in missing_names if name not in missing_allowlist)
    missing_allowlist_stale = sorted(name for name in missing_allowlist if name not in missing_names)
    probe_unexpected = sorted(
        mismatch["id"] for mismatch in probe_mismatches if mismatch["id"] not in probe_allowlist
    )
    probe_allowlist_stale = sorted(
        probe_id
        for probe_id in probe_allowlist
        if probe_id not in {m["id"] for m in probe_mismatches}
    )

    return {
        "timestamp_utc": _dt.datetime.now(tz=_dt.timezone.utc).isoformat(),
        "config": {
            "cpython_cmd": cpython_cmd,
            "pyrs_cmd": pyrs_cmd,
            "missing_allowlist": sorted(missing_allowlist),
            "probe_allowlist": sorted(probe_allowlist),
        },
        "inventory": {
            "cpython": cpython_inventory["runtime"],
            "pyrs": pyrs_inventory["runtime"],
            "cpython_count": len(cpython_names),
            "pyrs_count": len(pyrs_names),
            "missing_in_pyrs": missing_names,
            "extra_in_pyrs": extra_names,
            "callability_mismatches": callability_mismatches,
            "signature_shape_mismatches": signature_mismatches,
        },
        "probes": {
            "total": len(DEFAULT_PROBES),
            "mismatches": probe_mismatches,
            "results": {
                "cpython": cpython_probes,
                "pyrs": pyrs_probes,
            },
        },
        "gate": {
            "unexpected_missing": missing_unexpected,
            "missing_allowlist_stale": missing_allowlist_stale,
            "unexpected_probe_mismatches": probe_unexpected,
            "probe_allowlist_stale": probe_allowlist_stale,
            "ok": (
                not missing_unexpected
                and not probe_unexpected
                and not missing_allowlist_stale
                and not probe_allowlist_stale
            ),
        },
    }


def _default_pyrs_bin() -> pathlib.Path:
    candidate = pathlib.Path("target/debug/pyrs")
    if candidate.is_file():
        return candidate
    return candidate


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--cpython-bin",
        default=os.environ.get("PYRS_CPYTHON_BIN", sys.executable),
        help="CPython interpreter command (default: PYRS_CPYTHON_BIN or current interpreter)",
    )
    parser.add_argument(
        "--pyrs-bin",
        default=os.environ.get("PYRS_BIN", str(_default_pyrs_bin())),
        help="pyrs interpreter binary path (default: target/debug/pyrs)",
    )
    parser.add_argument(
        "--missing-allowlist",
        default="tests/builtin_missing_allowlist.txt",
        help="allowlist file for missing builtin names",
    )
    parser.add_argument(
        "--probe-allowlist",
        default="tests/builtin_probe_allowlist.txt",
        help="allowlist file for builtin probe mismatch ids",
    )
    parser.add_argument(
        "--output-json",
        default="perf/builtin_parity_report.json",
        help="machine-readable output report path",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail non-zero on unexpected missing builtins or probe mismatches",
    )
    args = parser.parse_args()

    cpython_cmd = [args.cpython_bin]
    pyrs_cmd = [args.pyrs_bin]

    missing_allowlist = _read_allowlist(pathlib.Path(args.missing_allowlist))
    probe_allowlist = _read_allowlist(pathlib.Path(args.probe_allowlist))

    cpython_inventory = _run_runtime_json(cpython_cmd, INVENTORY_SNIPPET)
    pyrs_inventory = _run_runtime_json(pyrs_cmd, INVENTORY_SNIPPET)
    probes_json = json.dumps(DEFAULT_PROBES, separators=(",", ":"))
    cpython_probes = _run_runtime_json(
        cpython_cmd,
        PROBE_SNIPPET,
        extra_env={"PYRS_BUILTIN_PROBES_JSON": probes_json},
    )
    pyrs_probes = _run_runtime_json(
        pyrs_cmd,
        PROBE_SNIPPET,
        extra_env={"PYRS_BUILTIN_PROBES_JSON": probes_json},
    )

    report = _build_report(
        cpython_inventory=cpython_inventory,
        pyrs_inventory=pyrs_inventory,
        cpython_probes=cpython_probes,
        pyrs_probes=pyrs_probes,
        missing_allowlist=missing_allowlist,
        probe_allowlist=probe_allowlist,
        cpython_cmd=cpython_cmd,
        pyrs_cmd=pyrs_cmd,
    )

    output_path = pathlib.Path(args.output_json)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    gate = report["gate"]
    inventory = report["inventory"]
    probes = report["probes"]

    print(
        "[builtin-parity] cpython_count="
        f"{inventory['cpython_count']} pyrs_count={inventory['pyrs_count']}"
    )
    print(
        "[builtin-parity] missing_in_pyrs="
        f"{len(inventory['missing_in_pyrs'])} "
        f"unexpected_missing={len(gate['unexpected_missing'])}"
    )
    print(
        "[builtin-parity] probe_mismatches="
        f"{len(probes['mismatches'])} "
        f"unexpected_probe_mismatches={len(gate['unexpected_probe_mismatches'])}"
    )
    print(f"[builtin-parity] report={output_path}")

    if args.check and not gate["ok"]:
        if gate["unexpected_missing"]:
            print("[builtin-parity] unexpected missing builtins:")
            for name in gate["unexpected_missing"]:
                print(f"  - {name}")
        if gate["missing_allowlist_stale"]:
            print("[builtin-parity] stale missing-builtin allowlist entries:")
            for name in gate["missing_allowlist_stale"]:
                print(f"  - {name}")
        if gate["unexpected_probe_mismatches"]:
            print("[builtin-parity] unexpected probe mismatches:")
            for probe_id in gate["unexpected_probe_mismatches"]:
                print(f"  - {probe_id}")
        if gate["probe_allowlist_stale"]:
            print("[builtin-parity] stale probe allowlist entries:")
            for probe_id in gate["probe_allowlist_stale"]:
                print(f"  - {probe_id}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
