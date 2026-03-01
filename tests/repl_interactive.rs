use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "pyrs_{prefix}_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

fn pyrs_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_pyrs") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return path;
        }
    }
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_pyrs") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return path;
        }
    }
    let from_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/pyrs");
    if from_manifest.is_file() {
        return from_manifest;
    }
    panic!("unable to locate pyrs binary for REPL tests");
}

fn run_repl_under_pty(
    input: &[u8],
    history_prefix: &str,
) -> (std::process::Output, String, String) {
    let history_path = temp_path(history_prefix);
    if let Some(parent) = history_path.parent() {
        fs::create_dir_all(parent).expect("create history dir");
    }
    let mut cmd = Command::new("script");
    cmd.arg("-q")
        .arg("/dev/null")
        .arg(pyrs_bin())
        .env("PYRS_REPL_HISTORY", &history_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("spawn script");
    {
        let mut stdin = child.stdin.take().expect("script stdin");
        stdin.write_all(input).expect("write interactive commands");
    }
    let output = child.wait_with_output().expect("wait for script");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (output, stdout, stderr)
}

#[test]
fn repl_banner_and_expression_echo_work_under_pty() {
    if std::env::var("PYRS_RUN_PTY_REPL_TEST").ok().as_deref() != Some("1") {
        eprintln!("skipping PTY repl test (set PYRS_RUN_PTY_REPL_TEST=1 to enable)");
        return;
    }
    if !cfg!(target_os = "macos") {
        eprintln!("skipping PTY repl test on non-macos host");
        return;
    }

    let (output, stdout, stderr) = run_repl_under_pty(b"1+1\n:exit\n", "repl_history");
    if !output.status.success()
        && stdout.contains("cursor position could not be read within a normal duration")
    {
        eprintln!("skipping PTY repl test due unsupported terminal cursor-query behavior");
        return;
    }
    assert!(
        output.status.success(),
        "script status failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("PYRS"),
        "missing REPL banner\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("\n2\r\n") || stdout.contains("\r\n2\r\n") || stdout.contains("\n2\n"),
        "missing echoed expression result\nstdout:\n{stdout}"
    );
}

#[test]
fn repl_allows_multi_line_class_body_without_unexpected_indent() {
    if std::env::var("PYRS_RUN_PTY_REPL_TEST").ok().as_deref() != Some("1") {
        eprintln!("skipping PTY repl test (set PYRS_RUN_PTY_REPL_TEST=1 to enable)");
        return;
    }
    if !cfg!(target_os = "macos") {
        eprintln!("skipping PTY repl test on non-macos host");
        return;
    }

    let (output, stdout, stderr) = run_repl_under_pty(
        b"from dataclasses import dataclass\n@dataclass\nclass User:\n    name: str\n    id: int\n\nu = User('ada', 7)\nu.id\n:exit\n",
        "repl_dataclass_history",
    );
    if !output.status.success()
        && stdout.contains("cursor position could not be read within a normal duration")
    {
        eprintln!("skipping PTY repl test due unsupported terminal cursor-query behavior");
        return;
    }
    assert!(
        output.status.success(),
        "script status failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stdout.contains("IndentationError: unexpected indent"),
        "unexpected-indent regression in REPL class body flow\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("\n7\r\n") || stdout.contains("\r\n7\r\n") || stdout.contains("\n7\n"),
        "missing class attribute lookup result\nstdout:\n{stdout}"
    );
}
