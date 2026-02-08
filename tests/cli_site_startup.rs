use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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
