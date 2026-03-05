#![cfg(not(target_arch = "wasm32"))]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use pyrs::{compiler, parser, vm::Vm};

const ALLOWLIST_FILE: &str = "tests/cpython_allowlist.txt";
const STRICT_ALLOWLIST_FILE: &str = "tests/cpython_allowlist_strict.txt";
const DEFERRED_PICKLE_ALLOWLIST_FILE: &str = "tests/cpython_allowlist_deferred_pickle.txt";
const LANGUAGE_SUITE: &str = "tests/cpython_suite_language.txt";
const IMPORT_SUITE: &str = "tests/cpython_suite_imports.txt";
const STRICT_STDLIB_SUITE: &str = "tests/cpython_suite_strict_stdlib.txt";
const DEFERRED_PICKLE_SUITE: &str = "tests/cpython_suite_deferred_pickle.txt";

fn enabled(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
struct AllowEntry {
    category: String,
    owner: String,
}

fn read_list(path: &str) -> Vec<String> {
    let data = fs::read_to_string(path).unwrap_or_else(|err| {
        panic!("failed to read {path}: {err}");
    });
    data.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect()
}

fn read_allowlist(path: &str) -> HashMap<String, AllowEntry> {
    let mut allow = HashMap::new();
    for line in read_list(path) {
        let mut parts = line.split('|');
        let test = parts.next().unwrap_or_default().trim().to_string();
        let category = parts.next().unwrap_or_default().trim().to_string();
        let owner = parts.next().unwrap_or_default().trim().to_string();
        assert!(
            !test.is_empty() && !category.is_empty() && !owner.is_empty(),
            "invalid allowlist row (need 3 pipe-delimited fields): {line}"
        );
        let replaced = allow.insert(test.clone(), AllowEntry { category, owner });
        assert!(
            replaced.is_none(),
            "duplicate allowlist entry for {test} in {path}"
        );
    }
    allow
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
    for candidate in candidates {
        if candidate.join("test").is_dir() {
            return Some(candidate);
        }
    }
    None
}

fn cpython_lib_or_panic() -> PathBuf {
    if let Some(path) = detect_cpython_lib() {
        return path;
    }
    if std::env::var("PYRS_CPYTHON_OPTIONAL").as_deref() == Ok("1") {
        eprintln!("CPython Lib path not found; skipping harness due to PYRS_CPYTHON_OPTIONAL=1");
        return PathBuf::new();
    }
    panic!(
        "CPython Lib path not found. Set PYRS_CPYTHON_LIB (expected <...>/Lib with test/ directory)."
    );
}

fn module_path(lib: &Path, entry: &str) -> Option<PathBuf> {
    let candidate = lib.join(entry);
    if candidate.is_file() {
        return Some(candidate);
    }
    if !entry.ends_with(".py") {
        let test_module = lib.join("test").join(format!("{entry}.py"));
        if test_module.is_file() {
            return Some(test_module);
        }
        let package = lib
            .join("test")
            .join(entry.replace('.', "/"))
            .join("__init__.py");
        if package.is_file() {
            return Some(package);
        }
    }
    None
}

fn module_name(entry: &str) -> Option<String> {
    let without_suffix = entry.strip_suffix(".py")?;
    let without_init = without_suffix
        .strip_suffix("/__init__")
        .unwrap_or(without_suffix);
    let normalized = without_init.replace('/', ".");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn pyrs_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_SUBPROCESS_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(mode) = std::env::var("PYRS_SUBPROCESS_BIN_MODE") {
        let mode = mode.trim().to_ascii_lowercase();
        if matches!(mode.as_str(), "debug" | "release") {
            let candidate =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/{mode}/pyrs"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    if let Some(path) = option_env!("CARGO_BIN_EXE_pyrs") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_pyrs") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let from_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/pyrs");
    if from_manifest.is_file() {
        return Some(from_manifest);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(debug_dir) = exe.parent().and_then(|deps| deps.parent())
    {
        let sibling = debug_dir.join("pyrs");
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    None
}

fn strict_subprocess_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_SUBPROCESS_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    if let Ok(mode) = std::env::var("PYRS_SUBPROCESS_BIN_MODE") {
        let mode = mode.trim().to_ascii_lowercase();
        if matches!(mode.as_str(), "debug" | "release") {
            let candidate =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/{mode}/pyrs"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // Default strict behavior: prefer release subprocesses for long-running stdlib suites.
    // If debug is newer than release, prefer debug to avoid stale binary mismatches.
    let prefer_release = std::env::var("PYRS_STRICT_PREFER_RELEASE")
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "no" | "off")
        })
        .unwrap_or(true);

    let debug = pyrs_bin();
    if prefer_release {
        let release = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/pyrs");
        if release.is_file() {
            match debug.as_ref() {
                Some(debug_bin) if is_older_file(&release, debug_bin) => {}
                _ => return Some(release),
            }
        }
    }

    debug
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

fn is_older_file(candidate: &Path, reference: &Path) -> bool {
    let Some(candidate_time) = file_mtime(candidate) else {
        return false;
    };
    let Some(reference_time) = file_mtime(reference) else {
        return false;
    };
    candidate_time < reference_time
}

fn strict_unittest_timeout() -> Duration {
    let secs = std::env::var("PYRS_STRICT_HARNESS_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(600);
    Duration::from_secs(secs.max(1))
}

fn strict_allowlisted_timeout() -> Duration {
    let fallback = strict_unittest_timeout();
    let secs = std::env::var("PYRS_STRICT_ALLOWLIST_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(fallback.as_secs().min(30));
    Duration::from_secs(secs.max(1))
}

fn deferred_pickle_timeout() -> Duration {
    let fallback = strict_unittest_timeout().as_secs().max(600);
    let secs = std::env::var("PYRS_DEFERRED_PICKLE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(fallback);
    Duration::from_secs(secs.max(1))
}

fn strict_timeout_for_entry(suite_file: &str, is_allowlisted: bool) -> Duration {
    if suite_file == DEFERRED_PICKLE_SUITE {
        return deferred_pickle_timeout();
    }
    if is_allowlisted {
        strict_allowlisted_timeout()
    } else {
        strict_unittest_timeout()
    }
}

fn strict_stdlib_enabled() -> bool {
    enabled("PYRS_RUN_STRICT_STDLIB") || enabled("PYRS_PARITY_STRICT")
}

fn deferred_pickle_enabled() -> bool {
    enabled("PYRS_RUN_DEFERRED_PICKLE")
}

fn strict_run_allowlisted_entries() -> bool {
    enabled("PYRS_RUN_ALLOWLISTED_STRICT")
}

fn strict_timing_trace_enabled() -> bool {
    enabled("PYRS_STRICT_TIMING")
}

fn run_source_in_subprocess(bin: &Path, source: &str, timeout: Duration) -> Result<(), String> {
    let mut child = Command::new(bin)
        .arg("-c")
        .arg(source)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to spawn subprocess harness: {err}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let (stdout, stderr) = capture_child_output(&mut child);
                if status.success() {
                    return Ok(());
                }
                return Err(format!(
                    "subprocess harness failed with status {status}\nstdout:\n{stdout}\nstderr:\n{stderr}"
                ));
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let (stdout, stderr) = capture_child_output(&mut child);
                    return Err(format!(
                        "subprocess harness timed out after {}s\nstdout:\n{stdout}\nstderr:\n{stderr}",
                        timeout.as_secs(),
                    ));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(format!("failed to poll subprocess harness: {err}")),
        }
    }
}

fn capture_child_output(child: &mut Child) -> (String, String) {
    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut stdout_buf);
    }
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_end(&mut stderr_buf);
    }
    (
        String::from_utf8_lossy(&stdout_buf).to_string(),
        String::from_utf8_lossy(&stderr_buf).to_string(),
    )
}

#[derive(Clone, Copy)]
enum SuiteMode {
    ImportOnly,
    StrictUnittest,
}

fn run_entry(
    lib: &Path,
    entry: &str,
    mode: SuiteMode,
    strict_subprocess_bin: Option<&Path>,
    strict_timeout: Option<Duration>,
) -> Result<(), String> {
    let _path = module_path(lib, entry).ok_or_else(|| "missing module".to_string())?;
    let import_name = module_name(entry).ok_or_else(|| "invalid module entry".to_string())?;
    let lib_path = lib.to_string_lossy();
    let executable_patch = strict_subprocess_bin
        .map(Path::to_path_buf)
        .or_else(pyrs_bin)
        .map(|path| format!("sys.executable = {:?}\n", path.to_string_lossy()))
        .unwrap_or_default();
    let source = match mode {
        SuiteMode::ImportOnly => format!(
            "import sys\nimport importlib\n{executable_patch}sys.path = [{lib_path:?}]\nimportlib.import_module({import_name:?})\n"
        ),
        SuiteMode::StrictUnittest => format!(
            "import os\nimport sys\nimport importlib\nimport unittest\nimport test.support\n{executable_patch}sys.path = [{lib_path:?}]\ntest.support.use_resources = {{}}\nmodule = importlib.import_module({import_name:?})\nloader = unittest.defaultTestLoader\nbefore_errors = len(getattr(loader, 'errors', []))\nsuite = loader.loadTestsFromModule(module)\nafter_errors = len(getattr(loader, 'errors', []))\nif after_errors > before_errors:\n    raise RuntimeError('strict unittest loader failed')\nverbosity = int(os.environ.get('PYRS_STRICT_UNITTEST_VERBOSITY', '0'))\nresult = unittest.TextTestRunner(verbosity=verbosity, failfast=True).run(suite)\nif not result.wasSuccessful():\n    raise RuntimeError('strict unittest suite failed')\n"
        ),
    };

    if matches!(mode, SuiteMode::StrictUnittest)
        && let Some(bin) = strict_subprocess_bin
    {
        let timeout = strict_timeout.unwrap_or_else(strict_unittest_timeout);
        return run_source_in_subprocess(bin, &source, timeout);
    }

    let module =
        parser::parse_module(&source).map_err(|err| format!("parse error {}", err.message))?;
    let code = compiler::compile_module_with_filename(&module, "<cpython_harness>")
        .map_err(|err| format!("compile error {}", err.message))?;
    let mut vm = Vm::new();
    vm.add_module_path(lib);
    vm.execute(&code)
        .map(|_| ())
        .map_err(|err| format!("runtime error {}", err.message))
}

fn run_suite_file(suite_file: &str, allowlist_file: &str, mode: SuiteMode) {
    let lib = cpython_lib_or_panic();
    if lib.as_os_str().is_empty() {
        return;
    }
    let suite = read_list(suite_file);
    let allow = read_allowlist(allowlist_file);

    let strict_mode = matches!(mode, SuiteMode::StrictUnittest);
    let timing_trace = strict_mode && strict_timing_trace_enabled();
    let strict_bin = if strict_mode {
        strict_subprocess_bin()
    } else {
        None
    };

    let mut unexpected_failures = Vec::new();
    let mut stale_allowlist = Vec::new();
    let mut passed = 0usize;
    let mut allowed = 0usize;
    let mut skipped_allowlisted = 0usize;
    let run_allowlisted = !strict_mode || strict_run_allowlisted_entries();

    if strict_mode {
        if let Some(bin) = &strict_bin {
            eprintln!("strict harness subprocess bin: {}", bin.to_string_lossy());
        } else {
            eprintln!("strict harness subprocess bin unavailable; using in-process VM fallback");
        }
    }

    for entry in &suite {
        let is_allowlisted = allow.contains_key(entry);
        if !run_allowlisted && is_allowlisted {
            allowed += 1;
            skipped_allowlisted += 1;
            continue;
        }

        let entry_timeout =
            strict_mode.then(|| strict_timeout_for_entry(suite_file, is_allowlisted));

        let start = Instant::now();
        let result = run_entry(&lib, entry, mode, strict_bin.as_deref(), entry_timeout);
        let elapsed = start.elapsed();
        if timing_trace {
            let tag = if is_allowlisted {
                "allowlisted"
            } else {
                "owned"
            };
            if let Some(timeout) = entry_timeout {
                eprintln!(
                    "strict timing: entry={entry} tag={tag} elapsed={:.3}s timeout={}s result={}",
                    elapsed.as_secs_f64(),
                    timeout.as_secs(),
                    if result.is_ok() { "ok" } else { "err" }
                );
            }
        }

        match result {
            Ok(()) => {
                if let Some(allow_entry) = allow.get(entry) {
                    stale_allowlist.push(format!(
                        "{entry}: now passing; remove allowlist ({}/{})",
                        allow_entry.category, allow_entry.owner
                    ));
                } else {
                    passed += 1;
                }
            }
            Err(reason) => {
                if allow.contains_key(entry) {
                    allowed += 1;
                } else {
                    unexpected_failures.push(format!("{entry}: {reason}"));
                }
            }
        }
    }

    if skipped_allowlisted > 0 {
        eprintln!(
            "skipped {skipped_allowlisted} allowlisted strict entries (set PYRS_RUN_ALLOWLISTED_STRICT=1 to run them)"
        );
    }

    if !stale_allowlist.is_empty() || !unexpected_failures.is_empty() {
        let mut lines = Vec::new();
        lines.push(format!(
            "CPython harness parity mismatch for {suite_file} (pass={passed}, allowlisted_fail={allowed}, total={})",
            suite.len()
        ));
        if !unexpected_failures.is_empty() {
            lines.push("Unexpected failures:".to_string());
            lines.extend(unexpected_failures);
        }
        if !stale_allowlist.is_empty() {
            lines.push("Stale allowlist entries:".to_string());
            lines.extend(stale_allowlist);
        }
        panic!("{}", lines.join("\n"));
    }
}

#[test]
fn allowlist_entries_are_referenced_by_suites() {
    let suite_entries: HashSet<String> = read_list(LANGUAGE_SUITE)
        .into_iter()
        .chain(read_list(IMPORT_SUITE))
        .chain(read_list(STRICT_STDLIB_SUITE))
        .chain(read_list(DEFERRED_PICKLE_SUITE))
        .collect();
    let mut unused = Vec::new();
    for allowlist_path in [
        ALLOWLIST_FILE,
        STRICT_ALLOWLIST_FILE,
        DEFERRED_PICKLE_ALLOWLIST_FILE,
    ] {
        let allow = read_allowlist(allowlist_path);
        for key in allow.keys() {
            if !suite_entries.contains(key) {
                unused.push(format!("{allowlist_path}: {key}"));
            }
        }
    }
    if !unused.is_empty() {
        panic!(
            "allowlist entries not present in any suite:\n{}",
            unused.join("\n")
        );
    }
}

#[test]
fn subprocess_harness_helper_succeeds_for_short_program() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping subprocess helper test (pyrs binary not found)");
        return;
    };
    run_source_in_subprocess(&bin, "value = 1 + 2\n", Duration::from_secs(5))
        .expect("short subprocess program should succeed");
}

#[test]
fn subprocess_harness_helper_times_out_hanging_program() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping subprocess helper timeout test (pyrs binary not found)");
        return;
    };
    let err = run_source_in_subprocess(&bin, "while True:\n    pass\n", Duration::from_secs(1))
        .expect_err("hanging subprocess should timeout");
    assert!(
        err.contains("timed out"),
        "expected timeout error, got: {err}"
    );
}

#[test]
fn runs_cpython_language_suite() {
    let handle = std::thread::Builder::new()
        .name("cpython-language-suite".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            run_suite_file(LANGUAGE_SUITE, ALLOWLIST_FILE, SuiteMode::ImportOnly);
        })
        .expect("spawn language harness thread");
    handle
        .join()
        .expect("language harness thread should complete");
}

#[test]
fn runs_cpython_import_suite() {
    let handle = std::thread::Builder::new()
        .name("cpython-import-suite".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            run_suite_file(IMPORT_SUITE, ALLOWLIST_FILE, SuiteMode::ImportOnly);
        })
        .expect("spawn import harness thread");
    handle
        .join()
        .expect("import harness thread should complete");
}

#[test]
fn runs_cpython_strict_stdlib_suite() {
    if !strict_stdlib_enabled() {
        eprintln!(
            "skipping strict stdlib suite (set PYRS_RUN_STRICT_STDLIB=1 or PYRS_PARITY_STRICT=1 to enable)"
        );
        return;
    }
    let handle = std::thread::Builder::new()
        .name("cpython-strict-stdlib".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            run_suite_file(
                STRICT_STDLIB_SUITE,
                STRICT_ALLOWLIST_FILE,
                SuiteMode::StrictUnittest,
            );
        })
        .expect("spawn strict stdlib harness thread");
    handle
        .join()
        .expect("strict stdlib harness thread should complete");
}

#[test]
fn runs_cpython_deferred_pickle_suite() {
    if !deferred_pickle_enabled() {
        eprintln!(
            "skipping deferred pickle strict suite (set PYRS_RUN_DEFERRED_PICKLE=1 to enable)"
        );
        return;
    }
    let handle = std::thread::Builder::new()
        .name("cpython-deferred-pickle".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            run_suite_file(
                DEFERRED_PICKLE_SUITE,
                DEFERRED_PICKLE_ALLOWLIST_FILE,
                SuiteMode::StrictUnittest,
            );
        })
        .expect("spawn deferred pickle harness thread");
    handle
        .join()
        .expect("deferred pickle harness thread should complete");
}
