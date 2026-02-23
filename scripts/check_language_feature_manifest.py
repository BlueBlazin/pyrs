#!/usr/bin/env python3
"""Validate source-language feature accounting against CPython 3.14 probes."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


Probe = dict[str, Any]


PROBES: list[Probe] = [
    {
        "id": "pattern_matching_sequence_and_mapping",
        "mode": "json_result",
        "source": """items = [{"x": 2, "y": 3}, [1, 2, 3], "other"]
out = []
for item in items:
    match item:
        case {"x": a, "y": b}:
            out.append(["map", a + b])
        case [first, *rest]:
            out.append(["seq", first, len(rest)])
        case _:
            out.append(["other", None])
result = out
""",
    },
    {
        "id": "exception_group_except_star_split",
        "mode": "json_result",
        "source": """try:
    raise ExceptionGroup("eg", [ValueError(1), TypeError(2)])
except* ValueError as eg:
    left = [len(eg.exceptions), type(eg.exceptions[0]).__name__]
except* TypeError as tg:
    right = [len(tg.exceptions), type(tg.exceptions[0]).__name__]
result = {"left": left, "right": right}
""",
    },
    {
        "id": "positional_only_and_kwonly_calling",
        "mode": "json_result",
        "source": """def f(a, /, b=2, *, c=3):
    return [a, b, c]
result = [f(1), f(1, 4, c=5)]
""",
    },
    {
        "id": "runtime_function_type_params",
        "mode": "json_result",
        "source": """def ident[T, *Ts, **P](x):
    return x
params = ident.__type_params__
result = {
    "kind_names": [type(tp).__name__ for tp in params],
    "names": [tp.__name__ for tp in params],
    "call_result": ident(7),
}
""",
    },
    {
        "id": "runtime_class_type_params",
        "mode": "json_result",
        "source": """class Box[T, *Ts, **P]:
    pass
params = Box.__type_params__
result = {
    "kind_names": [type(tp).__name__ for tp in params],
    "names": [tp.__name__ for tp in params],
        }
""",
    },
    {
        "id": "runtime_type_param_bound_constraints_defaults",
        "mode": "json_result",
        "source": """def f[T: int = str](x):
    return x
def g[T: (int, str)](x):
    return x
def h[*Ts = [int]](x):
    return x
def p[**P = [int, str]](x):
    return x
ft = f.__type_params__[0]
gt = g.__type_params__[0]
ht = h.__type_params__[0]
pt = p.__type_params__[0]
result = {
    "fb": getattr(ft, "__bound__", None) is int,
    "fd": getattr(ft, "__default__", None) is str,
    "gc": [c.__name__ for c in getattr(gt, "__constraints__", ())],
    "hd": repr(getattr(ht, "__default__", None)),
    "pd": repr(getattr(pt, "__default__", None)),
}
""",
    },
    {
        "id": "runtime_type_param_cross_references",
        "mode": "json_result",
        "source": """def f[T, U: T](x):
    return x
def g[T = int, U = list[T]](x):
    return x
ft, fu = f.__type_params__
gt, gu = g.__type_params__
default = gu.__default__
result = {
    "bound_is_prior_param": fu.__bound__ is ft,
    "default_type": type(default).__name__,
    "default_origin_is_list": getattr(default, "__origin__", None) is list,
    "default_arg0_is_prior_param": (
        hasattr(default, "__args__")
        and len(default.__args__) == 1
        and default.__args__[0] is gt
    ),
}
""",
    },
    {
        "id": "runtime_builtin_generic_alias_without_types_import",
        "mode": "json_result",
        "source": """before = type(list[int]).__name__
import types
alias = list[int]
result = {
    "before_type": before,
    "isinstance": isinstance(alias, types.GenericAlias),
    "origin_is_list": getattr(alias, "__origin__", None) is list,
    "arg0_is_int": (
        hasattr(alias, "__args__")
        and len(alias.__args__) == 1
        and alias.__args__[0] is int
    ),
}
""",
    },
    {
        "id": "runtime_type_alias_type_params",
        "mode": "json_result",
        "source": """type Pair[T] = tuple[T, T]
result = {
    "type_name": type(Pair).__name__,
    "names": [tp.__name__ for tp in Pair.__type_params__],
    "repr": repr(Pair),
}
""",
    },
    {
        "id": "template_literal_basic_type",
        "mode": "json_result",
        "source": """variety = "Stilton"
template = t"Try some {variety} cheese!"
interp = template.interpolations[0]
result = {
    "type": repr(type(template)),
    "strings": list(template.strings),
    "interp": [interp.value, interp.expression, interp.conversion, interp.format_spec],
}
""",
    },
    {
        "id": "template_literal_debug_and_concat",
        "mode": "json_result",
        "source": """x = 7
t1 = t"{x=}"
t2 = t"{x=:>4}"
t3 = t"a{1}" t"b{2}"
result = {
    "t1_strings": list(t1.strings),
    "t1_interp": [[i.expression, i.conversion, i.format_spec, i.value] for i in t1.interpolations],
    "t2_strings": list(t2.strings),
    "t2_interp": [[i.expression, i.conversion, i.format_spec, i.value] for i in t2.interpolations],
    "t3_strings": list(t3.strings),
    "t3_interp": [[i.expression, i.conversion, i.format_spec, i.value] for i in t3.interpolations],
}
""",
    },
    {
        "id": "template_literal_mixed_literal_rejected",
        "mode": "stderr_contains",
        "source": "x = 'a' t'b'\n",
        "needle": "cannot mix t-string literals with string or bytes literals",
    },
    {
        "id": "template_literal_incompatible_f_prefix_rejected",
        "mode": "stderr_contains",
        "source": "x = tf'{x}'\n",
        "needle": "'f' and 't' prefixes are incompatible",
    },
    {
        "id": "template_literal_incompatible_b_prefix_rejected",
        "mode": "stderr_contains",
        "source": "x = bt'raw'\n",
        "needle": "'b' and 't' prefixes are incompatible",
    },
    {
        "id": "template_literal_incompatible_u_prefix_rejected",
        "mode": "stderr_contains",
        "source": "x = ut'raw'\n",
        "needle": "'u' and 't' prefixes are incompatible",
    },
    {
        "id": "umbrella_tokenization_and_operators_surface",
        "mode": "json_result",
        "source": """expr = 1 + 2 - 3 * 4 / 5 // 1 % 2 ** 3
raw = r"a\\nb"
data = b"xy"
num = (0b1010, 0o77, 0xFF, 1_234_567, 1.25e2, 3j)
template = t"v={expr}"
ftext = f"{1+2}"
result = {
    "num0": num[0],
    "raw": raw,
    "bytes": list(data),
    "ftext": ftext,
    "template_strings": list(template.strings),
}
""",
    },
    {
        "id": "grammar_section_statements_surface",
        "mode": "json_result",
        "source": """x = 0
import math as m
from math import sqrt as s

def g():
    global x
    x = 3
    y = 4
    def inner():
        nonlocal y
        y = 5
    inner()
    assert y == 5
    del y
    return x

result = {
    "g": g(),
    "sqrt": s(9),
    "import_ok": int(m.pi),
}
""",
    },
    {
        "id": "grammar_section_compound_surface",
        "mode": "json_result",
        "source": """from contextlib import nullcontext

class C[T]:
    def m(self, y):
        return y + 1

def f(a, /, b = 2, *, c = 3):
    acc = 0
    if a:
        acc += 1
    for i in [1, 2]:
        if i == 1:
            continue
        acc += i
    j = 0
    while j < 1:
        j += 1
    else:
        acc += 1
    try:
        raise ExceptionGroup("eg", [ValueError(1), TypeError(2)])
    except* ValueError as e:
        acc += len(e.exceptions)
    except* TypeError as e:
        acc += len(e.exceptions)
    with nullcontext(5) as n:
        acc += n
    match {"k": 5}:
        case {"k": q}:
            acc += q
    return acc + b + c

type Alias[X] = list[X]
result = {
    "f": f(1),
    "c": C().m(2),
    "class_generic_name": type(C[int]).__name__,
    "alias_type": type(Alias).__name__,
}
""",
    },
    {
        "id": "grammar_section_expressions_surface",
        "mode": "json_result",
        "source": """vals = [x * x for x in range(5) if x % 2 == 0]
gen = sum(x for x in range(4))
lam = (lambda a, b = 2, *, c = 3: a + b + c)(1, c = 4)
wal = (y := 5) + y
sub = ([10, 20, 30][1], {"a": 1}["a"], (1, 2, 3)[0])
arith = ((2 + 3) * 4 - 5) // 3
bitwise = (7 & 3, 7 | 3, 7 ^ 3, 8 >> 1, 1 << 3)
result = {
    "vals": vals,
    "gen": gen,
    "lam": lam,
    "wal": wal,
    "sub": sub,
    "arith": arith,
    "bitwise": bitwise,
}
""",
    },
    {
        "id": "grammar_section_literals_surface",
        "mode": "json_result",
        "source": """raw = r"a\\nb"
escaped = "a\\nb"
bytes_data = b"abc"
ftext = f"{1+2}"
template = t"v={1+2}"
nums = [0b11, 0o7, 0x10, 1_000, 1.25e2, 3j]
d = {"a": 1, **{"b": 2}}
s = {1, 2, 3}
result = {
    "raw": raw,
    "escaped_len": len(escaped),
    "bytes": list(bytes_data),
    "ftext": ftext,
    "template_strings": list(template.strings),
    "nums0": nums[0],
    "dict_keys": sorted(d.keys()),
    "set_len": len(s),
}
""",
    },
    {
        "id": "grammar_section_invalid_syntax_surface",
        "mode": "json_result",
        "source": """sources = [
    "def broken(:\\n    pass\\n",
    "x = (1 2)\\n",
    "if True print(1)\\n",
    "a if b\\n",
]
flags = []
for src in sources:
    try:
        compile(src, "<broken>", "exec")
    except SyntaxError:
        flags.append(True)
    else:
        flags.append(False)
result = {"ok": all(flags), "n": len(flags)}
""",
    },
    {
        "id": "umbrella_reference_expressions_surface",
        "mode": "json_result",
        "source": """vals = [x * x for x in range(5) if x % 2 == 0]
gen = sum(x for x in range(4))
lam = (lambda a, b = 2, *, c = 3: a + b + c)(1, c = 4)
wal = (y := 5) + y
sub = ([10, 20, 30][1], {"a": 1}["a"], (1, 2, 3)[0])
result = {
    "vals": vals,
    "gen": gen,
    "lam": lam,
    "wal": wal,
    "sub": sub,
}
""",
    },
    {
        "id": "umbrella_reference_lexical_analysis_surface",
        "mode": "json_result",
        "source": """raw = r"a\\nb"
escaped = "a\\nb"
bytes_data = b"abc"
ftext = f"{1+2}"
template = t"v={1+2}"
nums = [0b11, 0o7, 0x10, 1_000, 1.25e2, 3j]
result = {
    "raw": raw,
    "escaped_len": len(escaped),
    "bytes": list(bytes_data),
    "ftext": ftext,
    "template_strings": list(template.strings),
    "nums0": nums[0],
}
""",
    },
    {
        "id": "umbrella_reference_simple_stmts_surface",
        "mode": "json_result",
        "source": """x = 1
assert x == 1

def g():
    y = 3
    del y
    yield 5

v = next(g())

def h():
    global _g
    _g = 7
    z = 1
    def inner():
        nonlocal z
        z = 9
    inner()
    return z

r = h()
import math as _m
from math import sqrt as _sqrt
type T = int
result = {
    "v": v,
    "r": r,
    "sqrt": _sqrt(16),
    "g": _g,
    "type_stmt": type(T).__name__,
}
""",
    },
    {
        "id": "umbrella_reference_compound_stmts_surface",
        "mode": "json_result",
        "source": """from contextlib import nullcontext
out = []
if True:
    out.append("if")
for i in range(3):
    if i == 1:
        continue
    out.append(i)
j = 0
while j < 2:
    j += 1
else:
    out.append("while_else")
try:
    raise ValueError("x")
except ValueError:
    out.append("except")
finally:
    out.append("finally")
with nullcontext(5) as n:
    out.append(n)
match ("a", 1):
    case ("a", x):
        out.append(x)
async def af():
    return 3
class K:
    pass
result = out
""",
    },
    {
        "id": "umbrella_reference_import_surface",
        "mode": "json_result",
        "source": """import math
import importlib
module = importlib.import_module("json")
from math import factorial as f
result = {
    "pi": round(math.pi, 3),
    "fact": f(5),
    "json": hasattr(module, "dumps"),
}
""",
    },
    {
        "id": "umbrella_reference_executionmodel_surface",
        "mode": "json_result",
        "source": """x = 10
def outer():
    y = 20
    def inner():
        nonlocal y
        return x + y
    return inner()

class C:
    z = 30
    def m(self):
        return self.z

result = {
    "scope": outer(),
    "method": C().m(),
}
""",
    },
    {
        "id": "umbrella_reference_datamodel_surface",
        "mode": "json_result",
        "source": """class D:
    def __init__(self):
        self.v = [1, 2, 3]
    def __len__(self):
        return len(self.v)
    def __iter__(self):
        return iter(self.v)
    def __getitem__(self, i):
        return self.v[i]
    def __enter__(self):
        return self
    def __exit__(self, exc_type, exc, tb):
        return False

d = D()
with d as q:
    total = sum(q)
result = {
    "len": len(d),
    "item": d[1],
    "total": total,
}
""",
    },
    {
        "id": "umbrella_reference_toplevel_components_surface",
        "mode": "json_result",
        "source": """code = compile("a=1\\nb=2\\nresult=a+b", "<x>", "exec")
ns = {}
exec(code, ns, ns)
result = {"compiled_exec": ns["result"]}
""",
    },
]


def run_cmd(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, text=True, capture_output=True, check=False)


def run_json_result(interpreter: Path, source: str) -> tuple[bool, Any]:
    wrapped = f"{source}\nimport json\nprint(json.dumps(result))\n"
    proc = run_cmd([str(interpreter), "-S", "-c", wrapped])
    if proc.returncode != 0:
        return False, proc.stderr.strip()
    payload = proc.stdout.strip()
    try:
        return True, json.loads(payload)
    except json.JSONDecodeError:
        return False, payload


def run_stderr_contains(interpreter: Path, source: str, needle: str) -> tuple[bool, str]:
    proc = run_cmd([str(interpreter), "-S", "-c", source])
    if proc.returncode == 0:
        return False, "expected failure but command succeeded"
    stderr = proc.stderr
    return needle in stderr, stderr.strip()


def load_manifest(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def validate_manifest_ids(manifest: dict[str, Any]) -> tuple[list[str], list[str]]:
    manifest_ids = {entry["id"] for entry in manifest.get("features", [])}
    probe_ids = {probe["id"] for probe in PROBES}
    missing = sorted(probe_ids - manifest_ids)
    unknown = sorted(manifest_ids - probe_ids)
    return missing, unknown


def build_manifest_index(manifest: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {entry["id"]: entry for entry in manifest.get("features", [])}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pyrs", required=True, type=Path, help="Path to pyrs binary")
    parser.add_argument(
        "--cpython-bin",
        default="python3",
        type=Path,
        help="CPython 3.14 binary",
    )
    parser.add_argument(
        "--manifest",
        required=True,
        type=Path,
        help="docs/LANGUAGE_FEATURE_MANIFEST.json path",
    )
    parser.add_argument(
        "--out",
        required=True,
        type=Path,
        help="Output JSON path (perf/language_feature_manifest_latest.json)",
    )
    args = parser.parse_args()

    manifest = load_manifest(args.manifest)
    missing, unknown = validate_manifest_ids(manifest)
    index = build_manifest_index(manifest)

    report: dict[str, Any] = {
        "schema_version": 1,
        "cpython_bin": str(args.cpython_bin),
        "pyrs_bin": str(args.pyrs),
        "manifest_path": str(args.manifest),
        "missing_manifest_ids": missing,
        "unknown_manifest_ids": unknown,
        "features": [],
        "summary": {
            "total": 0,
            "pass": 0,
            "fail": 0,
            "required_fail": 0,
        },
    }

    for probe in PROBES:
        feature_id = probe["id"]
        mode = probe["mode"]
        source = probe["source"]

        if mode == "json_result":
            cp_ok, cp_payload = run_json_result(args.cpython_bin, source)
            py_ok, py_payload = run_json_result(args.pyrs, source)
            passed = cp_ok and py_ok and cp_payload == py_payload
            details = {
                "cpython_ok": cp_ok,
                "pyrs_ok": py_ok,
                "cpython_payload": cp_payload,
                "pyrs_payload": py_payload,
            }
        elif mode == "stderr_contains":
            needle = probe["needle"]
            cp_ok, cp_payload = run_stderr_contains(args.cpython_bin, source, needle)
            py_ok, py_payload = run_stderr_contains(args.pyrs, source, needle)
            passed = cp_ok and py_ok
            details = {
                "needle": needle,
                "cpython_ok": cp_ok,
                "pyrs_ok": py_ok,
                "cpython_stderr": cp_payload,
                "pyrs_stderr": py_payload,
            }
        else:
            raise RuntimeError(f"unknown probe mode: {mode}")

        manifest_entry = index.get(feature_id, {})
        required = bool(manifest_entry.get("required", False))
        status = manifest_entry.get("status", "untracked")
        owner = manifest_entry.get("owner", "")
        closure = manifest_entry.get("closure_criteria", "")

        row = {
            "id": feature_id,
            "required": required,
            "status": status,
            "owner": owner,
            "closure_criteria": closure,
            "pass": passed,
            "details": details,
        }
        report["features"].append(row)
        report["summary"]["total"] += 1
        if passed:
            report["summary"]["pass"] += 1
        else:
            report["summary"]["fail"] += 1
            if required:
                report["summary"]["required_fail"] += 1

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")

    failed = False
    if missing or unknown:
        failed = True
    if report["summary"]["required_fail"] > 0:
        failed = True

    if failed:
        print("language feature manifest check failed", file=sys.stderr)
        print(
            json.dumps(
                {
                    "missing_manifest_ids": missing,
                    "unknown_manifest_ids": unknown,
                    "required_fail": report["summary"]["required_fail"],
                    "out": str(args.out),
                },
                indent=2,
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1

    print(
        f"language feature manifest: {report['summary']['pass']}/{report['summary']['total']} probes passed"
    )
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
