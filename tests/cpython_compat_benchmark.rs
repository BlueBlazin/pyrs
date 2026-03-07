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

fn cpython_bin_file_or_skip() -> Option<PathBuf> {
    let Some(bin) = cpython_bin_or_skip() else {
        return None;
    };
    if bin.is_file() {
        return Some(bin);
    }
    let output = Command::new("which")
        .arg(&bin)
        .output()
        .expect("resolve CPython binary path");
    if !output.status.success() {
        eprintln!("skipping orchestrator benchmark test (could not resolve CPython binary path)");
        return None;
    }
    let resolved = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if resolved.is_file() {
        Some(resolved)
    } else {
        eprintln!("skipping orchestrator benchmark test (resolved CPython binary is not a file)");
        None
    }
}

fn detect_cpython_lib() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_CPYTHON_LIB") {
        let path = PathBuf::from(path);
        if path.join("test").is_dir() {
            return Some(path);
        }
    }
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        workspace.join(".local/Python-3.14.3/Lib"),
        PathBuf::from("/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14"),
    ];
    candidates
        .into_iter()
        .find(|candidate| candidate.join("test").is_dir())
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

fn orchestrator_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/run_cpython_compat_benchmark.py")
}

fn summarizer_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/summarize_cpython_compat_benchmark.py")
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

#[test]
fn orchestrator_emits_manifest_summary_and_shards_for_synthetic_suite() {
    let Some(cpython_bin) = cpython_bin_file_or_skip() else {
        return;
    };
    let Some(cpython_lib) = detect_cpython_lib() else {
        eprintln!("skipping orchestrator benchmark test (CPython Lib not found)");
        return;
    };

    let root = temp_root("compat_benchmark_orchestrator");
    let lib_root = root.join("lib");
    let test_dir = lib_root.join("test");
    fs::create_dir_all(&test_dir).expect("create synthetic test package");
    fs::write(test_dir.join("__init__.py"), "").expect("write synthetic test package init");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(cpython_lib.join("test/libregrtest"), test_dir.join("libregrtest"))
            .expect("symlink libregrtest package");
        std::os::unix::fs::symlink(cpython_lib.join("test/support"), test_dir.join("support"))
            .expect("symlink support package");
    }
    fs::write(
        test_dir.join("test_benchmark_orchestrator.py"),
        r#"
import unittest

class OrchestratorSuite(unittest.TestCase):
    def test_pass(self):
        self.assertEqual(4, 2 + 2)

    def test_subtests(self):
        for value in (1, 2):
            with self.subTest(value=value):
                self.assertLess(value, 2)
"#,
    )
    .expect("write synthetic orchestrator module");

    let out_dir = root.join("out");
    let output = Command::new(&cpython_bin)
        .arg(orchestrator_script())
        .arg("--runner-bin")
        .arg(&cpython_bin)
        .arg("--cpython-bin")
        .arg(&cpython_bin)
        .arg("--cpython-lib")
        .arg(&lib_root)
        .arg("--entry")
        .arg("test.test_benchmark_orchestrator")
        .arg("--out-dir")
        .arg(&out_dir)
        .arg("--jobs")
        .arg("1")
        .arg("--run-timeout")
        .arg("60")
        .output()
        .expect("run orchestrator benchmark");
    assert!(
        output.status.success(),
        "orchestrator should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest = fs::read_to_string(out_dir.join("manifest.json")).expect("read manifest");
    assert!(
        manifest.contains(r#""status": "completed""#),
        "manifest should mark the run completed: {manifest}"
    );
    assert!(
        manifest.contains(r#""count": 1"#),
        "manifest should record the single selected entry: {manifest}"
    );

    let progress = fs::read_to_string(out_dir.join("progress.json")).expect("read progress");
    for needle in [
        r#""phase": "completed""#,
        r#""inventory_completed": 1"#,
        r#""run_completed": 1"#,
    ] {
        assert!(
            progress.contains(needle),
            "progress missing expected fragment {needle}: {progress}"
        );
    }

    let summary = fs::read_to_string(out_dir.join("summary.json")).expect("read summary");
    for needle in [
        r#""discoverable_case_count": 2"#,
        r#""executed_entry_count": 1"#,
        r#""executed_case_count": 2"#,
        r#""executed_subtest_count": 2"#,
        r#""run_timeout_secs": 60"#,
        r#""git": {"#,
        r#""host": {"#,
        r#""result_shard": "results/test.test_benchmark_orchestrator.json""#,
    ] {
        assert!(
            summary.contains(needle),
            "summary missing expected fragment {needle}: {summary}"
        );
    }

    let summarize_output = Command::new(&cpython_bin)
        .arg(summarizer_script())
        .arg("--benchmark-dir")
        .arg(&out_dir)
        .output()
        .expect("run derived summary script");
    assert!(
        summarize_output.status.success(),
        "derived summary script should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&summarize_output.stdout),
        String::from_utf8_lossy(&summarize_output.stderr)
    );
    let derived =
        fs::read_to_string(out_dir.join("derived_summary.json")).expect("read derived summary");
    for needle in [
        r#""discoverable_case_count": 2"#,
        r#""executed_subtest_count": 2"#,
        r#""top_modules_by_nonpass": ["#,
        r#""failure_signatures": {"#,
        r#""slowest_cases": ["#,
        r#""slowest_subtests": ["#,
    ] {
        assert!(
            derived.contains(needle),
            "derived summary missing expected fragment {needle}: {derived}"
        );
    }

    let result =
        fs::read_to_string(out_dir.join("results/test.test_benchmark_orchestrator.json"))
            .expect("read result shard");
    for needle in [
        r#""status": "failed""#,
        r#""outcome": "passed""#,
        r#""outcome": "failed""#,
        r#""subtest_outcomes": {"#,
    ] {
        assert!(
            result.contains(needle),
            "result shard missing expected fragment {needle}: {result}"
        );
    }
}
