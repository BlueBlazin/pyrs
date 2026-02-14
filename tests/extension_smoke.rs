use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn pyrs_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_SUBPROCESS_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let debug = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/pyrs");
    if debug.is_file() {
        return Some(debug);
    }
    let release = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/pyrs");
    if release.is_file() {
        return Some(release);
    }
    None
}

fn unique_temp_dir(stem: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    std::env::temp_dir().join(format!("pyrs_{stem}_{nanos}"))
}

fn python_string_literal(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[test]
fn imports_manifest_backed_hello_extension() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping extension smoke (pyrs binary not found)");
        return;
    };

    let temp_root = unique_temp_dir("ext_smoke");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");
    let manifest_path = temp_root.join("hello_ext.pyrs-ext");
    fs::write(
        &manifest_path,
        "module=hello_ext\nabi=pyrs314\nentrypoint=hello_ext\n",
    )
    .expect("manifest should be written");

    let snippet = format!(
        "import sys\nsys.path.insert(0, \"{}\")\nimport hello_ext\nassert hello_ext.EXTENSION_LOADED is True\nassert hello_ext.ENTRYPOINT == 'hello_ext'\nassert hello_ext.MESSAGE == 'hello from hello_ext'\nassert hello_ext.__loader__ == 'pyrs.ExtensionFileLoader'\nassert hello_ext.__pyrs_extension__ is True",
        python_string_literal(&temp_root)
    );
    let output = Command::new(&bin)
        .arg("-S")
        .arg("-c")
        .arg(snippet)
        .output()
        .expect("pyrs subprocess should run");

    if !output.status.success() {
        panic!(
            "extension smoke failed (status={}):\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_dir_all(temp_root);
}
