use std::collections::HashMap;
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

fn query_pyrs_build_vars(bin: &Path) -> Result<HashMap<String, String>, String> {
    let snippet = r#"import sys
name = f"_sysconfigdata_{sys.abiflags}_{sys.platform}_{getattr(sys.implementation, '_multiarch', '')}"
mod = __import__(name)
keys = ["CC", "CFLAGS", "LDSHARED", "EXT_SUFFIX"]
for key in keys:
    value = mod.build_time_vars.get(key, "")
    print(f"{key}={value}")
"#;
    let output = Command::new(bin)
        .arg("-S")
        .arg("-c")
        .arg(snippet)
        .output()
        .map_err(|err| format!("failed to query pyrs build vars: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "failed to query pyrs build vars (status={}):\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let mut vars = HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        vars.insert(key.to_string(), value.to_string());
    }
    Ok(vars)
}

fn compile_shared_extension_with_build_vars(
    source_path: &Path,
    output_path: &Path,
    build_vars: &HashMap<String, String>,
) -> Result<(), String> {
    let include_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include");
    let compiler_cmd = build_vars
        .get("CC")
        .cloned()
        .unwrap_or_else(|| "cc".to_string());
    let mut compiler_parts = compiler_cmd.split_whitespace();
    let compiler = compiler_parts.next().unwrap_or("cc");
    let mut cmd = Command::new(compiler);
    for part in compiler_parts {
        cmd.arg(part);
    }
    for part in build_vars
        .get("CFLAGS")
        .map(String::as_str)
        .unwrap_or("")
        .split_whitespace()
    {
        cmd.arg(part);
    }
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
        .map_err(|err| format!("failed to invoke configured C compiler: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "configured C compiler failed (status={}):\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn shared_library_filename(stem: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else if cfg!(target_os = "windows") {
        format!("{stem}.pyd")
    } else {
        format!("lib{stem}.so")
    }
}

fn importable_module_library_filename(stem: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{stem}.pyd")
    } else {
        format!("{stem}.so")
    }
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

fn run_import_snippet_expect_error(
    bin: &Path,
    temp_root: &Path,
    snippet_body: &str,
    expected_substring: &str,
) -> Result<(), String> {
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
    if output.status.success() {
        return Err("expected import failure but subprocess succeeded".to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !stderr.contains(expected_substring) {
        return Err(format!(
            "expected stderr to contain '{}', got:\n{}",
            expected_substring, stderr
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
fn sysconfigdata_builtin_exposes_extension_build_keys() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping sysconfigdata smoke (pyrs binary not found)");
        return;
    };

    let temp_root = unique_temp_dir("ext_smoke_sysconfigdata");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    run_import_snippet(
        &bin,
        &temp_root,
        "import sys\nname = f\"_sysconfigdata_{sys.abiflags}_{sys.platform}_{getattr(sys.implementation, '_multiarch', '')}\"\nm = __import__(name)\nvars = m.build_time_vars\nassert isinstance(vars.get('SOABI'), str) and vars.get('SOABI')\nassert isinstance(vars.get('EXT_SUFFIX'), str) and vars.get('EXT_SUFFIX').endswith(('.so', '.pyd'))\nassert isinstance(vars.get('CC'), str) and vars.get('CC')\nassert vars.get('Py_GIL_DISABLED') in (0, 1)",
    )
    .expect("sysconfigdata build vars should expose extension keys");

    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn sysconfig_build_vars_can_compile_and_import_extension() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping sysconfig build-vars compile smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping sysconfig build-vars compile smoke (cc not available)");
        return;
    }

    let build_vars =
        query_pyrs_build_vars(&bin).expect("pyrs should expose baseline extension build vars");
    let ext_suffix = build_vars
        .get("EXT_SUFFIX")
        .cloned()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ".so".to_string());

    let temp_root = unique_temp_dir("ext_smoke_sysconfig_compile");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("syscfg_native.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_set_bool(module_ctx, "COMPILED_WITH_SYSCONFIG", 1) != 0) {
        return -2;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let output_library = temp_root.join(format!("syscfg_native{ext_suffix}"));
    compile_shared_extension_with_build_vars(&source_path, &output_library, &build_vars)
        .expect("configured compiler should build extension");

    run_import_snippet(
        &bin,
        &temp_root,
        "import syscfg_native\nassert syscfg_native.COMPILED_WITH_SYSCONFIG is True",
    )
    .expect("extension built with sysconfig vars should import");

    let _ = fs::remove_file(output_library);
    let _ = fs::remove_file(source_path);
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

    let library_file = shared_library_filename("native_manifest_ext");
    let library_path = temp_root.join(&library_file);
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

    let direct_library = temp_root.join(importable_module_library_filename("direct_native"));
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

#[test]
fn direct_cpython_style_symbol_reports_explicit_unsupported_error() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython-symbol smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython-symbol smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_symbol");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_symbol_only.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int PyInit_cpython_symbol_only(void) {
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename("cpython_symbol_only"));
    compile_shared_extension(&source_path, &library_path)
        .expect("cp-style symbol shared object should build");

    run_import_snippet_expect_error(
        &bin,
        &temp_root,
        "import cpython_symbol_only",
        "CPython-style extension symbols",
    )
    .expect("cpython-only symbol should produce explicit unsupported diagnostic");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_set_module_values_via_object_handles() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping object-handle extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping object-handle extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_handles");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_handles.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    PyrsObjectHandle answer = api->object_new_int(module_ctx, 99);
    PyrsObjectHandle none_value = api->object_new_none(module_ctx);
    PyrsObjectHandle ratio = api->object_new_float(module_ctx, 3.5);
    PyrsObjectHandle text = api->object_new_string(module_ctx, "from-object-handle");
    if (!answer || !none_value || !ratio || !text) {
        return -2;
    }
    double ratio_check = 0.0;
    if (api->object_get_float(module_ctx, ratio, &ratio_check) != 0 || ratio_check != 3.5) {
        return -9;
    }
    if (api->module_set_object(module_ctx, "ANSWER", answer) != 0) {
        return -3;
    }
    if (api->module_set_object(module_ctx, "NONE_VALUE", none_value) != 0) {
        return -10;
    }
    if (api->module_set_object(module_ctx, "RATIO", ratio) != 0) {
        return -11;
    }
    if (api->module_set_object(module_ctx, "TEXT", text) != 0) {
        return -4;
    }
    if (api->object_incref(module_ctx, answer) != 0) {
        return -5;
    }
    if (api->object_decref(module_ctx, answer) != 0) {
        return -6;
    }
    if (api->object_decref(module_ctx, answer) != 0) {
        return -7;
    }
    if (api->object_decref(module_ctx, none_value) != 0) {
        return -12;
    }
    if (api->object_decref(module_ctx, ratio) != 0) {
        return -13;
    }
    if (api->object_decref(module_ctx, text) != 0) {
        return -8;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_handles");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_handles.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_handles\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_handles\nassert native_handles.ANSWER == 99\nassert native_handles.NONE_VALUE is None\nassert abs(native_handles.RATIO - 3.5) < 1e-12\nassert native_handles.TEXT == 'from-object-handle'",
    )
    .expect("object-handle dynamic extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_error_state_is_propagated_to_import_failure() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping extension error-state smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping extension error-state smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_error_state");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_error_state.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    api->error_set(module_ctx, "native extension requested failure");
    return -17;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_error_state");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_error_state.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_error_state\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet_expect_error(
        &bin,
        &temp_root,
        "import native_error_state",
        "native extension requested failure",
    )
    .expect("error-state failure should propagate");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn imports_tagged_shared_object_extension_name() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping tagged extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping tagged extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_tagged_name");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("tagged_native.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_set_bool(module_ctx, "TAGGED", 1) != 0) {
        return -2;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let tagged_suffix = if cfg!(target_os = "macos") {
        ".cpython-314-darwin.so"
    } else if cfg!(target_os = "windows") {
        ".cp314-win_amd64.pyd"
    } else {
        ".cpython-314-x86_64-linux-gnu.so"
    };
    let tagged_library = temp_root.join(format!("tagged_native{tagged_suffix}"));
    compile_shared_extension(&source_path, &tagged_library)
        .expect("tagged extension shared object should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import tagged_native\nassert tagged_native.TAGGED is True",
    )
    .expect("tagged shared-object import should succeed");

    let _ = fs::remove_file(tagged_library);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_register_callable() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping callable extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping callable extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_callable");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_callable.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int native_add(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    if (!api || !argv || !result) {
        return -1;
    }
    if (argc != 2) {
        api->error_set(module_ctx, "native_add expects exactly 2 positional arguments");
        return -2;
    }
    if (api->object_type(module_ctx, argv[0]) != PYRS_TYPE_INT ||
        api->object_type(module_ctx, argv[1]) != PYRS_TYPE_INT) {
        api->error_set(module_ctx, "native_add only accepts ints");
        return -3;
    }
    int64_t left = 0;
    int64_t right = 0;
    if (api->object_get_int(module_ctx, argv[0], &left) != 0 ||
        api->object_get_int(module_ctx, argv[1], &right) != 0) {
        return -4;
    }
    *result = api->object_new_int(module_ctx, left + right);
    if (*result == 0) {
        return -5;
    }
    return 0;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_add_function(module_ctx, "add", native_add) != 0) {
        return -2;
    }
    if (api->module_set_string(module_ctx, "API_KIND", "callable") != 0) {
        return -3;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_callable");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_callable.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_callable\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_callable\nassert native_callable.API_KIND == 'callable'\nassert native_callable.add(20, 22) == 42",
    )
    .expect("callable extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_register_kw_callable() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping kw-callable extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping kw-callable extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_kw_callable");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_kw_callable.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"
#include <string.h>

int native_add_scaled(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    uintptr_t kwargc,
    const char* const* kwarg_names,
    const PyrsObjectHandle* kwarg_values,
    PyrsObjectHandle* result
) {
    if (!api || !argv || !result) {
        return -1;
    }
    if (argc != 2) {
        api->error_set(module_ctx, "native_add_scaled expects exactly 2 positional arguments");
        return -2;
    }
    if (api->object_type(module_ctx, argv[0]) != PYRS_TYPE_INT ||
        api->object_type(module_ctx, argv[1]) != PYRS_TYPE_INT) {
        api->error_set(module_ctx, "native_add_scaled only accepts ints");
        return -3;
    }

    int64_t scale = 1;
    if (kwargc > 1) {
        api->error_set(module_ctx, "native_add_scaled accepts at most one keyword");
        return -4;
    }
    if (kwargc == 1) {
        if (!kwarg_names || !kwarg_values) {
            api->error_set(module_ctx, "native_add_scaled missing keyword payload");
            return -5;
        }
        if (!kwarg_names[0] || strcmp(kwarg_names[0], "scale") != 0) {
            api->error_set(module_ctx, "native_add_scaled only accepts keyword 'scale'");
            return -6;
        }
        if (api->object_get_int(module_ctx, kwarg_values[0], &scale) != 0) {
            api->error_set(module_ctx, "native_add_scaled keyword 'scale' must be int");
            return -7;
        }
    }

    int64_t left = 0;
    int64_t right = 0;
    if (api->object_get_int(module_ctx, argv[0], &left) != 0 ||
        api->object_get_int(module_ctx, argv[1], &right) != 0) {
        return -8;
    }
    *result = api->object_new_int(module_ctx, (left + right) * scale);
    if (*result == 0) {
        return -9;
    }
    return 0;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_add_function_kw(module_ctx, "add_scaled", native_add_scaled) != 0) {
        return -2;
    }
    if (api->module_set_string(module_ctx, "API_KIND", "kw-callable") != 0) {
        return -3;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_kw_callable");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_kw_callable.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_kw_callable\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_kw_callable\nassert native_kw_callable.API_KIND == 'kw-callable'\nassert native_kw_callable.add_scaled(2, 3) == 5\nassert native_kw_callable.add_scaled(2, 3, scale=10) == 50",
    )
    .expect("kw-callable extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}
