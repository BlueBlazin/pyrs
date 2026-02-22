use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(prefix: &str) -> PathBuf {
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
    panic!("unable to locate pyrs binary for CLI startup tests");
}

fn run_pyrs(root: &Path, args: &[&str], extra_env: &[(&str, &Path)]) -> (i32, String, String) {
    let mut cmd = Command::new(pyrs_bin());
    for arg in args {
        cmd.arg(arg);
    }
    cmd.current_dir(root);
    for (name, value) in extra_env {
        cmd.env(name, value);
    }
    let output = cmd.output().expect("run pyrs");
    (
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn run_pyrs_with_stdin(
    root: &Path,
    args: &[&str],
    stdin_source: &str,
    extra_env: &[(&str, &Path)],
) -> (i32, String, String) {
    let mut cmd = Command::new(pyrs_bin());
    for arg in args {
        cmd.arg(arg);
    }
    cmd.current_dir(root);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    for (name, value) in extra_env {
        cmd.env(name, value);
    }
    let mut child = cmd.spawn().expect("spawn pyrs");
    {
        let mut stdin = child.stdin.take().expect("child stdin");
        stdin
            .write_all(stdin_source.as_bytes())
            .expect("write stdin source");
    }
    let output = child.wait_with_output().expect("wait pyrs");
    (
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn cli_imports_site_by_default_when_stdlib_is_available() {
    let root = temp_root("cli_site_default");
    let stdlib = root.join("Lib");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    let script = root.join("main.py");
    fs::write(&script, "import sys\nassert 'site' in sys.modules\n").expect("write script");

    let script_arg = script.to_string_lossy();
    let (code, _stdout, stderr) = run_pyrs(
        &root,
        &[script_arg.as_ref()],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
}

#[test]
fn cli_no_site_flag_skips_startup_site_import() {
    let root = temp_root("cli_site_no_site");
    let stdlib = root.join("Lib");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    let script = root.join("main.py");
    fs::write(&script, "import sys\nassert 'site' not in sys.modules\n").expect("write script");

    let script_arg = script.to_string_lossy();
    let (code, _stdout, stderr) = run_pyrs(
        &root,
        &["-S", script_arg.as_ref()],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
}

#[test]
fn cli_no_args_executes_stdin_when_not_interactive() {
    let root = temp_root("cli_stdin_exec");
    let stdlib = root.join("Lib");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    let (code, stdout, stderr) = run_pyrs_with_stdin(
        &root,
        &[],
        "print(21 + 21)\n",
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert_eq!(stdout.trim(), "42");
}

#[test]
fn cli_no_args_honors_site_import_flag_for_stdin_execution() {
    let root = temp_root("cli_stdin_site");
    let stdlib = root.join("Lib");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    let (code, _stdout, stderr) = run_pyrs_with_stdin(
        &root,
        &[],
        "import sys\nassert 'site' in sys.modules\n",
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
}

#[test]
fn cli_preserves_pythonpath_entries_that_are_not_stdlib_roots() {
    let root = temp_root("cli_pythonpath_entries");
    let stdlib = root.join("Lib");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    let extra = root.join("extra_path");
    fs::create_dir_all(&extra).expect("create extra module root");
    fs::write(extra.join("hello_from_path.py"), "VALUE = 123\n").expect("write helper module");

    let script = root.join("main.py");
    fs::write(
        &script,
        "import hello_from_path\nassert hello_from_path.VALUE == 123\n",
    )
    .expect("write script");

    let mut cmd = Command::new(pyrs_bin());
    cmd.current_dir(&root);
    cmd.arg(script.to_string_lossy().to_string());
    cmd.env("PYRS_CPYTHON_LIB", stdlib.as_os_str());
    let pythonpath = std::env::join_paths([extra.as_os_str()]).expect("join PYTHONPATH for test");
    cmd.env("PYTHONPATH", pythonpath);
    let output = cmd.output().expect("run pyrs");
    let code = output.status.code().unwrap_or(1);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(code, 0, "stderr:\n{stderr}");
}

#[test]
fn cli_site_startup_ignores_missing_sitecustomize_and_usercustomize() {
    let root = temp_root("cli_site_customize_missing");
    let stdlib = root.join("Lib");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::write(
        stdlib.join("site.py"),
        r#"import sys
for _name in ("sitecustomize", "usercustomize"):
    try:
        __import__(_name)
    except ImportError as exc:
        if getattr(exc, "name", None) != _name:
            print(f"Error in {_name}", file=sys.stderr)
            raise
"#,
    )
    .expect("write site.py");

    let script = root.join("main.py");
    fs::write(&script, "print('ok')\n").expect("write script");

    let script_arg = script.to_string_lossy();
    let (code, stdout, stderr) = run_pyrs(
        &root,
        &[script_arg.as_ref()],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert_eq!(stdout.trim(), "ok");
    assert!(
        stderr.trim().is_empty(),
        "site startup should be silent when custom modules are absent, got: {stderr}"
    );
}
