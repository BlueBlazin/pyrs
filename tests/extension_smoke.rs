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

fn has_c_compiler() -> bool {
    Command::new("cc")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn compile_shared_extension(source_path: &Path, output_path: &Path) -> Result<(), String> {
    let include_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include");
    let mut cmd = Command::new("cc");
    cmd.arg("-fPIC");
    #[cfg(target_os = "macos")]
    {
        cmd.arg("-dynamiclib");
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        cmd.arg("-shared");
    }
    #[cfg(target_os = "windows")]
    {
        cmd.arg("-shared");
    }
    cmd.arg("-I")
        .arg(include_dir)
        .arg(source_path)
        .arg("-o")
        .arg(output_path);

    let output = cmd
        .output()
        .map_err(|err| format!("failed to invoke C compiler: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "C compiler failed (status={}):\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn run_import_snippet(bin: &Path, temp_root: &Path, snippet_body: &str) -> Result<(), String> {
    let snippet = format!(
        "import sys\nsys.path.insert(0, \"{}\")\n{}",
        python_string_literal(temp_root),
        snippet_body
    );
    let output = Command::new(bin)
        .arg("-S")
        .arg("-c")
        .arg(snippet)
        .output()
        .map_err(|err| format!("pyrs subprocess failed: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "extension smoke failed (status={}):\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

#[test]
fn imports_manifest_backed_hello_extension() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping extension smoke (pyrs binary not found)");
        return;
    };

    let temp_root = unique_temp_dir("ext_smoke_manifest");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");
    let manifest_path = temp_root.join("hello_ext.pyrs-ext");
    fs::write(
        &manifest_path,
        "module=hello_ext\nabi=pyrs314\nentrypoint=hello_ext\n",
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import hello_ext\nassert hello_ext.EXTENSION_LOADED is True\nassert hello_ext.ENTRYPOINT == 'hello_ext'\nassert hello_ext.MESSAGE == 'hello from hello_ext'\nassert hello_ext.__loader__ == 'pyrs.ExtensionFileLoader'\nassert hello_ext.__pyrs_extension__ is True",
    )
    .expect("manifest hello_ext import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn imports_compiled_dynamic_extension_from_manifest() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping dynamic extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping dynamic extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_dynamic_manifest");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_manifest_ext.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_set_bool(module_ctx, "EXTENSION_LOADED", 1) != 0) {
        return -2;
    }
    if (api->module_set_int(module_ctx, "ANSWER", 42) != 0) {
        return -3;
    }
    if (api->module_set_string(module_ctx, "ENTRYPOINT", "pyrs_extension_init_v1") != 0) {
        return -4;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = if cfg!(target_os = "macos") {
        "libnative_manifest_ext.dylib"
    } else if cfg!(target_os = "windows") {
        "native_manifest_ext.pyd"
    } else {
        "libnative_manifest_ext.so"
    };
    let library_path = temp_root.join(library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_manifest_ext.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_manifest_ext\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_manifest_ext\nassert native_manifest_ext.EXTENSION_LOADED is True\nassert native_manifest_ext.ANSWER == 42\nassert native_manifest_ext.ENTRYPOINT == 'pyrs_extension_init_v1'\nassert native_manifest_ext.__loader__ == 'pyrs.ExtensionFileLoader'\nassert native_manifest_ext.__pyrs_extension_abi__ == 'pyrs314'",
    )
    .expect("manifest dynamic extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn imports_direct_shared_object_extension_without_manifest() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping direct extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping direct extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_direct");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("direct_native.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_set_bool(module_ctx, "DIRECT_LOADED", 1) != 0) {
        return -2;
    }
    if (api->module_set_string(module_ctx, "SOURCE", "direct-shared-object") != 0) {
        return -3;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let direct_library = temp_root.join("direct_native.so");
    compile_shared_extension(&source_path, &direct_library)
        .expect("direct extension shared object should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import direct_native\nassert direct_native.DIRECT_LOADED is True\nassert direct_native.SOURCE == 'direct-shared-object'\nassert direct_native.__loader__ == 'pyrs.ExtensionFileLoader'\nassert direct_native.__pyrs_extension_entrypoint__ == 'dynamic:pyrs_extension_init_v1'",
    )
    .expect("direct shared object import should succeed");

    let _ = fs::remove_file(direct_library);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}
