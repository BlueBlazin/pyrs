#![cfg(not(target_arch = "wasm32"))]

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

fn cpython_lib_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_CPYTHON_LIB") {
        let path = PathBuf::from(path);
        if path.join("test/test_argparse.py").is_file() {
            return Some(path);
        }
    }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".local/Python-3.14.3/Lib");
    if candidate.join("test/test_argparse.py").is_file() {
        return Some(candidate);
    }
    None
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

fn assert_argparse_progname_case(case_name: &str, label: &str) {
    let Some(stdlib) = cpython_lib_path() else {
        return;
    };
    let root = temp_root(label);
    fs::create_dir_all(&root).expect("create root");
    let source = format!(
        "import shutil\nimport test.test_argparse as mod\nshutil.rmtree('packageæ', ignore_errors=True)\ncase = mod.TestProgName('{case_name}')\nresult = case.defaultTestResult()\ncase.run(result)\nok = len(result.failures) == 0 and len(result.errors) == 0\nprint(ok)\n"
    );
    let (code, stdout, stderr) = run_pyrs(
        &root,
        &["-S", "-c", source.as_str()],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");
    assert_eq!(
        stdout.trim(),
        "True",
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
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
fn cli_script_keeps_live_module_dict_for_dataclass_generated_exec() {
    let Some(stdlib) = cpython_lib_path() else {
        return;
    };
    let root = temp_root("cli_dataclass_live_module_dict");
    fs::create_dir_all(&root).expect("create root");

    let script = root.join("main.py");
    fs::write(
        &script,
        concat!(
            "from dataclasses import dataclass\n",
            "@dataclass(frozen=True)\n",
            "class Location:\n",
            "    x: int\n",
            "@dataclass\n",
            "class Message:\n",
            "    y: int\n",
            "class C:\n",
            "    def f(self):\n",
            "        return Location(1), Message(2)\n",
            "assert 'Location' in C.f.__globals__\n",
            "assert 'Message' in C.f.__globals__\n",
            "first, second = C().f()\n",
            "assert first.x == 1 and second.y == 2\n",
        ),
    )
    .expect("write script");

    let script_arg = script.to_string_lossy();
    let (code, _stdout, stderr) = run_pyrs(
        &root,
        &[script_arg.as_ref()],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
}

#[test]
fn cli_matches_cpython_argparse_progname_directory_case() {
    let Some(stdlib) = cpython_lib_path() else {
        return;
    };
    let root = temp_root("cli_argparse_progname_directory");
    fs::create_dir_all(&root).expect("create root");
    let source = "import shutil\nimport test.test_argparse as mod\nshutil.rmtree('packageæ', ignore_errors=True)\ncase = mod.TestProgName('test_directory')\nresult = case.defaultTestResult()\ncase.run(result)\nok = len(result.failures) == 0 and len(result.errors) == 0\nprint(ok)\n";
    let (code, stdout, stderr) = run_pyrs(
        &root,
        &["-S", "-c", source],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert_eq!(
        stdout.trim(),
        "True",
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn cli_matches_cpython_argparse_progname_zipfile_case() {
    let Some(stdlib) = cpython_lib_path() else {
        return;
    };
    let root = temp_root("cli_argparse_progname_zipfile");
    fs::create_dir_all(&root).expect("create root");
    let source = "import shutil\nimport test.test_argparse as mod\nshutil.rmtree('packageæ', ignore_errors=True)\ncase = mod.TestProgName('test_zipfile')\nresult = case.defaultTestResult()\ncase.run(result)\nok = len(result.failures) == 0 and len(result.errors) == 0\nprint(ok)\n";
    let (code, stdout, stderr) = run_pyrs(
        &root,
        &["-S", "-c", source],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");
    assert_eq!(
        stdout.trim(),
        "True",
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn cli_matches_cpython_argparse_progname_directory_in_zipfile_case() {
    let Some(stdlib) = cpython_lib_path() else {
        return;
    };
    let root = temp_root("cli_argparse_progname_directory_in_zipfile");
    fs::create_dir_all(&root).expect("create root");
    let source = "import shutil\nimport test.test_argparse as mod\nshutil.rmtree('packageæ', ignore_errors=True)\ncase = mod.TestProgName('test_directory_in_zipfile')\nresult = case.defaultTestResult()\ncase.run(result)\nok = len(result.failures) == 0 and len(result.errors) == 0\nprint(ok)\n";
    let (code, stdout, stderr) = run_pyrs(
        &root,
        &["-S", "-c", source],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}\nstdout:\n{stdout}");
    assert_eq!(
        stdout.trim(),
        "True",
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn cli_accepts_cpython_compat_flag_prefixes_before_script_path() {
    let root = temp_root("cli_flag_prefixes");
    let stdlib = root.join("Lib");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    let script = root.join("main.py");
    fs::write(&script, "print('ok')\n").expect("write script");

    let script_arg = script.to_string_lossy();
    let (code, stdout, stderr) = run_pyrs(
        &root,
        &["-I", "-u", script_arg.as_ref()],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert_eq!(stdout.trim(), "ok");
}

#[test]
fn cli_accepts_compact_x_options_before_script_path() {
    let root = temp_root("cli_compact_x_options");
    let stdlib = root.join("Lib");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    let script = root.join("main.py");
    fs::write(
        &script,
        "import sys\nprint(sys.flags.utf8_mode)\nprint('utf8' in sys._xoptions)\nprint(sys._xoptions.get('tracemalloc'))\n",
    )
    .expect("write script");

    let script_arg = script.to_string_lossy();
    let (code, stdout, stderr) = run_pyrs(
        &root,
        &["-I", "-Xutf8", "-Xtracemalloc=5", script_arg.as_ref()],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines, vec!["1", "True", "5"], "stdout:\n{stdout}");
}

#[test]
fn cli_matches_cpython_argparse_progname_script_case() {
    assert_argparse_progname_case("test_script", "cli_argparse_progname");
}

#[test]
fn cli_matches_cpython_argparse_progname_script_compiled_case() {
    assert_argparse_progname_case(
        "test_script_compiled",
        "cli_argparse_progname_script_compiled",
    );
}

#[test]
fn cli_matches_cpython_argparse_progname_module_compiled_case() {
    assert_argparse_progname_case(
        "test_module_compiled",
        "cli_argparse_progname_module_compiled",
    );
}

#[test]
fn cli_matches_cpython_argparse_progname_package_compiled_case() {
    assert_argparse_progname_case(
        "test_package_compiled",
        "cli_argparse_progname_package_compiled",
    );
}

fn cli_script_sets_sys_argv_without_executable_prefix() {
    let root = temp_root("cli_sys_argv_script");
    let stdlib = root.join("Lib");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    let script = root.join("main.py");
    fs::write(&script, "import sys\nprint(repr(sys.argv))\n").expect("write script");
    let script_arg = script.to_string_lossy().to_string();

    let (code, stdout, stderr) = run_pyrs(
        &root,
        &[script_arg.as_str(), "--flag", "value"],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    let expected = format!("['{}', '--flag', 'value']", script_arg);
    assert_eq!(stdout.trim(), expected);
}

#[test]
fn cli_dash_c_sets_sys_argv_like_cpython() {
    let root = temp_root("cli_sys_argv_dash_c");
    let stdlib = root.join("Lib");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    let (code, stdout, stderr) = run_pyrs(
        &root,
        &["-c", "import sys; print(repr(sys.argv))", "extra", "args"],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert_eq!(stdout.trim(), "['-c', 'extra', 'args']");
}

#[test]
fn cli_dash_m_runs_module_and_sets_sys_argv_like_cpython() {
    let root = temp_root("cli_sys_argv_dash_m");
    let stdlib = root.join("Lib");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    fs::write(
        stdlib.join("argmodule.py"),
        "import os, sys\nprint(os.path.basename(sys.argv[0]))\nprint(repr(sys.argv[1:]))\n",
    )
    .expect("write module");

    let (code, stdout, stderr) = run_pyrs(
        &root,
        &["-m", "argmodule", "extra", "args"],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2, "stdout:\n{stdout}");
    assert_eq!(lines[0], "argmodule.py", "stdout:\n{stdout}");
    assert_eq!(lines[1], "['extra', 'args']", "stdout:\n{stdout}");
}

#[test]
fn cli_dash_m_requires_module_name() {
    let root = temp_root("cli_sys_argv_dash_m_missing_name");
    fs::create_dir_all(&root).expect("create root");
    let (code, _stdout, stderr) = run_pyrs(&root, &["-m"], &[]);
    assert_eq!(code, 2, "stderr:\n{stderr}");
    assert!(
        stderr.contains("error: -m expects module name"),
        "stderr:\n{stderr}"
    );
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

#[test]
fn cli_adds_local_lib_dynload_path_when_present_under_pyrs_cpython_lib() {
    let root = temp_root("cli_dynload_path");
    let stdlib = root.join("Lib");
    let local_dynload = stdlib.join("lib-dynload");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::create_dir_all(&local_dynload).expect("create local dynload");
    let local_dynload_normalized =
        fs::canonicalize(&local_dynload).unwrap_or_else(|_| local_dynload.clone());
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    let script = root.join("main.py");
    fs::write(
        &script,
        &format!(
            "import os, sys\nexpected = os.path.normpath({:?})\nassert any(os.path.normpath(p) == expected for p in sys.path), sys.path\n",
            local_dynload_normalized.to_string_lossy()
        ),
    )
    .expect("write script");

    let script_arg = script.to_string_lossy();
    let (code, _stdout, stderr) = run_pyrs(
        &root,
        &[script_arg.as_ref()],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
}

#[test]
fn cli_pyrs_cpython_lib_isolation_skips_host_framework_stdlib_paths() {
    let root = temp_root("cli_stdlib_isolation");
    let stdlib = root.join("Lib");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    let script = root.join("main.py");
    fs::write(
        &script,
        "import os, sys\nforbidden = os.path.normpath('/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14')\npaths = [os.path.normpath(p) for p in sys.path]\nassert forbidden not in paths, paths\n",
    )
    .expect("write script");

    let script_arg = script.to_string_lossy();
    let (code, _stdout, stderr) = run_pyrs(
        &root,
        &[script_arg.as_ref()],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
}

#[test]
fn cli_pyrs_cpython_lib_uses_host_dynload_fallback_without_host_stdlib_root() {
    let mut host_dynload: Option<PathBuf> = None;
    for candidate in [
        "/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14/lib-dynload",
        "/opt/homebrew/Frameworks/Python.framework/Versions/3.14/lib/python3.14/lib-dynload",
        "/usr/local/lib/python3.14/lib-dynload",
        "/usr/lib/python3.14/lib-dynload",
    ] {
        let path = PathBuf::from(candidate);
        if path.is_dir() {
            host_dynload = Some(path);
            break;
        }
    }
    let Some(host_dynload) = host_dynload else {
        eprintln!("skipping host dynload fallback test: no host lib-dynload found");
        return;
    };

    let root = temp_root("cli_dynload_host_fallback");
    let stdlib = root.join("Lib");
    fs::create_dir_all(&stdlib).expect("create stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    let host_dynload_normalized = fs::canonicalize(&host_dynload).unwrap_or(host_dynload);
    let host_stdlib_root = host_dynload_normalized
        .parent()
        .expect("host dynload should have parent")
        .to_path_buf();
    let script = root.join("main.py");
    fs::write(
        &script,
        &format!(
            "import os, sys\nexpected_dyn = os.path.normpath({:?})\nforbidden_root = os.path.normpath({:?})\npaths = [os.path.normpath(p) for p in sys.path]\nassert expected_dyn in paths, paths\nassert forbidden_root not in paths, paths\n",
            host_dynload_normalized.to_string_lossy(),
            host_stdlib_root.to_string_lossy()
        ),
    )
    .expect("write script");

    let script_arg = script.to_string_lossy();
    let (code, _stdout, stderr) = run_pyrs(
        &root,
        &[script_arg.as_ref()],
        &[("PYRS_CPYTHON_LIB", stdlib.as_path())],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
}

#[test]
fn cli_detects_install_managed_stdlib_under_xdg_data_home() {
    let root = temp_root("cli_xdg_managed_stdlib");
    let home = root.join("home");
    let xdg = root.join("xdg");
    let stdlib = xdg.join("pyrs/stdlib/3.14.3/Lib");
    fs::create_dir_all(&stdlib).expect("create xdg stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");

    let expected_suffix = PathBuf::from("xdg/pyrs/stdlib/3.14.3/Lib/site.py");
    let mut cmd = Command::new(pyrs_bin());
    cmd.current_dir(&root);
    cmd.arg("-c");
    cmd.arg(format!(
        "import os, site\nexpected = {:?}.replace('\\\\', '/')\nactual = getattr(site, '__file__', '').replace('\\\\', '/')\nassert actual.endswith(expected), (expected, actual)\n",
        expected_suffix.to_string_lossy()
    ));
    cmd.env_remove("PYRS_CPYTHON_LIB");
    cmd.env_remove("PYTHONHOME");
    cmd.env_remove("PYTHONPATH");
    cmd.env_remove("VIRTUAL_ENV");
    cmd.env("HOME", &home);
    cmd.env("XDG_DATA_HOME", &xdg);
    let output = cmd.output().expect("run pyrs");
    let code = output.status.code().unwrap_or(1);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(code, 0, "stderr:\n{stderr}");
}

#[test]
fn cli_legacy_local_share_stdlib_fallback_still_works_when_xdg_data_home_is_set() {
    let root = temp_root("cli_legacy_local_share_stdlib");
    let home = root.join("home");
    let xdg = root.join("xdg");
    let stdlib = home.join(".local/share/pyrs/stdlib/3.14.3/Lib");
    fs::create_dir_all(&stdlib).expect("create legacy stdlib");
    fs::write(stdlib.join("site.py"), "started = True\n").expect("write site.py");
    fs::create_dir_all(&xdg).expect("create xdg root");

    let expected_suffix = PathBuf::from(".local/share/pyrs/stdlib/3.14.3/Lib/site.py");
    let mut cmd = Command::new(pyrs_bin());
    cmd.current_dir(&root);
    cmd.arg("-c");
    cmd.arg(format!(
        "import os, site\nexpected = {:?}.replace('\\\\', '/')\nactual = getattr(site, '__file__', '').replace('\\\\', '/')\nassert actual.endswith(expected), (expected, actual)\n",
        expected_suffix.to_string_lossy()
    ));
    cmd.env_remove("PYRS_CPYTHON_LIB");
    cmd.env_remove("PYTHONHOME");
    cmd.env_remove("PYTHONPATH");
    cmd.env_remove("VIRTUAL_ENV");
    cmd.env("HOME", &home);
    cmd.env("XDG_DATA_HOME", &xdg);
    let output = cmd.output().expect("run pyrs");
    let code = output.status.code().unwrap_or(1);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(code, 0, "stderr:\n{stderr}");
}
