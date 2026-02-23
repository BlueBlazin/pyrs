#!/usr/bin/env python3
"""Run extended stdlib import/smoke probes against pyrs and emit JSON artifact.

Usage:
  python3 scripts/probe_stdlib_extended.py \
    --pyrs target/debug/pyrs \
    --cpython-lib /path/to/Python-3.14.3/Lib \
    --out perf/stdlib_compat_extended_latest.json
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import subprocess
import sys
from dataclasses import dataclass

REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class ProbeCase:
    module: str
    import_source: str
    smoke_source: str


def detect_cpython_lib(explicit: str | None) -> pathlib.Path:
    candidates = []
    if explicit:
        candidates.append(pathlib.Path(explicit))
    env = os.environ.get("PYRS_CPYTHON_LIB")
    if env:
        candidates.append(pathlib.Path(env))
    candidates.extend(
        [
            REPO_ROOT / ".local" / "Python-3.14.3" / "Lib",
            pathlib.Path("/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14"),
        ]
    )
    for candidate in candidates:
        if candidate.joinpath("test").is_dir():
            return candidate
    raise SystemExit("could not locate CPython Lib directory; pass --cpython-lib")


def run_snippet(pyrs_bin: pathlib.Path, cpython_lib: pathlib.Path, source: str, timeout_secs: int) -> tuple[bool, str]:
    env = os.environ.copy()
    env["PYRS_CPYTHON_LIB"] = str(cpython_lib)
    proc = subprocess.run(
        [str(pyrs_bin), "-S", "-c", source],
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout_secs,
    )
    out = (proc.stdout or "") + (proc.stderr or "")
    out = out.strip()
    if len(out) > 1600:
        out = out[:1600]
    return proc.returncode == 0, out


def probe_cases() -> list[ProbeCase]:
    return [
        ProbeCase("os", "import os", "import os; assert os.path.join('a','b') == 'a/b'"),
        ProbeCase("sys", "import sys", "import sys; assert sys.version_info[0] == 3"),
        ProbeCase("pathlib", "import pathlib", "from pathlib import Path; _ = Path('.').resolve()"),
        ProbeCase("re", "import re", "import re; m = re.search(r'a|b', 'b'); assert m and m.group(0) == 'b'"),
        ProbeCase("json", "import json", "import json; s = json.dumps({'a':[1,2]}, sort_keys=True); assert json.loads(s)['a'][1] == 2"),
        ProbeCase("datetime", "import datetime", "import datetime; assert datetime.datetime(2026,1,1).year == 2026"),
        ProbeCase("time", "import time", "import time; _ = time.monotonic(); _ = time.time()"),
        ProbeCase("math", "import math", "import math; assert math.sqrt(9) == 3.0 and math.factorial(8) == 40320"),
        ProbeCase("random", "import random", "import random; r = random.Random(0); _ = r.randint(1, 10)"),
        ProbeCase("collections", "import collections", "from collections import Counter, deque; assert Counter('abca')['a'] == 2; d = deque([1,2]); assert d.pop() == 2"),
        ProbeCase("itertools", "import itertools", "import itertools; assert list(itertools.islice(itertools.cycle([1,2]), 5)) == [1,2,1,2,1]"),
        ProbeCase("functools", "import functools", "import functools\n@functools.lru_cache(maxsize=None)\ndef f(x):\n    return x + 1\nassert f(2) == 3"),
        ProbeCase("logging", "import logging", "import logging; logging.getLogger('x').setLevel(logging.INFO)"),
        ProbeCase("subprocess", "import subprocess", "import subprocess; _ = subprocess.CompletedProcess(['x'], 0)"),
        ProbeCase("typing", "import typing", "import typing; _ = typing.Optional[int]"),
        ProbeCase("argparse", "import argparse", "import argparse; p = argparse.ArgumentParser(); _ = p.parse_args([])"),
        ProbeCase("unittest", "import unittest", "import unittest\nclass T(unittest.TestCase):\n    def runTest(self): self.assertEqual(2, 1+1)\nr = unittest.TextTestRunner(verbosity=0).run(T())\nassert r.wasSuccessful()"),
        ProbeCase("threading", "import threading", "import threading\nx = {'v': 0}\ndef f(): x['v'] = 1\nt = threading.Thread(target=f); t.start(); t.join(); assert x['v'] == 1"),
        ProbeCase("multiprocessing", "import multiprocessing", "import multiprocessing; _ = multiprocessing.get_start_method(allow_none=True)"),
        ProbeCase("asyncio", "import asyncio", "import asyncio\nasync def f(): return 3\nassert asyncio.run(f()) == 3"),
        ProbeCase("csv", "import csv", "import csv, io\nrows = list(csv.reader(io.StringIO('a,b\\n1,2\\n'))); assert rows[1][1] == '2'"),
        ProbeCase("sqlite3", "import sqlite3", "import sqlite3\nconn = sqlite3.connect(':memory:')\ncur = conn.cursor(); cur.execute('select 1')\nassert cur.fetchone()[0] == 1\nconn.close()"),
        ProbeCase("urllib", "import urllib.parse", "from urllib.parse import urlparse\nassert urlparse('https://example.com/x').scheme == 'https'"),
        ProbeCase("http", "import http.client", "from http.client import HTTPConnection\nassert HTTPConnection is not None"),
        ProbeCase("hashlib", "import hashlib", "import hashlib\nh = hashlib.sha256(b'x').hexdigest(); assert len(h) == 64"),
        ProbeCase("dataclasses", "import dataclasses", "from dataclasses import dataclass\n@dataclass\nclass X:\n    a: int\nassert X(1).a == 1"),
        ProbeCase("statistics", "import statistics", "import statistics; assert statistics.mean([1,2,3,4]) == 2.5"),
        ProbeCase("decimal", "import decimal", "from decimal import Decimal; assert Decimal('1.20') + Decimal('2.30') == Decimal('3.50')"),
        ProbeCase("fractions", "import fractions", "from fractions import Fraction; assert Fraction(1,3) + Fraction(1,6) == Fraction(1,2)"),
        ProbeCase("pprint", "import pprint", "import pprint; _ = pprint.pformat({'a':[1,2,3]})"),
        ProbeCase("copy", "import copy", "import copy; x=[1,[2]]; y=copy.deepcopy(x); y[1][0]=9; assert x[1][0]==2"),
        ProbeCase("enum", "import enum", "import enum\nclass C(enum.Enum):\n    A = 1\nassert C.A.name == 'A'"),
        ProbeCase("abc", "import abc", "import abc\nclass A(metaclass=abc.ABCMeta):\n    pass\nassert isinstance(A, abc.ABCMeta)"),
        ProbeCase("inspect", "import inspect", "import inspect\ndef f(a:int)->int: return a\ns = inspect.signature(f); assert 'a' in str(s)"),
        ProbeCase("contextlib", "import contextlib", "import contextlib\nwith contextlib.nullcontext(3) as v:\n    assert v == 3"),
        ProbeCase("weakref", "import weakref", "import weakref\nclass C: pass\no=C(); r=weakref.ref(o); assert r() is o"),
        ProbeCase("queue", "import queue", "import queue\nq=queue.Queue(); q.put(1); assert q.get() == 1"),
        ProbeCase("concurrent.futures", "import concurrent.futures", "import concurrent.futures\nwith concurrent.futures.ThreadPoolExecutor(max_workers=1) as ex:\n    fut = ex.submit(lambda: 5)\n    assert fut.result() == 5"),
        ProbeCase("socket", "import socket", "import socket; assert socket.AF_INET >= 0"),
        ProbeCase("ssl", "import ssl", "import ssl; assert hasattr(ssl, 'SSLContext')"),
        ProbeCase("email", "import email.message", "from email.message import EmailMessage\nm = EmailMessage(); m['Subject'] = 'x'; m.set_content('y'); assert 'Subject' in m"),
        ProbeCase("smtplib", "import smtplib", "import smtplib; s = smtplib.SMTP(); s.close()"),
        ProbeCase("imaplib", "import imaplib", "import imaplib; stamp = imaplib.Time2Internaldate(0); assert stamp.startswith('\"') and stamp.endswith('\"')"),
        ProbeCase("ftplib", "import ftplib", "import ftplib; assert ftplib.FTP is not None"),
        ProbeCase("xml", "import xml.etree.ElementTree", "import xml.etree.ElementTree as ET\nroot = ET.fromstring('<a><b/></a>'); assert root.tag == 'a'"),
        ProbeCase("html", "import html", "import html; assert html.escape('<') == '&lt;'"),
        ProbeCase("pickle", "import pickle", "import pickle; data = pickle.dumps({'a':[1,2]}); assert pickle.loads(data)['a'][0] == 1"),
        ProbeCase("gzip", "import gzip", "import gzip; data = gzip.compress(b'abc'); assert gzip.decompress(data) == b'abc'"),
        ProbeCase("bz2", "import bz2", "import bz2; data = bz2.compress(b'abc'); assert bz2.decompress(data) == b'abc'"),
        ProbeCase("lzma", "import lzma", "import lzma; data = lzma.compress(b'abc'); assert lzma.decompress(data) == b'abc'"),
    ]


def main() -> None:
    parser = argparse.ArgumentParser(description="Probe extended stdlib compat matrix against pyrs")
    parser.add_argument("--pyrs", default="target/debug/pyrs", help="Path to pyrs binary")
    parser.add_argument("--cpython-lib", default=None, help="Path to CPython 3.14 Lib directory")
    parser.add_argument("--out", default="perf/stdlib_compat_extended_latest.json", help="Output JSON artifact path")
    parser.add_argument("--timeout", type=int, default=20, help="Per-snippet timeout (seconds)")
    args = parser.parse_args()

    pyrs_bin = pathlib.Path(args.pyrs)
    if not pyrs_bin.is_file():
        raise SystemExit(f"pyrs binary not found: {pyrs_bin}")

    cpython_lib = detect_cpython_lib(args.cpython_lib)
    cases = probe_cases()

    rows = []
    import_pass = 0
    smoke_pass = 0
    for case in cases:
        import_ok, import_msg = run_snippet(pyrs_bin, cpython_lib, case.import_source, args.timeout)
        smoke_ok = False
        smoke_msg = ""
        if import_ok:
            smoke_ok, smoke_msg = run_snippet(pyrs_bin, cpython_lib, case.smoke_source, args.timeout)
        else:
            smoke_msg = "skipped due to import failure"
        if import_ok:
            import_pass += 1
        if smoke_ok:
            smoke_pass += 1
        rows.append(
            {
                "module": case.module,
                "import_ok": import_ok,
                "smoke_ok": smoke_ok,
                "import_err": import_msg,
                "smoke_err": smoke_msg,
            }
        )

    payload = {
        "pyrs_bin": str(pyrs_bin),
        "cpython_lib": str(cpython_lib),
        "total": len(cases),
        "import_pass": import_pass,
        "smoke_pass": smoke_pass,
        "rows": rows,
    }

    out_path = pathlib.Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    print(f"import_pass={import_pass}/{len(cases)} smoke_pass={smoke_pass}/{len(cases)}")
    print(f"wrote {out_path}")


if __name__ == "__main__":
    main()
