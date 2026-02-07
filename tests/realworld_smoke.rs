use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    if let Ok(exe) = std::env::current_exe() {
        if let Some(debug_dir) = exe.parent().and_then(|deps| deps.parent()) {
            let sibling = debug_dir.join("pyrs");
            if sibling.is_file() {
                return sibling;
            }
        }
    }
    panic!("unable to locate pyrs binary for sandboxed smoke tests");
}

fn run_sandboxed_script(root: &Path, entry_rel: &str) -> (i32, String, String) {
    let stdout_path = root.join("stdout.txt");
    let stderr_path = root.join("stderr.txt");
    let stdout = File::create(&stdout_path).expect("create stdout capture");
    let stderr = File::create(&stderr_path).expect("create stderr capture");
    let home = root.join("home");
    fs::create_dir_all(&home).expect("create home");

    let mut child = Command::new(pyrs_bin())
        .arg(root.join(entry_rel))
        .current_dir(root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("PYRS_SANDBOX", "1")
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn pyrs");

    let timeout = Duration::from_secs(10);
    let start = Instant::now();
    let exit_code = loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            break status.code().unwrap_or(1);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            panic!("sandboxed script exceeded timeout: {entry_rel}");
        }
        thread::sleep(Duration::from_millis(10));
    };

    let stdout_text = fs::read_to_string(stdout_path).unwrap_or_default();
    let stderr_text = fs::read_to_string(stderr_path).unwrap_or_default();
    (exit_code, stdout_text, stderr_text)
}

fn compact(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect()
}

#[test]
fn runs_curated_realworld_sync_app() {
    let root = temp_root("realworld_sync");
    fs::create_dir_all(&root).expect("create root");
    fs::write(
        root.join("main.py"),
        r#"
def summarize(records):
    counts = {}
    for value in records:
        bucket = value % 2
        if bucket in counts:
            counts[bucket] = counts[bucket] + value
        else:
            counts[bucket] = value
    return counts

rows = [1, 2, 3, 4, 5, 6]
result = summarize(rows)
print(result[0])
print(result[1])
"#,
    )
    .expect("write entry");

    let (code, stdout, stderr) = run_sandboxed_script(&root, "main.py");
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert_eq!(stdout.trim(), "12\n9");
}

#[test]
fn runs_curated_realworld_async_app() {
    let root = temp_root("realworld_async");
    fs::create_dir_all(&root).expect("create root");
    fs::write(
        root.join("main.py"),
        r#"
import asyncio

async def fetch(x):
    return x * 2

async def pipeline():
    values = await asyncio.gather(fetch(2), fetch(5), fetch(7))
    return sum(values)

result = asyncio.run(pipeline())
print(result)
"#,
    )
    .expect("write async app");

    let (code, stdout, stderr) = run_sandboxed_script(&root, "main.py");
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert_eq!(stdout.trim(), "28");
}

#[test]
fn runs_curated_realworld_package_cli() {
    let root = temp_root("realworld_pkg");
    let pkg = root.join("toolkit");
    fs::create_dir_all(&pkg).expect("create package");
    fs::write(pkg.join("__init__.py"), "").expect("write init");
    fs::write(
        pkg.join("core.py"),
        r#"
def transform(values):
    out = []
    for item in values:
        if item % 2 == 0:
            out += [item * item]
    return out
"#,
    )
    .expect("write core");
    fs::write(
        root.join("cli.py"),
        r#"
from toolkit.core import transform
import json

vals = [1, 2, 3, 4, 5, 6]
result = transform(vals)
print(json.dumps(result))
"#,
    )
    .expect("write cli");

    let (code, stdout, stderr) = run_sandboxed_script(&root, "cli.py");
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert_eq!(compact(stdout.trim()), compact("[4, 16, 36]"));
}
