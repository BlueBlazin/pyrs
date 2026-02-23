use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn detect_cpython_lib() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_CPYTHON_LIB") {
        let path = PathBuf::from(path);
        if path.join("test").is_dir() {
            return Some(path);
        }
    }
    let candidates = [
        "/Users/$USER/Downloads/Python-3.14.3/Lib",
        "/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14",
    ];
    for candidate in candidates {
        let path = PathBuf::from(candidate);
        if path.join("test").is_dir() {
            return Some(path);
        }
    }
    None
}

fn pyrs_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_SUBPROCESS_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let debug = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/pyrs");
    if debug.is_file() {
        return Some(debug);
    }
    let release = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/pyrs");
    if release.is_file() {
        return Some(release);
    }
    None
}

fn run_snippet(bin: &PathBuf, cpython_lib: &PathBuf, source: &str, timeout: Duration) -> bool {
    let mut child = match Command::new(bin)
        .env("PYRS_CPYTHON_LIB", cpython_lib)
        .arg("-S")
        .arg("-c")
        .arg(source)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return false,
        }
    }
}

#[test]
fn top_stdlib_common_usecase_baseline_gate() {
    if std::env::var("PYRS_RUN_STDLIB_COMMON_USECASES")
        .ok()
        .as_deref()
        != Some("1")
    {
        eprintln!(
            "skipping stdlib common-usecase gate (set PYRS_RUN_STDLIB_COMMON_USECASES=1 to enable)"
        );
        return;
    }
    let Some(cpython_lib) = detect_cpython_lib() else {
        eprintln!("skipping stdlib common-usecase gate (CPython Lib not found)");
        return;
    };
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping stdlib common-usecase gate (pyrs binary not found)");
        return;
    };

    // Baseline thresholds from docs/STDLIB_COMMON_USECASE_CHECKLIST.md.
    const MIN_IMPORT_PASS: usize = 26;
    const MIN_SMOKE_PASS: usize = 26;
    let timeout = Duration::from_secs(12);

    let cases = vec![
        (
            "os",
            "import os",
            "import os; assert os.path.join('a','b') == 'a/b'",
        ),
        (
            "sys",
            "import sys",
            "import sys; assert sys.version_info[0] == 3",
        ),
        (
            "pathlib",
            "import pathlib",
            "from pathlib import Path; _ = Path('.').resolve()",
        ),
        (
            "re",
            "import re",
            "import re; m = re.search(r'a|b', 'b'); assert m and m.group(0) == 'b'",
        ),
        (
            "json",
            "import json",
            "import json; s = json.dumps({'a':[1,2]}, sort_keys=True); assert json.loads(s)['a'][1] == 2",
        ),
        (
            "datetime",
            "import datetime",
            "import datetime; assert datetime.datetime(2026,1,1).year == 2026",
        ),
        (
            "time",
            "import time",
            "import time; _ = time.monotonic(); _ = time.time()",
        ),
        (
            "math",
            "import math",
            "import math; assert math.sqrt(9) == 3.0 and math.factorial(8) == 40320",
        ),
        (
            "random",
            "import random",
            "import random; r = random.Random(0); _ = r.randint(1, 10)",
        ),
        (
            "collections",
            "import collections",
            "from collections import Counter, deque; assert Counter('abca')['a'] == 2; d = deque([1,2]); assert d.pop() == 2",
        ),
        (
            "itertools",
            "import itertools",
            "import itertools; assert list(itertools.islice(itertools.cycle([1,2]), 5)) == [1,2,1,2,1]",
        ),
        (
            "functools",
            "import functools",
            "import functools\n@functools.lru_cache(maxsize=None)\ndef f(x):\n    return x + 1\nassert f(2) == 3",
        ),
        (
            "logging",
            "import logging",
            "import logging; logging.getLogger('x').setLevel(logging.INFO)",
        ),
        (
            "subprocess",
            "import subprocess",
            "import subprocess; _ = subprocess.CompletedProcess(['x'], 0)",
        ),
        (
            "typing",
            "import typing",
            "import typing; _ = typing.Optional[int]",
        ),
        (
            "argparse",
            "import argparse",
            "import argparse; p = argparse.ArgumentParser(); _ = p.parse_args([])",
        ),
        (
            "unittest",
            "import unittest",
            "import unittest\nclass T(unittest.TestCase):\n    def runTest(self): self.assertEqual(2, 1+1)\nr = unittest.TextTestRunner(verbosity=0).run(T())\nassert r.wasSuccessful()",
        ),
        (
            "threading",
            "import threading",
            "import threading\nx = {'v': 0}\ndef f(): x['v'] = 1\nt = threading.Thread(target=f); t.start(); t.join(); assert x['v'] == 1",
        ),
        (
            "multiprocessing",
            "import multiprocessing",
            "import multiprocessing; _ = multiprocessing.get_start_method(allow_none=True)",
        ),
        (
            "asyncio",
            "import asyncio",
            "import asyncio\nasync def f(): return 3\nassert asyncio.run(f()) == 3",
        ),
        (
            "csv",
            "import csv",
            "import csv, io\nrows = list(csv.reader(io.StringIO('a,b\\n1,2\\n')))\nassert rows[1][1] == '2'",
        ),
        (
            "sqlite3",
            "import sqlite3",
            "import sqlite3\nconn = sqlite3.connect(':memory:')\ncur = conn.cursor(); cur.execute('select 1')\nassert cur.fetchone()[0] == 1\nconn.close()",
        ),
        (
            "urllib",
            "import urllib.parse",
            "from urllib.parse import urlparse\nassert urlparse('https://example.com/x').scheme == 'https'",
        ),
        (
            "http",
            "import http.client",
            "from http.client import HTTPConnection\nassert HTTPConnection is not None",
        ),
        (
            "hashlib",
            "import hashlib",
            "import hashlib\nh = hashlib.sha256(b'x').hexdigest(); assert len(h) == 64",
        ),
        (
            "dataclasses",
            "import dataclasses",
            "from dataclasses import dataclass\n@dataclass\nclass X:\n    a: int\nassert X(1).a == 1",
        ),
    ];

    let mut import_pass = 0usize;
    let mut smoke_pass = 0usize;
    let mut regressions = Vec::new();
    for (name, import_source, smoke_source) in &cases {
        let import_ok = run_snippet(&bin, &cpython_lib, import_source, timeout);
        if import_ok {
            import_pass += 1;
        }
        let smoke_ok = import_ok && run_snippet(&bin, &cpython_lib, smoke_source, timeout);
        if smoke_ok {
            smoke_pass += 1;
        }
        if !import_ok || !smoke_ok {
            regressions.push(format!(
                "{name}: import_ok={import_ok}, smoke_ok={smoke_ok}"
            ));
        }
    }

    assert!(
        import_pass >= MIN_IMPORT_PASS && smoke_pass >= MIN_SMOKE_PASS,
        "top-stdlib baseline regression: import_pass={import_pass}, smoke_pass={smoke_pass}\n{}",
        regressions.join("\n")
    );
}
