#![cfg(not(target_arch = "wasm32"))]

use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const CPYTHON_STDLIB_VERSION: &str = "3.14.3";

fn temp_root(prefix: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "pyrs_{prefix}_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

fn target_triple() -> &'static str {
    match (env::consts::ARCH, env::consts::OS) {
        ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
        ("x86_64", "macos") => "x86_64-apple-darwin",
        ("aarch64", "macos") => "aarch64-apple-darwin",
        other => panic!("unsupported host target for installer test: {other:?}"),
    }
}

fn installer_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/install.sh")
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write executable");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod executable");
}

fn sha256_of(path: &Path) -> String {
    let sha256sum = Command::new("sha256sum").arg(path).output();
    if let Ok(output) = sha256sum
        && output.status.success()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        return stdout
            .split_whitespace()
            .next()
            .expect("sha256sum output")
            .to_string();
    }

    let output = Command::new("shasum")
        .arg("-a")
        .arg("256")
        .arg(path)
        .output()
        .expect("run shasum");
    assert!(output.status.success(), "shasum failed: {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .split_whitespace()
        .next()
        .expect("shasum output")
        .to_string()
}

fn create_tar_gz(out_path: &Path, root_dir: &Path, entry: &str) {
    let output = Command::new("tar")
        .arg("-czf")
        .arg(out_path)
        .arg("-C")
        .arg(root_dir)
        .arg(entry)
        .output()
        .expect("run tar");
    assert!(
        output.status.success(),
        "tar failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn make_release_assets(root: &Path, tag: &str) -> PathBuf {
    let assets = root.join("assets");
    fs::create_dir_all(&assets).expect("create assets dir");

    let binary_payload = root.join("binary_payload");
    fs::create_dir_all(&binary_payload).expect("create binary payload dir");
    fs::write(binary_payload.join("pyrs"), "#!/bin/sh\nexit 0\n").expect("write fake pyrs binary");

    let binary_asset = if tag == "nightly" {
        format!("pyrs-nightly-{}.tar.gz", target_triple())
    } else {
        format!("pyrs-{tag}-{}.tar.gz", target_triple())
    };
    create_tar_gz(&assets.join(&binary_asset), &binary_payload, "pyrs");

    let stdlib_root = root
        .join("stdlib_payload")
        .join(format!("pyrs-stdlib-cpython-{CPYTHON_STDLIB_VERSION}"));
    fs::create_dir_all(stdlib_root.join("Lib")).expect("create stdlib payload dir");
    fs::write(stdlib_root.join("Lib/site.py"), "started = True\n").expect("write site.py");
    fs::write(stdlib_root.join("LICENSE"), "CPython license\n").expect("write LICENSE");

    let stdlib_asset = format!("pyrs-stdlib-cpython-{CPYTHON_STDLIB_VERSION}.tar.gz");
    create_tar_gz(
        &assets.join(&stdlib_asset),
        stdlib_root.parent().expect("stdlib payload parent"),
        &format!("pyrs-stdlib-cpython-{CPYTHON_STDLIB_VERSION}"),
    );

    let checksums = format!(
        "{}  {}\n{}  {}\n",
        sha256_of(&assets.join(&binary_asset)),
        binary_asset,
        sha256_of(&assets.join(&stdlib_asset)),
        stdlib_asset
    );
    fs::write(assets.join("SHA256SUMS"), checksums).expect("write SHA256SUMS");
    assets
}

fn make_curl_wrapper(root: &Path) -> PathBuf {
    let bin_dir = root.join("bin");
    fs::create_dir_all(&bin_dir).expect("create wrapper dir");
    let curl_path = bin_dir.join("curl");
    write_executable(
        &curl_path,
        r#"#!/usr/bin/env bash
set -euo pipefail
out=""
url=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o)
      out="$2"
      shift 2
      ;;
    --retry)
      shift 2
      ;;
    --retry-all-errors|-fsSL|-f|-s|-S|-L)
      shift
      ;;
    http://*|https://*)
      url="$1"
      shift
      ;;
    *)
      shift
      ;;
  esac
done
printf '%s\n' "$url" >> "$PYRS_TEST_CURL_LOG"
case "$url" in
  https://api.github.com/repos/*/releases?per_page=1)
    cat "$PYRS_TEST_API_RESPONSE"
    ;;
  */SHA256SUMS|*/pyrs-*.tar.gz)
    cp "$PYRS_TEST_ASSET_DIR/${url##*/}" "$out"
    ;;
  *)
    echo "unexpected curl url: $url" >&2
    exit 1
    ;;
esac
"#,
    );
    let python314_path = bin_dir.join("python3.14");
    write_executable(
        &python314_path,
        r#"#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "${PYRS_TEST_HOST_STDLIB:-}"
"#,
    );
    bin_dir
}

fn write_uname_wrapper(wrapper_bin_dir: &Path, os_name: &str, arch: &str) {
    let uname_path = wrapper_bin_dir.join("uname");
    write_executable(
        &uname_path,
        &format!(
            "#!/usr/bin/env bash\nset -euo pipefail\ncase \"${{1:-}}\" in\n  -s) printf '%s\\n' {:?} ;;\n  -m) printf '%s\\n' {:?} ;;\n  *) /usr/bin/uname \"$@\" ;;\nesac\n",
            os_name, arch
        ),
    );
}

fn run_installer(
    root: &Path,
    wrapper_bin_dir: &Path,
    assets_dir: &Path,
    api_response: &Path,
    args: &[&str],
    extra_env: &[(&str, &Path)],
) -> (i32, String, String, String) {
    let curl_log = root.join("curl.log");
    let mut path = OsString::from(wrapper_bin_dir.as_os_str());
    if let Some(original_path) = env::var_os("PATH")
        && !original_path.is_empty()
    {
        path.push(":");
        path.push(original_path);
    }
    let output = Command::new("bash")
        .arg(installer_script())
        .args(args)
        .env("PATH", path)
        .env("PYRS_TEST_ASSET_DIR", assets_dir)
        .env("PYRS_TEST_API_RESPONSE", api_response)
        .env("PYRS_TEST_CURL_LOG", &curl_log)
        .env_remove("PYTHONHOME")
        .env_remove("PYRS_CPYTHON_LIB")
        .current_dir(root)
        .envs(extra_env.iter().map(|(name, value)| (*name, value.as_os_str())))
        .output()
        .expect("run installer");

    let curl_log_contents = fs::read_to_string(&curl_log).unwrap_or_default();
    (
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        curl_log_contents,
    )
}

#[test]
fn installer_force_bundled_stdlib_uses_xdg_data_home_and_uninstall_removes_files() {
    let root = temp_root("install_script_xdg");
    fs::create_dir_all(&root).expect("create temp root");
    let assets = make_release_assets(&root, "nightly");
    let api_response = root.join("releases.json");
    fs::write(&api_response, "[]\n").expect("write api response");
    let wrapper_bin_dir = make_curl_wrapper(&root);

    let home = root.join("home");
    let xdg = root.join("xdg");
    fs::create_dir_all(&home).expect("create home");
    fs::create_dir_all(&xdg).expect("create xdg");

    let (code, _stdout, stderr, _curl_log) = run_installer(
        &root,
        &wrapper_bin_dir,
        &assets,
        &api_response,
        &["--tag", "nightly", "--force-bundled-stdlib"],
        &[("HOME", &home), ("XDG_DATA_HOME", &xdg)],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");

    let installed_bin = home.join(".local/bin/pyrs");
    let installed_stdlib = xdg
        .join("pyrs")
        .join("stdlib")
        .join(CPYTHON_STDLIB_VERSION)
        .join("Lib/site.py");
    assert!(installed_bin.is_file(), "missing installed binary");
    assert!(installed_stdlib.is_file(), "missing installed stdlib");

    let (code, _stdout, stderr, _curl_log) = run_installer(
        &root,
        &wrapper_bin_dir,
        &assets,
        &api_response,
        &["--uninstall"],
        &[("HOME", &home), ("XDG_DATA_HOME", &xdg)],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert!(!installed_bin.exists(), "binary should be removed");
    assert!(
        !xdg.join("pyrs").join("stdlib").join(CPYTHON_STDLIB_VERSION).exists(),
        "managed stdlib should be removed"
    );
}

#[test]
fn installer_skips_bundled_stdlib_when_host_cpython_314_is_present() {
    let root = temp_root("install_script_host_skip");
    fs::create_dir_all(&root).expect("create temp root");
    let assets = make_release_assets(&root, "nightly");
    let api_response = root.join("releases.json");
    fs::write(&api_response, "[]\n").expect("write api response");
    let wrapper_bin_dir = make_curl_wrapper(&root);

    let home = root.join("home");
    let xdg = root.join("xdg");
    let host_stdlib = root.join("host-python-3.14");
    fs::create_dir_all(&home).expect("create home");
    fs::create_dir_all(&xdg).expect("create xdg");
    fs::create_dir_all(&host_stdlib).expect("create host stdlib");
    fs::write(host_stdlib.join("site.py"), "started = True\n").expect("write host site.py");

    let mut path = OsString::from(wrapper_bin_dir.as_os_str());
    if let Some(original_path) = env::var_os("PATH")
        && !original_path.is_empty()
    {
        path.push(":");
        path.push(original_path);
    }
    let (code, stdout, stderr, curl_log) = Command::new("bash")
        .arg(installer_script())
        .arg("--tag")
        .arg("nightly")
        .env("PATH", path)
        .env("PYRS_TEST_ASSET_DIR", &assets)
        .env("PYRS_TEST_API_RESPONSE", &api_response)
        .env("PYRS_TEST_CURL_LOG", root.join("curl.log"))
        .env("PYRS_TEST_HOST_STDLIB", &host_stdlib)
        .env("HOME", &home)
        .env("XDG_DATA_HOME", &xdg)
        .env_remove("PYTHONHOME")
        .env_remove("PYRS_CPYTHON_LIB")
        .current_dir(&root)
        .output()
        .map(|output| {
            let curl_log = fs::read_to_string(root.join("curl.log")).unwrap_or_default();
            (
                output.status.code().unwrap_or(1),
                String::from_utf8_lossy(&output.stdout).into_owned(),
                String::from_utf8_lossy(&output.stderr).into_owned(),
                curl_log,
            )
        })
        .expect("run installer");
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert!(
        stdout.contains("Using existing CPython 3.14 stdlib"),
        "stdout:\n{stdout}"
    );
    assert!(home.join(".local/bin/pyrs").is_file(), "missing installed binary");
    assert!(
        !xdg.join("pyrs").join("stdlib").join(CPYTHON_STDLIB_VERSION).exists(),
        "installer should skip managed stdlib when host 3.14 is present"
    );
    assert!(
        !curl_log.contains(&format!("pyrs-stdlib-cpython-{CPYTHON_STDLIB_VERSION}.tar.gz")),
        "installer should not download the bundled stdlib when host 3.14 is present\ncurl log:\n{curl_log}"
    );
}

#[test]
fn installer_stable_channel_resolves_latest_published_prerelease_tag() {
    let root = temp_root("install_script_stable");
    fs::create_dir_all(&root).expect("create temp root");
    let tag = "v0.1.0-beta.1";
    let assets = make_release_assets(&root, tag);
    let api_response = root.join("releases.json");
    fs::write(
        &api_response,
        format!(
            "[\n  {{\n    \"tag_name\": \"{tag}\",\n    \"prerelease\": true\n  }}\n]\n"
        ),
    )
    .expect("write api response");
    let wrapper_bin_dir = make_curl_wrapper(&root);

    let home = root.join("home");
    let data_dir = root.join("data");
    let bin_dir = home.join("bin");
    fs::create_dir_all(&home).expect("create home");

    let (code, _stdout, stderr, curl_log) = run_installer(
        &root,
        &wrapper_bin_dir,
        &assets,
        &api_response,
        &[
            "--stable",
            "--force-bundled-stdlib",
            "--bin-dir",
            bin_dir.to_str().expect("bin path"),
            "--data-dir",
            data_dir.to_str().expect("data dir"),
        ],
        &[("HOME", &home)],
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert!(
        curl_log.contains("/releases?per_page=1"),
        "stable installer should query the releases list\ncurl log:\n{curl_log}"
    );
    assert!(
        curl_log.contains(&format!("pyrs-{tag}-{}.tar.gz", target_triple())),
        "stable installer should fetch the prerelease-tagged archive\ncurl log:\n{curl_log}"
    );
    assert!(
        bin_dir.join("pyrs").is_file(),
        "expected installed binary under custom bin dir"
    );
    assert!(
        data_dir
            .join("stdlib")
            .join(CPYTHON_STDLIB_VERSION)
            .join("Lib/site.py")
            .is_file(),
        "expected installed stdlib under custom data dir"
    );
}

#[test]
fn installer_rejects_linux_arm64_native_target() {
    let root = temp_root("install_script_linux_arm64_unsupported");
    fs::create_dir_all(&root).expect("create temp root");
    let assets = make_release_assets(&root, "nightly");
    let api_response = root.join("releases.json");
    fs::write(&api_response, "[]\n").expect("write api response");
    let wrapper_bin_dir = make_curl_wrapper(&root);
    write_uname_wrapper(&wrapper_bin_dir, "Linux", "aarch64");

    let home = root.join("home");
    fs::create_dir_all(&home).expect("create home");

    let (code, _stdout, stderr, curl_log) = run_installer(
        &root,
        &wrapper_bin_dir,
        &assets,
        &api_response,
        &["--tag", "nightly"],
        &[("HOME", &home)],
    );
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.contains("native Linux arm64/aarch64 binaries are not part of the current release matrix"),
        "stderr:\n{stderr}"
    );
    assert!(
        curl_log.trim().is_empty(),
        "installer should fail before any network/download attempts\ncurl log:\n{curl_log}"
    );
}
