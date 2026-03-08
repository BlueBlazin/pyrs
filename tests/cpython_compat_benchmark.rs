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

fn dispatcher_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/dispatch_cpython_compat_benchmark.py")
}

fn summarizer_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/summarize_cpython_compat_benchmark.py")
}

fn create_synthetic_test_lib(root: &Path, cpython_lib: &Path) -> PathBuf {
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
    lib_root
}

fn create_sleep_runner(path: &Path) {
    fs::write(
        path,
        "#!/bin/sh\nsleep 2\n",
    )
    .expect("write sleep runner");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).expect("stat sleep runner").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod sleep runner");
    }
}

fn extract_json_string_value(body: &str, key: &str) -> String {
    let pattern = format!("\"{key}\": \"");
    let start = body
        .find(&pattern)
        .unwrap_or_else(|| panic!("missing string field {key} in {body}"))
        + pattern.len();
    let end = body[start..]
        .find('"')
        .unwrap_or_else(|| panic!("unterminated string field {key} in {body}"))
        + start;
    body[start..end].to_string()
}

fn extract_json_object_block(body: &str, key: &str) -> String {
    let pattern = format!("\"{key}\": {{");
    let start = body
        .find(&pattern)
        .unwrap_or_else(|| panic!("missing object field {key} in {body}"));
    let brace_start = start + pattern.len() - 1;
    let bytes = body.as_bytes();
    let mut depth = 0_i32;
    let mut end = None;
    for (index, byte) in bytes.iter().enumerate().skip(brace_start) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(index + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end.unwrap_or_else(|| panic!("unterminated object field {key} in {body}"));
    body[start..end].to_string()
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
    let lib_root = create_synthetic_test_lib(&root, &cpython_lib);

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
    for needle in [
        r#""requested_entries": ["#,
        r#""requested_entry_files": []"#,
        r#""unmatched_requested_entries": []"#,
        r#""selected_entry_count": 1"#,
        r#""discovered_entry_count": 1"#,
    ] {
        assert!(
            manifest.contains(needle),
            "manifest missing expected selection metadata {needle}: {manifest}"
        );
    }

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
        r#""selection": {"#,
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

#[test]
fn orchestrator_entry_file_selection_is_strict_by_default_and_records_unmatched_entries() {
    let Some(cpython_bin) = cpython_bin_file_or_skip() else {
        return;
    };
    let Some(cpython_lib) = detect_cpython_lib() else {
        eprintln!("skipping orchestrator selection benchmark test (CPython Lib not found)");
        return;
    };

    let root = temp_root("compat_benchmark_orchestrator_selection");
    let lib_root = create_synthetic_test_lib(&root, &cpython_lib);
    let entry_file = root.join("entries.txt");
    fs::write(
        &entry_file,
        "test.test_benchmark_orchestrator\n# comment\n\ntest.test_missing_entry\n",
    )
    .expect("write entry file");

    let strict_output = Command::new(&cpython_bin)
        .arg(orchestrator_script())
        .arg("--runner-bin")
        .arg(&cpython_bin)
        .arg("--cpython-bin")
        .arg(&cpython_bin)
        .arg("--cpython-lib")
        .arg(&lib_root)
        .arg("--entry-file")
        .arg(&entry_file)
        .arg("--out-dir")
        .arg(root.join("strict_out"))
        .output()
        .expect("run strict orchestrator selection");
    assert!(
        !strict_output.status.success(),
        "strict orchestrator selection should fail\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&strict_output.stdout),
        String::from_utf8_lossy(&strict_output.stderr)
    );
    let strict_stderr = String::from_utf8_lossy(&strict_output.stderr);
    assert!(
        strict_stderr.contains("test.test_missing_entry"),
        "strict orchestrator selection should report the unmatched entry\nstderr:\n{strict_stderr}"
    );

    let out_dir = root.join("allow_missing_out");
    let allow_output = Command::new(&cpython_bin)
        .arg(orchestrator_script())
        .arg("--runner-bin")
        .arg(&cpython_bin)
        .arg("--cpython-bin")
        .arg(&cpython_bin)
        .arg("--cpython-lib")
        .arg(&lib_root)
        .arg("--entry-file")
        .arg(&entry_file)
        .arg("--allow-missing-entries")
        .arg("--out-dir")
        .arg(&out_dir)
        .arg("--jobs")
        .arg("1")
        .arg("--run-timeout")
        .arg("60")
        .output()
        .expect("run allow-missing orchestrator selection");
    assert!(
        allow_output.status.success(),
        "allow-missing orchestrator selection should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&allow_output.stdout),
        String::from_utf8_lossy(&allow_output.stderr)
    );
    let allow_stderr = String::from_utf8_lossy(&allow_output.stderr);
    assert!(
        allow_stderr.contains("warning: requested benchmark entries are not discoverable on this host"),
        "allow-missing orchestrator selection should warn about the unmatched entry\nstderr:\n{allow_stderr}"
    );

    let manifest =
        fs::read_to_string(out_dir.join("manifest.json")).expect("read allow-missing manifest");
    for needle in [
        r#""requested_entry_files": ["#,
        "entries.txt",
        r#""requested_entries": ["#,
        r#""test.test_missing_entry""#,
        r#""unmatched_requested_entries": ["#,
        r#""selected_entry_count": 1"#,
        r#""allow_missing_entries": true"#,
    ] {
        assert!(
            manifest.contains(needle),
            "allow-missing manifest missing expected fragment {needle}: {manifest}"
        );
    }

    let summary =
        fs::read_to_string(out_dir.join("summary.json")).expect("read allow-missing summary");
    for needle in [
        r#""requested_entry_files": ["#,
        "entries.txt",
        r#""unmatched_requested_entries": ["#,
        r#""test.test_missing_entry""#,
        r#""discoverable_case_count": 2"#,
        r#""executed_entry_count": 1"#,
    ] {
        assert!(
            summary.contains(needle),
            "allow-missing summary missing expected fragment {needle}: {summary}"
        );
    }
}

#[test]
fn dispatcher_emits_combined_summary_for_multiple_batches() {
    let Some(cpython_bin) = cpython_bin_file_or_skip() else {
        return;
    };
    let Some(cpython_lib) = detect_cpython_lib() else {
        eprintln!("skipping dispatcher benchmark test (CPython Lib not found)");
        return;
    };

    let root = temp_root("compat_benchmark_dispatcher");
    let lib_root = create_synthetic_test_lib(&root, &cpython_lib);
    let test_dir = lib_root.join("test");
    fs::write(
        test_dir.join("test_benchmark_dispatch_extra.py"),
        r#"
import unittest

class DispatchExtraSuite(unittest.TestCase):
    def test_more_pass(self):
        self.assertTrue(True)
"#,
    )
    .expect("write synthetic dispatch module");

    let out_dir = root.join("dispatch_out");
    let output = Command::new(&cpython_bin)
        .arg(dispatcher_script())
        .arg("--runner-bin")
        .arg(&cpython_bin)
        .arg("--cpython-bin")
        .arg(&cpython_bin)
        .arg("--cpython-lib")
        .arg(&lib_root)
        .arg("--entries-per-batch")
        .arg("1")
        .arg("--out-dir")
        .arg(&out_dir)
        .arg("--jobs")
        .arg("1")
        .arg("--run-timeout")
        .arg("60")
        .output()
        .expect("run benchmark dispatcher");
    assert!(
        output.status.success(),
        "dispatcher should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let plan = fs::read_to_string(out_dir.join("plan.json")).expect("read dispatcher plan");
    for needle in [
        r#""planned_batch_count": 2"#,
        r#""batch-000"#,
        r#""batch-001"#,
    ] {
        assert!(
            plan.contains(needle),
            "dispatcher plan missing expected fragment {needle}: {plan}"
        );
    }

    let progress =
        fs::read_to_string(out_dir.join("progress.json")).expect("read dispatcher progress");
    for needle in [
        r#""phase": "completed""#,
        r#""batches_total": 2"#,
        r#""batches_completed": 2"#,
    ] {
        assert!(
            progress.contains(needle),
            "dispatcher progress missing expected fragment {needle}: {progress}"
        );
    }

    let summary = fs::read_to_string(out_dir.join("summary.json")).expect("read dispatcher summary");
    for needle in [
        r#""planned_batch_count": 2"#,
        r#""completed_batch_count": 2"#,
        r#""entry_count": 2"#,
        r#""executed_entry_count": 2"#,
        r#""executed_subtest_count": 2"#,
        r#""batch_id": "batch-000""#,
        r#""batch_id": "batch-001""#,
        r#""result_shard": "batches/batch-000/results/test.test_benchmark_dispatch_extra.json""#,
        r#""result_shard": "batches/batch-001/results/test.test_benchmark_orchestrator.json""#,
    ] {
        assert!(
            summary.contains(needle),
            "dispatcher summary missing expected fragment {needle}: {summary}"
        );
    }

    let derived =
        fs::read_to_string(out_dir.join("derived_summary.json")).expect("read dispatcher derived summary");
    for needle in [
        r#""discoverable_case_count": 3"#,
        r#""executed_case_count": 3"#,
        r#""slowest_cases": ["#,
    ] {
        assert!(
            derived.contains(needle),
            "dispatcher derived summary missing expected fragment {needle}: {derived}"
        );
    }

    assert!(
        out_dir.join("batches/batch-000/summary.json").is_file(),
        "dispatcher should emit nested batch summary for batch-000"
    );
    assert!(
        out_dir.join("batches/batch-001/summary.json").is_file(),
        "dispatcher should emit nested batch summary for batch-001"
    );
}

#[test]
fn orchestrator_timeout_payloads_remain_json_serializable() {
    let Some(cpython_bin) = cpython_bin_file_or_skip() else {
        return;
    };
    let Some(cpython_lib) = detect_cpython_lib() else {
        eprintln!("skipping orchestrator timeout serialization test (CPython Lib not found)");
        return;
    };

    let root = temp_root("compat_benchmark_timeout_serialization");
    let lib_root = create_synthetic_test_lib(&root, &cpython_lib);
    let sleep_runner = root.join("sleep_runner.sh");
    create_sleep_runner(&sleep_runner);

    let out_dir = root.join("timeout_out");
    let output = Command::new(&cpython_bin)
        .arg(orchestrator_script())
        .arg("--runner-bin")
        .arg(&sleep_runner)
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
        .arg("1")
        .output()
        .expect("run orchestrator timeout serialization benchmark");
    assert!(
        output.status.success(),
        "orchestrator should succeed even when the runner times out\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let result =
        fs::read_to_string(out_dir.join("results/test.test_benchmark_orchestrator.json"))
            .expect("read timeout result shard");
    for needle in [
        r#""status": "process_timeout""#,
        r#""timeout": true"#,
        r#""stderr": """#,
    ] {
        assert!(
            result.contains(needle),
            "timeout result shard missing expected fragment {needle}: {result}"
        );
    }

    let summary = fs::read_to_string(out_dir.join("summary.json")).expect("read timeout summary");
    for needle in [
        r#""module_statuses": {"#,
        r#""process_timeout": 1"#,
        r#""executed_case_count": 0"#,
    ] {
        assert!(
            summary.contains(needle),
            "timeout summary missing expected fragment {needle}: {summary}"
        );
    }
}

#[test]
fn orchestrator_invalidates_cached_results_when_runner_binary_changes() {
    let Some(cpython_bin) = cpython_bin_file_or_skip() else {
        return;
    };
    let Some(cpython_lib) = detect_cpython_lib() else {
        eprintln!("skipping orchestrator cache invalidation test (CPython Lib not found)");
        return;
    };

    let root = temp_root("compat_benchmark_cache_invalidation");
    let lib_root = create_synthetic_test_lib(&root, &cpython_lib);
    let out_dir = root.join("cache_out");

    let first = Command::new(&cpython_bin)
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
        .expect("run orchestrator cache baseline");
    assert!(
        first.status.success(),
        "baseline orchestrator run should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );

    let sleep_runner = root.join("sleep_runner.sh");
    create_sleep_runner(&sleep_runner);
    let second = Command::new(&cpython_bin)
        .arg(orchestrator_script())
        .arg("--runner-bin")
        .arg(&sleep_runner)
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
        .arg("1")
        .output()
        .expect("run orchestrator cache invalidation");
    assert!(
        second.status.success(),
        "orchestrator should rerun when the runner binary changes\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );

    let result =
        fs::read_to_string(out_dir.join("results/test.test_benchmark_orchestrator.json"))
            .expect("read invalidated result shard");
    assert!(
        result.contains(r#""status": "process_timeout""#),
        "changed runner binary should invalidate the old cache entry: {result}"
    );
}

#[test]
fn dispatcher_reuses_completed_batches_for_same_runner_binary() {
    let Some(cpython_bin) = cpython_bin_file_or_skip() else {
        return;
    };
    let Some(cpython_lib) = detect_cpython_lib() else {
        eprintln!("skipping dispatcher cache reuse test (CPython Lib not found)");
        return;
    };

    let root = temp_root("compat_benchmark_dispatcher_cache");
    let lib_root = create_synthetic_test_lib(&root, &cpython_lib);
    let test_dir = lib_root.join("test");
    fs::write(
        test_dir.join("test_benchmark_dispatch_cache_extra.py"),
        r#"
import unittest

class DispatchCacheSuite(unittest.TestCase):
    def test_cache_pass(self):
        self.assertTrue(True)
"#,
    )
    .expect("write synthetic dispatcher cache module");

    let out_dir = root.join("dispatch_cache_out");
    let first = Command::new(&cpython_bin)
        .arg(dispatcher_script())
        .arg("--runner-bin")
        .arg(&cpython_bin)
        .arg("--cpython-bin")
        .arg(&cpython_bin)
        .arg("--cpython-lib")
        .arg(&lib_root)
        .arg("--entries-per-batch")
        .arg("1")
        .arg("--out-dir")
        .arg(&out_dir)
        .arg("--jobs")
        .arg("1")
        .arg("--run-timeout")
        .arg("60")
        .output()
        .expect("run dispatcher cache baseline");
    assert!(
        first.status.success(),
        "dispatcher baseline should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let first_summary =
        fs::read_to_string(out_dir.join("summary.json")).expect("read baseline dispatcher summary");
    let first_manifest =
        fs::read_to_string(out_dir.join("manifest.json")).expect("read baseline dispatcher manifest");
    let first_generated_at = extract_json_string_value(&first_summary, "generated_at_utc");
    let first_run_state = extract_json_object_block(&first_summary, "run_state");
    let first_results = extract_json_object_block(&first_summary, "results");
    let first_manifest_completed_at = extract_json_string_value(&first_manifest, "completed_at_utc");

    std::thread::sleep(std::time::Duration::from_secs(1));

    let second = Command::new(&cpython_bin)
        .arg(dispatcher_script())
        .arg("--runner-bin")
        .arg(&cpython_bin)
        .arg("--cpython-bin")
        .arg(&cpython_bin)
        .arg("--cpython-lib")
        .arg(&lib_root)
        .arg("--entries-per-batch")
        .arg("1")
        .arg("--out-dir")
        .arg(&out_dir)
        .arg("--jobs")
        .arg("1")
        .arg("--run-timeout")
        .arg("60")
        .output()
        .expect("run dispatcher cache reuse");
    assert!(
        second.status.success(),
        "dispatcher cache reuse should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let second_stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        second_stdout.contains("[dispatch] batch-000 -> cached"),
        "dispatcher should reuse completed batch-000\nstdout:\n{second_stdout}"
    );
    assert!(
        second_stdout.contains("[dispatch] batch-001 -> cached"),
        "dispatcher should reuse completed batch-001\nstdout:\n{second_stdout}"
    );

    let summary =
        fs::read_to_string(out_dir.join("summary.json")).expect("read dispatcher cache summary");
    for needle in [
        r#""cached_batch_count": 2"#,
        r#""batch_status_counts": {"#,
        r#""cached": 2"#,
    ] {
        assert!(
            summary.contains(needle),
            "dispatcher cache summary missing expected fragment {needle}: {summary}"
        );
    }
    assert!(
        summary.contains(&format!(r#""generated_at_utc": "{first_generated_at}""#)),
        "cached rerun should preserve original summary generation time\nfirst:\n{first_summary}\nsecond:\n{summary}"
    );
    assert!(
        summary.contains(&first_run_state),
        "cached rerun should preserve original run_state block\nfirst:\n{first_summary}\nsecond:\n{summary}"
    );
    assert!(
        summary.contains(&first_results),
        "cached rerun should preserve original results block\nfirst:\n{first_summary}\nsecond:\n{summary}"
    );

    let second_manifest =
        fs::read_to_string(out_dir.join("manifest.json")).expect("read dispatcher cache manifest");
    assert!(
        second_manifest.contains(&format!(r#""generated_at_utc": "{first_generated_at}""#)),
        "cached rerun should preserve original manifest generation time\nfirst:\n{first_manifest}\nsecond:\n{second_manifest}"
    );
    assert!(
        second_manifest.contains(&format!(r#""completed_at_utc": "{first_manifest_completed_at}""#)),
        "cached rerun should preserve original manifest completion time\nfirst:\n{first_manifest}\nsecond:\n{second_manifest}"
    );
}
