#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn detect_cpython_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_CPYTHON_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let candidate = PathBuf::from("/Library/Frameworks/Python.framework/Versions/3.14/bin/python3");
    if candidate.is_file() {
        return Some(candidate);
    }
    let probe = Command::new("python3").arg("--version").output().ok()?;
    if probe.status.success() {
        return Some(PathBuf::from("python3"));
    }
    None
}

fn cpython_bin_or_skip() -> Option<PathBuf> {
    let Some(bin) = detect_cpython_bin() else {
        eprintln!("skipping benchmark worker test (CPython 3.14 binary not found)");
        return None;
    };
    Some(bin)
}

fn temp_root(label: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "pyrs_{label}_{}_{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&base).expect("create temp root");
    base
}

fn worker_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/cpython_compat_benchmark_worker.py")
}

fn run_worker(bin: &Path, module_name: &str, search_path: &Path, mode: &str) -> String {
    let output = Command::new(bin)
        .arg("-S")
        .arg(worker_script())
        .arg("--mode")
        .arg(mode)
        .arg("--module")
        .arg(module_name)
        .arg("--sys-path")
        .arg(search_path)
        .output()
        .expect("run benchmark worker");
    assert!(
        output.status.success(),
        "worker should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn benchmark_worker_inventory_and_run_capture_cases_and_subtests() {
    let Some(cpython_bin) = cpython_bin_or_skip() else {
        return;
    };
    let root = temp_root("compat_benchmark_worker");
    let module_path = root.join("sample_benchmark_suite.py");
    fs::write(
        &module_path,
        r#"
import unittest

class SampleBenchmarkSuite(unittest.TestCase):
    def test_pass(self):
        self.assertEqual(2, 1 + 1)

    def test_subtests(self):
        for value in (1, 2):
            with self.subTest(value=value):
                self.assertLess(value, 2)

    @unittest.expectedFailure
    def test_expected_failure(self):
        self.assertEqual(1, 2)

    @unittest.expectedFailure
    def test_unexpected_success(self):
        self.assertEqual(1, 1)

    @unittest.skip("skip me")
    def test_skipped(self):
        pass

    def test_error(self):
        raise RuntimeError("boom")
"#,
    )
    .expect("write synthetic benchmark suite");

    let inventory = run_worker(&cpython_bin, "sample_benchmark_suite", &root, "inventory");
    assert!(
        inventory.contains(r#""status": "ok""#),
        "inventory should load successfully: {inventory}"
    );
    assert!(
        inventory.contains(r#""case_count": 6"#),
        "inventory should report all discoverable test cases: {inventory}"
    );
    assert!(
        inventory.contains("SampleBenchmarkSuite.test_subtests"),
        "inventory should include the subtest parent case: {inventory}"
    );

    let run = run_worker(&cpython_bin, "sample_benchmark_suite", &root, "run");
    assert!(
        run.contains(r#""status": "failed""#),
        "synthetic suite should fail overall: {run}"
    );
    assert!(
        run.matches(r#""kind": "case""#).count() == 6,
        "run output should include one case record per test: {run}"
    );
    assert!(
        run.matches(r#""kind": "subtest""#).count() == 2,
        "run output should include both subtest events: {run}"
    );
    for needle in [
        r#""outcome": "passed""#,
        r#""outcome": "failed""#,
        r#""outcome": "error""#,
        r#""outcome": "skipped""#,
        r#""outcome": "expected_failure""#,
        r#""outcome": "unexpected_success""#,
    ] {
        assert!(run.contains(needle), "missing expected outcome {needle}: {run}");
    }
    assert!(
        run.contains(r#""subtest_outcome_counts": {"#),
        "case records should summarize subtest outcomes: {run}"
    );
    assert!(
        run.contains(r#""duration_secs": "#),
        "run output should record durations: {run}"
    );
}
