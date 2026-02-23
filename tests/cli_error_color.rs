use std::path::PathBuf;
use std::process::Command;

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
    panic!("unable to locate pyrs binary for cli error color tests");
}

fn run_pyrs(args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut cmd = Command::new(pyrs_bin());
    for key in [
        "PYTHON_COLORS",
        "NO_COLOR",
        "FORCE_COLOR",
        "TERM",
        "COLORFGBG",
    ] {
        cmd.env_remove(key);
    }
    for arg in args {
        cmd.arg(arg);
    }
    for (name, value) in env {
        cmd.env(name, value);
    }
    let output = cmd.output().expect("run pyrs");
    (
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn cli_traceback_uses_ansi_colors_when_forced() {
    let (code, _stdout, stderr) = run_pyrs(
        &["-S", "-c", "raise ValueError('boom')"],
        &[("FORCE_COLOR", "1")],
    );
    assert_eq!(code, 2, "stderr:\n{stderr}");
    assert!(
        stderr.contains("\x1b["),
        "forced color traceback should include ANSI escapes, got:\n{stderr}"
    );
    assert!(
        stderr.contains("ValueError"),
        "expected traceback exception type, got:\n{stderr}"
    );
}

#[test]
fn cli_traceback_respects_no_color_even_when_force_color_is_set() {
    let (code, _stdout, stderr) = run_pyrs(
        &["-S", "-c", "raise RuntimeError('boom')"],
        &[("FORCE_COLOR", "1"), ("NO_COLOR", "1")],
    );
    assert_eq!(code, 2, "stderr:\n{stderr}");
    assert!(
        !stderr.contains("\x1b["),
        "NO_COLOR should suppress ANSI escapes, got:\n{stderr}"
    );
    assert!(
        stderr.contains("RuntimeError"),
        "expected traceback exception type, got:\n{stderr}"
    );
}
