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

fn c_string_literal(path: &Path) -> String {
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

fn compile_shared_extension_with_cpython_compat(
    source_path: &Path,
    output_path: &Path,
) -> Result<(), String> {
    let include_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include");
    let mut cmd = Command::new("cc");
    cmd.arg("-fPIC");
    #[cfg(target_os = "macos")]
    {
        cmd.arg("-dynamiclib");
        cmd.arg("-undefined").arg("dynamic_lookup");
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
        "import sys\nname = f\"_sysconfigdata_{sys.abiflags}_{sys.platform}_{getattr(sys.implementation, '_multiarch', '')}\"\nm = __import__(name)\nvars = m.build_time_vars\nassert isinstance(vars.get('SOABI'), str) and vars.get('SOABI')\nassert isinstance(vars.get('EXT_SUFFIX'), str) and vars.get('EXT_SUFFIX').endswith(('.so', '.pyd'))\nassert isinstance(vars.get('CC'), str) and vars.get('CC')\nassert isinstance(vars.get('AR'), str) and vars.get('AR')\nassert isinstance(vars.get('CCSHARED'), str) and vars.get('CCSHARED')\nassert isinstance(vars.get('LDSHARED'), str) and vars.get('LDSHARED')\nassert isinstance(vars.get('BLDSHARED'), str) and vars.get('BLDSHARED')\nassert isinstance(vars.get('LIBPL'), str) and vars.get('LIBPL')\nassert isinstance(vars.get('INCLUDEDIR'), str) and vars.get('INCLUDEDIR')\nassert vars.get('Py_GIL_DISABLED') in (0, 1)\nassert vars.get('Py_ENABLE_SHARED') in (0, 1)",
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
        "import native_manifest_ext\nassert native_manifest_ext.EXTENSION_LOADED is True\nassert native_manifest_ext.ANSWER == 42\nassert native_manifest_ext.ENTRYPOINT == 'pyrs_extension_init_v1'\nassert native_manifest_ext.__loader__ == 'pyrs.ExtensionFileLoader'\nassert native_manifest_ext.__pyrs_extension_abi__ == 'pyrs314'\nassert native_manifest_ext.__pyrs_extension_expected_symbol__ == 'pyrs_extension_init_v1'\nassert native_manifest_ext.__pyrs_extension_symbol_family__ == 'pyrs-v1'",
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
        "import direct_native\nassert direct_native.DIRECT_LOADED is True\nassert direct_native.SOURCE == 'direct-shared-object'\nassert direct_native.__loader__ == 'pyrs.ExtensionFileLoader'\nassert direct_native.__pyrs_extension_entrypoint__ == 'dynamic:pyrs_extension_init_v1'\nassert direct_native.__pyrs_extension_expected_symbol__ == 'pyrs_extension_init_v1'\nassert direct_native.__pyrs_extension_symbol_family__ == 'pyrs-v1'",
    )
    .expect("direct shared object import should succeed");

    let _ = fs::remove_file(direct_library);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn imports_direct_cpython_style_single_phase_extension() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython-init smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython-init smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_init");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_single_phase.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"
extern int PyDict_SetItemString(PyObject *dict, const char *key, PyObject *value);

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_single_phase",
    "single phase module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_single_phase(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "ANSWER", 42) != 0) {
        return 0;
    }
    if (PyModule_AddStringConstant(module, "SOURCE", "cpython-single-phase") != 0) {
        return 0;
    }
    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename("cpython_single_phase"));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython-style extension shared object should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_single_phase\nassert cpython_single_phase.ANSWER == 42\nassert cpython_single_phase.SOURCE == 'cpython-single-phase'\nassert cpython_single_phase.__pyrs_extension_symbol_family__ == 'cpython'\nassert cpython_single_phase.__pyrs_extension_expected_symbol__ == 'PyInit_cpython_single_phase'",
    )
    .expect("cpython single-phase extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_varargs_parse_and_call_helpers_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython varargs helper smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython varargs helper smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_varargs");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_varargs_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"
extern int PyDict_SetItemString(PyObject *dict, const char *key, PyObject *value);

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_varargs_probe",
    "cpython varargs helpers probe",
    -1,
    0,
    0,
    0,
    0,
    0
};

static int unicode_to_cstr(PyObject *value, void *out) {
    const char *text = PyUnicode_AsUTF8(value);
    if (!text) {
        return 0;
    }
    *(const char **)out = text;
    return 1;
}

PyMODINIT_FUNC
PyInit_cpython_varargs_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *args = PyTuple_New(0);
    PyObject *kwargs = Py_BuildValue("{s:O}", "coerce", PyBool_FromLong(0));
    if (kwargs && PyDict_SetItemString(kwargs, "na_object", PyUnicode_FromString("NA")) != 0) {
        return 0;
    }
    if (!args || !kwargs) {
        return 0;
    }

    int coerce = 1;
    const char *na_object = "";
    const char *const keywords[] = {"coerce", "na_object", 0};
    if (!PyArg_ParseTupleAndKeywords(
            args,
            kwargs,
            "|$pO&:StringDType",
            keywords,
            &coerce,
            unicode_to_cstr,
            &na_object
        )) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "PARSED_COERCE", coerce) != 0) {
        return 0;
    }
    if (PyModule_AddStringConstant(module, "PARSED_NA_OBJECT", na_object) != 0) {
        return 0;
    }

    PyObject *mapping = Py_BuildValue("{s:O}", "x", PyLong_FromLong(7));
    PyObject *mapped = PyObject_CallMethod(mapping, "__getitem__", "s", "x");
    if (!mapped) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "CALL_METHOD_VALUE", PyLong_AsLong(mapped)) != 0) {
        return 0;
    }

    PyObject *text = PyUnicode_FromString("alpha");
    PyObject *startswith = PyObject_GetAttrString(text, "startswith");
    PyObject *prefix = PyUnicode_FromString("a");
    PyObject *starts_with = PyObject_CallFunctionObjArgs(startswith, prefix, (PyObject *)0);
    if (!starts_with) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "CALL_OBJARGS_VALUE", PyObject_IsTrue(starts_with)) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename("cpython_varargs_probe"));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython varargs probe extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_varargs_probe\nassert cpython_varargs_probe.PARSED_COERCE == 0\nassert cpython_varargs_probe.PARSED_NA_OBJECT == 'NA'\nassert cpython_varargs_probe.CALL_METHOD_VALUE == 7\nassert cpython_varargs_probe.CALL_OBJARGS_VALUE == 1",
    )
    .expect("cpython varargs helper extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_dict_capsule_and_bytearray_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api probe smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api probe smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_probe");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"
extern int PyDict_Size(PyObject *dict);
extern PyObject *PyDict_GetItemString(PyObject *dict, const char *key);
extern long long PyList_Size(PyObject *list);

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_probe",
    "cpython api probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

static void probe_capsule_destructor(PyObject *capsule) {
    (void)capsule;
}

PyMODINIT_FUNC
PyInit_cpython_api_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *dict_values = Py_BuildValue("{s:i,s:i}", "a", 1, "b", 2);
    PyObject *keys = PyDict_Keys(dict_values);
    PyObject *values = PyDict_Values(dict_values);
    PyObject *items = PyDict_Items(dict_values);
    if (!dict_values || !keys || !values || !items) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "DICT_KEYS_LEN", PyList_Size(keys)) != 0) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "DICT_VALUES_LEN", PyList_Size(values)) != 0) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "DICT_ITEMS_LEN", PyList_Size(items)) != 0) {
        return 0;
    }
    PyDict_Clear(dict_values);
    if (PyModule_AddIntConstant(module, "DICT_SIZE_AFTER_CLEAR", PyDict_Size(dict_values)) != 0) {
        return 0;
    }

    PyObject *base = Py_BuildValue("{s:i}", "x", 1);
    PyObject *other = Py_BuildValue("{s:i,s:i}", "x", 2, "y", 3);
    if (!base || !other) {
        return 0;
    }
    if (PyDict_Update(base, other) != 0) {
        return 0;
    }
    PyObject *updated_x = PyDict_GetItemString(base, "x");
    if (PyModule_AddIntConstant(module, "DICT_UPDATE_X", (int)PyLong_AsLong(updated_x)) != 0) {
        return 0;
    }
    PyObject *base2 = Py_BuildValue("{s:i}", "x", 1);
    PyObject *other2 = Py_BuildValue("{s:i,s:i}", "x", 2, "y", 3);
    if (!base2 || !other2) {
        return 0;
    }
    if (PyDict_Merge(base2, other2, 0) != 0) {
        return 0;
    }
    PyObject *merged_x = PyDict_GetItemString(base2, "x");
    PyObject *merged_y = PyDict_GetItemString(base2, "y");
    if (PyModule_AddIntConstant(module, "DICT_MERGE_X", (int)PyLong_AsLong(merged_x)) != 0) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "DICT_MERGE_Y", (int)PyLong_AsLong(merged_y)) != 0) {
        return 0;
    }

    PyObject *bytearray_value = PyByteArray_FromStringAndSize("abc", 3);
    PyObject *bytearray_bang = PyByteArray_FromStringAndSize("!", 1);
    if (!bytearray_value || !bytearray_bang) {
        return 0;
    }
    char *bytearray_data = PyByteArray_AsString(bytearray_value);
    if (!bytearray_data) {
        return 0;
    }
    bytearray_data[1] = 'Z';
    if (PyModule_AddIntConstant(module, "BYTEARRAY_LEN", (int)PyByteArray_Size(bytearray_value)) != 0) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "BYTEARRAY_MIDDLE_CHAR", (int)bytearray_data[1]) != 0) {
        return 0;
    }
    PyObject *joined = PyByteArray_Concat(bytearray_value, bytearray_bang);
    if (!joined) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "BYTEARRAY_JOINED_LEN", (int)PyByteArray_Size(joined)) != 0) {
        return 0;
    }
    Py_buffer view;
    int blocked_resize = 0;
    int resize_after_release = 0;
    if (PyObject_GetBuffer(bytearray_value, &view, 0) == 0) {
        blocked_resize = (PyByteArray_Resize(bytearray_value, 8) != 0);
        PyBuffer_Release(&view);
        resize_after_release = (PyByteArray_Resize(bytearray_value, 8) == 0);
    }
    if (PyModule_AddIntConstant(module, "BYTEARRAY_RESIZE_BLOCKED", blocked_resize) != 0) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "BYTEARRAY_RESIZE_AFTER_RELEASE", resize_after_release) != 0) {
        return 0;
    }

    int sentinel_a = 11;
    int sentinel_b = 22;
    PyObject *capsule = PyCapsule_New((void *)&sentinel_a, "probe.capsule", 0);
    if (!capsule) {
        return 0;
    }
    const char *capsule_name = PyCapsule_GetName(capsule);
    int capsule_valid = PyCapsule_IsValid(capsule, "probe.capsule");
    int capsule_name_matches = capsule_name && capsule_name[0] == 'p';
    void *capsule_ptr_before = PyCapsule_GetPointer(capsule, "probe.capsule");
    if (PyCapsule_SetPointer(capsule, (void *)&sentinel_b) != 0) {
        return 0;
    }
    void *capsule_ptr_after = PyCapsule_GetPointer(capsule, "probe.capsule");
    if (PyCapsule_SetDestructor(capsule, probe_capsule_destructor) != 0) {
        return 0;
    }
    int capsule_has_destructor = (PyCapsule_GetDestructor(capsule) != 0);
    if (PyModule_AddIntConstant(module, "CAPSULE_VALID", capsule_valid) != 0) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "CAPSULE_NAME_MATCHES", capsule_name_matches) != 0) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "CAPSULE_POINTER_CHANGED", capsule_ptr_before != capsule_ptr_after) != 0) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "CAPSULE_HAS_DESTRUCTOR", capsule_has_destructor) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename("cpython_api_probe"));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api probe extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_probe\nassert cpython_api_probe.DICT_KEYS_LEN == 2\nassert cpython_api_probe.DICT_VALUES_LEN == 2\nassert cpython_api_probe.DICT_ITEMS_LEN == 2\nassert cpython_api_probe.DICT_SIZE_AFTER_CLEAR == 0\nassert cpython_api_probe.DICT_UPDATE_X == 2\nassert cpython_api_probe.DICT_MERGE_X == 1\nassert cpython_api_probe.DICT_MERGE_Y == 3\nassert cpython_api_probe.BYTEARRAY_LEN == 3\nassert cpython_api_probe.BYTEARRAY_MIDDLE_CHAR == ord('Z')\nassert cpython_api_probe.BYTEARRAY_JOINED_LEN == 4\nassert cpython_api_probe.BYTEARRAY_RESIZE_BLOCKED == 1\nassert cpython_api_probe.BYTEARRAY_RESIZE_AFTER_RELEASE == 1\nassert cpython_api_probe.CAPSULE_VALID == 1\nassert cpython_api_probe.CAPSULE_NAME_MATCHES == 1\nassert cpython_api_probe.CAPSULE_POINTER_CHANGED == 1\nassert cpython_api_probe.CAPSULE_HAS_DESTRUCTOR == 1",
    )
    .expect("cpython api probe extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_list_set_exception_gc_and_float_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch2 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch2 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch2");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch2_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch2_probe",
    "cpython api batch2 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch2_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *list = PyList_New(3);
    if (!list) {
        return 0;
    }
    if (PyList_SetItem(list, 0, PyLong_FromLong(3)) != 0 ||
        PyList_SetItem(list, 1, PyLong_FromLong(1)) != 0 ||
        PyList_SetItem(list, 2, PyLong_FromLong(2)) != 0) {
        return 0;
    }
    if (PyList_Sort(list) != 0 || PyList_Reverse(list) != 0) {
        return 0;
    }
    if (PyList_Insert(list, 1, PyLong_FromLong(9)) != 0) {
        return 0;
    }
    if (PyList_SetItem(list, 2, PyLong_FromLong(8)) != 0) {
        return 0;
    }
    PyObject *slice = PyList_GetSlice(list, 1, 3);
    if (!slice) {
        return 0;
    }
    PyObject *replacement = PyList_New(2);
    if (!replacement) {
        return 0;
    }
    if (PyList_SetItem(replacement, 0, PyLong_FromLong(7)) != 0 ||
        PyList_SetItem(replacement, 1, PyLong_FromLong(6)) != 0) {
        return 0;
    }
    if (PyList_SetSlice(list, 1, 3, replacement) != 0) {
        return 0;
    }
    int list_first = (int)PyLong_AsLong(PyList_GetItem(list, 0));
    int list_second = (int)PyLong_AsLong(PyList_GetItem(list, 1));
    int list_len = (int)PyList_Size(list);
    int slice_len = (int)PyList_Size(slice);
    int list_neg_index_fail = 0;
    if (!PyList_GetItem(list, -1) && PyErr_Occurred()) {
        list_neg_index_fail = 1;
        PyErr_Clear();
    }
    if (PyModule_AddIntConstant(module, "LIST_FIRST", list_first) != 0 ||
        PyModule_AddIntConstant(module, "LIST_SECOND", list_second) != 0 ||
        PyModule_AddIntConstant(module, "LIST_LEN", list_len) != 0 ||
        PyModule_AddIntConstant(module, "LIST_SLICE_LEN", slice_len) != 0 ||
        PyModule_AddIntConstant(module, "LIST_NEG_INDEX_FAIL", list_neg_index_fail) != 0) {
        return 0;
    }

    PyObject *set = PySet_New(0);
    if (!set) {
        return 0;
    }
    if (PySet_Add(set, PyLong_FromLong(1)) != 0 ||
        PySet_Add(set, PyLong_FromLong(2)) != 0 ||
        PySet_Add(set, PyLong_FromLong(2)) != 0) {
        return 0;
    }
    int set_size_before_pop = (int)PySet_Size(set);
    int set_contains_2 = PySet_Contains(set, PyLong_FromLong(2));
    int set_contains_5 = PySet_Contains(set, PyLong_FromLong(5));
    if (PySet_Discard(set, PyLong_FromLong(5)) != 0) {
        return 0;
    }
    PyObject *popped = PySet_Pop(set);
    if (!popped) {
        return 0;
    }
    long long popped_value = PyLong_AsLong(popped);
    int set_pop_member = (popped_value == 1 || popped_value == 2) ? 1 : 0;
    if (PySet_Clear(set) != 0) {
        return 0;
    }
    int set_size_after_clear = (int)PySet_Size(set);
    PyObject *frozen_src = PyList_New(2);
    if (!frozen_src) {
        return 0;
    }
    if (PyList_SetItem(frozen_src, 0, PyLong_FromLong(4)) != 0 ||
        PyList_SetItem(frozen_src, 1, PyLong_FromLong(5)) != 0) {
        return 0;
    }
    PyObject *frozen = PyFrozenSet_New(frozen_src);
    if (!frozen) {
        return 0;
    }
    int frozenset_size = (int)PySet_Size(frozen);
    if (PyModule_AddIntConstant(module, "SET_SIZE_BEFORE_POP", set_size_before_pop) != 0 ||
        PyModule_AddIntConstant(module, "SET_CONTAINS_2", set_contains_2) != 0 ||
        PyModule_AddIntConstant(module, "SET_CONTAINS_5", set_contains_5) != 0 ||
        PyModule_AddIntConstant(module, "SET_POP_MEMBER", set_pop_member) != 0 ||
        PyModule_AddIntConstant(module, "SET_SIZE_AFTER_CLEAR", set_size_after_clear) != 0 ||
        PyModule_AddIntConstant(module, "FROZENSET_SIZE", frozenset_size) != 0) {
        return 0;
    }

    PyObject *exc = PyObject_CallFunction((PyObject *)PyExc_RuntimeError, "s", "boom");
    if (!exc) {
        return 0;
    }
    PyObject *args_before = PyException_GetArgs(exc);
    if (!args_before) {
        return 0;
    }
    int exc_args_before_len = (int)PyTuple_Size(args_before);
    PyObject *new_args = PyTuple_Pack(2, PyUnicode_FromString("x"), PyUnicode_FromString("y"));
    if (!new_args) {
        return 0;
    }
    PyException_SetArgs(exc, new_args);
    PyObject *args_after = PyException_GetArgs(exc);
    if (!args_after) {
        return 0;
    }
    int exc_args_after_len = (int)PyTuple_Size(args_after);
    PyObject *cause = PyObject_CallFunction((PyObject *)PyExc_ValueError, "s", "cause");
    PyObject *context = PyObject_CallFunction((PyObject *)PyExc_TypeError, "s", "ctx");
    if (!cause || !context) {
        return 0;
    }
    PyException_SetCause(exc, cause);
    PyException_SetContext(exc, context);
    int exc_has_cause = PyException_GetCause(exc) ? 1 : 0;
    int exc_has_context = PyException_GetContext(exc) ? 1 : 0;
    PyException_SetTraceback(exc, 0);
    int exc_traceback_cleared = PyException_GetTraceback(exc) ? 0 : 1;
    if (PyModule_AddIntConstant(module, "EXC_ARGS_BEFORE_LEN", exc_args_before_len) != 0 ||
        PyModule_AddIntConstant(module, "EXC_ARGS_AFTER_LEN", exc_args_after_len) != 0 ||
        PyModule_AddIntConstant(module, "EXC_HAS_CAUSE", exc_has_cause) != 0 ||
        PyModule_AddIntConstant(module, "EXC_HAS_CONTEXT", exc_has_context) != 0 ||
        PyModule_AddIntConstant(module, "EXC_TRACEBACK_CLEARED", exc_traceback_cleared) != 0) {
        return 0;
    }

    int gc_prev_disable = PyGC_Disable();
    int gc_after_disable = PyGC_IsEnabled();
    int gc_prev_enable = PyGC_Enable();
    int gc_after_enable = PyGC_IsEnabled();
    long long gc_collected = (long long)PyGC_Collect();
    if (PyModule_AddIntConstant(module, "GC_PREV_DISABLE", gc_prev_disable) != 0 ||
        PyModule_AddIntConstant(module, "GC_AFTER_DISABLE", gc_after_disable) != 0 ||
        PyModule_AddIntConstant(module, "GC_PREV_ENABLE", gc_prev_enable) != 0 ||
        PyModule_AddIntConstant(module, "GC_AFTER_ENABLE", gc_after_enable) != 0 ||
        PyModule_AddIntConstant(module, "GC_COLLECT_NONNEG", gc_collected >= 0 ? 1 : 0) != 0) {
        return 0;
    }

    double fmax = PyFloat_GetMax();
    double fmin = PyFloat_GetMin();
    PyObject *finfo = PyFloat_GetInfo();
    if (!finfo) {
        return 0;
    }
    PyObject *finfo_max = PyObject_GetAttrString(finfo, "max");
    if (!finfo_max) {
        return 0;
    }
    double info_max = PyFloat_AsDouble(finfo_max);
    int float_info_ok = (fmax > fmin && fmin > 0.0 && info_max == fmax) ? 1 : 0;
    if (PyModule_AddIntConstant(module, "FLOAT_INFO_OK", float_info_ok) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch2_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch2 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch2_probe as m\nassert m.LIST_FIRST == 3\nassert m.LIST_SECOND == 7\nassert m.LIST_LEN == 4\nassert m.LIST_SLICE_LEN == 2\nassert m.LIST_NEG_INDEX_FAIL == 1\nassert m.SET_SIZE_BEFORE_POP == 2\nassert m.SET_CONTAINS_2 == 1\nassert m.SET_CONTAINS_5 == 0\nassert m.SET_POP_MEMBER == 1\nassert m.SET_SIZE_AFTER_CLEAR == 0\nassert m.FROZENSET_SIZE == 2\nassert m.EXC_ARGS_BEFORE_LEN == 1\nassert m.EXC_ARGS_AFTER_LEN == 2\nassert m.EXC_HAS_CAUSE == 1\nassert m.EXC_HAS_CONTEXT == 1\nassert m.EXC_TRACEBACK_CLEARED == 1\nassert m.GC_AFTER_DISABLE == 0\nassert m.GC_AFTER_ENABLE == 1\nassert m.GC_COLLECT_NONNEG == 1\nassert m.FLOAT_INFO_OK == 1",
    )
    .expect("cpython api batch2 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_bytes_error_and_cfunction_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch3 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch3 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch3");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch3_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static PyObject *probe_echo(PyObject *self, PyObject *args) {
    (void)self;
    Py_INCREF(args);
    return args;
}

static PyObject *probe_noargs(PyObject *self, PyObject *unused) {
    (void)self;
    (void)unused;
    return PyLong_FromLong(42);
}

static PyObject *probe_method(PyObject *self, PyObject *cls, PyObject *const *args, unsigned long nargs, PyObject *kwnames) {
    (void)self;
    (void)cls;
    (void)args;
    (void)kwnames;
    return PyLong_FromLong((long long)nargs);
}

static PyMethodDef noargs_def = {"generated_noargs", probe_noargs, METH_NOARGS, "generated noargs"};
static PyMethodDef varargs_def = {"generated_varargs", probe_echo, METH_VARARGS, "generated varargs"};
static PyMethodDef method_def = {"generated_method", (PyObject *(*)(PyObject *, PyObject *))probe_method, METH_METHOD | METH_FASTCALL | METH_KEYWORDS, "generated method"};

static PyMethodDef module_methods[] = {
    {"echo", probe_echo, METH_VARARGS, "echo tuple args"},
    {0, 0, 0, 0}
};

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch3_probe",
    "cpython api batch3 probe module",
    -1,
    module_methods,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch3_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *bytearray_obj = PyByteArray_FromStringAndSize("xy", 2);
    PyObject *from_obj = PyBytes_FromObject(bytearray_obj);
    if (!from_obj) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "FROM_OBJ_LEN", (int)PyBytes_Size(from_obj)) != 0) {
        return 0;
    }

    PyObject *list_obj = PyList_New(3);
    if (!list_obj) {
        return 0;
    }
    if (PyList_SetItem(list_obj, 0, PyLong_FromLong(65)) != 0 ||
        PyList_SetItem(list_obj, 1, PyLong_FromLong(66)) != 0 ||
        PyList_SetItem(list_obj, 2, PyLong_FromLong(67)) != 0) {
        return 0;
    }
    PyObject *from_list = PyBytes_FromObject(list_obj);
    if (!from_list) {
        return 0;
    }
    char *from_list_data = PyBytes_AsString(from_list);
    if (!from_list_data) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "FROM_LIST_FIRST", (int)(unsigned char)from_list_data[0]) != 0) {
        return 0;
    }

    int int_fail = 0;
    if (!PyBytes_FromObject(PyLong_FromLong(3)) && PyErr_Occurred()) {
        int_fail = 1;
        PyErr_Clear();
    }
    if (PyModule_AddIntConstant(module, "INT_FAIL", int_fail) != 0) {
        return 0;
    }

    PyObject *left = PyBytes_FromStringAndSize("ab", 2);
    PyObject *right = PyBytes_FromStringAndSize("cd", 2);
    if (!left || !right) {
        return 0;
    }
    PyBytes_Concat(&left, right);
    if (!left) {
        return 0;
    }
    int concat_len = (int)PyBytes_Size(left);
    PyObject *right2 = PyBytes_FromStringAndSize("ef", 2);
    if (!right2) {
        return 0;
    }
    PyBytes_ConcatAndDel(&left, right2);
    if (!left) {
        return 0;
    }
    int concat_and_del_len = (int)PyBytes_Size(left);
    PyObject *clear_target = PyBytes_FromStringAndSize("zz", 2);
    PyBytes_Concat(&clear_target, 0);
    int concat_null_clears = clear_target == 0 ? 1 : 0;
    if (PyModule_AddIntConstant(module, "CONCAT_LEN", concat_len) != 0 ||
        PyModule_AddIntConstant(module, "CONCAT_AND_DEL_LEN", concat_and_del_len) != 0 ||
        PyModule_AddIntConstant(module, "CONCAT_NULL_CLEARS", concat_null_clears) != 0) {
        return 0;
    }

    int bad_argument_result = PyErr_BadArgument();
    int bad_argument_set = PyErr_Occurred() ? 1 : 0;
    PyErr_Clear();
    PyErr_BadInternalCall();
    int bad_internal_set = PyErr_Occurred() ? 1 : 0;
    PyErr_Clear();
    PyErr_PrintEx(0);
    int print_noop_ok = PyErr_Occurred() == 0 ? 1 : 0;
    PyErr_Display(0, 0, 0);
    int display_noop_ok = PyErr_Occurred() == 0 ? 1 : 0;
    PyErr_DisplayException(0);
    int display_exception_noop_ok = PyErr_Occurred() == 0 ? 1 : 0;
    if (PyModule_AddIntConstant(module, "BAD_ARGUMENT_RESULT", bad_argument_result) != 0 ||
        PyModule_AddIntConstant(module, "BAD_ARGUMENT_SET", bad_argument_set) != 0 ||
        PyModule_AddIntConstant(module, "BAD_INTERNAL_SET", bad_internal_set) != 0 ||
        PyModule_AddIntConstant(module, "PRINT_NOOP_OK", print_noop_ok) != 0 ||
        PyModule_AddIntConstant(module, "DISPLAY_NOOP_OK", display_noop_ok) != 0 ||
        PyModule_AddIntConstant(module, "DISPLAY_EXCEPTION_NOOP_OK", display_exception_noop_ok) != 0) {
        return 0;
    }

    PyObject *module_name = PyUnicode_FromString("cpython_api_batch3_probe");
    if (!module_name) {
        return 0;
    }
    PyObject *generated_noargs = PyCFunction_New(&noargs_def, module);
    if (!generated_noargs) {
        return 0;
    }
    PyObject *noargs_result = PyCFunction_Call(generated_noargs, PyTuple_New(0), 0);
    int noargs_ok = noargs_result && PyLong_AsLong(noargs_result) == 42 ? 1 : 0;

    PyObject *generated_varargs = PyCFunction_NewEx(&varargs_def, module, module_name);
    if (!generated_varargs) {
        return 0;
    }
    PyObject *generated_varargs_mod = PyObject_GetAttrString(generated_varargs, "__module__");
    int generated_varargs_mod_ok = generated_varargs_mod && PyUnicode_AsUTF8(generated_varargs_mod) &&
        PyUnicode_AsUTF8(generated_varargs_mod)[0] == 'c' ? 1 : 0;
    PyObject *generated_varargs_call = PyCFunction_Call(
        generated_varargs,
        PyTuple_Pack(1, PyLong_FromLong(7)),
        0
    );
    int generated_varargs_len = generated_varargs_call ? (int)PyTuple_Size(generated_varargs_call) : -1;
    int generated_varargs_flags = PyCFunction_GetFlags(generated_varargs);
    PyObject *generated_varargs_self = PyCFunction_GetSelf(generated_varargs);
    void *generated_varargs_fn = (void *)PyCFunction_GetFunction(generated_varargs);

    PyObject *generated_method = PyCMethod_New(&method_def, module, module_name, module);
    if (!generated_method) {
        return 0;
    }
    PyObject *generated_method_call = PyCFunction_Call(
        generated_method,
        PyTuple_Pack(3, PyLong_FromLong(1), PyLong_FromLong(2), PyLong_FromLong(3)),
        0
    );
    int generated_method_nargs = generated_method_call ? (int)PyLong_AsLong(generated_method_call) : -1;
    int generated_method_flags = PyCFunction_GetFlags(generated_method);

    PyObject *bad_missing_cls = PyCMethod_New(&method_def, module, module_name, 0);
    int bad_missing_cls_error = (!bad_missing_cls && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();
    PyObject *bad_unexpected_cls = PyCMethod_New(&noargs_def, module, module_name, module);
    int bad_unexpected_cls_error = (!bad_unexpected_cls && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();

    int cfunc_flags = PyCFunction_GetFlags(0);
    int cfunc_flags_fail = (cfunc_flags == -1 && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();
    PyObject *cfunc_self = PyCFunction_GetSelf(0);
    int cfunc_self_fail = (!cfunc_self && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();
    void *cfunc_fn = (void *)PyCFunction_GetFunction(0);
    int cfunc_fn_fail = (!cfunc_fn && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();
    PyObject *cfunc_call = PyCFunction_Call(0, 0, 0);
    int cfunc_call_fail = (!cfunc_call && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();
    if (PyModule_AddIntConstant(module, "CFUNC_NOARGS_OK", noargs_ok) != 0 ||
        PyModule_AddIntConstant(module, "CFUNC_VARARGS_MODULE_OK", generated_varargs_mod_ok) != 0 ||
        PyModule_AddIntConstant(module, "CFUNC_VARARGS_LEN", generated_varargs_len) != 0 ||
        PyModule_AddIntConstant(module, "CFUNC_VARARGS_FLAGS", generated_varargs_flags) != 0 ||
        PyModule_AddIntConstant(module, "CFUNC_VARARGS_SELF_NON_NULL", generated_varargs_self ? 1 : 0) != 0 ||
        PyModule_AddIntConstant(module, "CFUNC_VARARGS_FN_NON_NULL", generated_varargs_fn ? 1 : 0) != 0 ||
        PyModule_AddIntConstant(module, "CFUNC_METHOD_NARGS", generated_method_nargs) != 0 ||
        PyModule_AddIntConstant(module, "CFUNC_METHOD_FLAGS", generated_method_flags) != 0 ||
        PyModule_AddIntConstant(module, "CFUNC_METHOD_MISSING_CLS_ERROR", bad_missing_cls_error) != 0 ||
        PyModule_AddIntConstant(module, "CFUNC_METHOD_UNEXPECTED_CLS_ERROR", bad_unexpected_cls_error) != 0 ||
        PyModule_AddIntConstant(module, "CFUNC_FLAGS_FAIL", cfunc_flags_fail) != 0 ||
        PyModule_AddIntConstant(module, "CFUNC_SELF_FAIL", cfunc_self_fail) != 0 ||
        PyModule_AddIntConstant(module, "CFUNC_FN_FAIL", cfunc_fn_fail) != 0 ||
        PyModule_AddIntConstant(module, "CFUNC_CALL_FAIL", cfunc_call_fail) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch3_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch3 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch3_probe as m\nassert m.FROM_OBJ_LEN == 2\nassert m.FROM_LIST_FIRST == 65\nassert m.INT_FAIL == 1\nassert m.CONCAT_LEN == 4\nassert m.CONCAT_AND_DEL_LEN == 6\nassert m.CONCAT_NULL_CLEARS == 1\nassert m.BAD_ARGUMENT_RESULT == 0\nassert m.BAD_ARGUMENT_SET == 1\nassert m.BAD_INTERNAL_SET == 1\nassert m.PRINT_NOOP_OK == 1\nassert m.DISPLAY_NOOP_OK == 1\nassert m.DISPLAY_EXCEPTION_NOOP_OK == 1\nassert m.CFUNC_NOARGS_OK == 1\nassert m.CFUNC_VARARGS_MODULE_OK == 1\nassert m.CFUNC_VARARGS_LEN == 1\nassert m.CFUNC_VARARGS_FLAGS == 1\nassert m.CFUNC_VARARGS_SELF_NON_NULL == 1\nassert m.CFUNC_VARARGS_FN_NON_NULL == 1\nassert m.CFUNC_METHOD_NARGS == 3\nassert m.CFUNC_METHOD_FLAGS == 642\nassert m.CFUNC_METHOD_MISSING_CLS_ERROR == 1\nassert m.CFUNC_METHOD_UNEXPECTED_CLS_ERROR == 1\nassert m.CFUNC_FLAGS_FAIL == 1\nassert m.CFUNC_SELF_FAIL == 1\nassert m.CFUNC_FN_FAIL == 1\nassert m.CFUNC_CALL_FAIL == 1",
    )
    .expect("cpython api batch3 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_import_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch4 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch4 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch4");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch4_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"
#include <string.h>

static int module_name_is(PyObject *module, const char *expected) {
    if (!module) {
        return 0;
    }
    PyObject *name = PyObject_GetAttrString(module, "__name__");
    if (!name) {
        return 0;
    }
    const char *text = PyUnicode_AsUTF8(name);
    if (!text) {
        return 0;
    }
    return strcmp(text, expected) == 0 ? 1 : 0;
}

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch4_probe",
    "cpython api batch4 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch4_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *modules_dict = PyImport_GetModuleDict();
    int modules_dict_ok = modules_dict ? 1 : 0;
    if (PyModule_AddIntConstant(module, "MODULES_DICT_OK", modules_dict_ok) != 0) {
        return 0;
    }

    PyObject *added_ref = PyImport_AddModuleRef("batch4_add_ref");
    int add_ref_ok = module_name_is(added_ref, "batch4_add_ref");
    if (PyModule_AddIntConstant(module, "ADD_REF_OK", add_ref_ok) != 0) {
        return 0;
    }

    PyObject *added_obj_name = PyUnicode_FromString("batch4_add_obj");
    if (!added_obj_name) {
        return 0;
    }
    PyObject *added_obj = PyImport_AddModuleObject(added_obj_name);
    int add_obj_ok = module_name_is(added_obj, "batch4_add_obj");
    if (PyModule_AddIntConstant(module, "ADD_OBJ_OK", add_obj_ok) != 0) {
        return 0;
    }

    PyObject *added_legacy = PyImport_AddModule("batch4_add_legacy");
    int add_legacy_ok = module_name_is(added_legacy, "batch4_add_legacy");
    if (PyModule_AddIntConstant(module, "ADD_LEGACY_OK", add_legacy_ok) != 0) {
        return 0;
    }

    PyObject *got_obj = PyImport_GetModule(added_obj_name);
    int get_module_ok = module_name_is(got_obj, "batch4_add_obj");
    if (PyModule_AddIntConstant(module, "GET_MODULE_OK", get_module_ok) != 0) {
        return 0;
    }

    PyObject *missing_name = PyUnicode_FromString("batch4_missing_mod");
    if (!missing_name) {
        return 0;
    }
    PyObject *missing = PyImport_GetModule(missing_name);
    int missing_returns_null = (missing == 0) ? 1 : 0;
    int missing_sets_no_error = PyErr_Occurred() ? 0 : 1;
    if (PyModule_AddIntConstant(module, "MISSING_RETURNS_NULL", missing_returns_null) != 0 ||
        PyModule_AddIntConstant(module, "MISSING_NO_ERROR", missing_sets_no_error) != 0) {
        return 0;
    }

    PyObject *math_module = PyImport_ImportModuleNoBlock("math");
    int import_noblock_ok = module_name_is(math_module, "math");
    if (PyModule_AddIntConstant(module, "IMPORT_NOBLOCK_OK", import_noblock_ok) != 0) {
        return 0;
    }

    PyObject *json_module = PyImport_ImportModuleLevel("json", 0, 0, 0, 0);
    int import_level_ok = module_name_is(json_module, "json");
    if (PyModule_AddIntConstant(module, "IMPORT_LEVEL_OK", import_level_ok) != 0) {
        return 0;
    }

    PyObject *email_name = PyUnicode_FromString("email");
    if (!email_name) {
        return 0;
    }
    PyObject *fromlist = PyTuple_Pack(1, PyUnicode_FromString("message"));
    if (!fromlist) {
        return 0;
    }
    PyObject *email_module = PyImport_ImportModuleLevelObject(email_name, 0, 0, fromlist, 0);
    int import_level_obj_ok = module_name_is(email_module, "email");
    if (PyModule_AddIntConstant(module, "IMPORT_LEVEL_OBJECT_OK", import_level_obj_ok) != 0) {
        return 0;
    }

    PyObject *reloaded_math = PyImport_ReloadModule(math_module);
    int reload_ok = module_name_is(reloaded_math, "math");
    if (PyModule_AddIntConstant(module, "RELOAD_OK", reload_ok) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch4_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch4 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch4_probe as m\nassert m.MODULES_DICT_OK == 1\nassert m.ADD_REF_OK == 1\nassert m.ADD_OBJ_OK == 1\nassert m.ADD_LEGACY_OK == 1\nassert m.GET_MODULE_OK == 1\nassert m.MISSING_RETURNS_NULL == 1\nassert m.MISSING_NO_ERROR == 1\nassert m.IMPORT_NOBLOCK_OK == 1\nassert m.IMPORT_LEVEL_OK == 1\nassert m.IMPORT_LEVEL_OBJECT_OK == 1\nassert m.RELOAD_OK == 1",
    )
    .expect("cpython api batch4 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_error_state_and_file_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch5 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch5 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch5");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch5_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch5_probe",
    "cpython api batch5 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch5_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyErr_SetString(PyExc_ValueError, "boom");
    PyObject *raised = PyErr_GetRaisedException();
    int raised_non_null = raised ? 1 : 0;
    int raised_cleared = PyErr_Occurred() ? 0 : 1;
    PyErr_SetRaisedException(raised);
    int raised_restored = PyErr_Occurred() ? 1 : 0;
    PyObject *raised_again = PyErr_GetRaisedException();
    int raised_again_non_null = raised_again ? 1 : 0;
    PyErr_SetRaisedException(0);
    int raised_cleared_again = PyErr_Occurred() ? 0 : 1;

    PyObject *handled = PyObject_CallFunction((PyObject *)PyExc_RuntimeError, "s", "handled");
    if (!handled) {
        return 0;
    }
    PyErr_SetHandledException(handled);
    PyObject *handled_readback = PyErr_GetHandledException();
    int handled_roundtrip = handled_readback ? 1 : 0;
    PyObject *etype = 0;
    PyObject *evalue = 0;
    PyObject *etb = 0;
    PyErr_GetExcInfo(&etype, &evalue, &etb);
    int excinfo_type = etype ? 1 : 0;
    int excinfo_value = evalue ? 1 : 0;
    int excinfo_tb = etb ? 1 : 0;
    PyErr_SetExcInfo(etype, evalue, etb);
    PyObject *handled_after_setexc = PyErr_GetHandledException();
    int setexc_roundtrip = handled_after_setexc ? 1 : 0;
    PyErr_SetHandledException(0);
    PyObject *handled_after_clear = PyErr_GetHandledException();
    int handled_cleared = handled_after_clear ? 0 : 1;

    PyObject *line_null = PyFile_GetLine(0, 0);
    int getline_null_error = (!line_null && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();
    PyObject *text = PyUnicode_FromString("hello");
    if (!text) {
        return 0;
    }
    int writeobj_null_status = PyFile_WriteObject(text, 0, 0);
    int writeobj_null_error = (writeobj_null_status == -1 && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();
    int writestr_null_status = PyFile_WriteString("hello", 0);
    int writestr_null_error = (writestr_null_status == -1 && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();

    PyObject *sys_module = PyImport_ImportModule("sys");
    if (!sys_module) {
        return 0;
    }
    PyObject *stderr_obj = PyObject_GetAttrString(sys_module, "stderr");
    if (!stderr_obj) {
        return 0;
    }
    int writeobj_sys_ok = (PyFile_WriteObject(text, stderr_obj, Py_PRINT_RAW) == 0) ? 1 : 0;
    int writestr_sys_ok = (PyFile_WriteString("!", stderr_obj) == 0) ? 1 : 0;

    if (PyModule_AddIntConstant(module, "RAISED_NON_NULL", raised_non_null) != 0 ||
        PyModule_AddIntConstant(module, "RAISED_CLEARED", raised_cleared) != 0 ||
        PyModule_AddIntConstant(module, "RAISED_RESTORED", raised_restored) != 0 ||
        PyModule_AddIntConstant(module, "RAISED_AGAIN_NON_NULL", raised_again_non_null) != 0 ||
        PyModule_AddIntConstant(module, "RAISED_CLEARED_AGAIN", raised_cleared_again) != 0 ||
        PyModule_AddIntConstant(module, "HANDLED_ROUNDTRIP", handled_roundtrip) != 0 ||
        PyModule_AddIntConstant(module, "EXCINFO_TYPE", excinfo_type) != 0 ||
        PyModule_AddIntConstant(module, "EXCINFO_VALUE", excinfo_value) != 0 ||
        PyModule_AddIntConstant(module, "EXCINFO_TB", excinfo_tb) != 0 ||
        PyModule_AddIntConstant(module, "SETEXC_ROUNDTRIP", setexc_roundtrip) != 0 ||
        PyModule_AddIntConstant(module, "HANDLED_CLEARED", handled_cleared) != 0 ||
        PyModule_AddIntConstant(module, "GETLINE_NULL_ERROR", getline_null_error) != 0 ||
        PyModule_AddIntConstant(module, "WRITEOBJECT_NULL_ERROR", writeobj_null_error) != 0 ||
        PyModule_AddIntConstant(module, "WRITESTRING_NULL_ERROR", writestr_null_error) != 0 ||
        PyModule_AddIntConstant(module, "WRITEOBJECT_SYS_OK", writeobj_sys_ok) != 0 ||
        PyModule_AddIntConstant(module, "WRITESTRING_SYS_OK", writestr_sys_ok) != 0) {
        return 0;
    }
    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch5_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch5 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch5_probe as m\nassert m.RAISED_NON_NULL == 1\nassert m.RAISED_CLEARED == 1\nassert m.RAISED_RESTORED == 1\nassert m.RAISED_AGAIN_NON_NULL == 1\nassert m.RAISED_CLEARED_AGAIN == 1\nassert m.HANDLED_ROUNDTRIP == 1\nassert m.EXCINFO_TYPE == 1\nassert m.EXCINFO_VALUE == 1\nassert m.EXCINFO_TB == 1\nassert m.SETEXC_ROUNDTRIP == 1\nassert m.HANDLED_CLEARED == 1\nassert m.GETLINE_NULL_ERROR == 1\nassert m.WRITEOBJECT_NULL_ERROR == 1\nassert m.WRITESTRING_NULL_ERROR == 1\nassert m.WRITEOBJECT_SYS_OK == 1\nassert m.WRITESTRING_SYS_OK == 1",
    )
    .expect("cpython api batch5 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_long_abi_batch6_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch6 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch6 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch6");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch6_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"
#include <stdint.h>

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch6_probe",
    "cpython api batch6 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch6_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *signed32 = PyLong_FromInt32(-1234);
    PyObject *unsigned32 = PyLong_FromUInt32(123U);
    PyObject *signed64 = PyLong_FromInt64(-9876543210LL);
    PyObject *unsigned64 = PyLong_FromUInt64(UINT64_MAX);
    PyObject *from_size = PyLong_FromSize_t((size_t)7);
    if (!signed32 || !unsigned32 || !signed64 || !unsigned64 || !from_size) {
        return 0;
    }

    int as_int_ok = (PyLong_AsInt(signed32) == -1234) ? 1 : 0;
    int32_t out_i32 = 0;
    int as_int32_ok = (PyLong_AsInt32(signed32, &out_i32) == 0 && out_i32 == -1234) ? 1 : 0;

    int64_t out_i64 = 0;
    int as_int64_ok = (PyLong_AsInt64(signed64, &out_i64) == 0 && out_i64 == -9876543210LL) ? 1 : 0;

    uint32_t out_u32 = 0;
    int as_uint32_ok = (PyLong_AsUInt32(unsigned32, &out_u32) == 0 && out_u32 == 123U) ? 1 : 0;

    uint64_t out_u64 = 0;
    int as_uint64_ok = (PyLong_AsUInt64(unsigned64, &out_u64) == 0 && out_u64 == UINT64_MAX) ? 1 : 0;

    size_t out_size = PyLong_AsSize_t(from_size);
    int as_size_ok = (!PyErr_Occurred() && out_size == (size_t)7) ? 1 : 0;

    double out_double = PyLong_AsDouble(signed32);
    int as_double_ok = (!PyErr_Occurred() && out_double == -1234.0) ? 1 : 0;

    int64_t overflow_i64 = 0;
    int overflow_i64_error = (PyLong_AsInt64(unsigned64, &overflow_i64) == -1 && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();

    int overflow_int_error = (PyLong_AsInt(unsigned64) == -1 && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();

    uint64_t negative_u64 = 0;
    int negative_uint_error = (PyLong_AsUInt64(signed32, &negative_u64) == -1 && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();

    if (PyModule_AddIntConstant(module, "AS_INT_OK", as_int_ok) != 0 ||
        PyModule_AddIntConstant(module, "AS_INT32_OK", as_int32_ok) != 0 ||
        PyModule_AddIntConstant(module, "AS_INT64_OK", as_int64_ok) != 0 ||
        PyModule_AddIntConstant(module, "AS_UINT32_OK", as_uint32_ok) != 0 ||
        PyModule_AddIntConstant(module, "AS_UINT64_OK", as_uint64_ok) != 0 ||
        PyModule_AddIntConstant(module, "AS_SIZE_OK", as_size_ok) != 0 ||
        PyModule_AddIntConstant(module, "AS_DOUBLE_OK", as_double_ok) != 0 ||
        PyModule_AddIntConstant(module, "OVERFLOW_INT64_ERROR", overflow_i64_error) != 0 ||
        PyModule_AddIntConstant(module, "OVERFLOW_INT_ERROR", overflow_int_error) != 0 ||
        PyModule_AddIntConstant(module, "NEGATIVE_UINT_ERROR", negative_uint_error) != 0) {
        return 0;
    }
    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch6_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch6 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch6_probe as m\nassert m.AS_INT_OK == 1\nassert m.AS_INT32_OK == 1\nassert m.AS_INT64_OK == 1\nassert m.AS_UINT32_OK == 1\nassert m.AS_UINT64_OK == 1\nassert m.AS_SIZE_OK == 1\nassert m.AS_DOUBLE_OK == 1\nassert m.OVERFLOW_INT64_ERROR == 1\nassert m.OVERFLOW_INT_ERROR == 1\nassert m.NEGATIVE_UINT_ERROR == 1",
    )
    .expect("cpython api batch6 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_long_abi_batch7_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch7 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch7 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch7");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch7_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"
#include <stdint.h>

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch7_probe",
    "cpython api batch7 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch7_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *neg_one = PyLong_FromInt64(-1);
    PyObject *five = PyLong_FromInt64(5);
    PyObject *huge = PyLong_FromString("18446744073709551617", 0, 10);
    if (!neg_one || !five || !huge) {
        return 0;
    }

    int mask_neg_ok = (PyLong_AsUnsignedLongMask(neg_one) == UINT64_MAX) ? 1 : 0;
    int mask_pos_ok = (PyLong_AsUnsignedLongLongMask(five) == 5ULL) ? 1 : 0;
    int mask_huge_ok = (PyLong_AsUnsignedLongLongMask(huge) == 1ULL) ? 1 : 0;

    char *end_ok = 0;
    PyObject *parsed_hex = PyLong_FromString("0x10", &end_ok, 0);
    int parsed_hex_ok = (parsed_hex && PyLong_AsInt(parsed_hex) == 16 && end_ok && *end_ok == '\0') ? 1 : 0;

    char *end_bad = 0;
    PyObject *parsed_bad = PyLong_FromString("zz", &end_bad, 10);
    int parsed_bad_error = (!parsed_bad && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();

    PyObject *info = PyLong_GetInfo();
    int info_ok = info ? 1 : 0;
    int info_bits_ok = 0;
    if (info) {
        PyObject *bits = PyObject_GetAttrString(info, "bits_per_digit");
        if (bits) {
            int bits_value = PyLong_AsInt(bits);
            info_bits_ok = (!PyErr_Occurred() && bits_value > 0) ? 1 : 0;
            PyErr_Clear();
        }
    }

    if (PyModule_AddIntConstant(module, "MASK_NEG_OK", mask_neg_ok) != 0 ||
        PyModule_AddIntConstant(module, "MASK_POS_OK", mask_pos_ok) != 0 ||
        PyModule_AddIntConstant(module, "MASK_HUGE_OK", mask_huge_ok) != 0 ||
        PyModule_AddIntConstant(module, "PARSED_HEX_OK", parsed_hex_ok) != 0 ||
        PyModule_AddIntConstant(module, "PARSED_BAD_ERROR", parsed_bad_error) != 0 ||
        PyModule_AddIntConstant(module, "INFO_OK", info_ok) != 0 ||
        PyModule_AddIntConstant(module, "INFO_BITS_OK", info_bits_ok) != 0) {
        return 0;
    }
    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch7_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch7 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch7_probe as m\nassert m.MASK_NEG_OK == 1\nassert m.MASK_POS_OK == 1\nassert m.MASK_HUGE_OK == 1\nassert m.PARSED_HEX_OK == 1\nassert m.PARSED_BAD_ERROR == 1\nassert m.INFO_OK == 1\nassert m.INFO_BITS_OK == 1",
    )
    .expect("cpython api batch7 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_long_abi_batch8_native_bytes_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch8 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch8 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch8");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch8_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch8_probe",
    "cpython api batch8 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch8_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *neg_two = PyLong_FromLongLong(-2);
    PyObject *pos_ff = PyLong_FromLongLong(255);
    if (!neg_two || !pos_ff) {
        return 0;
    }

    unsigned char neg_buf[2] = {0, 0};
    long long neg_req = PyLong_AsNativeBytes(
        neg_two, neg_buf, 2, Py_ASNATIVEBYTES_LITTLE_ENDIAN
    );
    int neg_bytes_ok = (neg_req == 1 && neg_buf[0] == 0xFE && neg_buf[1] == 0xFF) ? 1 : 0;

    unsigned char pos_signed_buf[1] = {0};
    long long pos_signed_req = PyLong_AsNativeBytes(
        pos_ff, pos_signed_buf, 1, Py_ASNATIVEBYTES_BIG_ENDIAN
    );
    int pos_signed_req_ok = (pos_signed_req == 2) ? 1 : 0;

    unsigned char pos_unsigned_buf[1] = {0};
    long long pos_unsigned_req = PyLong_AsNativeBytes(
        pos_ff,
        pos_unsigned_buf,
        1,
        Py_ASNATIVEBYTES_BIG_ENDIAN | Py_ASNATIVEBYTES_UNSIGNED_BUFFER
    );
    int pos_unsigned_ok = (pos_unsigned_req == 1 && pos_unsigned_buf[0] == 0xFF) ? 1 : 0;

    long long query_bytes = PyLong_AsNativeBytes(pos_ff, 0, 0, Py_ASNATIVEBYTES_DEFAULTS);
    int query_ok = (query_bytes == 1) ? 1 : 0;

    unsigned char reject_buf[1] = {0};
    long long reject_status = PyLong_AsNativeBytes(
        neg_two,
        reject_buf,
        1,
        Py_ASNATIVEBYTES_LITTLE_ENDIAN | Py_ASNATIVEBYTES_REJECT_NEGATIVE
    );
    int reject_negative_error = (reject_status == -1 && PyErr_Occurred()) ? 1 : 0;
    PyErr_Clear();

    unsigned char unsigned_src[2] = {0x34, 0x12};
    PyObject *from_unsigned = PyLong_FromNativeBytes(
        unsigned_src,
        2,
        Py_ASNATIVEBYTES_LITTLE_ENDIAN | Py_ASNATIVEBYTES_UNSIGNED_BUFFER
    );
    int from_unsigned_ok = from_unsigned && (PyLong_AsLong(from_unsigned) == 0x1234LL);

    unsigned char signed_src[1] = {0xFF};
    PyObject *from_signed = PyLong_FromNativeBytes(
        signed_src, 1, Py_ASNATIVEBYTES_LITTLE_ENDIAN
    );
    int from_signed_ok = from_signed && (PyLong_AsLong(from_signed) == -1LL);

    PyObject *from_unsigned_explicit = PyLong_FromUnsignedNativeBytes(
        signed_src, 1, Py_ASNATIVEBYTES_LITTLE_ENDIAN
    );
    int from_unsigned_explicit_ok = from_unsigned_explicit &&
        (PyLong_AsLong(from_unsigned_explicit) == 255LL);

    if (PyModule_AddIntConstant(module, "NEG_BYTES_OK", neg_bytes_ok) != 0 ||
        PyModule_AddIntConstant(module, "POS_SIGNED_REQ_OK", pos_signed_req_ok) != 0 ||
        PyModule_AddIntConstant(module, "POS_UNSIGNED_OK", pos_unsigned_ok) != 0 ||
        PyModule_AddIntConstant(module, "QUERY_OK", query_ok) != 0 ||
        PyModule_AddIntConstant(module, "REJECT_NEGATIVE_ERROR", reject_negative_error) != 0 ||
        PyModule_AddIntConstant(module, "FROM_UNSIGNED_OK", from_unsigned_ok ? 1 : 0) != 0 ||
        PyModule_AddIntConstant(module, "FROM_SIGNED_OK", from_signed_ok ? 1 : 0) != 0 ||
        PyModule_AddIntConstant(module, "FROM_UNSIGNED_EXPLICIT_OK", from_unsigned_explicit_ok ? 1 : 0) != 0) {
        return 0;
    }
    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch8_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch8 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch8_probe as m\nassert m.NEG_BYTES_OK == 1\nassert m.POS_SIGNED_REQ_OK == 1\nassert m.POS_UNSIGNED_OK == 1\nassert m.QUERY_OK == 1\nassert m.REJECT_NEGATIVE_ERROR == 1\nassert m.FROM_UNSIGNED_OK == 1\nassert m.FROM_SIGNED_OK == 1\nassert m.FROM_UNSIGNED_EXPLICIT_OK == 1",
    )
    .expect("cpython api batch8 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_buffer_abi_batch9_helpers_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch9 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch9 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch9");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch9_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"
#include <string.h>

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch9_probe",
    "cpython api batch9 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch9_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    long long shape[2] = {2, 3};
    long long c_strides[2] = {0, 0};
    long long f_strides[2] = {0, 0};
    PyBuffer_FillContiguousStrides(2, shape, c_strides, 1, 'C');
    PyBuffer_FillContiguousStrides(2, shape, f_strides, 1, 'F');
    int fill_strides_ok = (c_strides[0] == 3 && c_strides[1] == 1 &&
                           f_strides[0] == 1 && f_strides[1] == 2) ? 1 : 0;

    unsigned char storage[6] = {0, 0, 0, 0, 0, 0};
    Py_buffer info_view;
    int fill_info_status = PyBuffer_FillInfo(
        &info_view, 0, storage, 6, 0, PyBUF_FORMAT | PyBUF_ND | PyBUF_STRIDES
    );
    int fill_info_ok = (fill_info_status == 0 &&
                        info_view.ndim == 1 &&
                        info_view.shape && *(info_view.shape) == 6 &&
                        info_view.strides && *(info_view.strides) == 1 &&
                        info_view.format && info_view.format[0] == 'B') ? 1 : 0;
    PyBuffer_Release(&info_view);

    unsigned char contiguous_storage[6] = {0, 1, 2, 3, 4, 5};
    Py_buffer view_c = {0};
    view_c.buf = contiguous_storage;
    view_c.len = 6;
    view_c.itemsize = 1;
    view_c.ndim = 2;
    view_c.shape = shape;
    view_c.strides = c_strides;
    int is_c_ok = PyBuffer_IsContiguous(&view_c, 'C') == 1 ? 1 : 0;
    int is_f_ok = PyBuffer_IsContiguous(&view_c, 'F') == 0 ? 1 : 0;
    int is_a_ok = PyBuffer_IsContiguous(&view_c, 'A') == 1 ? 1 : 0;

    long long idx[2] = {1, 2};
    unsigned char *ptr = (unsigned char *)PyBuffer_GetPointer(&view_c, idx);
    int get_pointer_ok = (ptr == contiguous_storage + 5) ? 1 : 0;

    unsigned char noncontig_storage[6] = {0, 0, 0, 0, 0, 0};
    Py_buffer noncontig = {0};
    noncontig.buf = noncontig_storage;
    noncontig.len = 6;
    noncontig.itemsize = 1;
    noncontig.ndim = 2;
    noncontig.shape = shape;
    noncontig.strides = f_strides; /* force non-C layout */

    const unsigned char src[6] = {'A', 'B', 'C', 'D', 'E', 'F'};
    int from_contiguous_ok = PyBuffer_FromContiguous(&noncontig, src, 6, 'C') == 0 ? 1 : 0;
    int storage_layout_ok = (noncontig_storage[0] == 'A' &&
                             noncontig_storage[1] == 'D' &&
                             noncontig_storage[2] == 'B' &&
                             noncontig_storage[3] == 'E' &&
                             noncontig_storage[4] == 'C' &&
                             noncontig_storage[5] == 'F') ? 1 : 0;

    unsigned char roundtrip[6] = {0, 0, 0, 0, 0, 0};
    int to_contiguous_ok = PyBuffer_ToContiguous(roundtrip, &noncontig, 6, 'C') == 0 ? 1 : 0;
    int roundtrip_ok = (memcmp(roundtrip, src, 6) == 0) ? 1 : 0;

    int size_from_format_ok = (PyBuffer_SizeFromFormat("B") == 1 &&
                               PyBuffer_SizeFromFormat("I") == 4) ? 1 : 0;

    if (PyModule_AddIntConstant(module, "FILL_STRIDES_OK", fill_strides_ok) != 0 ||
        PyModule_AddIntConstant(module, "FILL_INFO_OK", fill_info_ok) != 0 ||
        PyModule_AddIntConstant(module, "IS_C_OK", is_c_ok) != 0 ||
        PyModule_AddIntConstant(module, "IS_F_OK", is_f_ok) != 0 ||
        PyModule_AddIntConstant(module, "IS_A_OK", is_a_ok) != 0 ||
        PyModule_AddIntConstant(module, "GET_POINTER_OK", get_pointer_ok) != 0 ||
        PyModule_AddIntConstant(module, "FROM_CONTIGUOUS_OK", from_contiguous_ok) != 0 ||
        PyModule_AddIntConstant(module, "STORAGE_LAYOUT_OK", storage_layout_ok) != 0 ||
        PyModule_AddIntConstant(module, "TO_CONTIGUOUS_OK", to_contiguous_ok) != 0 ||
        PyModule_AddIntConstant(module, "ROUNDTRIP_OK", roundtrip_ok) != 0 ||
        PyModule_AddIntConstant(module, "SIZE_FROM_FORMAT_OK", size_from_format_ok) != 0) {
        return 0;
    }
    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch9_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch9 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch9_probe as m\nassert m.FILL_STRIDES_OK == 1\nassert m.FILL_INFO_OK == 1\nassert m.IS_C_OK == 1\nassert m.IS_F_OK == 1\nassert m.IS_A_OK == 1\nassert m.GET_POINTER_OK == 1\nassert m.FROM_CONTIGUOUS_OK == 1\nassert m.STORAGE_LAYOUT_OK == 1\nassert m.TO_CONTIGUOUS_OK == 1\nassert m.ROUNDTRIP_OK == 1\nassert m.SIZE_FROM_FORMAT_OK == 1",
    )
    .expect("cpython api batch9 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_sequence_abi_batch10_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch10 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch10 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch10");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch10_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch10_probe",
    "cpython api batch10 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch10_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *list = PyList_New(3);
    if (!list) {
        return 0;
    }
    PyList_SetItem(list, 0, PyLong_FromLong(1));
    PyList_SetItem(list, 1, PyLong_FromLong(2));
    PyList_SetItem(list, 2, PyLong_FromLong(3));

    PyObject *two = PyLong_FromLong(2);
    PyObject *three = PyLong_FromLong(3);
    long long count_two = PySequence_Count(list, two);
    long long index_three = PySequence_Index(list, three);
    int in_ok = (PySequence_In(list, three) == 1) ? 1 : 0;
    Py_DECREF(two);
    Py_DECREF(three);

    PyObject *slice = PySequence_GetSlice(list, 1, 3);
    int get_slice_ok = (slice &&
                        PyList_Size(slice) == 2 &&
                        PyLong_AsInt(PyList_GetItem(slice, 0)) == 2 &&
                        PyLong_AsInt(PyList_GetItem(slice, 1)) == 3) ? 1 : 0;
    Py_XDECREF(slice);

    PyObject *nine = PyLong_FromLong(9);
    int set_item_ok = (PySequence_SetItem(list, 0, nine) == 0 &&
                       PyLong_AsInt(PyList_GetItem(list, 0)) == 9) ? 1 : 0;
    Py_DECREF(nine);

    int del_item_ok = (PySequence_DelItem(list, 1) == 0 &&
                       PyList_Size(list) == 2 &&
                       PyLong_AsInt(PyList_GetItem(list, 0)) == 9 &&
                       PyLong_AsInt(PyList_GetItem(list, 1)) == 3) ? 1 : 0;

    PyObject *replacement = PyList_New(2);
    if (!replacement) {
        return 0;
    }
    PyList_SetItem(replacement, 0, PyLong_FromLong(7));
    PyList_SetItem(replacement, 1, PyLong_FromLong(8));
    int set_slice_ok = (PySequence_SetSlice(list, 0, 1, replacement) == 0 &&
                        PyList_Size(list) == 3 &&
                        PyLong_AsInt(PyList_GetItem(list, 0)) == 7 &&
                        PyLong_AsInt(PyList_GetItem(list, 1)) == 8 &&
                        PyLong_AsInt(PyList_GetItem(list, 2)) == 3) ? 1 : 0;
    Py_DECREF(replacement);

    int del_slice_ok = (PySequence_DelSlice(list, 1, 2) == 0 &&
                        PyList_Size(list) == 2 &&
                        PyLong_AsInt(PyList_GetItem(list, 0)) == 7 &&
                        PyLong_AsInt(PyList_GetItem(list, 1)) == 3) ? 1 : 0;

    long long length_final = PySequence_Length(list);

    PyObject *tuple = PyTuple_New(2);
    if (!tuple) {
        return 0;
    }
    PyTuple_SetItem(tuple, 0, PyLong_FromLong(10));
    PyTuple_SetItem(tuple, 1, PyLong_FromLong(11));
    PyObject *list_from_tuple = PySequence_List(tuple);
    int list_ok = (list_from_tuple &&
                   PyList_Size(list_from_tuple) == 2 &&
                   PyLong_AsInt(PyList_GetItem(list_from_tuple, 0)) == 10 &&
                   PyLong_AsInt(PyList_GetItem(list_from_tuple, 1)) == 11) ? 1 : 0;
    Py_DECREF(tuple);
    Py_XDECREF(list_from_tuple);

    if (PyModule_AddIntConstant(module, "COUNT_TWO", count_two) != 0 ||
        PyModule_AddIntConstant(module, "INDEX_THREE", index_three) != 0 ||
        PyModule_AddIntConstant(module, "IN_OK", in_ok) != 0 ||
        PyModule_AddIntConstant(module, "GET_SLICE_OK", get_slice_ok) != 0 ||
        PyModule_AddIntConstant(module, "SET_ITEM_OK", set_item_ok) != 0 ||
        PyModule_AddIntConstant(module, "DEL_ITEM_OK", del_item_ok) != 0 ||
        PyModule_AddIntConstant(module, "SET_SLICE_OK", set_slice_ok) != 0 ||
        PyModule_AddIntConstant(module, "DEL_SLICE_OK", del_slice_ok) != 0 ||
        PyModule_AddIntConstant(module, "LENGTH_FINAL", length_final) != 0 ||
        PyModule_AddIntConstant(module, "LIST_OK", list_ok) != 0) {
        return 0;
    }

    Py_DECREF(list);
    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch10_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch10 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch10_probe as m\nassert m.COUNT_TWO == 1\nassert m.INDEX_THREE == 2\nassert m.IN_OK == 1\nassert m.GET_SLICE_OK == 1\nassert m.SET_ITEM_OK == 1\nassert m.DEL_ITEM_OK == 1\nassert m.SET_SLICE_OK == 1\nassert m.DEL_SLICE_OK == 1\nassert m.LENGTH_FINAL == 2\nassert m.LIST_OK == 1",
    )
    .expect("cpython api batch10 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_slice_abi_batch11_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch11 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch11 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch11");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch11_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch11_probe",
    "cpython api batch11 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch11_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *slice = PySlice_New(PyLong_FromLong(-5), PyLong_FromLong(5), PyLong_FromLong(2));
    if (!slice) {
        return 0;
    }
    long long start = 0, stop = 0, step = 0, slice_len = 0;
    int get_ok = PySlice_GetIndices(slice, 6, &start, &stop, &step) == 0 ? 1 : 0;
    int get_ex_ok = PySlice_GetIndicesEx(slice, 6, &start, &stop, &step, &slice_len) == 0 ? 1 : 0;
    Py_DECREF(slice);

    PyObject *out_of_range = PySlice_New(PyLong_FromLong(0), PyLong_FromLong(9), 0);
    if (!out_of_range) {
        return 0;
    }
    long long tmp_start = 0, tmp_stop = 0, tmp_step = 0;
    int get_range_err = (PySlice_GetIndices(out_of_range, 6, &tmp_start, &tmp_stop, &tmp_step) == -1) ? 1 : 0;
    PyErr_Clear();
    Py_DECREF(out_of_range);

    PyObject *default_slice = PySlice_New(0, 0, 0);
    if (!default_slice) {
        return 0;
    }
    long long d_start = 0, d_stop = 0, d_step = 0, d_len = 0;
    int default_ok = PySlice_GetIndicesEx(default_slice, 4, &d_start, &d_stop, &d_step, &d_len) == 0 ? 1 : 0;
    Py_DECREF(default_slice);

    if (PyModule_AddIntConstant(module, "GET_OK", get_ok) != 0 ||
        PyModule_AddIntConstant(module, "GET_EX_OK", get_ex_ok) != 0 ||
        PyModule_AddIntConstant(module, "GET_START", start) != 0 ||
        PyModule_AddIntConstant(module, "GET_STOP", stop) != 0 ||
        PyModule_AddIntConstant(module, "GET_STEP", step) != 0 ||
        PyModule_AddIntConstant(module, "GET_SLICE_LEN", slice_len) != 0 ||
        PyModule_AddIntConstant(module, "GET_RANGE_ERR", get_range_err) != 0 ||
        PyModule_AddIntConstant(module, "DEFAULT_OK", default_ok) != 0 ||
        PyModule_AddIntConstant(module, "DEFAULT_START", d_start) != 0 ||
        PyModule_AddIntConstant(module, "DEFAULT_STOP", d_stop) != 0 ||
        PyModule_AddIntConstant(module, "DEFAULT_STEP", d_step) != 0 ||
        PyModule_AddIntConstant(module, "DEFAULT_LEN", d_len) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch11_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch11 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch11_probe as m\nassert m.GET_OK == 1\nassert m.GET_EX_OK == 1\nassert m.GET_START == 1\nassert m.GET_STOP == 5\nassert m.GET_STEP == 2\nassert m.GET_SLICE_LEN == 2\nassert m.GET_RANGE_ERR == 1\nassert m.DEFAULT_OK == 1\nassert m.DEFAULT_START == 0\nassert m.DEFAULT_STOP == 4\nassert m.DEFAULT_STEP == 1\nassert m.DEFAULT_LEN == 4",
    )
    .expect("cpython api batch11 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_iter_and_memoryview_abi_batch12_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch12 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch12 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch12");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch12_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch12_probe",
    "cpython api batch12 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch12_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *seq = PyList_New(2);
    if (!seq) {
        return 0;
    }
    PyList_SetItem(seq, 0, PyLong_FromLong(10));
    PyList_SetItem(seq, 1, PyLong_FromLong(20));

    PyObject *iter = PySeqIter_New(seq);
    if (!iter) {
        return 0;
    }
    int iter_check_ok = (PyIter_Check(iter) == 1 && PyIter_Check(seq) == 0) ? 1 : 0;

    PyObject *item = 0;
    int next_item_1 = PyIter_NextItem(iter, &item);
    int next_item_1_ok = (next_item_1 == 1 && item && PyLong_AsInt(item) == 10) ? 1 : 0;
    Py_XDECREF(item);
    item = 0;
    int next_item_2 = PyIter_NextItem(iter, &item);
    int next_item_2_ok = (next_item_2 == 1 && item && PyLong_AsInt(item) == 20) ? 1 : 0;
    Py_XDECREF(item);
    item = 0;
    int next_item_end = PyIter_NextItem(iter, &item);
    int next_item_end_ok = (next_item_end == 0 && item == 0 && PyErr_Occurred() == 0) ? 1 : 0;
    Py_DECREF(iter);

    PyObject *iter_send = PySeqIter_New(seq);
    if (!iter_send) {
        return 0;
    }
    PyObject *none_arg = Py_BuildValue("");
    if (!none_arg) {
        return 0;
    }
    PyObject *send_result = 0;
    int send_status_1 = PyIter_Send(iter_send, none_arg, &send_result);
    int send_next_1_ok = (send_status_1 == PYGEN_NEXT && send_result && PyLong_AsInt(send_result) == 10) ? 1 : 0;
    Py_XDECREF(send_result);
    send_result = 0;
    int send_status_2 = PyIter_Send(iter_send, none_arg, &send_result);
    int send_next_2_ok = (send_status_2 == PYGEN_NEXT && send_result && PyLong_AsInt(send_result) == 20) ? 1 : 0;
    Py_XDECREF(send_result);
    send_result = 0;
    int send_status_3 = PyIter_Send(iter_send, none_arg, &send_result);
    int send_return_ok = (send_status_3 == PYGEN_RETURN && send_result != 0) ? 1 : 0;
    Py_XDECREF(send_result);
    Py_DECREF(none_arg);
    Py_DECREF(iter_send);

    PyObject *bytearray_obj = PyByteArray_FromStringAndSize("abcd", 4);
    if (!bytearray_obj) {
        return 0;
    }
    PyObject *number_obj = PyLong_FromLong(7);
    int check_buffer_ok = (PyObject_CheckBuffer(bytearray_obj) == 1 && PyObject_CheckBuffer(number_obj) == 0) ? 1 : 0;
    Py_XDECREF(number_obj);

    PyObject *mv_from_object = PyMemoryView_FromObject(bytearray_obj);
    Py_buffer mv_from_object_buf = {0};
    int from_object_ok = 0;
    if (mv_from_object && PyObject_GetBuffer(mv_from_object, &mv_from_object_buf, PyBUF_SIMPLE) == 0) {
        from_object_ok = (mv_from_object_buf.len == 4) ? 1 : 0;
        PyBuffer_Release(&mv_from_object_buf);
    }
    Py_XDECREF(mv_from_object);

    char ro_mem[3] = {'x', 'y', 'z'};
    PyObject *mv_from_memory_ro = PyMemoryView_FromMemory(ro_mem, 3, PyBUF_READ);
    Py_buffer ro_buf = {0};
    int from_memory_ro_ok = 0;
    if (mv_from_memory_ro && PyObject_GetBuffer(mv_from_memory_ro, &ro_buf, PyBUF_SIMPLE) == 0) {
        from_memory_ro_ok = (ro_buf.len == 3 && ro_buf.readonly == 1) ? 1 : 0;
        PyBuffer_Release(&ro_buf);
    }
    Py_XDECREF(mv_from_memory_ro);

    char rw_mem[2] = {'m', 'n'};
    PyObject *mv_from_memory_rw = PyMemoryView_FromMemory(rw_mem, 2, PyBUF_WRITE);
    Py_buffer rw_buf = {0};
    int from_memory_rw_ok = 0;
    if (mv_from_memory_rw && PyObject_GetBuffer(mv_from_memory_rw, &rw_buf, PyBUF_SIMPLE) == 0) {
        from_memory_rw_ok = (rw_buf.len == 2 && rw_buf.readonly == 0) ? 1 : 0;
        PyBuffer_Release(&rw_buf);
    }
    Py_XDECREF(mv_from_memory_rw);

    unsigned char source[4] = {1, 2, 3, 4};
    Py_buffer source_info = {0};
    source_info.buf = source;
    source_info.len = 4;
    source_info.itemsize = 1;
    source_info.readonly = 1;
    PyObject *mv_from_buffer = PyMemoryView_FromBuffer(&source_info);
    Py_buffer from_buffer_buf = {0};
    int from_buffer_ok = 0;
    if (mv_from_buffer && PyObject_GetBuffer(mv_from_buffer, &from_buffer_buf, PyBUF_SIMPLE) == 0) {
        from_buffer_ok = (from_buffer_buf.len == 4 && from_buffer_buf.readonly == 1) ? 1 : 0;
        PyBuffer_Release(&from_buffer_buf);
    }
    Py_XDECREF(mv_from_buffer);

    PyObject *contiguous_ro = PyMemoryView_GetContiguous(bytearray_obj, PyBUF_READ, 'C');
    Py_buffer contiguous_ro_buf = {0};
    int get_contiguous_ro_ok = 0;
    if (contiguous_ro && PyObject_GetBuffer(contiguous_ro, &contiguous_ro_buf, PyBUF_SIMPLE) == 0) {
        get_contiguous_ro_ok = (contiguous_ro_buf.len == 4) ? 1 : 0;
        PyBuffer_Release(&contiguous_ro_buf);
    }
    Py_XDECREF(contiguous_ro);

    PyObject *bytes_obj = PyBytes_FromStringAndSize("xy", 2);
    PyObject *contiguous_rw = PyMemoryView_GetContiguous(bytes_obj, PyBUF_WRITE, 'C');
    int get_contiguous_rw_error_ok = (contiguous_rw == 0 && PyErr_Occurred() != 0) ? 1 : 0;
    PyErr_Clear();
    Py_XDECREF(contiguous_rw);
    Py_XDECREF(bytes_obj);
    Py_XDECREF(bytearray_obj);
    Py_XDECREF(seq);

    if (PyModule_AddIntConstant(module, "ITER_CHECK_OK", iter_check_ok) != 0 ||
        PyModule_AddIntConstant(module, "NEXT_ITEM_1_OK", next_item_1_ok) != 0 ||
        PyModule_AddIntConstant(module, "NEXT_ITEM_2_OK", next_item_2_ok) != 0 ||
        PyModule_AddIntConstant(module, "NEXT_ITEM_END_OK", next_item_end_ok) != 0 ||
        PyModule_AddIntConstant(module, "SEND_NEXT_1_OK", send_next_1_ok) != 0 ||
        PyModule_AddIntConstant(module, "SEND_NEXT_2_OK", send_next_2_ok) != 0 ||
        PyModule_AddIntConstant(module, "SEND_RETURN_OK", send_return_ok) != 0 ||
        PyModule_AddIntConstant(module, "CHECK_BUFFER_OK", check_buffer_ok) != 0 ||
        PyModule_AddIntConstant(module, "FROM_OBJECT_OK", from_object_ok) != 0 ||
        PyModule_AddIntConstant(module, "FROM_MEMORY_RO_OK", from_memory_ro_ok) != 0 ||
        PyModule_AddIntConstant(module, "FROM_MEMORY_RW_OK", from_memory_rw_ok) != 0 ||
        PyModule_AddIntConstant(module, "FROM_BUFFER_OK", from_buffer_ok) != 0 ||
        PyModule_AddIntConstant(module, "GET_CONTIGUOUS_RO_OK", get_contiguous_ro_ok) != 0 ||
        PyModule_AddIntConstant(module, "GET_CONTIGUOUS_RW_ERROR_OK", get_contiguous_rw_error_ok) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch12_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch12 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch12_probe as m\nassert m.ITER_CHECK_OK == 1\nassert m.NEXT_ITEM_1_OK == 1\nassert m.NEXT_ITEM_2_OK == 1\nassert m.NEXT_ITEM_END_OK == 1\nassert m.SEND_NEXT_1_OK == 1\nassert m.SEND_NEXT_2_OK == 1\nassert m.SEND_RETURN_OK == 1\nassert m.CHECK_BUFFER_OK == 1\nassert m.FROM_OBJECT_OK == 1\nassert m.FROM_MEMORY_RO_OK == 1\nassert m.FROM_MEMORY_RW_OK == 1\nassert m.FROM_BUFFER_OK == 1\nassert m.GET_CONTIGUOUS_RO_OK == 1\nassert m.GET_CONTIGUOUS_RW_ERROR_OK == 1",
    )
    .expect("cpython api batch12 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_object_abi_batch13_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch13 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch13 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch13");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch13_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch13_probe",
    "cpython api batch13 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch13_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *attr_name = PyUnicode_FromString("flag");
    PyObject *attr_value = PyLong_FromLong(7);
    if (!attr_name || !attr_value) {
        return 0;
    }
    int set_attr_ok = (PyObject_SetAttr(module, attr_name, attr_value) == 0) ? 1 : 0;
    int has_attr_ok = (PyObject_HasAttr(module, attr_name) == 1) ? 1 : 0;
    int has_attr_with_error_ok = (PyObject_HasAttrWithError(module, attr_name) == 1) ? 1 : 0;

    PyObject *missing_name = PyUnicode_FromString("missing_attr");
    int has_attr_missing_ok = (PyObject_HasAttrWithError(module, missing_name) == 0 && PyErr_Occurred() == 0) ? 1 : 0;
    Py_XDECREF(missing_name);

    PyObject *bad_name = PyLong_FromLong(5);
    int has_attr_error = PyObject_HasAttrWithError(module, bad_name);
    int has_attr_error_path_ok = (has_attr_error == -1 && PyErr_Occurred() != 0) ? 1 : 0;
    PyErr_Clear();
    Py_XDECREF(bad_name);

    PyObject *optional_value = 0;
    int optional_found = PyObject_GetOptionalAttrString(module, "flag", &optional_value);
    int optional_found_ok = (optional_found == 1 && optional_value && PyLong_AsInt(optional_value) == 7) ? 1 : 0;
    Py_XDECREF(optional_value);
    optional_value = 0;
    int optional_missing = PyObject_GetOptionalAttrString(module, "missing_attr", &optional_value);
    int optional_missing_ok = (optional_missing == 0 && optional_value == 0 && PyErr_Occurred() == 0) ? 1 : 0;

    PyObject *repr_obj = PyObject_Repr(module);
    int repr_ok = (repr_obj && PyObject_Length(repr_obj) > 0) ? 1 : 0;
    Py_XDECREF(repr_obj);

    PyObject *dir_obj = PyObject_Dir(module);
    int dir_ok = (dir_obj && PyObject_Length(dir_obj) >= 0) ? 1 : 0;
    Py_XDECREF(dir_obj);

    PyObject *call_target = PyList_New(0);
    PyObject *len_method = call_target ? PyObject_GetAttrString(call_target, "__len__") : 0;
    PyObject *len_result = len_method ? PyObject_CallNoArgs(len_method) : 0;
    int call_noargs_ok = (len_result && PyLong_AsInt(len_result) == 0) ? 1 : 0;
    Py_XDECREF(len_result);
    Py_XDECREF(len_method);

    PyObject *list = PyList_New(0);
    PyObject *len_name = PyUnicode_FromString("__len__");
    PyObject *method_result = 0;
    if (list && len_name) {
        method_result = PyObject_CallMethodObjArgs(list, len_name, 0);
    }
    int call_method_objargs_ok = (method_result && PyLong_AsInt(method_result) == 0) ? 1 : 0;
    Py_XDECREF(method_result);
    Py_XDECREF(len_name);
    Py_XDECREF(list);
    Py_XDECREF(call_target);

    int del_attr_ok = (PyObject_DelAttr(module, attr_name) == 0 &&
                       PyObject_HasAttr(module, attr_name) == 0) ? 1 : 0;

    PyObject *attr_value_2 = PyLong_FromLong(8);
    int reset_attr_ok = (attr_value_2 && PyObject_SetAttr(module, attr_name, attr_value_2) == 0) ? 1 : 0;
    Py_XDECREF(attr_value_2);
    int del_attr_string_ok = (PyObject_DelAttrString(module, "flag") == 0 &&
                              PyObject_HasAttrStringWithError(module, "flag") == 0) ? 1 : 0;

    PyObject *dict_obj = Py_BuildValue("{}");
    PyObject *dict_key = PyUnicode_FromString("gone_via_item");
    PyObject *dict_value = PyLong_FromLong(1);
    int dict_seed_ok = (dict_obj &&
                        dict_key &&
                        dict_value &&
                        PyObject_SetItem(dict_obj, dict_key, dict_value) == 0 &&
                        PyObject_Length(dict_obj) == 1) ? 1 : 0;
    int del_item_string_ok = (dict_seed_ok &&
                              PyObject_DelItemString(dict_obj, "gone_via_item") == 0 &&
                              PyObject_Length(dict_obj) == 0) ? 1 : 0;
    Py_XDECREF(dict_value);
    Py_XDECREF(dict_key);
    Py_XDECREF(dict_obj);

    PyObject *length_list = PyList_New(3);
    int length_ok = (length_list && PyObject_Length(length_list) == 3) ? 1 : 0;
    Py_XDECREF(length_list);

    Py_XDECREF(attr_name);
    Py_XDECREF(attr_value);

    if (PyModule_AddIntConstant(module, "SET_ATTR_OK", set_attr_ok) != 0 ||
        PyModule_AddIntConstant(module, "HAS_ATTR_OK", has_attr_ok) != 0 ||
        PyModule_AddIntConstant(module, "HAS_ATTR_WITH_ERROR_OK", has_attr_with_error_ok) != 0 ||
        PyModule_AddIntConstant(module, "HAS_ATTR_MISSING_OK", has_attr_missing_ok) != 0 ||
        PyModule_AddIntConstant(module, "HAS_ATTR_ERROR_PATH_OK", has_attr_error_path_ok) != 0 ||
        PyModule_AddIntConstant(module, "OPTIONAL_FOUND_OK", optional_found_ok) != 0 ||
        PyModule_AddIntConstant(module, "OPTIONAL_MISSING_OK", optional_missing_ok) != 0 ||
        PyModule_AddIntConstant(module, "REPR_OK", repr_ok) != 0 ||
        PyModule_AddIntConstant(module, "DIR_OK", dir_ok) != 0 ||
        PyModule_AddIntConstant(module, "CALL_NOARGS_OK", call_noargs_ok) != 0 ||
        PyModule_AddIntConstant(module, "CALL_METHOD_OBJARGS_OK", call_method_objargs_ok) != 0 ||
        PyModule_AddIntConstant(module, "DEL_ATTR_OK", del_attr_ok) != 0 ||
        PyModule_AddIntConstant(module, "RESET_ATTR_OK", reset_attr_ok) != 0 ||
        PyModule_AddIntConstant(module, "DEL_ATTR_STRING_OK", del_attr_string_ok) != 0 ||
        PyModule_AddIntConstant(module, "DEL_ITEM_STRING_OK", del_item_string_ok) != 0 ||
        PyModule_AddIntConstant(module, "LENGTH_OK", length_ok) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch13_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch13 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch13_probe as m\nassert m.SET_ATTR_OK == 1\nassert m.HAS_ATTR_OK == 1\nassert m.HAS_ATTR_WITH_ERROR_OK == 1\nassert m.HAS_ATTR_MISSING_OK == 1\nassert m.HAS_ATTR_ERROR_PATH_OK == 1\nassert m.OPTIONAL_FOUND_OK == 1\nassert m.OPTIONAL_MISSING_OK == 1\nassert m.REPR_OK == 1\nassert m.DIR_OK == 1\nassert m.CALL_NOARGS_OK == 1\nassert m.CALL_METHOD_OBJARGS_OK == 1\nassert m.DEL_ATTR_OK == 1\nassert m.RESET_ATTR_OK == 1\nassert m.DEL_ATTR_STRING_OK == 1\nassert m.DEL_ITEM_STRING_OK == 1\nassert m.LENGTH_OK == 1",
    )
    .expect("cpython api batch13 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_object_buffer_abi_batch14_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch14 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch14 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch14");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch14_probe.c");
    fs::write(
        &source_path,
        r#"#include <string.h>
#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch14_probe",
    "cpython api batch14 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch14_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *unicode_obj = PyUnicode_FromString("A\xc3\xa9");
    PyObject *ascii_obj = unicode_obj ? PyObject_ASCII(unicode_obj) : 0;
    const char *ascii_text = ascii_obj ? PyUnicode_AsUTF8(ascii_obj) : 0;
    int ascii_ok = (ascii_text && strstr(ascii_text, "\\xe9") != 0) ? 1 : 0;
    Py_XDECREF(ascii_obj);
    Py_XDECREF(unicode_obj);

    unsigned char *alloc = (unsigned char *)PyObject_Calloc(4, 2);
    int calloc_ok = (alloc != 0);
    if (alloc) {
        int zero_ok = 1;
        for (int i = 0; i < 8; i++) {
            if (alloc[i] != 0) {
                zero_ok = 0;
                break;
            }
        }
        calloc_ok = calloc_ok && zero_ok;
        PyObject_Free(alloc);
    }

    PyObject *bytes_obj = PyBytes_FromStringAndSize("abcd", 4);
    const void *read_buf = 0;
    long long read_len = 0;
    int read_ok = (bytes_obj &&
                   PyObject_AsReadBuffer(bytes_obj, &read_buf, &read_len) == 0 &&
                   read_buf != 0 &&
                   read_len == 4 &&
                   ((const unsigned char *)read_buf)[0] == 'a') ? 1 : 0;
    const char *char_buf = 0;
    long long char_len = 0;
    int char_ok = (bytes_obj &&
                   PyObject_AsCharBuffer(bytes_obj, &char_buf, &char_len) == 0 &&
                   char_buf != 0 &&
                   char_len == 4 &&
                   char_buf[3] == 'd') ? 1 : 0;

    PyObject *bytearray_obj = PyByteArray_FromStringAndSize("abcd", 4);
    void *write_buf = 0;
    long long write_len = 0;
    int write_ok = 0;
    if (bytearray_obj &&
        PyObject_AsWriteBuffer(bytearray_obj, &write_buf, &write_len) == 0 &&
        write_buf != 0 &&
        write_len == 4) {
        ((unsigned char *)write_buf)[0] = 'Z';
        char *text = PyByteArray_AsString(bytearray_obj);
        write_ok = (text && text[0] == 'Z') ? 1 : 0;
    }

    PyObject *number_obj = PyLong_FromLong(7);
    int check_read_buffer_ok = (PyObject_CheckReadBuffer(bytes_obj) == 1 &&
                                PyObject_CheckReadBuffer(number_obj) == 0) ? 1 : 0;
    Py_XDECREF(number_obj);

    PyObject *copy_src = PyBytes_FromStringAndSize("wxyz", 4);
    PyObject *copy_dst = PyByteArray_FromStringAndSize("----", 4);
    int copy_data_ok = 0;
    if (copy_src && copy_dst && PyObject_CopyData(copy_dst, copy_src) == 0) {
        char *dst_text = PyByteArray_AsString(copy_dst);
        copy_data_ok = (dst_text &&
                        dst_text[0] == 'w' &&
                        dst_text[1] == 'x' &&
                        dst_text[2] == 'y' &&
                        dst_text[3] == 'z') ? 1 : 0;
    }
    Py_XDECREF(copy_dst);
    Py_XDECREF(copy_src);
    Py_XDECREF(bytearray_obj);
    Py_XDECREF(bytes_obj);

    if (PyModule_AddIntConstant(module, "ASCII_OK", ascii_ok) != 0 ||
        PyModule_AddIntConstant(module, "CALLOC_OK", calloc_ok) != 0 ||
        PyModule_AddIntConstant(module, "READ_OK", read_ok) != 0 ||
        PyModule_AddIntConstant(module, "CHAR_OK", char_ok) != 0 ||
        PyModule_AddIntConstant(module, "WRITE_OK", write_ok) != 0 ||
        PyModule_AddIntConstant(module, "CHECK_READ_BUFFER_OK", check_read_buffer_ok) != 0 ||
        PyModule_AddIntConstant(module, "COPY_DATA_OK", copy_data_ok) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch14_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch14 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch14_probe as m\nassert m.ASCII_OK == 1\nassert m.CALLOC_OK == 1\nassert m.READ_OK == 1\nassert m.CHAR_OK == 1\nassert m.WRITE_OK == 1\nassert m.CHECK_READ_BUFFER_OK == 1\nassert m.COPY_DATA_OK == 1",
    )
    .expect("cpython api batch14 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_object_gc_and_async_abi_batch15_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch15 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch15 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch15");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch15_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch15_probe",
    "cpython api batch15 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch15_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *list_obj = PyList_New(0);
    PyObject *dict_obj = Py_BuildValue("{}");
    PyObject *number_obj = PyLong_FromLong(7);
    PyObject *builtins_mod = PyImport_ImportModule("builtins");
    PyObject *list_type = builtins_mod ? PyObject_GetAttrString(builtins_mod, "list") : 0;

    int gc_tracked_list_ok = list_obj ? (PyObject_GC_IsTracked(list_obj) == 1) : 0;
    int gc_tracked_int_ok = number_obj ? (PyObject_GC_IsTracked(number_obj) == 0) : 0;
    int gc_finalized_ok = list_obj ? (PyObject_GC_IsFinalized(list_obj) == 0) : 0;

    int hash_not_implemented_ok = 0;
    if (list_obj) {
        PyErr_Clear();
        hash_not_implemented_ok = (PyObject_HashNotImplemented(list_obj) == -1 && PyErr_Occurred() != 0) ? 1 : 0;
        PyErr_Clear();
    }

    int getaiter_error_ok = 0;
    if (list_obj) {
        PyErr_Clear();
        PyObject *aiter = PyObject_GetAIter(list_obj);
        getaiter_error_ok = (aiter == 0 && PyErr_Occurred() != 0) ? 1 : 0;
        PyErr_Clear();
        Py_XDECREF(aiter);
    }

    int gettypedata_error_ok = 0;
    if (dict_obj && list_type) {
        PyErr_Clear();
        void *type_data = PyObject_GetTypeData(dict_obj, list_type);
        gettypedata_error_ok = (type_data == 0 && PyErr_Occurred() != 0) ? 1 : 0;
        PyErr_Clear();
    }

    Py_XDECREF(list_type);
    Py_XDECREF(builtins_mod);
    Py_XDECREF(number_obj);
    Py_XDECREF(dict_obj);
    Py_XDECREF(list_obj);

    if (PyModule_AddIntConstant(module, "GC_TRACKED_LIST_OK", gc_tracked_list_ok) != 0 ||
        PyModule_AddIntConstant(module, "GC_TRACKED_INT_OK", gc_tracked_int_ok) != 0 ||
        PyModule_AddIntConstant(module, "GC_FINALIZED_OK", gc_finalized_ok) != 0 ||
        PyModule_AddIntConstant(module, "HASH_NOT_IMPLEMENTED_OK", hash_not_implemented_ok) != 0 ||
        PyModule_AddIntConstant(module, "GET_AITER_ERROR_OK", getaiter_error_ok) != 0 ||
        PyModule_AddIntConstant(module, "GET_TYPEDATA_ERROR_OK", gettypedata_error_ok) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch15_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch15 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch15_probe as m\nassert m.GC_TRACKED_LIST_OK == 1\nassert m.GC_TRACKED_INT_OK == 1\nassert m.GC_FINALIZED_OK == 1\nassert m.HASH_NOT_IMPLEMENTED_OK == 1\nassert m.GET_AITER_ERROR_OK == 1\nassert m.GET_TYPEDATA_ERROR_OK == 1",
    )
    .expect("cpython api batch15 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_mapping_abi_batch16_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch16 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch16 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch16");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch16_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch16_probe",
    "cpython api batch16 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch16_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *dict_obj = Py_BuildValue("{s:i,s:i}", "a", 1, "b", 2);
    PyObject *list_obj = PyList_New(0);
    PyObject *set_obj = PySet_New(0);

    int mapping_check_dict_ok = dict_obj ? (PyMapping_Check(dict_obj) == 1) : 0;
    int mapping_check_list_ok = list_obj ? (PyMapping_Check(list_obj) == 1) : 0;
    int mapping_check_set_ok = set_obj ? (PyMapping_Check(set_obj) == 0) : 0;
    int aiter_check_list_ok = list_obj ? (PyAIter_Check(list_obj) == 0) : 0;

    int mapping_size_ok = dict_obj ? (PyMapping_Size(dict_obj) == 2 && PyMapping_Length(dict_obj) == 2) : 0;

    PyObject *key_a = PyUnicode_FromString("a");
    PyObject *key_missing = PyUnicode_FromString("missing");
    PyObject *value = 0;
    int optional_item_present_ok = 0;
    if (dict_obj && key_a && PyMapping_GetOptionalItem(dict_obj, key_a, &value) == 1 && value) {
        optional_item_present_ok = (PyLong_AsInt(value) == 1) ? 1 : 0;
    }
    Py_XDECREF(value);
    value = 0;

    int optional_item_missing_ok = 0;
    if (dict_obj && key_missing) {
        int rc = PyMapping_GetOptionalItem(dict_obj, key_missing, &value);
        optional_item_missing_ok = (rc == 0 && value == 0 && PyErr_Occurred() == 0) ? 1 : 0;
    }
    Py_XDECREF(value);
    value = 0;

    int optional_item_string_present_ok = 0;
    if (dict_obj && PyMapping_GetOptionalItemString(dict_obj, "b", &value) == 1 && value) {
        optional_item_string_present_ok = (PyLong_AsInt(value) == 2) ? 1 : 0;
    }
    Py_XDECREF(value);
    value = 0;

    int optional_item_error_ok = 0;
    if (dict_obj && list_obj) {
        int rc = PyMapping_GetOptionalItem(dict_obj, list_obj, &value);
        optional_item_error_ok = (rc == -1 && PyErr_Occurred() != 0) ? 1 : 0;
        PyErr_Clear();
    }
    Py_XDECREF(value);
    value = 0;

    int has_key_with_error_ok = (dict_obj && key_a && key_missing &&
                                 PyMapping_HasKeyWithError(dict_obj, key_a) == 1 &&
                                 PyMapping_HasKeyWithError(dict_obj, key_missing) == 0) ? 1 : 0;
    int has_key_string_with_error_ok = (dict_obj &&
                                        PyMapping_HasKeyStringWithError(dict_obj, "a") == 1 &&
                                        PyMapping_HasKeyStringWithError(dict_obj, "missing") == 0) ? 1 : 0;

    int has_key_suppresses_error_ok = 0;
    if (dict_obj && list_obj) {
        has_key_suppresses_error_ok = (PyMapping_HasKey(dict_obj, list_obj) == 0 && PyErr_Occurred() == 0) ? 1 : 0;
    }

    int has_key_string_suppresses_error_ok = 0;
    if (dict_obj) {
        has_key_string_suppresses_error_ok =
            (PyMapping_HasKeyString(dict_obj, ((const char*)0)) == 0 && PyErr_Occurred() == 0) ? 1 : 0;
    }

    PyObject *value_c = PyLong_FromLong(3);
    int set_item_string_ok = 0;
    PyObject *item_c = 0;
    if (dict_obj && value_c && PyMapping_SetItemString(dict_obj, "c", value_c) == 0) {
        item_c = PyMapping_GetItemString(dict_obj, "c");
        set_item_string_ok = (item_c && PyLong_AsInt(item_c) == 3) ? 1 : 0;
    }
    Py_XDECREF(item_c);
    Py_XDECREF(value_c);

    int mapping_views_ok = 0;
    PyObject *keys = 0;
    PyObject *items = 0;
    PyObject *values = 0;
    if (dict_obj) {
        keys = PyMapping_Keys(dict_obj);
        items = PyMapping_Items(dict_obj);
        values = PyMapping_Values(dict_obj);
        mapping_views_ok = (keys && items && values &&
                            PyObject_Length(keys) == 3 &&
                            PyObject_Length(items) == 3 &&
                            PyObject_Length(values) == 3) ? 1 : 0;
    }
    Py_XDECREF(values);
    Py_XDECREF(items);
    Py_XDECREF(keys);

    int mapping_view_error_ok = 0;
    if (list_obj) {
        PyObject *list_keys = PyMapping_Keys(list_obj);
        mapping_view_error_ok = (list_keys == 0 && PyErr_Occurred() != 0) ? 1 : 0;
        PyErr_Clear();
        Py_XDECREF(list_keys);
    }

    Py_XDECREF(key_missing);
    Py_XDECREF(key_a);
    Py_XDECREF(set_obj);
    Py_XDECREF(list_obj);
    Py_XDECREF(dict_obj);

    if (PyModule_AddIntConstant(module, "MAPPING_CHECK_DICT_OK", mapping_check_dict_ok) != 0 ||
        PyModule_AddIntConstant(module, "MAPPING_CHECK_LIST_OK", mapping_check_list_ok) != 0 ||
        PyModule_AddIntConstant(module, "MAPPING_CHECK_SET_OK", mapping_check_set_ok) != 0 ||
        PyModule_AddIntConstant(module, "AITER_CHECK_LIST_OK", aiter_check_list_ok) != 0 ||
        PyModule_AddIntConstant(module, "MAPPING_SIZE_OK", mapping_size_ok) != 0 ||
        PyModule_AddIntConstant(module, "OPTIONAL_ITEM_PRESENT_OK", optional_item_present_ok) != 0 ||
        PyModule_AddIntConstant(module, "OPTIONAL_ITEM_MISSING_OK", optional_item_missing_ok) != 0 ||
        PyModule_AddIntConstant(module, "OPTIONAL_ITEM_STRING_PRESENT_OK", optional_item_string_present_ok) != 0 ||
        PyModule_AddIntConstant(module, "OPTIONAL_ITEM_ERROR_OK", optional_item_error_ok) != 0 ||
        PyModule_AddIntConstant(module, "HAS_KEY_WITH_ERROR_OK", has_key_with_error_ok) != 0 ||
        PyModule_AddIntConstant(module, "HAS_KEY_STRING_WITH_ERROR_OK", has_key_string_with_error_ok) != 0 ||
        PyModule_AddIntConstant(module, "HAS_KEY_SUPPRESSES_ERROR_OK", has_key_suppresses_error_ok) != 0 ||
        PyModule_AddIntConstant(module, "HAS_KEY_STRING_SUPPRESSES_ERROR_OK", has_key_string_suppresses_error_ok) != 0 ||
        PyModule_AddIntConstant(module, "SET_ITEM_STRING_OK", set_item_string_ok) != 0 ||
        PyModule_AddIntConstant(module, "MAPPING_VIEWS_OK", mapping_views_ok) != 0 ||
        PyModule_AddIntConstant(module, "MAPPING_VIEW_ERROR_OK", mapping_view_error_ok) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch16_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch16 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch16_probe as m\nassert m.MAPPING_CHECK_DICT_OK == 1\nassert m.MAPPING_CHECK_LIST_OK == 1\nassert m.MAPPING_CHECK_SET_OK == 1\nassert m.AITER_CHECK_LIST_OK == 1\nassert m.MAPPING_SIZE_OK == 1\nassert m.OPTIONAL_ITEM_PRESENT_OK == 1\nassert m.OPTIONAL_ITEM_MISSING_OK == 1\nassert m.OPTIONAL_ITEM_STRING_PRESENT_OK == 1\nassert m.OPTIONAL_ITEM_ERROR_OK == 1\nassert m.HAS_KEY_WITH_ERROR_OK == 1\nassert m.HAS_KEY_STRING_WITH_ERROR_OK == 1\nassert m.HAS_KEY_SUPPRESSES_ERROR_OK == 1\nassert m.HAS_KEY_STRING_SUPPRESSES_ERROR_OK == 1\nassert m.SET_ITEM_STRING_OK == 1\nassert m.MAPPING_VIEWS_OK == 1\nassert m.MAPPING_VIEW_ERROR_OK == 1",
    )
    .expect("cpython api batch16 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_module_abi_batch17_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch17 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch17 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch17");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch17_probe.c");
    fs::write(
        &source_path,
        r#"#include <string.h>
#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch17_probe",
    "cpython api batch17 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch17_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *m1 = PyModule_New("batch17_mod");
    PyObject *m1_name_obj = m1 ? PyModule_GetNameObject(m1) : 0;
    const char *m1_name_c = m1 ? PyModule_GetName(m1) : 0;
    int module_new_ok = (m1 && m1_name_obj && m1_name_c && strcmp(m1_name_c, "batch17_mod") == 0) ? 1 : 0;

    int set_doc_ok = 0;
    if (m1 && PyModule_SetDocString(m1, "batch17 doc") == 0) {
        PyObject *doc = PyObject_GetAttrString(m1, "__doc__");
        const char *doc_c = doc ? PyUnicode_AsUTF8(doc) : 0;
        set_doc_ok = (doc_c && strcmp(doc_c, "batch17 doc") == 0) ? 1 : 0;
        Py_XDECREF(doc);
    }

    int module_add_ok = 0;
    if (m1) {
        PyObject *answer = PyLong_FromLong(42);
        if (answer && PyModule_Add(m1, "answer", answer) == 0) {
            PyObject *stored = PyObject_GetAttrString(m1, "answer");
            module_add_ok = (stored && PyLong_AsInt(stored) == 42) ? 1 : 0;
            Py_XDECREF(stored);
        }
    }

    int filename_missing_error_ok = 0;
    if (m1) {
        PyErr_Clear();
        PyObject *missing = PyModule_GetFilenameObject(m1);
        filename_missing_error_ok = (missing == 0 && PyErr_Occurred() != 0) ? 1 : 0;
        PyErr_Clear();
        Py_XDECREF(missing);
    }

    int filename_present_ok = 0;
    if (m1) {
        PyObject *file_value = PyUnicode_FromString("/tmp/batch17_mod.py");
        if (file_value && PyModule_AddObjectRef(m1, "__file__", file_value) == 0) {
            PyObject *file_obj = PyModule_GetFilenameObject(m1);
            const char *file_c = PyModule_GetFilename(m1);
            const char *file_text = file_obj ? PyUnicode_AsUTF8(file_obj) : 0;
            filename_present_ok = (file_text && file_c &&
                                   strcmp(file_text, "/tmp/batch17_mod.py") == 0 &&
                                   strcmp(file_c, "/tmp/batch17_mod.py") == 0) ? 1 : 0;
            Py_XDECREF(file_obj);
        }
        Py_XDECREF(file_value);
    }

    PyObject *name2 = PyUnicode_FromString("batch17_mod_object");
    PyObject *m2 = name2 ? PyModule_NewObject(name2) : 0;
    const char *m2_name = m2 ? PyModule_GetName(m2) : 0;
    int module_new_object_ok = (m2 && m2_name && strcmp(m2_name, "batch17_mod_object") == 0) ? 1 : 0;

    Py_XDECREF(m2);
    Py_XDECREF(name2);
    Py_XDECREF(m1_name_obj);
    Py_XDECREF(m1);

    if (PyModule_AddIntConstant(module, "MODULE_NEW_OK", module_new_ok) != 0 ||
        PyModule_AddIntConstant(module, "SET_DOC_OK", set_doc_ok) != 0 ||
        PyModule_AddIntConstant(module, "MODULE_ADD_OK", module_add_ok) != 0 ||
        PyModule_AddIntConstant(module, "FILENAME_MISSING_ERROR_OK", filename_missing_error_ok) != 0 ||
        PyModule_AddIntConstant(module, "FILENAME_PRESENT_OK", filename_present_ok) != 0 ||
        PyModule_AddIntConstant(module, "MODULE_NEW_OBJECT_OK", module_new_object_ok) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch17_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch17 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch17_probe as m\nassert m.MODULE_NEW_OK == 1\nassert m.SET_DOC_OK == 1\nassert m.MODULE_ADD_OK == 1\nassert m.FILENAME_MISSING_ERROR_OK == 1\nassert m.FILENAME_PRESENT_OK == 1\nassert m.MODULE_NEW_OBJECT_OK == 1",
    )
    .expect("cpython api batch17 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_module_helpers_abi_batch18_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch18 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch18 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch18");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch18_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static PyObject *
batch18_answer(PyObject *self, PyObject *args) {
    return PyLong_FromLong(123);
}

static PyMethodDef batch18_methods[] = {
    {"answer", batch18_answer, METH_NOARGS, "batch18 answer"},
    {0, 0, 0, 0}
};

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch18_probe",
    "cpython api batch18 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch18_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    int add_functions_ok = (PyModule_AddFunctions(module, batch18_methods) == 0) ? 1 : 0;
    int answer_ok = 0;
    if (add_functions_ok) {
        PyObject *fn = PyObject_GetAttrString(module, "answer");
        PyObject *result = fn ? PyObject_CallNoArgs(fn) : 0;
        answer_ok = (result && PyLong_AsInt(result) == 123) ? 1 : 0;
        Py_XDECREF(result);
        Py_XDECREF(fn);
    }

    int add_type_ok = 0;
    PyObject *builtins_mod = PyImport_ImportModule("builtins");
    PyObject *int_type = builtins_mod ? PyObject_GetAttrString(builtins_mod, "int") : 0;
    if (int_type) {
        add_type_ok = (PyModule_AddType(module, (PyTypeObject *)int_type) == 0 &&
                       PyObject_HasAttrStringWithError(module, "int") == 1) ? 1 : 0;
    }
    Py_XDECREF(int_type);
    Py_XDECREF(builtins_mod);

    if (PyModule_AddIntConstant(module, "ADD_FUNCTIONS_OK", add_functions_ok) != 0 ||
        PyModule_AddIntConstant(module, "ANSWER_OK", answer_ok) != 0 ||
        PyModule_AddIntConstant(module, "ADD_TYPE_OK", add_type_ok) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch18_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch18 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch18_probe as m\nassert m.ADD_FUNCTIONS_OK == 1\nassert m.ANSWER_OK == 1\nassert m.ADD_TYPE_OK == 1",
    )
    .expect("cpython api batch18 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_exception_factory_abi_batch19_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch19 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch19 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch19");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch19_probe.c");
    fs::write(
        &source_path,
        r#"#include <string.h>
#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch19_probe",
    "cpython api batch19 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch19_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *custom = PyErr_NewException("batch19.CustomError", 0, 0);
    int custom_ok = 0;
    if (custom) {
        const char *tp_name = PyExceptionClass_Name(custom);
        PyObject *module_attr = PyObject_GetAttrString(custom, "__module__");
        const char *module_name = module_attr ? PyUnicode_AsUTF8(module_attr) : 0;
        custom_ok = (tp_name && module_name &&
                     strcmp(tp_name, "CustomError") == 0 &&
                     strcmp(module_name, "batch19") == 0) ? 1 : 0;
        Py_XDECREF(module_attr);
    }

    PyObject *with_doc = PyErr_NewExceptionWithDoc("batch19.DocError", "doc string value", 0, 0);
    int with_doc_ok = 0;
    if (with_doc) {
        PyObject *doc = PyObject_GetAttrString(with_doc, "__doc__");
        const char *doc_text = doc ? PyUnicode_AsUTF8(doc) : 0;
        with_doc_ok = (doc_text && strcmp(doc_text, "doc string value") == 0) ? 1 : 0;
        Py_XDECREF(doc);
    }

    PyObject *base = PyErr_NewException("batch19.BaseError", 0, 0);
    PyObject *derived = base ? PyErr_NewException("batch19.DerivedError", base, 0) : 0;
    PyObject *base_tuple = base ? PyTuple_Pack(1, base) : 0;
    PyObject *derived_tuple = base_tuple ? PyErr_NewException("batch19.DerivedTupleError", base_tuple, 0) : 0;
    PyObject *derived_instance = derived ? PyObject_CallNoArgs(derived) : 0;
    PyObject *derived_tuple_instance = derived_tuple ? PyObject_CallNoArgs(derived_tuple) : 0;
    int subclass_ok = (base && derived && derived_tuple &&
                       derived_instance && derived_tuple_instance &&
                       PyObject_IsInstance(derived_instance, base) == 1 &&
                       PyObject_IsInstance(derived_tuple_instance, base) == 1) ? 1 : 0;

    PyErr_Clear();
    PyObject *invalid_name = PyErr_NewException("NoDotName", 0, 0);
    int invalid_name_error_ok = (invalid_name == 0 && PyErr_Occurred() != 0) ? 1 : 0;
    PyErr_Clear();
    Py_XDECREF(invalid_name);

    Py_XDECREF(derived_tuple_instance);
    Py_XDECREF(derived_instance);
    Py_XDECREF(derived_tuple);
    Py_XDECREF(base_tuple);
    Py_XDECREF(derived);
    Py_XDECREF(base);
    Py_XDECREF(with_doc);
    Py_XDECREF(custom);

    if (PyModule_AddIntConstant(module, "CUSTOM_OK", custom_ok) != 0 ||
        PyModule_AddIntConstant(module, "WITH_DOC_OK", with_doc_ok) != 0 ||
        PyModule_AddIntConstant(module, "SUBCLASS_OK", subclass_ok) != 0 ||
        PyModule_AddIntConstant(module, "INVALID_NAME_ERROR_OK", invalid_name_error_ok) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch19_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch19 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch19_probe as m\nassert m.CUSTOM_OK == 1\nassert m.WITH_DOC_OK == 1\nassert m.SUBCLASS_OK == 1\nassert m.INVALID_NAME_ERROR_OK == 1",
    )
    .expect("cpython api batch19 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_number_abi_batch20_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch20 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch20 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch20");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch20_probe.c");
    fs::write(
        &source_path,
        r#"#include <math.h>
#include <string.h>
#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch20_probe",
    "cpython api batch20 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

static int as_long_is(PyObject *value, long expected) {
    return value && PyLong_AsLong(value) == expected;
}

PyMODINIT_FUNC
PyInit_cpython_api_batch20_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *v21 = PyLong_FromLong(21);
    PyObject *v4 = PyLong_FromLong(4);
    PyObject *v2 = PyLong_FromLong(2);
    PyObject *v5 = PyLong_FromLong(5);

    int add_ok = as_long_is(PyNumber_InPlaceAdd(v21, v4), 25);
    int sub_ok = as_long_is(PyNumber_InPlaceSubtract(v21, v4), 17);
    int mul_ok = as_long_is(PyNumber_InPlaceMultiply(v21, v4), 84);
    int floordiv_ok = as_long_is(PyNumber_InPlaceFloorDivide(v21, v4), 5);
    PyObject *true_div = PyNumber_InPlaceTrueDivide(v21, v4);
    int truediv_ok = 0;
    if (true_div) {
        double value = PyFloat_AsDouble(true_div);
        truediv_ok = fabs(value - 5.25) < 1e-12;
    }
    int rem_ok = as_long_is(PyNumber_InPlaceRemainder(v21, v4), 1);
    int pow_ok = as_long_is(PyNumber_InPlacePower(v2, v5, 0), 32);
    int lshift_ok = as_long_is(PyNumber_InPlaceLshift(v21, v2), 84);
    int rshift_ok = as_long_is(PyNumber_InPlaceRshift(v21, v2), 5);
    int and_ok = as_long_is(PyNumber_InPlaceAnd(v21, v4), 4);
    int or_ok = as_long_is(PyNumber_InPlaceOr(v21, v4), 21);
    int xor_ok = as_long_is(PyNumber_InPlaceXor(v21, v4), 17);
    int inplace_ok = add_ok && sub_ok && mul_ok && floordiv_ok && truediv_ok &&
                     rem_ok && pow_ok && lshift_ok && rshift_ok &&
                     and_ok && or_ok && xor_ok;

    PyObject *matrix = PyNumber_MatrixMultiply(v21, v4);
    int matrix_error_ok = (matrix == 0 && PyErr_Occurred() != 0) ? 1 : 0;
    PyErr_Clear();
    Py_XDECREF(matrix);

    PyObject *hex_text = PyNumber_ToBase(PyLong_FromLong(26), 16);
    PyObject *bin_text = PyNumber_ToBase(PyLong_FromLong(-10), 2);
    PyObject *dec_text = PyNumber_ToBase(PyLong_FromLong(42), 10);
    const char *hex_utf8 = hex_text ? PyUnicode_AsUTF8(hex_text) : 0;
    const char *bin_utf8 = bin_text ? PyUnicode_AsUTF8(bin_text) : 0;
    const char *dec_utf8 = dec_text ? PyUnicode_AsUTF8(dec_text) : 0;
    int tobase_ok = (hex_utf8 && strcmp(hex_utf8, "0x1a") == 0 &&
                     bin_utf8 && strcmp(bin_utf8, "-0b1010") == 0 &&
                     dec_utf8 && strcmp(dec_utf8, "42") == 0) ? 1 : 0;

    PyErr_Clear();
    PyObject *invalid_base = PyNumber_ToBase(PyLong_FromLong(7), 3);
    int invalid_base_error_ok = (invalid_base == 0 && PyErr_Occurred() != 0) ? 1 : 0;
    PyErr_Clear();
    Py_XDECREF(invalid_base);

    Py_XDECREF(dec_text);
    Py_XDECREF(bin_text);
    Py_XDECREF(hex_text);
    Py_XDECREF(true_div);
    Py_XDECREF(v5);
    Py_XDECREF(v2);
    Py_XDECREF(v4);
    Py_XDECREF(v21);

    if (PyModule_AddIntConstant(module, "INPLACE_OK", inplace_ok) != 0 ||
        PyModule_AddIntConstant(module, "MATRIX_ERROR_OK", matrix_error_ok) != 0 ||
        PyModule_AddIntConstant(module, "TOBASE_OK", tobase_ok) != 0 ||
        PyModule_AddIntConstant(module, "TOBASE_INVALID_ERROR_OK", invalid_base_error_ok) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch20_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch20 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch20_probe as m\nassert m.INPLACE_OK == 1\nassert m.MATRIX_ERROR_OK == 1\nassert m.TOBASE_OK == 1\nassert m.TOBASE_INVALID_ERROR_OK == 1",
    )
    .expect("cpython api batch20 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_error_abi_batch21_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch21 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch21 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch21");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch21_probe.c");
    fs::write(
        &source_path,
        r#"#include <string.h>
#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch21_probe",
    "cpython api batch21 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

static int error_is_set(void) {
    return PyErr_Occurred() != 0 ? 1 : 0;
}

PyMODINIT_FUNC
PyInit_cpython_api_batch21_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    int errno_family_ok = 1;
    errno_family_ok &= (PyErr_SetFromErrnoWithFilename(PyExc_OSError, "/tmp/errno_a.txt") == 0 && error_is_set());
    PyErr_Clear();
    PyObject *fname_obj = PyUnicode_FromString("/tmp/errno_b.txt");
    errno_family_ok &= (PyErr_SetFromErrnoWithFilenameObject(PyExc_OSError, fname_obj) == 0 && error_is_set());
    PyErr_Clear();
    errno_family_ok &= (PyErr_SetFromErrnoWithFilenameObjects(PyExc_OSError, fname_obj, fname_obj) == 0 && error_is_set());
    PyErr_Clear();
    Py_XDECREF(fname_obj);

    int windows_family_ok = 1;
    windows_family_ok &= (PyErr_SetExcFromWindowsErr(PyExc_OSError, 5) == 0 && error_is_set());
    PyErr_Clear();
    windows_family_ok &= (PyErr_SetExcFromWindowsErrWithFilename(PyExc_OSError, 6, "C:/tmp/win_a.txt") == 0 && error_is_set());
    PyErr_Clear();
    PyObject *win_fname = PyUnicode_FromString("C:/tmp/win_b.txt");
    windows_family_ok &= (PyErr_SetExcFromWindowsErrWithFilenameObject(PyExc_OSError, 7, win_fname) == 0 && error_is_set());
    PyErr_Clear();
    windows_family_ok &= (PyErr_SetExcFromWindowsErrWithFilenameObjects(PyExc_OSError, 8, win_fname, win_fname) == 0 && error_is_set());
    PyErr_Clear();
    windows_family_ok &= (PyErr_SetFromWindowsErr(9) == 0 && error_is_set());
    PyErr_Clear();
    windows_family_ok &= (PyErr_SetFromWindowsErrWithFilename(10, "C:/tmp/win_c.txt") == 0 && error_is_set());
    PyErr_Clear();
    Py_XDECREF(win_fname);

    int interrupt_ok = 1;
    PyErr_SetInterrupt();
    interrupt_ok &= error_is_set();
    PyErr_Clear();
    interrupt_ok &= (PyErr_SetInterruptEx(2) == 0 && error_is_set());
    PyErr_Clear();
    interrupt_ok &= (PyErr_SetInterruptEx(-1) == -1);

    int syntax_ok = 1;
    PyErr_SyntaxLocation("syntax_a.py", 12);
    syntax_ok &= error_is_set();
    PyErr_Clear();
    PyErr_SyntaxLocationEx("syntax_b.py", 34, 5);
    syntax_ok &= error_is_set();
    PyErr_Clear();

    PyObject *line = PyErr_ProgramText(__FILE__, 1);
    const char *line_text = line ? PyUnicode_AsUTF8(line) : 0;
    int program_text_ok = (line_text && strstr(line_text, "include") != 0) ? 1 : 0;
    Py_XDECREF(line);

    if (PyModule_AddIntConstant(module, "ERRNO_FAMILY_OK", errno_family_ok) != 0 ||
        PyModule_AddIntConstant(module, "WINDOWS_FAMILY_OK", windows_family_ok) != 0 ||
        PyModule_AddIntConstant(module, "INTERRUPT_OK", interrupt_ok) != 0 ||
        PyModule_AddIntConstant(module, "SYNTAX_OK", syntax_ok) != 0 ||
        PyModule_AddIntConstant(module, "PROGRAM_TEXT_OK", program_text_ok) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch21_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch21 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch21_probe as m\nassert m.ERRNO_FAMILY_OK == 1\nassert m.WINDOWS_FAMILY_OK == 1\nassert m.INTERRUPT_OK == 1\nassert m.SYNTAX_OK == 1\nassert m.PROGRAM_TEXT_OK == 1",
    )
    .expect("cpython api batch21 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_import_error_abi_batch22_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch22 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch22 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch22");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch22_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch22_probe",
    "cpython api batch22 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

static int raised_is(PyObject *expected) {
    PyObject *raised = PyErr_Occurred();
    return (raised == expected) ? 1 : 0;
}

PyMODINIT_FUNC
PyInit_cpython_api_batch22_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *msg = PyUnicode_FromString("probe import error");
    PyObject *name = PyUnicode_FromString("pkg.mod");
    PyObject *path = PyUnicode_FromString("/tmp/pkg/mod.py");
    if (!msg || !name || !path) {
        return 0;
    }

    int import_error_ok = 1;
    import_error_ok &= (PyErr_SetImportError(msg, name, path) == 0 && raised_is(PyExc_ImportError));
    PyErr_Clear();

    PyObject *custom = PyErr_NewException(
        "cpython_api_batch22_probe.CustomImportError",
        PyExc_ImportError,
        0
    );
    if (!custom) {
        return 0;
    }
    import_error_ok &= (
        PyErr_SetImportErrorSubclass(custom, msg, name, path) == 0 &&
        raised_is(custom)
    );
    PyErr_Clear();

    int subclass_validation_ok = 1;
    subclass_validation_ok &= (
        PyErr_SetImportErrorSubclass(PyExc_TypeError, msg, name, path) == 0 &&
        raised_is(PyExc_TypeError)
    );
    PyErr_Clear();
    subclass_validation_ok &= (
        PyErr_SetImportErrorSubclass(PyExc_ImportError, 0, name, path) == 0 &&
        raised_is(PyExc_TypeError)
    );
    PyErr_Clear();

    Py_XDECREF(custom);
    Py_XDECREF(path);
    Py_XDECREF(name);
    Py_XDECREF(msg);

    if (PyModule_AddIntConstant(module, "IMPORT_ERROR_OK", import_error_ok) != 0 ||
        PyModule_AddIntConstant(module, "SUBCLASS_VALIDATION_OK", subclass_validation_ok) != 0) {
        return 0;
    }

    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch22_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch22 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch22_probe as m\nassert m.IMPORT_ERROR_OK == 1\nassert m.SUBCLASS_VALIDATION_OK == 1",
    )
    .expect("cpython api batch22 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_warning_abi_batch23_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch23 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch23 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch23");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch23_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch23_probe",
    "cpython api batch23 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch23_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    int explicit_ok = (
        PyErr_WarnExplicit(
            0,
            "batch23 explicit warning",
            "batch23_probe.py",
            11,
            "batch23_mod",
            0
        ) == 0 &&
        PyErr_Occurred() == 0
    );
    int resource_ok = (
        PyErr_ResourceWarning(0, 1, "batch23 resource warning") == 0 &&
        PyErr_Occurred() == 0
    );

    if (PyModule_AddIntConstant(module, "EXPLICIT_OK", explicit_ok) != 0 ||
        PyModule_AddIntConstant(module, "RESOURCE_OK", resource_ok) != 0) {
        return 0;
    }
    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch23_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch23 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch23_probe as m\nassert m.EXPLICIT_OK == 1\nassert m.RESOURCE_OK == 1",
    )
    .expect("cpython api batch23 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_import_magic_abi_batch24_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch24 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch24 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch24");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch24_probe.c");
    fs::write(
        &source_path,
        r#"#include <string.h>
#include "pyrs_cpython_compat.h"

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch24_probe",
    "cpython api batch24 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch24_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }
    long magic = PyImport_GetMagicNumber();
    const char *tag = PyImport_GetMagicTag();
    int magic_ok = (magic == 168627755L) ? 1 : 0;
    int tag_ok = (tag && strcmp(tag, "cpython-314") == 0) ? 1 : 0;

    if (PyModule_AddIntConstant(module, "MAGIC_OK", magic_ok) != 0 ||
        PyModule_AddIntConstant(module, "TAG_OK", tag_ok) != 0) {
        return 0;
    }
    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch24_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch24 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch24_probe as m\nassert m.MAGIC_OK == 1\nassert m.TAG_OK == 1",
    )
    .expect("cpython api batch24 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_descriptor_abi_batch25_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch25 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch25 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch25");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch25_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

#define T_INT 1

static PyObject *
method_noargs(PyObject *self, PyObject *ignored) {
    (void)self;
    (void)ignored;
    return PyLong_FromLong(7);
}

static PyObject *
getset_getter(PyObject *self, void *closure) {
    (void)self;
    (void)closure;
    return PyBool_FromLong(1);
}

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch25_probe",
    "cpython api batch25 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch25_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *owner_type = PyObject_Type(module);
    if (!owner_type) {
        return 0;
    }

    static PyMethodDef good_method = {"good_method", method_noargs, METH_NOARGS, "good"};
    static PyMethodDef good_class_method = {"good_class_method", method_noargs, METH_NOARGS, "good classmethod"};
    static PyMethodDef bad_flags_method = {"bad_flags", method_noargs, METH_NOARGS | METH_O, "bad"};
    static PyMemberDef good_member = {"member_value", T_INT, 0, 0, "member"};
    static PyMemberDef relative_member = {"member_relative", T_INT, 0, Py_RELATIVE_OFFSET, "member"};
    static PyGetSetDef good_getset = {"managed", (void *)getset_getter, 0, "getset", 0};

    PyObject *method_descr = PyDescr_NewMethod((PyTypeObject *)owner_type, &good_method);
    PyObject *class_method_descr = PyDescr_NewClassMethod((PyTypeObject *)owner_type, &good_class_method);
    PyObject *member_descr = PyDescr_NewMember((PyTypeObject *)owner_type, &good_member);
    PyObject *getset_descr = PyDescr_NewGetSet((PyTypeObject *)owner_type, &good_getset);

    int create_ok = (method_descr != 0 && class_method_descr != 0 && member_descr != 0 && getset_descr != 0) ? 1 : 0;

    int descriptor_types_ok = 1;
    PyObject *method_type = method_descr ? PyObject_Type(method_descr) : 0;
    PyObject *class_method_type = class_method_descr ? PyObject_Type(class_method_descr) : 0;
    PyObject *member_type = member_descr ? PyObject_Type(member_descr) : 0;
    PyObject *getset_type = getset_descr ? PyObject_Type(getset_descr) : 0;
    descriptor_types_ok &= (method_type == (PyObject *)&PyMethodDescr_Type) ? 1 : 0;
    descriptor_types_ok &= (class_method_type == (PyObject *)&PyClassMethodDescr_Type) ? 1 : 0;
    descriptor_types_ok &= (member_type == (PyObject *)&PyMemberDescr_Type) ? 1 : 0;
    descriptor_types_ok &= (getset_type == (PyObject *)&PyGetSetDescr_Type) ? 1 : 0;

    PyObject *bad_flags = PyDescr_NewMethod((PyTypeObject *)owner_type, &bad_flags_method);
    int bad_flags_ok = (bad_flags == 0 && PyErr_Occurred() != 0) ? 1 : 0;
    PyErr_Clear();

    PyObject *relative = PyDescr_NewMember((PyTypeObject *)owner_type, &relative_member);
    int relative_offset_ok = (relative == 0 && PyErr_Occurred() != 0) ? 1 : 0;
    PyErr_Clear();

    Py_XDECREF(relative);
    Py_XDECREF(bad_flags);
    Py_XDECREF(getset_type);
    Py_XDECREF(member_type);
    Py_XDECREF(class_method_type);
    Py_XDECREF(method_type);
    Py_XDECREF(getset_descr);
    Py_XDECREF(member_descr);
    Py_XDECREF(class_method_descr);
    Py_XDECREF(method_descr);
    Py_XDECREF(owner_type);

    if (PyModule_AddIntConstant(module, "CREATE_OK", create_ok) != 0 ||
        PyModule_AddIntConstant(module, "DESCRIPTOR_TYPES_OK", descriptor_types_ok) != 0 ||
        PyModule_AddIntConstant(module, "BAD_FLAGS_OK", bad_flags_ok) != 0 ||
        PyModule_AddIntConstant(module, "RELATIVE_OFFSET_OK", relative_offset_ok) != 0) {
        return 0;
    }
    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch25_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch25 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch25_probe as m\nassert m.CREATE_OK == 1\nassert m.DESCRIPTOR_TYPES_OK == 1\nassert m.BAD_FLAGS_OK == 1\nassert m.RELATIVE_OFFSET_OK == 1",
    )
    .expect("cpython api batch25 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_parse_abi_batch26_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch26 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch26 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch26");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch26_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static int
run_vaparse(PyObject *args, const char *format, ...) {
    va_list ap;
    va_start(ap, format);
    int ok = PyArg_VaParse(args, format, ap);
    va_end(ap);
    return ok;
}

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch26_probe",
    "cpython api batch26 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch26_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *forty_two = PyLong_FromLong(42);
    if (!forty_two) {
        return 0;
    }

    PyObject *parsed_from_parse = 0;
    int parse_ok = PyArg_Parse(forty_two, "O", &parsed_from_parse);
    int parse_identity_ok = (parse_ok && parsed_from_parse == forty_two) ? 1 : 0;

    PyObject *tuple = PyTuple_New(1);
    if (!tuple) {
        return 0;
    }
    Py_INCREF(forty_two);
    if (PyTuple_SetItem(tuple, 0, forty_two) != 0) {
        return 0;
    }

    PyObject *parsed_from_va = 0;
    int vaparse_ok = run_vaparse(tuple, "O", &parsed_from_va);
    int vaparse_identity_ok = (vaparse_ok && parsed_from_va == forty_two) ? 1 : 0;

    int vaparse_non_tuple_ok = (run_vaparse(forty_two, "O", &parsed_from_va) == 0 && PyErr_Occurred() != 0) ? 1 : 0;
    PyErr_Clear();

    int parse_new_features_ok =
        (PyArg_Parse(forty_two, "OO", &parsed_from_parse, &parsed_from_va) == 0 && PyErr_Occurred() != 0) ? 1 : 0;
    PyErr_Clear();

    int parse_noargs_ok = PyArg_Parse(0, "") ? 1 : 0;

    PyObject *valid_kwargs = PyDict_New();
    PyObject *valid_key = PyUnicode_FromString("alpha");
    if (!valid_kwargs || !valid_key) {
        return 0;
    }
    if (PyDict_SetItem(valid_kwargs, valid_key, forty_two) != 0) {
        return 0;
    }
    int validate_ok = PyArg_ValidateKeywordArguments(valid_kwargs) ? 1 : 0;

    PyObject *invalid_kwargs = PyDict_New();
    PyObject *invalid_key = PyLong_FromLong(1);
    if (!invalid_kwargs || !invalid_key) {
        return 0;
    }
    if (PyDict_SetItem(invalid_kwargs, invalid_key, forty_two) != 0) {
        return 0;
    }
    int validate_non_string_key_ok =
        (PyArg_ValidateKeywordArguments(invalid_kwargs) == 0 && PyErr_Occurred() != 0) ? 1 : 0;
    PyErr_Clear();

    int validate_not_dict_ok = (PyArg_ValidateKeywordArguments(forty_two) == 0 && PyErr_Occurred() != 0) ? 1 : 0;
    PyErr_Clear();

    Py_XDECREF(invalid_key);
    Py_XDECREF(invalid_kwargs);
    Py_XDECREF(valid_key);
    Py_XDECREF(valid_kwargs);
    Py_XDECREF(parsed_from_va);
    Py_XDECREF(tuple);
    Py_XDECREF(parsed_from_parse);
    Py_XDECREF(forty_two);

    if (PyModule_AddIntConstant(module, "PARSE_OK", parse_ok) != 0 ||
        PyModule_AddIntConstant(module, "PARSE_IDENTITY_OK", parse_identity_ok) != 0 ||
        PyModule_AddIntConstant(module, "VAPARSE_OK", vaparse_ok) != 0 ||
        PyModule_AddIntConstant(module, "VAPARSE_IDENTITY_OK", vaparse_identity_ok) != 0 ||
        PyModule_AddIntConstant(module, "VAPARSE_NON_TUPLE_OK", vaparse_non_tuple_ok) != 0 ||
        PyModule_AddIntConstant(module, "PARSE_NEW_FEATURES_OK", parse_new_features_ok) != 0 ||
        PyModule_AddIntConstant(module, "PARSE_NOARGS_OK", parse_noargs_ok) != 0 ||
        PyModule_AddIntConstant(module, "VALIDATE_OK", validate_ok) != 0 ||
        PyModule_AddIntConstant(module, "VALIDATE_NON_STRING_KEY_OK", validate_non_string_key_ok) != 0 ||
        PyModule_AddIntConstant(module, "VALIDATE_NOT_DICT_OK", validate_not_dict_ok) != 0) {
        return 0;
    }
    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch26_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch26 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch26_probe as m\nassert m.PARSE_OK == 1\nassert m.PARSE_IDENTITY_OK == 1\nassert m.VAPARSE_OK == 1\nassert m.VAPARSE_IDENTITY_OK == 1\nassert m.VAPARSE_NON_TUPLE_OK == 1\nassert m.PARSE_NEW_FEATURES_OK == 1\nassert m.PARSE_NOARGS_OK == 1\nassert m.VALIDATE_OK == 1\nassert m.VALIDATE_NON_STRING_KEY_OK == 1\nassert m.VALIDATE_NOT_DICT_OK == 1",
    )
    .expect("cpython api batch26 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_eval_abi_batch27_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch27 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch27 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch27");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch27_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"

static PyObject *
adder(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *left_obj = 0;
    PyObject *right_obj = 0;
    if (!PyArg_ParseTuple(args, "OO", &left_obj, &right_obj)) {
        return 0;
    }
    long long left = PyLong_AsLong(left_obj);
    long long right = PyLong_AsLong(right_obj);
    return PyLong_FromLong(left + right);
}

static PyMethodDef module_methods[] = {
    {"adder", adder, METH_VARARGS, "add two ints"},
    {0, 0, 0, 0}
};

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch27_probe",
    "cpython api batch27 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch27_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }
    if (PyModule_AddFunctions(module, module_methods) != 0) {
        return 0;
    }

    PyObject *adder_fn = PyObject_GetAttrString(module, "adder");
    if (!adder_fn) {
        return 0;
    }

    PyObject *adder_args = Py_BuildValue("(ii)", 20, 22);
    if (!adder_args) {
        return 0;
    }
    PyObject *call_obj = PyEval_CallObjectWithKeywords(adder_fn, adder_args, 0);
    long long call_obj_value = call_obj ? PyLong_AsLong(call_obj) : -1;

    PyObject *call_fn = PyEval_CallFunction(adder_fn, "(ii)", 10, 33);
    long long call_fn_value = call_fn ? PyLong_AsLong(call_fn) : -1;

    PyObject *sample = PyUnicode_FromString("abc");
    if (!sample) {
        return 0;
    }
    PyObject *method_result = PyEval_CallMethod(sample, "startswith", "s", "a");
    int method_true = method_result ? PyObject_IsTrue(method_result) : 0;

    PyEval_InitThreads();
    int threads_initialized = PyEval_ThreadsInitialized();
    PyEval_AcquireLock();
    PyEval_ReleaseLock();
    PyEval_AcquireThread(0);
    PyEval_ReleaseThread(0);

    Py_XDECREF(method_result);
    Py_XDECREF(sample);
    Py_XDECREF(call_fn);
    Py_XDECREF(call_obj);
    Py_XDECREF(adder_args);
    Py_XDECREF(adder_fn);

    int call_object_ok = (call_obj_value == 42) ? 1 : 0;
    int call_function_ok = (call_fn_value == 43) ? 1 : 0;
    int call_method_ok = (method_true == 1) ? 1 : 0;

    if (PyModule_AddIntConstant(module, "CALL_OBJECT_OK", call_object_ok) != 0 ||
        PyModule_AddIntConstant(module, "CALL_FUNCTION_OK", call_function_ok) != 0 ||
        PyModule_AddIntConstant(module, "CALL_METHOD_OK", call_method_ok) != 0 ||
        PyModule_AddIntConstant(module, "THREADS_INIT_OK", threads_initialized == 1 ? 1 : 0) != 0) {
        return 0;
    }
    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch27_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch27 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch27_probe as m\nassert m.CALL_OBJECT_OK == 1\nassert m.CALL_FUNCTION_OK == 1\nassert m.CALL_METHOD_OK == 1\nassert m.THREADS_INIT_OK == 1",
    )
    .expect("cpython api batch27 extension import should succeed");

    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn cpython_compat_bytes_abi_batch28_apis_work() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping cpython api batch28 smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping cpython api batch28 smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_cpython_api_batch28");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("cpython_api_batch28_probe.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_cpython_compat.h"
#include <string.h>

static PyObject *
call_from_format_v(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    PyObject *result = PyBytes_FromFormatV(fmt, ap);
    va_end(ap);
    return result;
}

static struct PyModuleDef module_def = {
    PyModuleDef_HEAD_INIT,
    "cpython_api_batch28_probe",
    "cpython api batch28 probe module",
    -1,
    0,
    0,
    0,
    0,
    0
};

PyMODINIT_FUNC
PyInit_cpython_api_batch28_probe(void) {
    PyObject *module = PyModule_Create(&module_def);
    if (!module) {
        return 0;
    }

    PyObject *formatted = PyBytes_FromFormat(
        "x=%d y=%u z=%ld q=%lu a=%zd b=%zu i=%i h=%x c=%c p=%p %% s=%s",
        -3,
        (unsigned int)5,
        (long)-7,
        (unsigned long)9,
        (long long)-11,
        (size_t)13,
        17,
        31,
        (int)'A',
        (void *)module,
        "ok"
    );
    const char *formatted_text = formatted ? PyBytes_AsString(formatted) : 0;
    int formatted_ok = 0;
    if (formatted_text) {
        formatted_ok =
            strstr(formatted_text, "x=-3") &&
            strstr(formatted_text, "y=5") &&
            strstr(formatted_text, "z=-7") &&
            strstr(formatted_text, "q=9") &&
            strstr(formatted_text, "a=-11") &&
            strstr(formatted_text, "b=13") &&
            strstr(formatted_text, "i=17") &&
            strstr(formatted_text, "h=1f") &&
            strstr(formatted_text, "c=A") &&
            strstr(formatted_text, "0x") &&
            strstr(formatted_text, "% s=ok");
    }

    PyObject *formatted_v = call_from_format_v("%s:%d", "va", 42);
    const char *formatted_v_text = formatted_v ? PyBytes_AsString(formatted_v) : 0;
    int formatted_v_ok = (formatted_v_text && strcmp(formatted_v_text, "va:42") == 0) ? 1 : 0;

    PyObject *unknown_spec = PyBytes_FromFormat("bad%Qtail", 1234);
    const char *unknown_text = unknown_spec ? PyBytes_AsString(unknown_spec) : 0;
    int unknown_spec_ok = (unknown_text && strcmp(unknown_text, "bad%Qtail") == 0) ? 1 : 0;

    PyObject *bytes_for_repr = PyBytes_FromString("a'b");
    PyObject *repr_single = bytes_for_repr ? PyBytes_Repr(bytes_for_repr, 0) : 0;
    PyObject *repr_smart = bytes_for_repr ? PyBytes_Repr(bytes_for_repr, 1) : 0;
    const char *repr_single_text = repr_single ? PyUnicode_AsUTF8(repr_single) : 0;
    const char *repr_smart_text = repr_smart ? PyUnicode_AsUTF8(repr_smart) : 0;
    int repr_ok =
        repr_single_text && repr_smart_text &&
        strcmp(repr_single_text, "b'a\\'b'") == 0 &&
        strcmp(repr_smart_text, "b\"a'b\"") == 0;

    const char *esc = "\\n\\x41\\101\\q";
    PyObject *decoded = PyBytes_DecodeEscape(esc, (long long)strlen(esc), 0, 0, 0);
    const char *decoded_text = decoded ? PyBytes_AsString(decoded) : 0;
    long long decoded_len = decoded ? PyBytes_Size(decoded) : -1;
    int decode_ok =
        decoded_text && decoded_len == 4 &&
        decoded_text[0] == '\n' &&
        decoded_text[1] == 'A' &&
        decoded_text[2] == 'A' &&
        decoded_text[3] == 'q';

    const char *bad_escape = "\\xZ";
    PyObject *decode_strict = PyBytes_DecodeEscape(
        bad_escape,
        (long long)strlen(bad_escape),
        "strict",
        0,
        0
    );
    int decode_strict_ok = (decode_strict == 0 && PyErr_Occurred() != 0) ? 1 : 0;
    PyErr_Clear();

    PyObject *decode_replace = PyBytes_DecodeEscape(
        bad_escape,
        (long long)strlen(bad_escape),
        "replace",
        0,
        0
    );
    const char *decode_replace_text = decode_replace ? PyBytes_AsString(decode_replace) : 0;
    int decode_replace_ok = decode_replace_text && strcmp(decode_replace_text, "?Z") == 0;

    Py_XDECREF(decode_replace);
    Py_XDECREF(decode_strict);
    Py_XDECREF(decoded);
    Py_XDECREF(repr_smart);
    Py_XDECREF(repr_single);
    Py_XDECREF(bytes_for_repr);
    Py_XDECREF(unknown_spec);
    Py_XDECREF(formatted_v);
    Py_XDECREF(formatted);

    if (PyModule_AddIntConstant(module, "FORMATTED_OK", formatted_ok ? 1 : 0) != 0 ||
        PyModule_AddIntConstant(module, "FORMATTED_V_OK", formatted_v_ok) != 0 ||
        PyModule_AddIntConstant(module, "UNKNOWN_SPEC_OK", unknown_spec_ok) != 0 ||
        PyModule_AddIntConstant(module, "REPR_OK", repr_ok ? 1 : 0) != 0 ||
        PyModule_AddIntConstant(module, "DECODE_OK", decode_ok ? 1 : 0) != 0 ||
        PyModule_AddIntConstant(module, "DECODE_STRICT_OK", decode_strict_ok) != 0 ||
        PyModule_AddIntConstant(module, "DECODE_REPLACE_OK", decode_replace_ok ? 1 : 0) != 0) {
        return 0;
    }
    return module;
}
"#,
    )
    .expect("source should be written");

    let library_path = temp_root.join(importable_module_library_filename(
        "cpython_api_batch28_probe",
    ));
    compile_shared_extension_with_cpython_compat(&source_path, &library_path)
        .expect("cpython api batch28 extension should build");

    run_import_snippet(
        &bin,
        &temp_root,
        "import cpython_api_batch28_probe as m\nassert m.FORMATTED_OK == 1\nassert m.FORMATTED_V_OK == 1\nassert m.UNKNOWN_SPEC_OK == 1\nassert m.REPR_OK == 1\nassert m.DECODE_OK == 1\nassert m.DECODE_STRICT_OK == 1\nassert m.DECODE_REPLACE_OK == 1",
    )
    .expect("cpython api batch28 extension import should succeed");

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
    const uint8_t payload[] = {104, 105}; /* b"hi" */
    PyrsObjectHandle answer = api->object_new_int(module_ctx, 99);
    PyrsObjectHandle none_value = api->object_new_none(module_ctx);
    PyrsObjectHandle ratio = api->object_new_float(module_ctx, 3.5);
    PyrsObjectHandle blob = api->object_new_bytes(module_ctx, payload, 2);
    PyrsObjectHandle blob_mut = api->object_new_bytearray(module_ctx, payload, 2);
    PyrsObjectHandle blob_view = api->object_new_memoryview(module_ctx, blob_mut);
    PyrsObjectHandle sequence_items[2];
    sequence_items[0] = answer;
    sequence_items[1] = ratio;
    PyrsObjectHandle pair_tuple = api->object_new_tuple(module_ctx, 2, sequence_items);
    PyrsObjectHandle pair_list = api->object_new_list(module_ctx, 2, sequence_items);
    PyrsObjectHandle mapping = api->object_new_dict(module_ctx);
    PyrsObjectHandle key_ratio = api->object_new_string(module_ctx, "ratio");
    PyrsObjectHandle text = api->object_new_string(module_ctx, "from-object-handle");
    if (!answer || !none_value || !ratio || !blob || !blob_mut || !blob_view || !pair_tuple || !pair_list || !mapping || !key_ratio || !text) {
        return -2;
    }
    double ratio_check = 0.0;
    if (api->object_get_float(module_ctx, ratio, &ratio_check) != 0 || ratio_check != 3.5) {
        return -9;
    }
    const uint8_t* blob_data = 0;
    uintptr_t blob_len = 0;
    if (api->object_get_bytes(module_ctx, blob, &blob_data, &blob_len) != 0 ||
        blob_len != 2 || !blob_data || blob_data[0] != 104 || blob_data[1] != 105) {
        return -14;
    }
    const uint8_t* blob_mut_data = 0;
    uintptr_t blob_mut_len = 0;
    if (api->object_get_bytes(module_ctx, blob_mut, &blob_mut_data, &blob_mut_len) != 0 ||
        blob_mut_len != 2 || !blob_mut_data || blob_mut_data[0] != 104 || blob_mut_data[1] != 105) {
        return -15;
    }
    PyrsBufferViewV1 blob_view_data;
    if (api->object_get_buffer(module_ctx, blob_view, &blob_view_data) != 0 ||
        blob_view_data.len != 2 || blob_view_data.readonly != 0 ||
        !blob_view_data.data || blob_view_data.data[0] != 104 || blob_view_data.data[1] != 105) {
        return -51;
    }
    if (api->object_release_buffer(module_ctx, blob_view) != 0) {
        return -52;
    }
    PyrsObjectHandle invalid_view = api->object_new_memoryview(module_ctx, answer);
    if (invalid_view != 0 || api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -53;
    }
    uintptr_t tuple_len = 0;
    if (api->object_sequence_len(module_ctx, pair_tuple, &tuple_len) != 0 || tuple_len != 2) {
        return -17;
    }
    if (api->object_list_append(module_ctx, pair_list, answer) != 0) {
        return -18;
    }
    if (api->object_list_set_item(module_ctx, pair_list, 0, ratio) != 0) {
        return -19;
    }
    if (api->object_list_set_item(module_ctx, pair_list, 99, ratio) == 0) {
        return -20;
    }
    if (api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -21;
    }
    uintptr_t pair_list_len = 0;
    if (api->object_sequence_len(module_ctx, pair_list, &pair_list_len) != 0 || pair_list_len != 3) {
        return -22;
    }
    PyrsObjectHandle list_first = 0;
    if (api->object_sequence_get_item(module_ctx, pair_list, 0, &list_first) != 0 || !list_first) {
        return -23;
    }
    double list_first_float = 0.0;
    if (api->object_get_float(module_ctx, list_first, &list_first_float) != 0 || list_first_float != 3.5) {
        return -24;
    }
    if (api->object_decref(module_ctx, list_first) != 0) {
        return -25;
    }
    PyrsObjectHandle list_third = 0;
    if (api->object_sequence_get_item(module_ctx, pair_list, 2, &list_third) != 0 || !list_third) {
        return -26;
    }
    int64_t list_third_int = 0;
    if (api->object_get_int(module_ctx, list_third, &list_third_int) != 0 || list_third_int != 99) {
        return -27;
    }
    if (api->object_decref(module_ctx, list_third) != 0) {
        return -28;
    }
    PyrsObjectHandle list_second = 0;
    if (api->object_sequence_get_item(module_ctx, pair_list, 1, &list_second) != 0 || !list_second) {
        return -29;
    }
    double list_second_float = 0.0;
    if (api->object_get_float(module_ctx, list_second, &list_second_float) != 0 || list_second_float != 3.5) {
        return -30;
    }
    if (api->object_decref(module_ctx, list_second) != 0) {
        return -31;
    }
    if (api->object_dict_set_item(module_ctx, mapping, key_ratio, ratio) != 0) {
        return -32;
    }
    if (api->object_dict_contains(module_ctx, mapping, key_ratio) != 1) {
        return -33;
    }
    if (api->object_dict_del_item(module_ctx, mapping, key_ratio) != 0) {
        return -34;
    }
    if (api->object_dict_del_item(module_ctx, mapping, key_ratio) == 0) {
        return -35;
    }
    if (api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -36;
    }
    if (api->object_dict_contains(module_ctx, mapping, key_ratio) != 0) {
        return -37;
    }
    uintptr_t mapping_len = 0;
    if (api->object_dict_len(module_ctx, mapping, &mapping_len) != 0 || mapping_len != 0) {
        return -38;
    }
    if (api->object_dict_set_item(module_ctx, mapping, key_ratio, ratio) != 0) {
        return -39;
    }
    if (api->object_dict_contains(module_ctx, mapping, key_ratio) != 1) {
        return -40;
    }
    if (api->object_dict_len(module_ctx, mapping, &mapping_len) != 0 || mapping_len != 1) {
        return -41;
    }
    PyrsObjectHandle fetched_ratio = 0;
    if (api->object_dict_get_item(module_ctx, mapping, key_ratio, &fetched_ratio) != 0 || !fetched_ratio) {
        return -42;
    }
    double fetched_ratio_value = 0.0;
    if (api->object_get_float(module_ctx, fetched_ratio, &fetched_ratio_value) != 0 || fetched_ratio_value != 3.5) {
        return -43;
    }
    if (api->object_decref(module_ctx, fetched_ratio) != 0) {
        return -44;
    }
    if (api->module_set_object(module_ctx, "ANSWER", answer) != 0) {
        return -3;
    }
    PyrsObjectHandle module_answer = 0;
    if (api->module_get_object(module_ctx, "ANSWER", &module_answer) != 0 || !module_answer) {
        return -48;
    }
    int64_t module_answer_int = 0;
    if (api->object_get_int(module_ctx, module_answer, &module_answer_int) != 0 || module_answer_int != 99) {
        return -49;
    }
    if (api->object_decref(module_ctx, module_answer) != 0) {
        return -50;
    }
    if (api->module_set_object(module_ctx, "NONE_VALUE", none_value) != 0) {
        return -10;
    }
    if (api->module_set_object(module_ctx, "RATIO", ratio) != 0) {
        return -11;
    }
    if (api->module_set_object(module_ctx, "BLOB", blob) != 0) {
        return -15;
    }
    if (api->module_set_object(module_ctx, "BLOB_VIEW", blob_view) != 0) {
        return -54;
    }
    if (api->module_set_object(module_ctx, "PAIR_TUPLE", pair_tuple) != 0) {
        return -21;
    }
    if (api->module_set_object(module_ctx, "PAIR_LIST", pair_list) != 0) {
        return -22;
    }
    if (api->module_set_object(module_ctx, "MAPPING", mapping) != 0) {
        return -45;
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
    if (api->object_decref(module_ctx, blob) != 0) {
        return -16;
    }
    if (api->object_decref(module_ctx, blob_mut) != 0) {
        return -55;
    }
    if (api->object_decref(module_ctx, blob_view) != 0) {
        return -56;
    }
    if (api->object_decref(module_ctx, pair_tuple) != 0) {
        return -23;
    }
    if (api->object_decref(module_ctx, pair_list) != 0) {
        return -24;
    }
    if (api->object_decref(module_ctx, mapping) != 0) {
        return -46;
    }
    if (api->object_decref(module_ctx, key_ratio) != 0) {
        return -47;
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
        "import native_handles\nassert native_handles.ANSWER == 99\nassert native_handles.NONE_VALUE is None\nassert abs(native_handles.RATIO - 3.5) < 1e-12\nassert native_handles.BLOB == b'hi'\nassert bytes(native_handles.BLOB_VIEW) == b'hi'\nassert native_handles.PAIR_TUPLE == (99, 3.5)\nassert native_handles.PAIR_LIST == [3.5, 3.5, 99]\nassert native_handles.MAPPING['ratio'] == 3.5\nassert native_handles.TEXT == 'from-object-handle'",
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
        "import native_callable\nassert native_callable.API_KIND == 'callable'\nassert native_callable.add(20, 22) == 42\nraised = False\ntry:\n    native_callable.add(20, 22, extra=1)\nexcept RuntimeError as exc:\n    raised = True\n    assert 'does not accept keyword arguments' in str(exc)\nassert raised",
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
        "import native_kw_callable\nassert native_kw_callable.API_KIND == 'kw-callable'\nassert native_kw_callable.add_scaled(2, 3) == 5\nassert native_kw_callable.add_scaled(2, 3, scale=10) == 50\nraised_unknown = False\ntry:\n    native_kw_callable.add_scaled(2, 3, bad=1)\nexcept RuntimeError as exc:\n    raised_unknown = True\n    assert \"only accepts keyword 'scale'\" in str(exc)\nassert raised_unknown\nraised_type = False\ntry:\n    native_kw_callable.add_scaled(2, 3, scale='x')\nexcept RuntimeError as exc:\n    raised_type = True\n    assert \"keyword 'scale' must be int\" in str(exc)\nassert raised_type",
    )
    .expect("kw-callable extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_call_python_callable_handles() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping object-call extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping object-call extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_object_call");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_object_call.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int native_invoke(
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
        api->error_set(module_ctx, "invoke expects callable and one positional argument");
        return -2;
    }
    if (api->object_call(module_ctx, argv[0], 1, &argv[1], kwargc, kwarg_names, kwarg_values, result) != 0) {
        if (api->error_occurred(module_ctx) == 0) {
            api->error_set(module_ctx, "object_call failed");
        }
        return -3;
    }
    if (!*result) {
        api->error_set(module_ctx, "object_call returned null result handle");
        return -4;
    }
    return 0;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_add_function_kw(module_ctx, "invoke", native_invoke) != 0) {
        return -2;
    }
    if (api->module_set_string(module_ctx, "API_KIND", "object-call") != 0) {
        return -3;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_object_call");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_object_call.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_object_call\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_object_call\nassert native_object_call.API_KIND == 'object-call'\ndef py_scale(value, scale=1):\n    return value * scale\nassert native_object_call.invoke(py_scale, 7) == 7\nassert native_object_call.invoke(py_scale, 7, scale=3) == 21\nraised_non_callable = False\ntry:\n    native_object_call.invoke(42, 7)\nexcept Exception:\n    raised_non_callable = True\nassert raised_non_callable\ndef boom(value, scale=1):\n    raise ValueError('boom')\nraised_inner = False\ntry:\n    native_object_call.invoke(boom, 1)\nexcept Exception:\n    raised_inner = True\nassert raised_inner",
    )
    .expect("object-call extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_use_object_call_fastpaths() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping call-fastpath extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping call-fastpath extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_call_fastpaths");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_call_fastpaths.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int invoke0(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    if (!api || !argv || !result) {
        return -1;
    }
    if (argc != 1) {
        api->error_set(module_ctx, "invoke0 expects one argument");
        return -2;
    }
    if (api->object_call_noargs(module_ctx, argv[0], result) != 0) {
        if (api->error_occurred(module_ctx) == 0) {
            api->error_set(module_ctx, "object_call_noargs failed");
        }
        return -3;
    }
    return 0;
}

int invoke1(
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
        api->error_set(module_ctx, "invoke1 expects two arguments");
        return -2;
    }
    if (api->object_call_onearg(module_ctx, argv[0], argv[1], result) != 0) {
        if (api->error_occurred(module_ctx) == 0) {
            api->error_set(module_ctx, "object_call_onearg failed");
        }
        return -3;
    }
    return 0;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_add_function(module_ctx, "invoke0", invoke0) != 0) {
        return -2;
    }
    if (api->module_add_function(module_ctx, "invoke1", invoke1) != 0) {
        return -3;
    }
    if (api->module_set_string(module_ctx, "API_KIND", "call-fastpaths") != 0) {
        return -4;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_call_fastpaths");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_call_fastpaths.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_call_fastpaths\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_call_fastpaths\nassert native_call_fastpaths.API_KIND == 'call-fastpaths'\ndef f0():\n    return 9\ndef f1(x):\n    return x * 2\nassert native_call_fastpaths.invoke0(f0) == 9\nassert native_call_fastpaths.invoke1(f1, 5) == 10\nraised = False\ntry:\n    native_call_fastpaths.invoke0(123)\nexcept Exception:\n    raised = True\nassert raised",
    )
    .expect("call-fastpath extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_read_and_clear_error_message() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping error-message extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping error-message extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_error_message");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_error_message.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    PyrsObjectHandle value = api->object_new_int(module_ctx, 1);
    if (!value) {
        return -2;
    }
    PyrsObjectHandle out = 0;
    if (api->object_get_attr(module_ctx, value, "missing_attr", &out) == 0) {
        return -3;
    }
    if (api->error_occurred(module_ctx) == 0) {
        return -4;
    }
    const char* message = api->error_get_message(module_ctx);
    if (!message || !message[0]) {
        return -5;
    }
    if (api->module_set_string(module_ctx, "ERRMSG", message) != 0) {
        return -6;
    }
    if (api->error_clear(module_ctx) != 0) {
        return -7;
    }
    if (api->error_occurred(module_ctx) != 0) {
        return -8;
    }
    if (api->error_get_message(module_ctx) != 0) {
        return -9;
    }
    if (api->module_set_bool(module_ctx, "ERROR_CLEARED", 1) != 0) {
        return -10;
    }
    if (api->object_decref(module_ctx, value) != 0) {
        return -11;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_error_message");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_error_message.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_error_message\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_error_message\nassert native_error_message.ERROR_CLEARED is True\nassert isinstance(native_error_message.ERRMSG, str)\nassert len(native_error_message.ERRMSG) > 0",
    )
    .expect("error-message extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_get_set_and_del_object_attributes() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping object-attr extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping object-attr extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_object_attr");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_object_attr.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int native_touch(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    if (!api || !argv || !result) {
        return -1;
    }
    if (argc != 1) {
        api->error_set(module_ctx, "touch expects one argument");
        return -2;
    }
    if (api->object_has_attr(module_ctx, argv[0], "ext_value") != 0) {
        return -13;
    }

    PyrsObjectHandle marker = api->object_new_int(module_ctx, 123);
    if (!marker) {
        return -3;
    }
    if (api->object_set_attr(module_ctx, argv[0], "ext_value", marker) != 0) {
        return -4;
    }
    if (api->object_has_attr(module_ctx, argv[0], "ext_value") != 1) {
        return -14;
    }
    if (api->object_decref(module_ctx, marker) != 0) {
        return -5;
    }

    PyrsObjectHandle fetched = 0;
    if (api->object_get_attr(module_ctx, argv[0], "ext_value", &fetched) != 0 || !fetched) {
        return -6;
    }
    int64_t fetched_int = 0;
    if (api->object_get_int(module_ctx, fetched, &fetched_int) != 0 || fetched_int != 123) {
        return -7;
    }
    if (api->object_decref(module_ctx, fetched) != 0) {
        return -8;
    }

    if (api->object_del_attr(module_ctx, argv[0], "ext_value") != 0) {
        return -9;
    }
    if (api->object_has_attr(module_ctx, argv[0], "ext_value") != 0) {
        return -15;
    }
    if (api->object_get_attr(module_ctx, argv[0], "ext_value", &fetched) == 0) {
        return -10;
    }
    if (api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -11;
    }

    *result = api->object_new_bool(module_ctx, 1);
    return *result ? 0 : -12;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_add_function(module_ctx, "touch", native_touch) != 0) {
        return -2;
    }
    if (api->module_set_string(module_ctx, "API_KIND", "object-attr") != 0) {
        return -3;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_object_attr");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_object_attr.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_object_attr\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_object_attr\nassert native_object_attr.API_KIND == 'object-attr'\nclass Box:\n    pass\nbox = Box()\nassert native_object_attr.touch(box) is True\nassert not hasattr(box, 'ext_value')",
    )
    .expect("object-attr extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_import_module_and_export_attribute() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping module-import extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping module-import extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_module_import");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_module_import.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    PyrsObjectHandle math_mod = 0;
    if (api->module_import(module_ctx, "math", &math_mod) != 0 || !math_mod) {
        return -2;
    }
    PyrsObjectHandle pi_value = 0;
    if (api->module_get_attr(module_ctx, math_mod, "pi", &pi_value) != 0 || !pi_value) {
        return -3;
    }
    PyrsObjectHandle bogus = 0;
    if (api->module_get_attr(module_ctx, pi_value, "pi", &bogus) == 0) {
        return -8;
    }
    if (api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -9;
    }
    if (api->module_set_object(module_ctx, "PI", pi_value) != 0) {
        return -4;
    }
    if (api->object_decref(module_ctx, pi_value) != 0) {
        return -5;
    }
    if (api->object_decref(module_ctx, math_mod) != 0) {
        return -6;
    }
    if (api->module_set_string(module_ctx, "API_KIND", "module-import") != 0) {
        return -7;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_module_import");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_module_import.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_module_import\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_module_import\nassert native_module_import.API_KIND == 'module-import'\nassert abs(native_module_import.PI - 3.141592653589793) < 1e-12",
    )
    .expect("module-import extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_check_isinstance_and_issubclass() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping type-check extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping type-check extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_type_checks");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_type_checks.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int native_probe(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    if (!api || !argv || !result) {
        return -1;
    }
    if (argc != 3) {
        api->error_set(module_ctx, "probe expects (obj, cls, base)");
        return -2;
    }
    int is_inst = api->object_is_instance(module_ctx, argv[0], argv[1]);
    if (is_inst < 0) {
        return -3;
    }
    int is_sub = api->object_is_subclass(module_ctx, argv[1], argv[2]);
    if (is_sub < 0) {
        return -4;
    }
    *result = api->object_new_int(module_ctx, (is_inst * 10) + is_sub);
    return *result ? 0 : -5;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_add_function(module_ctx, "probe", native_probe) != 0) {
        return -2;
    }
    if (api->module_set_string(module_ctx, "API_KIND", "type-checks") != 0) {
        return -3;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_type_checks");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_type_checks.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_type_checks\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_type_checks\nassert native_type_checks.API_KIND == 'type-checks'\nclass A: pass\nclass B(A): pass\nassert native_type_checks.probe(B(), B, A) == 11\nassert native_type_checks.probe(A(), B, A) == 1\nassert native_type_checks.probe(A(), A, B) == 10\nraised = False\ntry:\n    native_type_checks.probe(1, 1, object)\nexcept Exception:\n    raised = True\nassert raised",
    )
    .expect("type-check extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_mixed_surface_roundtrip() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping mixed-surface extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping mixed-surface extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_mixed_surface");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_mixed_surface.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    PyrsObjectHandle math_mod = 0;
    if (api->module_import(module_ctx, "math", &math_mod) != 0 || !math_mod) {
        return -2;
    }
    PyrsObjectHandle pi = 0;
    if (api->module_get_attr(module_ctx, math_mod, "pi", &pi) != 0 || !pi) {
        return -3;
    }
    if (api->module_set_object(module_ctx, "PI", pi) != 0) {
        return -4;
    }
    PyrsObjectHandle pi_roundtrip = 0;
    if (api->module_get_object(module_ctx, "PI", &pi_roundtrip) != 0 || !pi_roundtrip) {
        return -5;
    }
    if (api->object_type(module_ctx, pi_roundtrip) != PYRS_TYPE_FLOAT) {
        return -6;
    }

    PyrsObjectHandle builtins_mod = 0;
    if (api->module_import(module_ctx, "builtins", &builtins_mod) != 0 || !builtins_mod) {
        return -7;
    }
    PyrsObjectHandle len_fn = 0;
    if (api->module_get_attr(module_ctx, builtins_mod, "len", &len_fn) != 0 || !len_fn) {
        return -8;
    }
    PyrsObjectHandle list_items[1];
    list_items[0] = pi;
    PyrsObjectHandle value_list = api->object_new_list(module_ctx, 1, list_items);
    if (!value_list) {
        return -9;
    }
    PyrsObjectHandle zero = api->object_new_int(module_ctx, 0);
    if (!zero) {
        return -10;
    }
    if (api->object_list_append(module_ctx, value_list, zero) != 0) {
        return -11;
    }
    PyrsObjectHandle len_args[1];
    len_args[0] = value_list;
    PyrsObjectHandle len_value = 0;
    if (api->object_call(module_ctx, len_fn, 1, len_args, 0, 0, 0, &len_value) != 0 || !len_value) {
        return -12;
    }
    int64_t len_int = 0;
    if (api->object_get_int(module_ctx, len_value, &len_int) != 0 || len_int != 2) {
        return -13;
    }
    PyrsObjectHandle float_cls = 0;
    if (api->module_get_attr(module_ctx, builtins_mod, "float", &float_cls) != 0 || !float_cls) {
        return -14;
    }
    if (api->object_is_instance(module_ctx, pi, float_cls) != 1) {
        return -15;
    }
    if (api->object_is_subclass(module_ctx, float_cls, float_cls) != 1) {
        return -16;
    }

    PyrsObjectHandle mapping = api->object_new_dict(module_ctx);
    if (!mapping) {
        return -17;
    }
    PyrsObjectHandle key_pi = api->object_new_string(module_ctx, "pi");
    if (!key_pi) {
        return -18;
    }
    if (api->object_dict_set_item(module_ctx, mapping, key_pi, pi) != 0) {
        return -19;
    }
    if (api->object_dict_contains(module_ctx, mapping, key_pi) != 1) {
        return -20;
    }
    PyrsObjectHandle fetched = 0;
    if (api->object_dict_get_item(module_ctx, mapping, key_pi, &fetched) != 0 || !fetched) {
        return -21;
    }
    if (api->object_type(module_ctx, fetched) != PYRS_TYPE_FLOAT) {
        return -22;
    }
    if (api->object_decref(module_ctx, fetched) != 0) {
        return -23;
    }
    if (api->object_dict_del_item(module_ctx, mapping, key_pi) != 0) {
        return -24;
    }
    if (api->object_dict_contains(module_ctx, mapping, key_pi) != 0) {
        return -25;
    }

    if (api->module_set_bool(module_ctx, "MIXED_OK", 1) != 0) {
        return -26;
    }
    if (api->module_set_int(module_ctx, "LEN_VALUE", len_int) != 0) {
        return -27;
    }
    if (api->module_set_string(module_ctx, "API_KIND", "mixed-surface") != 0) {
        return -28;
    }

    if (api->object_decref(module_ctx, key_pi) != 0) {
        return -29;
    }
    if (api->object_decref(module_ctx, mapping) != 0) {
        return -30;
    }
    if (api->object_decref(module_ctx, float_cls) != 0) {
        return -31;
    }
    if (api->object_decref(module_ctx, len_value) != 0) {
        return -32;
    }
    if (api->object_decref(module_ctx, zero) != 0) {
        return -33;
    }
    if (api->object_decref(module_ctx, value_list) != 0) {
        return -34;
    }
    if (api->object_decref(module_ctx, len_fn) != 0) {
        return -35;
    }
    if (api->object_decref(module_ctx, builtins_mod) != 0) {
        return -36;
    }
    if (api->object_decref(module_ctx, pi_roundtrip) != 0) {
        return -37;
    }
    if (api->object_decref(module_ctx, pi) != 0) {
        return -38;
    }
    if (api->object_decref(module_ctx, math_mod) != 0) {
        return -39;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_mixed_surface");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_mixed_surface.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_mixed_surface\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_mixed_surface\nassert native_mixed_surface.API_KIND == 'mixed-surface'\nassert native_mixed_surface.MIXED_OK is True\nassert native_mixed_surface.LEN_VALUE == 2\nassert abs(native_mixed_surface.PI - 3.141592653589793) < 1e-12",
    )
    .expect("mixed-surface extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_iterate_with_iterator_apis() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping iterator-api extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping iterator-api extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_iterator_api");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_iterator_api.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int sum_iterable(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    if (!api || !argv || !result) {
        return -1;
    }
    if (argc != 1) {
        api->error_set(module_ctx, "sum_iterable expects one argument");
        return -2;
    }
    PyrsObjectHandle iter_handle = 0;
    if (api->object_get_iter(module_ctx, argv[0], &iter_handle) != 0 || !iter_handle) {
        return -3;
    }
    int64_t total = 0;
    for (;;) {
        PyrsObjectHandle item = 0;
        int next_status = api->object_iter_next(module_ctx, iter_handle, &item);
        if (next_status == 0) {
            break;
        }
        if (next_status < 0 || !item) {
            api->object_decref(module_ctx, iter_handle);
            return -4;
        }
        int64_t value = 0;
        if (api->object_get_int(module_ctx, item, &value) != 0) {
            api->object_decref(module_ctx, item);
            api->object_decref(module_ctx, iter_handle);
            return -5;
        }
        total += value;
        if (api->object_decref(module_ctx, item) != 0) {
            api->object_decref(module_ctx, iter_handle);
            return -6;
        }
    }
    if (api->object_decref(module_ctx, iter_handle) != 0) {
        return -7;
    }
    *result = api->object_new_int(module_ctx, total);
    return *result ? 0 : -8;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_add_function(module_ctx, "sum_iterable", sum_iterable) != 0) {
        return -2;
    }
    if (api->module_set_string(module_ctx, "API_KIND", "iterator-api") != 0) {
        return -3;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_iterator_api");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_iterator_api.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_iterator_api\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_iterator_api\nassert native_iterator_api.API_KIND == 'iterator-api'\nassert native_iterator_api.sum_iterable([1, 2, 3, 4]) == 10\nassert native_iterator_api.sum_iterable((5, 6)) == 11\nraised = False\ntry:\n    native_iterator_api.sum_iterable(42)\nexcept Exception:\n    raised = True\nassert raised",
    )
    .expect("iterator-api extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_use_len_and_getitem_apis() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping len/getitem extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping len/getitem extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_len_getitem");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_len_getitem.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int len_plus_item(
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
        api->error_set(module_ctx, "len_plus_item expects two arguments");
        return -2;
    }
    uintptr_t len_out = 0;
    if (api->object_len(module_ctx, argv[0], &len_out) != 0) {
        return -3;
    }
    PyrsObjectHandle item = 0;
    if (api->object_get_item(module_ctx, argv[0], argv[1], &item) != 0 || !item) {
        return -4;
    }
    int64_t item_int = 0;
    if (api->object_get_int(module_ctx, item, &item_int) != 0) {
        api->object_decref(module_ctx, item);
        return -5;
    }
    if (api->object_decref(module_ctx, item) != 0) {
        return -6;
    }
    *result = api->object_new_int(module_ctx, (int64_t)len_out + item_int);
    return *result ? 0 : -7;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_add_function(module_ctx, "len_plus_item", len_plus_item) != 0) {
        return -2;
    }
    if (api->module_set_string(module_ctx, "API_KIND", "len-getitem") != 0) {
        return -3;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_len_getitem");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_len_getitem.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_len_getitem\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_len_getitem\nassert native_len_getitem.API_KIND == 'len-getitem'\nassert native_len_getitem.len_plus_item([10, 20, 30], 1) == 23\nassert native_len_getitem.len_plus_item({'a': 5, 'b': 7}, 'b') == 9\nraised = False\ntry:\n    native_len_getitem.len_plus_item(42, 0)\nexcept Exception:\n    raised = True\nassert raised",
    )
    .expect("len/getitem extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_invalid_handles_report_errors_consistently() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping invalid-handle extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping invalid-handle extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_invalid_handles");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_invalid_handles.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int expect_error(const PyrsApiV1* api, void* module_ctx) {
    const char* msg = api->error_get_message(module_ctx);
    if (!msg || !msg[0]) {
        return 0;
    }
    if (api->error_clear(module_ctx) != 0) {
        return 0;
    }
    return api->error_occurred(module_ctx) == 0;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    PyrsObjectHandle invalid = 999999;
    int64_t int_out = 0;
    uintptr_t len_out = 0;
    PyrsObjectHandle handle_out = 0;

    if (api->object_get_int(module_ctx, invalid, &int_out) == 0 || !expect_error(api, module_ctx)) {
        return -2;
    }
    if (api->object_len(module_ctx, invalid, &len_out) == 0 || !expect_error(api, module_ctx)) {
        return -17;
    }
    if (api->object_get_item(module_ctx, invalid, invalid, &handle_out) == 0 || !expect_error(api, module_ctx)) {
        return -18;
    }
    if (api->object_sequence_len(module_ctx, invalid, &len_out) == 0 || !expect_error(api, module_ctx)) {
        return -3;
    }
    if (api->object_get_iter(module_ctx, invalid, &handle_out) == 0 || !expect_error(api, module_ctx)) {
        return -15;
    }
    if (api->object_iter_next(module_ctx, invalid, &handle_out) >= 0 || !expect_error(api, module_ctx)) {
        return -16;
    }
    if (api->object_dict_len(module_ctx, invalid, &len_out) == 0 || !expect_error(api, module_ctx)) {
        return -4;
    }
    if (api->object_get_attr(module_ctx, invalid, "x", &handle_out) == 0 || !expect_error(api, module_ctx)) {
        return -5;
    }
    if (api->object_set_attr(module_ctx, invalid, "x", invalid) == 0 || !expect_error(api, module_ctx)) {
        return -6;
    }
    if (api->object_del_attr(module_ctx, invalid, "x") == 0 || !expect_error(api, module_ctx)) {
        return -7;
    }
    if (api->object_has_attr(module_ctx, invalid, "x") >= 0 || !expect_error(api, module_ctx)) {
        return -8;
    }
    if (api->object_call_noargs(module_ctx, invalid, &handle_out) == 0 || !expect_error(api, module_ctx)) {
        return -9;
    }
    if (api->object_call_onearg(module_ctx, invalid, invalid, &handle_out) == 0 || !expect_error(api, module_ctx)) {
        return -10;
    }
    if (api->module_get_attr(module_ctx, invalid, "x", &handle_out) == 0 || !expect_error(api, module_ctx)) {
        return -11;
    }
    if (api->module_get_object(module_ctx, "does_not_exist", &handle_out) == 0 || !expect_error(api, module_ctx)) {
        return -12;
    }

    if (api->module_set_bool(module_ctx, "INVALID_HANDLE_CHECKS_OK", 1) != 0) {
        return -13;
    }
    if (api->module_set_string(module_ctx, "API_KIND", "invalid-handles") != 0) {
        return -14;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_invalid_handles");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_invalid_handles.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_invalid_handles\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_invalid_handles\nassert native_invalid_handles.API_KIND == 'invalid-handles'\nassert native_invalid_handles.INVALID_HANDLE_CHECKS_OK is True",
    )
    .expect("invalid-handle extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_set_module_attrs_and_items() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping module/item API extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping module/item API extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_module_item_apis");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_module_item_apis.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    PyrsObjectHandle self_mod = 0;
    if (api->module_import(module_ctx, "native_module_item_apis", &self_mod) != 0 || !self_mod) {
        return -2;
    }
    PyrsObjectHandle attr_value = api->object_new_int(module_ctx, 321);
    if (!attr_value) {
        return -3;
    }
    if (api->module_set_attr(module_ctx, self_mod, "TEMP_ATTR", attr_value) != 0) {
        return -4;
    }
    if (api->module_has_attr(module_ctx, self_mod, "TEMP_ATTR") != 1) {
        return -5;
    }
    PyrsObjectHandle attr_out = 0;
    if (api->module_get_attr(module_ctx, self_mod, "TEMP_ATTR", &attr_out) != 0 || !attr_out) {
        return -6;
    }
    int64_t attr_int = 0;
    if (api->object_get_int(module_ctx, attr_out, &attr_int) != 0 || attr_int != 321) {
        return -7;
    }
    if (api->module_del_attr(module_ctx, self_mod, "TEMP_ATTR") != 0) {
        return -8;
    }
    if (api->module_has_attr(module_ctx, self_mod, "TEMP_ATTR") != 0) {
        return -9;
    }
    if (api->module_del_attr(module_ctx, self_mod, "TEMP_ATTR") == 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -10;
    }

    PyrsObjectHandle dict_obj = api->object_new_dict(module_ctx);
    PyrsObjectHandle dict_key = api->object_new_string(module_ctx, "k");
    PyrsObjectHandle dict_value = api->object_new_int(module_ctx, 7);
    if (!dict_obj || !dict_key || !dict_value) {
        return -11;
    }
    if (api->object_set_item(module_ctx, dict_obj, dict_key, dict_value) != 0) {
        return -12;
    }
    PyrsObjectHandle dict_out = 0;
    if (api->object_get_item(module_ctx, dict_obj, dict_key, &dict_out) != 0 || !dict_out) {
        return -13;
    }
    int64_t dict_int = 0;
    if (api->object_get_int(module_ctx, dict_out, &dict_int) != 0 || dict_int != 7) {
        return -14;
    }
    if (api->object_del_item(module_ctx, dict_obj, dict_key) != 0) {
        return -15;
    }
    if (api->object_del_item(module_ctx, dict_obj, dict_key) == 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -16;
    }

    PyrsObjectHandle list_items[1];
    list_items[0] = dict_value;
    PyrsObjectHandle list_obj = api->object_new_list(module_ctx, 1, list_items);
    PyrsObjectHandle idx0 = api->object_new_int(module_ctx, 0);
    PyrsObjectHandle list_value = api->object_new_int(module_ctx, 11);
    if (!list_obj || !idx0 || !list_value) {
        return -17;
    }
    if (api->object_set_item(module_ctx, list_obj, idx0, list_value) != 0) {
        return -18;
    }
    PyrsObjectHandle list_out = 0;
    if (api->object_get_item(module_ctx, list_obj, idx0, &list_out) != 0 || !list_out) {
        return -19;
    }
    int64_t list_int = 0;
    if (api->object_get_int(module_ctx, list_out, &list_int) != 0 || list_int != 11) {
        return -20;
    }
    if (api->object_del_item(module_ctx, list_obj, idx0) != 0) {
        return -21;
    }
    uintptr_t list_len = 99;
    if (api->object_len(module_ctx, list_obj, &list_len) != 0 || list_len != 0) {
        return -22;
    }
    if (api->object_del_item(module_ctx, list_obj, idx0) == 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -23;
    }

    if (api->module_set_bool(module_ctx, "MODULE_ITEM_APIS_OK", 1) != 0) {
        return -24;
    }
    if (api->module_set_int(module_ctx, "LIST_LEN_AFTER_DEL", (int64_t)list_len) != 0) {
        return -25;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_module_item_apis");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_module_item_apis.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_module_item_apis\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_module_item_apis\nassert native_module_item_apis.MODULE_ITEM_APIS_OK is True\nassert native_module_item_apis.LIST_LEN_AFTER_DEL == 0\nassert not hasattr(native_module_item_apis, 'TEMP_ATTR')",
    )
    .expect("module/item API extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_item_mutation_falls_back_to_special_methods() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping item fallback extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping item fallback extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_item_fallback");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let helper_module_path = temp_root.join("item_target.py");
    fs::write(
        &helper_module_path,
        r#"class Box:
    def __init__(self):
        self._data = {}

    def __getitem__(self, key):
        return self._data[key]

    def __setitem__(self, key, value):
        self._data[key] = value

    def __delitem__(self, key):
        del self._data[key]

    def size(self):
        return len(self._data)
"#,
    )
    .expect("helper module should be written");

    let source_path = temp_root.join("native_item_fallback.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    PyrsObjectHandle helper_mod = 0;
    if (api->module_import(module_ctx, "item_target", &helper_mod) != 0 || !helper_mod) {
        return -2;
    }
    PyrsObjectHandle box_cls = 0;
    if (api->module_get_attr(module_ctx, helper_mod, "Box", &box_cls) != 0 || !box_cls) {
        return -3;
    }
    PyrsObjectHandle box = 0;
    if (api->object_call_noargs(module_ctx, box_cls, &box) != 0 || !box) {
        return -4;
    }
    PyrsObjectHandle key = api->object_new_string(module_ctx, "k");
    PyrsObjectHandle value = api->object_new_int(module_ctx, 77);
    if (!key || !value) {
        return -5;
    }
    if (api->object_set_item(module_ctx, box, key, value) != 0) {
        return -6;
    }
    PyrsObjectHandle out = 0;
    if (api->object_get_item(module_ctx, box, key, &out) != 0 || !out) {
        return -7;
    }
    int64_t out_int = 0;
    if (api->object_get_int(module_ctx, out, &out_int) != 0 || out_int != 77) {
        return -8;
    }
    if (api->object_del_item(module_ctx, box, key) != 0) {
        return -9;
    }
    if (api->object_del_item(module_ctx, box, key) == 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -10;
    }
    PyrsObjectHandle size_fn = 0;
    if (api->object_get_attr(module_ctx, box, "size", &size_fn) != 0 || !size_fn) {
        return -11;
    }
    PyrsObjectHandle size_out = 0;
    if (api->object_call_noargs(module_ctx, size_fn, &size_out) != 0 || !size_out) {
        return -12;
    }
    int64_t size = -1;
    if (api->object_get_int(module_ctx, size_out, &size) != 0 || size != 0) {
        return -13;
    }
    if (api->module_set_bool(module_ctx, "ITEM_FALLBACK_OK", 1) != 0) {
        return -14;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_item_fallback");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_item_fallback.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_item_fallback\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_item_fallback\nassert native_item_fallback.ITEM_FALLBACK_OK is True",
    )
    .expect("item fallback extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(helper_module_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_use_contains_and_dict_view_apis() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping contains/dict-view extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping contains/dict-view extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_contains_dict_views");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_contains_dict_views.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    PyrsObjectHandle dict_obj = api->object_new_dict(module_ctx);
    PyrsObjectHandle key_a = api->object_new_string(module_ctx, "a");
    PyrsObjectHandle key_b = api->object_new_string(module_ctx, "b");
    PyrsObjectHandle key_z = api->object_new_string(module_ctx, "z");
    PyrsObjectHandle value_1 = api->object_new_int(module_ctx, 1);
    PyrsObjectHandle value_2 = api->object_new_int(module_ctx, 2);
    if (!dict_obj || !key_a || !key_b || !key_z || !value_1 || !value_2) {
        return -2;
    }
    if (api->object_set_item(module_ctx, dict_obj, key_a, value_1) != 0) {
        return -3;
    }
    if (api->object_set_item(module_ctx, dict_obj, key_b, value_2) != 0) {
        return -4;
    }
    if (api->object_contains(module_ctx, dict_obj, key_a) != 1) {
        return -5;
    }
    if (api->object_contains(module_ctx, dict_obj, key_z) != 0) {
        return -6;
    }

    PyrsObjectHandle keys = 0;
    if (api->object_dict_keys(module_ctx, dict_obj, &keys) != 0 || !keys) {
        return -7;
    }
    uintptr_t keys_len = 0;
    if (api->object_len(module_ctx, keys, &keys_len) != 0 || keys_len != 2) {
        return -8;
    }
    if (api->object_contains(module_ctx, keys, key_a) != 1) {
        return -9;
    }

    PyrsObjectHandle items = 0;
    if (api->object_dict_items(module_ctx, dict_obj, &items) != 0 || !items) {
        return -10;
    }
    uintptr_t items_len = 0;
    if (api->object_len(module_ctx, items, &items_len) != 0 || items_len != 2) {
        return -11;
    }
    PyrsObjectHandle pair_values[2];
    pair_values[0] = key_a;
    pair_values[1] = value_1;
    PyrsObjectHandle pair_a = api->object_new_tuple(module_ctx, 2, pair_values);
    if (!pair_a) {
        return -12;
    }
    if (api->object_contains(module_ctx, items, pair_a) != 1) {
        return -13;
    }

    PyrsObjectHandle bogus = 0;
    if (api->object_dict_keys(module_ctx, key_a, &bogus) == 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -14;
    }

    if (api->module_set_bool(module_ctx, "CONTAINS_DICT_VIEWS_OK", 1) != 0) {
        return -15;
    }
    if (api->module_set_int(module_ctx, "DICT_KEYS_LEN", (int64_t)keys_len) != 0) {
        return -16;
    }
    if (api->module_set_int(module_ctx, "DICT_ITEMS_LEN", (int64_t)items_len) != 0) {
        return -17;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_contains_dict_views");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_contains_dict_views.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_contains_dict_views\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_contains_dict_views\nassert native_contains_dict_views.CONTAINS_DICT_VIEWS_OK is True\nassert native_contains_dict_views.DICT_KEYS_LEN == 2\nassert native_contains_dict_views.DICT_ITEMS_LEN == 2",
    )
    .expect("contains/dict-view extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_use_buffer_apis() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping buffer API extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping buffer API extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_buffer_apis");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_buffer_apis.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    const uint8_t payload[] = {65, 66, 67}; /* ABC */
    PyrsObjectHandle bytes_obj = api->object_new_bytes(module_ctx, payload, 3);
    if (!bytes_obj) {
        return -2;
    }

    PyrsBufferViewV1 view;
    if (api->object_get_buffer(module_ctx, bytes_obj, &view) != 0) {
        return -3;
    }
    if (!view.data || view.len != 3 || view.readonly != 1 ||
        view.data[0] != 65 || view.data[1] != 66 || view.data[2] != 67) {
        return -4;
    }
    if (api->object_release_buffer(module_ctx, bytes_obj) != 0) {
        return -5;
    }
    if (api->object_release_buffer(module_ctx, bytes_obj) == 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -6;
    }

    PyrsObjectHandle builtins_mod = 0;
    if (api->module_import(module_ctx, "builtins", &builtins_mod) != 0 || !builtins_mod) {
        return -7;
    }
    PyrsObjectHandle bytearray_cls = 0;
    if (api->module_get_attr(module_ctx, builtins_mod, "bytearray", &bytearray_cls) != 0 || !bytearray_cls) {
        return -8;
    }
    PyrsObjectHandle bytearray_obj = 0;
    if (api->object_call_onearg(module_ctx, bytearray_cls, bytes_obj, &bytearray_obj) != 0 || !bytearray_obj) {
        return -9;
    }
    if (api->object_get_buffer(module_ctx, bytearray_obj, &view) != 0) {
        return -10;
    }
    if (!view.data || view.len != 3 || view.readonly != 0 ||
        view.data[0] != 65 || view.data[1] != 66 || view.data[2] != 67) {
        return -11;
    }
    if (api->object_release_buffer(module_ctx, bytearray_obj) != 0) {
        return -12;
    }

    PyrsObjectHandle memoryview_cls = 0;
    if (api->module_get_attr(module_ctx, builtins_mod, "memoryview", &memoryview_cls) != 0 || !memoryview_cls) {
        return -13;
    }
    PyrsObjectHandle memoryview_obj = 0;
    if (api->object_call_onearg(module_ctx, memoryview_cls, bytearray_obj, &memoryview_obj) != 0 || !memoryview_obj) {
        return -14;
    }
    if (api->object_get_buffer(module_ctx, memoryview_obj, &view) != 0) {
        return -15;
    }
    if (!view.data || view.len != 3 || view.readonly != 0) {
        return -16;
    }
    if (api->object_release_buffer(module_ctx, memoryview_obj) != 0) {
        return -17;
    }

    PyrsObjectHandle int_obj = api->object_new_int(module_ctx, 9);
    if (!int_obj) {
        return -18;
    }
    if (api->object_get_buffer(module_ctx, int_obj, &view) == 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -19;
    }

    if (api->module_set_bool(module_ctx, "BUFFER_APIS_OK", 1) != 0) {
        return -20;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_buffer_apis");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_buffer_apis.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_buffer_apis\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_buffer_apis\nassert native_buffer_apis.BUFFER_APIS_OK is True",
    )
    .expect("buffer API extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_use_writable_buffer_apis() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping writable buffer API extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping writable buffer API extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_writable_buffer_apis");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_writable_buffer_apis.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    const uint8_t payload[] = {65, 66, 67, 68}; /* ABCD */
    PyrsObjectHandle bytearray_obj = api->object_new_bytearray(module_ctx, payload, 4);
    if (!bytearray_obj) {
        return -2;
    }

    PyrsWritableBufferViewV1 writable;
    if (api->object_get_writable_buffer(module_ctx, bytearray_obj, &writable) != 0) {
        return -3;
    }
    if (!writable.data || writable.len != 4) {
        return -4;
    }
    writable.data[1] = 90; /* Z */
    if (api->object_release_buffer(module_ctx, bytearray_obj) != 0) {
        return -5;
    }

    PyrsBufferViewV1 readonly;
    if (api->object_get_buffer(module_ctx, bytearray_obj, &readonly) != 0) {
        return -6;
    }
    if (!readonly.data || readonly.len != 4 || readonly.readonly != 0 ||
        readonly.data[0] != 65 || readonly.data[1] != 90 || readonly.data[2] != 67 || readonly.data[3] != 68) {
        return -7;
    }
    if (api->object_release_buffer(module_ctx, bytearray_obj) != 0) {
        return -8;
    }

    PyrsObjectHandle memoryview_obj = api->object_new_memoryview(module_ctx, bytearray_obj);
    if (!memoryview_obj) {
        return -9;
    }
    if (api->object_get_writable_buffer(module_ctx, memoryview_obj, &writable) != 0) {
        return -10;
    }
    if (!writable.data || writable.len != 4) {
        return -11;
    }
    writable.data[2] = 88; /* X */
    if (api->object_release_buffer(module_ctx, memoryview_obj) != 0) {
        return -12;
    }

    if (api->object_get_buffer(module_ctx, bytearray_obj, &readonly) != 0) {
        return -13;
    }
    if (!readonly.data || readonly.data[2] != 88) {
        return -14;
    }
    if (api->object_release_buffer(module_ctx, bytearray_obj) != 0) {
        return -15;
    }

    PyrsObjectHandle bytes_obj = api->object_new_bytes(module_ctx, payload, 4);
    if (!bytes_obj) {
        return -16;
    }
    if (api->object_get_writable_buffer(module_ctx, bytes_obj, &writable) == 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -17;
    }

    PyrsObjectHandle readonly_memoryview_obj = api->object_new_memoryview(module_ctx, bytes_obj);
    if (!readonly_memoryview_obj) {
        return -18;
    }
    if (api->object_get_writable_buffer(module_ctx, readonly_memoryview_obj, &writable) == 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -19;
    }

    if (api->module_set_bool(module_ctx, "WRITABLE_BUFFER_APIS_OK", 1) != 0) {
        return -20;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_writable_buffer_apis");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_writable_buffer_apis.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_writable_buffer_apis\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_writable_buffer_apis\nassert native_writable_buffer_apis.WRITABLE_BUFFER_APIS_OK is True",
    )
    .expect("writable buffer API extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_read_buffer_info_metadata() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping buffer-info API extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping buffer-info API extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_buffer_info_apis");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_buffer_info_apis.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"
#include <string.h>

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    const uint8_t payload[] = {65, 66, 67, 68}; /* ABCD */
    PyrsObjectHandle bytearray_obj = api->object_new_bytearray(module_ctx, payload, 4);
    if (!bytearray_obj) {
        return -2;
    }

    PyrsBufferInfoV1 info;
    PyrsBufferInfoV2 info2;
    if (api->object_get_buffer_info(module_ctx, bytearray_obj, &info) != 0) {
        return -3;
    }
    if (!info.data || info.len != 4 || info.readonly != 0 || info.itemsize != 1 ||
        info.ndim != 1 || info.shape0 != 4 || info.stride0 != 1 || info.contiguous != 1 ||
        !info.format || strcmp(info.format, "B") != 0) {
        return -4;
    }
    if (api->object_release_buffer(module_ctx, bytearray_obj) != 0) {
        return -5;
    }
    if (api->object_get_buffer_info_v2(module_ctx, bytearray_obj, &info2) != 0) {
        return -6;
    }
    if (!info2.data || info2.len != 4 || info2.readonly != 0 || info2.itemsize != 1 ||
        info2.ndim != 1 || !info2.shape || !info2.strides ||
        info2.shape[0] != 4 || info2.strides[0] != 1 || info2.contiguous != 1 ||
        !info2.format || strcmp(info2.format, "B") != 0) {
        return -7;
    }
    if (api->object_release_buffer(module_ctx, bytearray_obj) != 0) {
        return -8;
    }

    PyrsObjectHandle bytes_obj = api->object_new_bytes(module_ctx, payload, 4);
    if (!bytes_obj) {
        return -9;
    }
    if (api->object_get_buffer_info(module_ctx, bytes_obj, &info) != 0) {
        return -10;
    }
    if (!info.data || info.len != 4 || info.readonly != 1 ||
        info.ndim != 1 || info.shape0 != 4 || info.stride0 != 1 || info.contiguous != 1) {
        return -11;
    }
    if (api->object_release_buffer(module_ctx, bytes_obj) != 0) {
        return -12;
    }
    if (api->object_get_buffer_info_v2(module_ctx, bytes_obj, &info2) != 0) {
        return -13;
    }
    if (!info2.data || info2.len != 4 || info2.readonly != 1 || info2.itemsize != 1 ||
        info2.ndim != 1 || !info2.shape || !info2.strides ||
        info2.shape[0] != 4 || info2.strides[0] != 1 || info2.contiguous != 1 ||
        !info2.format || strcmp(info2.format, "B") != 0) {
        return -14;
    }
    if (api->object_release_buffer(module_ctx, bytes_obj) != 0) {
        return -15;
    }

    PyrsObjectHandle view_obj = api->object_new_memoryview(module_ctx, bytearray_obj);
    if (!view_obj) {
        return -16;
    }
    if (api->object_get_buffer_info(module_ctx, view_obj, &info) != 0) {
        return -17;
    }
    if (!info.data || info.len != 4 || info.readonly != 0 ||
        info.ndim != 1 || info.shape0 != 4 || info.stride0 != 1 || info.contiguous != 1) {
        return -18;
    }
    if (api->object_release_buffer(module_ctx, view_obj) != 0) {
        return -19;
    }
    if (api->object_get_buffer_info_v2(module_ctx, view_obj, &info2) != 0) {
        return -20;
    }
    if (!info2.data || info2.len != 4 || info2.readonly != 0 || info2.itemsize != 1 ||
        info2.ndim != 1 || !info2.shape || !info2.strides ||
        info2.shape[0] != 4 || info2.strides[0] != 1 || info2.contiguous != 1 ||
        !info2.format || strcmp(info2.format, "B") != 0) {
        return -21;
    }
    if (api->object_release_buffer(module_ctx, view_obj) != 0) {
        return -22;
    }

    if (api->module_set_bool(module_ctx, "BUFFER_INFO_APIS_OK", 1) != 0) {
        return -23;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_buffer_info_apis");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_buffer_info_apis.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_buffer_info_apis\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_buffer_info_apis\nassert native_buffer_info_apis.BUFFER_INFO_APIS_OK is True",
    )
    .expect("buffer-info API extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_buffer_info_v2_reports_invalid_and_null_output_errors() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping buffer-info-v2 negative-path smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping buffer-info-v2 negative-path smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_buffer_info_v2_negative_paths");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_buffer_info_v2_negative_paths.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"
#include <string.h>

static int expect_error_contains(const PyrsApiV1* api, void* module_ctx, const char* needle) {
    const char* msg = api->error_get_message(module_ctx);
    if (!msg || !msg[0] || !needle || !needle[0]) {
        return 0;
    }
    if (!strstr(msg, needle)) {
        return 0;
    }
    if (api->error_clear(module_ctx) != 0) {
        return 0;
    }
    return api->error_occurred(module_ctx) == 0;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }

    PyrsBufferInfoV2 info2;
    PyrsObjectHandle invalid = 999999;
    if (api->object_get_buffer_info_v2(module_ctx, invalid, &info2) == 0 ||
        !expect_error_contains(api, module_ctx, "invalid object handle")) {
        return -2;
    }

    PyrsObjectHandle int_obj = api->object_new_int(module_ctx, 7);
    if (!int_obj) {
        return -3;
    }
    if (api->object_get_buffer_info_v2(module_ctx, int_obj, &info2) == 0 ||
        !expect_error_contains(api, module_ctx, "does not support buffer info access")) {
        return -4;
    }

    const uint8_t payload[] = {1, 2, 3, 4};
    PyrsObjectHandle bytearray_obj = api->object_new_bytearray(module_ctx, payload, 4);
    if (!bytearray_obj) {
        return -5;
    }
    if (api->object_get_buffer_info_v2(module_ctx, bytearray_obj, 0) == 0 ||
        !expect_error_contains(api, module_ctx, "object_get_buffer_info_v2 received null output pointer")) {
        return -6;
    }

    if (api->object_get_buffer_info_v2(module_ctx, bytearray_obj, &info2) != 0) {
        return -7;
    }
    if (!info2.data || info2.len != 4 || info2.ndim != 1 || !info2.shape || !info2.strides) {
        return -8;
    }
    if (api->object_release_buffer(module_ctx, bytearray_obj) != 0) {
        return -9;
    }

    if (api->module_set_bool(module_ctx, "BUFFER_INFO_V2_NEGATIVE_PATHS_OK", 1) != 0) {
        return -10;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_buffer_info_v2_negative_paths");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_buffer_info_v2_negative_paths.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_buffer_info_v2_negative_paths\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_buffer_info_v2_negative_paths\nassert native_buffer_info_v2_negative_paths.BUFFER_INFO_V2_NEGATIVE_PATHS_OK is True",
    )
    .expect("buffer-info-v2 negative-path extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_buffer_info_marks_noncontiguous_slice_views() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping non-contiguous buffer-info smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping non-contiguous buffer-info smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_buffer_info_noncontig");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_buffer_info_noncontig.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"
#include <string.h>

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    const uint8_t payload[] = {65, 66, 67, 68}; /* ABCD */
    PyrsObjectHandle bytearray_obj = api->object_new_bytearray(module_ctx, payload, 4);
    if (!bytearray_obj) {
        return -2;
    }
    PyrsObjectHandle view_obj = api->object_new_memoryview(module_ctx, bytearray_obj);
    if (!view_obj) {
        return -3;
    }

    PyrsObjectHandle builtins_mod = 0;
    if (api->module_import(module_ctx, "builtins", &builtins_mod) != 0 || !builtins_mod) {
        return -4;
    }
    PyrsObjectHandle slice_cls = 0;
    if (api->module_get_attr(module_ctx, builtins_mod, "slice", &slice_cls) != 0 || !slice_cls) {
        return -5;
    }
    PyrsObjectHandle slice_args[3];
    slice_args[0] = api->object_new_int(module_ctx, 0);
    slice_args[1] = api->object_new_int(module_ctx, 4);
    slice_args[2] = api->object_new_int(module_ctx, 2);
    if (!slice_args[0] || !slice_args[1] || !slice_args[2]) {
        return -6;
    }
    PyrsObjectHandle slice_obj = 0;
    if (api->object_call(module_ctx, slice_cls, 3, slice_args, 0, 0, 0, &slice_obj) != 0 || !slice_obj) {
        return -7;
    }
    PyrsObjectHandle subview = 0;
    if (api->object_get_item(module_ctx, view_obj, slice_obj, &subview) != 0 || !subview) {
        return -8;
    }

    PyrsBufferInfoV1 info;
    PyrsBufferInfoV2 info2;
    if (api->object_get_buffer_info(module_ctx, subview, &info) != 0) {
        return -9;
    }
    if (!info.data || info.len != 2 || info.readonly != 0 || info.itemsize != 1 ||
        info.ndim != 1 || info.shape0 != 2 || info.stride0 != 2 || info.contiguous != 0 ||
        !info.format || strcmp(info.format, "B") != 0) {
        return -10;
    }
    if (api->object_release_buffer(module_ctx, subview) != 0) {
        return -11;
    }
    if (api->object_get_buffer_info_v2(module_ctx, subview, &info2) != 0) {
        return -12;
    }
    if (!info2.data || info2.len != 2 || info2.readonly != 0 || info2.itemsize != 1 ||
        info2.ndim != 1 || !info2.shape || !info2.strides ||
        info2.shape[0] != 2 || info2.strides[0] != 2 || info2.contiguous != 0 ||
        !info2.format || strcmp(info2.format, "B") != 0) {
        return -13;
    }
    if (api->object_release_buffer(module_ctx, subview) != 0) {
        return -14;
    }

    PyrsWritableBufferViewV1 writable;
    if (api->object_get_writable_buffer(module_ctx, subview, &writable) == 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -15;
    }

    if (api->module_set_bool(module_ctx, "BUFFER_INFO_NONCONTIG_OK", 1) != 0) {
        return -16;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_buffer_info_noncontig");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_buffer_info_noncontig.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_buffer_info_noncontig\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_buffer_info_noncontig\nassert native_buffer_info_noncontig.BUFFER_INFO_NONCONTIG_OK is True",
    )
    .expect("non-contiguous buffer-info extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_buffer_info_reflects_memoryview_cast_itemsize() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping memoryview-cast buffer-info smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping memoryview-cast buffer-info smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_buffer_info_cast_itemsize");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_buffer_info_cast_itemsize.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"
#include <string.h>

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    const uint8_t payload[] = {1, 2, 3, 4, 5, 6, 7, 8};
    PyrsObjectHandle bytearray_obj = api->object_new_bytearray(module_ctx, payload, 8);
    if (!bytearray_obj) {
        return -2;
    }
    PyrsObjectHandle view_obj = api->object_new_memoryview(module_ctx, bytearray_obj);
    if (!view_obj) {
        return -3;
    }

    PyrsObjectHandle cast_fn = 0;
    if (api->object_get_attr(module_ctx, view_obj, "cast", &cast_fn) != 0 || !cast_fn) {
        return -4;
    }
    PyrsObjectHandle fmt = api->object_new_string(module_ctx, "I");
    if (!fmt) {
        return -5;
    }
    PyrsObjectHandle casted = 0;
    if (api->object_call_onearg(module_ctx, cast_fn, fmt, &casted) != 0 || !casted) {
        return -6;
    }

    PyrsBufferInfoV1 info;
    PyrsBufferInfoV2 info2;
    if (api->object_get_buffer_info(module_ctx, casted, &info) != 0) {
        return -7;
    }
    if (!info.data || info.len != 8 || info.readonly != 0 || info.itemsize != 4 ||
        info.ndim != 1 || info.shape0 != 2 || info.stride0 != 4 || info.contiguous != 1 ||
        !info.format || strcmp(info.format, "I") != 0) {
        return -8;
    }
    if (api->object_release_buffer(module_ctx, casted) != 0) {
        return -9;
    }
    if (api->object_get_buffer_info_v2(module_ctx, casted, &info2) != 0) {
        return -10;
    }
    if (!info2.data || info2.len != 8 || info2.readonly != 0 || info2.itemsize != 4 ||
        info2.ndim != 1 || !info2.shape || !info2.strides ||
        info2.shape[0] != 2 || info2.strides[0] != 4 || info2.contiguous != 1 ||
        !info2.format || strcmp(info2.format, "I") != 0) {
        return -11;
    }
    if (api->object_release_buffer(module_ctx, casted) != 0) {
        return -12;
    }

    PyrsObjectHandle fmt_b = api->object_new_string(module_ctx, "B");
    PyrsObjectHandle dim_values[2];
    dim_values[0] = api->object_new_int(module_ctx, 2);
    dim_values[1] = api->object_new_int(module_ctx, 4);
    if (!fmt_b || !dim_values[0] || !dim_values[1]) {
        return -13;
    }
    PyrsObjectHandle shape_list = api->object_new_list(module_ctx, 2, dim_values);
    if (!shape_list) {
        return -14;
    }
    PyrsObjectHandle cast_args[2] = {fmt_b, shape_list};
    PyrsObjectHandle casted_shaped = 0;
    if (api->object_call(module_ctx, cast_fn, 2, cast_args, 0, 0, 0, &casted_shaped) != 0 || !casted_shaped) {
        return -15;
    }
    if (api->object_get_buffer_info(module_ctx, casted_shaped, &info) != 0) {
        return -16;
    }
    if (!info.data || info.len != 8 || info.readonly != 0 || info.itemsize != 1 ||
        info.ndim != 2 || info.shape0 != 2 || info.stride0 != 4 || info.contiguous != 1 ||
        !info.format || strcmp(info.format, "B") != 0) {
        return -17;
    }
    if (api->object_release_buffer(module_ctx, casted_shaped) != 0) {
        return -18;
    }
    if (api->object_get_buffer_info_v2(module_ctx, casted_shaped, &info2) != 0) {
        return -19;
    }
    if (!info2.data || info2.len != 8 || info2.readonly != 0 || info2.itemsize != 1 ||
        info2.ndim != 2 || !info2.shape || !info2.strides ||
        info2.shape[0] != 2 || info2.shape[1] != 4 ||
        info2.strides[0] != 4 || info2.strides[1] != 1 ||
        info2.contiguous != 1 || !info2.format || strcmp(info2.format, "B") != 0) {
        return -20;
    }
    if (api->object_release_buffer(module_ctx, casted_shaped) != 0) {
        return -21;
    }

    PyrsWritableBufferViewV1 writable;
    if (api->object_get_writable_buffer(module_ctx, casted, &writable) != 0) {
        return -22;
    }
    if (!writable.data || writable.len != 8) {
        return -23;
    }
    writable.data[0] = 42;
    if (api->object_release_buffer(module_ctx, casted) != 0) {
        return -24;
    }

    PyrsBufferViewV1 readonly;
    if (api->object_get_buffer(module_ctx, bytearray_obj, &readonly) != 0) {
        return -25;
    }
    if (!readonly.data || readonly.len != 8 || readonly.data[0] != 42) {
        return -26;
    }
    if (api->object_release_buffer(module_ctx, bytearray_obj) != 0) {
        return -27;
    }

    if (api->module_set_bool(module_ctx, "BUFFER_INFO_CAST_ITEMSIZE_OK", 1) != 0) {
        return -28;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_buffer_info_cast_itemsize");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_buffer_info_cast_itemsize.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_buffer_info_cast_itemsize\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_buffer_info_cast_itemsize\nassert native_buffer_info_cast_itemsize.BUFFER_INFO_CAST_ITEMSIZE_OK is True",
    )
    .expect("memoryview-cast buffer-info extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_buffer_pin_blocks_bytearray_resize_until_release() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping buffer-pin resize-block smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping buffer-pin resize-block smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_buffer_pin_resize_block");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_buffer_pin_resize_block.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"
#include <string.h>

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }

    const uint8_t payload[] = {97, 98, 99}; /* abc */
    const uint8_t extra[] = {120}; /* x */
    PyrsObjectHandle bytearray_obj = api->object_new_bytearray(module_ctx, payload, 3);
    if (!bytearray_obj) {
        return -2;
    }
    PyrsObjectHandle extra_obj = api->object_new_bytes(module_ctx, extra, 1);
    if (!extra_obj) {
        return -3;
    }

    PyrsBufferViewV1 view;
    if (api->object_get_buffer(module_ctx, bytearray_obj, &view) != 0) {
        return -4;
    }
    if (!view.data || view.len != 3 || view.readonly != 0) {
        return -5;
    }

    PyrsObjectHandle extend = 0;
    if (api->object_get_attr(module_ctx, bytearray_obj, "extend", &extend) != 0 || !extend) {
        return -6;
    }
    PyrsObjectHandle ignored = 0;
    if (api->object_call_onearg(module_ctx, extend, extra_obj, &ignored) == 0) {
        return -7;
    }
    if (api->error_occurred(module_ctx) == 0) {
        return -8;
    }
    const char* msg = api->error_get_message(module_ctx);
    if (!msg || strstr(msg, "BufferError") == 0) {
        return -9;
    }
    if (api->error_clear(module_ctx) != 0) {
        return -10;
    }

    if (api->object_release_buffer(module_ctx, bytearray_obj) != 0) {
        return -11;
    }
    if (api->object_call_onearg(module_ctx, extend, extra_obj, &ignored) != 0) {
        return -12;
    }
    uintptr_t length = 0;
    if (api->object_len(module_ctx, bytearray_obj, &length) != 0 || length != 4) {
        return -13;
    }

    if (api->module_set_bool(module_ctx, "BUFFER_PIN_RESIZE_BLOCK_OK", 1) != 0) {
        return -14;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_buffer_pin_resize_block");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_buffer_pin_resize_block.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_buffer_pin_resize_block\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_buffer_pin_resize_block\nassert native_buffer_pin_resize_block.BUFFER_PIN_RESIZE_BLOCK_OK is True",
    )
    .expect("buffer pin resize-block extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_unreleased_buffer_pin_is_cleared_on_context_drop() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping leaked-buffer-pin cleanup smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping leaked-buffer-pin cleanup smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_buffer_pin_drop_cleanup");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_buffer_pin_drop_cleanup.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    const uint8_t payload[] = {97, 98, 99}; /* abc */
    PyrsObjectHandle bytearray_obj = api->object_new_bytearray(module_ctx, payload, 3);
    if (!bytearray_obj) {
        return -2;
    }
    if (api->module_set_object(module_ctx, "BUF", bytearray_obj) != 0) {
        return -3;
    }
    PyrsBufferViewV1 view;
    if (api->object_get_buffer(module_ctx, bytearray_obj, &view) != 0) {
        return -4;
    }
    if (!view.data || view.len != 3 || view.readonly != 0) {
        return -5;
    }
    /* Intentionally skip object_release_buffer(...): context-drop cleanup should unpin. */
    if (api->module_set_bool(module_ctx, "BUFFER_PIN_DROP_CLEANUP_OK", 1) != 0) {
        return -6;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_buffer_pin_drop_cleanup");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_buffer_pin_drop_cleanup.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_buffer_pin_drop_cleanup\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import sys\nimport native_buffer_pin_drop_cleanup as mod\nassert mod.BUFFER_PIN_DROP_CLEANUP_OK is True\nmod.BUF.extend(b'x')\nassert bytes(mod.BUF) == b'abcx'\nfor _ in range(5):\n    del sys.modules['native_buffer_pin_drop_cleanup']\n    import native_buffer_pin_drop_cleanup as mod\n    mod.BUF.extend(b'x')\n    assert bytes(mod.BUF) == b'abcx'",
    )
    .expect("leaked-buffer-pin cleanup extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_buffer_api_handles_memoryview_slices_and_release() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping buffer slice/release extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping buffer slice/release extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_buffer_slice_release");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_buffer_slice_release.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    const uint8_t payload[] = {65, 66, 67, 68, 69}; /* ABCDE */
    PyrsObjectHandle bytes_obj = api->object_new_bytes(module_ctx, payload, 5);
    if (!bytes_obj) {
        return -2;
    }

    PyrsObjectHandle builtins_mod = 0;
    if (api->module_import(module_ctx, "builtins", &builtins_mod) != 0 || !builtins_mod) {
        return -3;
    }
    PyrsObjectHandle bytearray_cls = 0;
    if (api->module_get_attr(module_ctx, builtins_mod, "bytearray", &bytearray_cls) != 0 || !bytearray_cls) {
        return -4;
    }
    PyrsObjectHandle memoryview_cls = 0;
    if (api->module_get_attr(module_ctx, builtins_mod, "memoryview", &memoryview_cls) != 0 || !memoryview_cls) {
        return -5;
    }
    PyrsObjectHandle slice_cls = 0;
    if (api->module_get_attr(module_ctx, builtins_mod, "slice", &slice_cls) != 0 || !slice_cls) {
        return -6;
    }

    PyrsObjectHandle bytearray_obj = 0;
    if (api->object_call_onearg(module_ctx, bytearray_cls, bytes_obj, &bytearray_obj) != 0 || !bytearray_obj) {
        return -7;
    }
    PyrsObjectHandle memoryview_obj = 0;
    if (api->object_call_onearg(module_ctx, memoryview_cls, bytearray_obj, &memoryview_obj) != 0 || !memoryview_obj) {
        return -8;
    }

    PyrsObjectHandle slice_args[2];
    slice_args[0] = api->object_new_int(module_ctx, 1);
    slice_args[1] = api->object_new_int(module_ctx, 4);
    if (!slice_args[0] || !slice_args[1]) {
        return -9;
    }
    PyrsObjectHandle slice_obj = 0;
    if (api->object_call(module_ctx, slice_cls, 2, slice_args, 0, 0, 0, &slice_obj) != 0 || !slice_obj) {
        return -10;
    }
    PyrsObjectHandle subview = 0;
    if (api->object_get_item(module_ctx, memoryview_obj, slice_obj, &subview) != 0 || !subview) {
        return -11;
    }

    PyrsBufferViewV1 view;
    if (api->object_get_buffer(module_ctx, subview, &view) != 0) {
        return -12;
    }
    if (!view.data || view.len != 3 || view.readonly != 0 ||
        view.data[0] != 66 || view.data[1] != 67 || view.data[2] != 68) {
        return -13;
    }
    if (api->object_release_buffer(module_ctx, subview) != 0) {
        return -14;
    }

    PyrsObjectHandle release_fn = 0;
    if (api->object_get_attr(module_ctx, subview, "release", &release_fn) != 0 || !release_fn) {
        return -15;
    }
    PyrsObjectHandle ignored = 0;
    if (api->object_call_noargs(module_ctx, release_fn, &ignored) != 0) {
        return -16;
    }
    if (api->object_get_buffer(module_ctx, subview, &view) == 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -17;
    }
    if (api->module_set_bool(module_ctx, "BUFFER_SLICE_RELEASE_OK", 1) != 0) {
        return -18;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_buffer_slice_release");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_buffer_slice_release.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_buffer_slice_release\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_buffer_slice_release\nassert native_buffer_slice_release.BUFFER_SLICE_RELEASE_OK is True",
    )
    .expect("buffer slice/release extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_use_capsule_apis() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping capsule API extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping capsule API extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_capsule_apis");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_capsule_apis.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"
#include <stdint.h>

static int g_destructor_calls = 0;
static uintptr_t g_last_ptr = 0;
static uintptr_t g_last_ctx = 0;

static void capsule_destructor(void* pointer, void* context) {
    g_destructor_calls += 1;
    g_last_ptr = (uintptr_t)pointer;
    g_last_ctx = (uintptr_t)context;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    void* raw_ptr = (void*)(uintptr_t)0x1234;
    PyrsObjectHandle cap = api->capsule_new(module_ctx, raw_ptr, "demo.cap");
    if (!cap) {
        return -2;
    }
    const char* name = api->capsule_get_name(module_ctx, cap);
    if (!name) {
        return -3;
    }
    if (!(name[0] == 'd' && name[1] == 'e' && name[2] == 'm' && name[3] == 'o')) {
        return -4;
    }
    void* got_ptr = api->capsule_get_pointer(module_ctx, cap, "demo.cap");
    if (got_ptr != raw_ptr) {
        return -5;
    }
    if (api->capsule_set_pointer(module_ctx, cap, raw_ptr) != 0) {
        return -26;
    }
    got_ptr = api->capsule_get_pointer(module_ctx, cap, "demo.cap");
    if (got_ptr != raw_ptr) {
        return -27;
    }
    if (api->capsule_is_valid(module_ctx, cap, "demo.cap") != 1 ||
        api->capsule_is_valid(module_ctx, cap, "demo.other") != 0) {
        return -20;
    }
    if (api->capsule_get_destructor(module_ctx, cap) != 0) {
        return -21;
    }
    void* raw_ctx = (void*)(uintptr_t)0xBEEF;
    if (api->capsule_set_context(module_ctx, cap, raw_ctx) != 0) {
        return -13;
    }
    if (api->capsule_set_destructor(module_ctx, cap, capsule_destructor) != 0) {
        return -17;
    }
    if (api->capsule_get_destructor(module_ctx, cap) != capsule_destructor) {
        return -22;
    }
    if (api->capsule_set_name(module_ctx, cap, "demo.renamed") != 0) {
        return -23;
    }
    if (api->capsule_is_valid(module_ctx, cap, "demo.cap") != 0 ||
        api->capsule_is_valid(module_ctx, cap, "demo.renamed") != 1) {
        return -24;
    }
    got_ptr = api->capsule_get_pointer(module_ctx, cap, "demo.renamed");
    if (got_ptr != raw_ptr) {
        return -25;
    }
    void* got_ctx = api->capsule_get_context(module_ctx, cap);
    if (got_ctx != raw_ctx) {
        return -14;
    }
    void* mismatch = api->capsule_get_pointer(module_ctx, cap, "demo.other");
    if (mismatch != 0 || api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -6;
    }
    if (api->object_incref(module_ctx, cap) != 0) {
        return -7;
    }
    if (api->object_decref(module_ctx, cap) != 0) {
        return -8;
    }
    if (g_destructor_calls != 0) {
        return -18;
    }
    if (api->object_decref(module_ctx, cap) != 0) {
        return -9;
    }
    if (g_destructor_calls != 1 || g_last_ptr != (uintptr_t)raw_ptr || g_last_ctx != (uintptr_t)raw_ctx) {
        return -19;
    }
    if (api->capsule_get_name(module_ctx, cap) != 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -10;
    }
    if (api->capsule_new(module_ctx, 0, "null.ptr") != 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -11;
    }
    if (api->capsule_set_pointer(module_ctx, cap, 0) == 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -28;
    }
    if (api->capsule_set_context(module_ctx, cap, raw_ctx) == 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -15;
    }
    if (api->capsule_get_context(module_ctx, cap) != 0 ||
        api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -16;
    }
    if (api->module_set_bool(module_ctx, "CAPSULE_APIS_OK", 1) != 0) {
        return -12;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_capsule_apis");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_capsule_apis.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_capsule_apis\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_capsule_apis\nassert native_capsule_apis.CAPSULE_APIS_OK is True",
    )
    .expect("capsule API extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_runs_capsule_destructor_on_context_drop() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping capsule destructor drop smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping capsule destructor drop smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_capsule_drop_destructor");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_capsule_drop_destructor.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"
#include <stdint.h>

static int g_drop_calls = 0;
static int g_drop_ok = 0;

static void capsule_drop(void* pointer, void* context) {
    g_drop_calls += 1;
    if ((uintptr_t)pointer == (uintptr_t)0xCAFE &&
        (uintptr_t)context == (uintptr_t)0xBEEF) {
        g_drop_ok = 1;
    }
}

static int get_drop_calls(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    (void)argv;
    if (argc != 0 || !result) {
        return -1;
    }
    PyrsObjectHandle value = api->object_new_int(module_ctx, g_drop_calls);
    if (!value) {
        return -2;
    }
    *result = value;
    return 0;
}

static int get_drop_ok(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    (void)argv;
    if (argc != 0 || !result) {
        return -3;
    }
    PyrsObjectHandle value = api->object_new_int(module_ctx, g_drop_ok);
    if (!value) {
        return -4;
    }
    *result = value;
    return 0;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -10;
    }
    PyrsObjectHandle cap = api->capsule_new(module_ctx, (void*)(uintptr_t)0xCAFE, "drop.cap");
    if (!cap) {
        return -11;
    }
    if (api->capsule_set_context(module_ctx, cap, (void*)(uintptr_t)0xBEEF) != 0) {
        return -12;
    }
    if (api->capsule_set_destructor(module_ctx, cap, capsule_drop) != 0) {
        return -13;
    }
    if (api->module_add_function(module_ctx, "drop_calls", get_drop_calls) != 0) {
        return -14;
    }
    if (api->module_add_function(module_ctx, "drop_ok", get_drop_ok) != 0) {
        return -15;
    }
    if (api->module_set_bool(module_ctx, "INIT_OK", 1) != 0) {
        return -16;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_capsule_drop_destructor");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_capsule_drop_destructor.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_capsule_drop_destructor\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_capsule_drop_destructor as m\nassert m.INIT_OK is True\nassert m.drop_calls() == 1\nassert m.drop_ok() == 1",
    )
    .expect("capsule destructor drop import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_import_exported_capsule_by_name() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping capsule import/export smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping capsule import/export smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_capsule_import_export");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let provider_source = temp_root.join("native_capsule_provider.c");
    fs::write(
        &provider_source,
        r#"#include "pyrs_capi.h"
#include <stdint.h>

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    PyrsObjectHandle cap = api->capsule_new(module_ctx, (void*)(uintptr_t)0xABCDEF, "native_capsule_provider.CAPSULE_API");
    if (!cap) {
        return -2;
    }
    if (api->capsule_set_context(module_ctx, cap, (void*)(uintptr_t)0x1234) != 0) {
        return -3;
    }
    if (api->capsule_export(module_ctx, cap) != 0) {
        return -4;
    }
    if (api->object_decref(module_ctx, cap) != 0) {
        return -5;
    }
    if (api->module_set_bool(module_ctx, "EXPORTED", 1) != 0) {
        return -6;
    }
    return 0;
}
"#,
    )
    .expect("provider source should be written");
    let provider_library_file = shared_library_filename("native_capsule_provider");
    let provider_library_path = temp_root.join(&provider_library_file);
    compile_shared_extension(&provider_source, &provider_library_path)
        .expect("provider extension should build");
    let provider_manifest = temp_root.join("native_capsule_provider.pyrs-ext");
    fs::write(
        &provider_manifest,
        format!(
            "module=native_capsule_provider\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={provider_library_file}\n"
        ),
    )
    .expect("provider manifest should be written");

    let consumer_source = temp_root.join("native_capsule_consumer.c");
    fs::write(
        &consumer_source,
        r#"#include "pyrs_capi.h"
#include <stdint.h>

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    void* ptr = api->capsule_import(module_ctx, "native_capsule_provider.CAPSULE_API", 0);
    if (ptr != (void*)(uintptr_t)0xABCDEF) {
        return -2;
    }
    void* missing = api->capsule_import(module_ctx, "native_capsule_provider.MISSING", 0);
    if (missing != 0 || api->error_occurred(module_ctx) == 0 || api->error_clear(module_ctx) != 0) {
        return -3;
    }
    void* missing_module = api->capsule_import(module_ctx, "missing_capsule_provider.API", 0);
    if (missing_module != 0 || api->error_occurred(module_ctx) == 0) {
        return -5;
    }
    const char* msg = api->error_get_message(module_ctx);
    if (!msg || msg[0] != 'P' || msg[1] != 'y') {
        return -6;
    }
    if (api->error_clear(module_ctx) != 0) {
        return -7;
    }
    if (api->module_set_bool(module_ctx, "IMPORTED", 1) != 0) {
        return -4;
    }
    return 0;
}
"#,
    )
    .expect("consumer source should be written");
    let consumer_library_file = shared_library_filename("native_capsule_consumer");
    let consumer_library_path = temp_root.join(&consumer_library_file);
    compile_shared_extension(&consumer_source, &consumer_library_path)
        .expect("consumer extension should build");
    let consumer_manifest = temp_root.join("native_capsule_consumer.pyrs-ext");
    fs::write(
        &consumer_manifest,
        format!(
            "module=native_capsule_consumer\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={consumer_library_file}\n"
        ),
    )
    .expect("consumer manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import sys\nimport native_capsule_provider as p\nassert p.EXPORTED is True\nimport native_capsule_consumer as c\nassert c.IMPORTED is True\nfor _ in range(25):\n    del sys.modules['native_capsule_provider']\n    del sys.modules['native_capsule_consumer']\n    import native_capsule_provider as p\n    assert p.EXPORTED is True\n    import native_capsule_consumer as c\n    assert c.IMPORTED is True",
    )
    .expect("capsule import/export extension flow should succeed");

    let _ = fs::remove_file(provider_manifest);
    let _ = fs::remove_file(provider_library_path);
    let _ = fs::remove_file(provider_source);
    let _ = fs::remove_file(consumer_manifest);
    let _ = fs::remove_file(consumer_library_path);
    let _ = fs::remove_file(consumer_source);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_bridge_buffer_pointer_through_capsule() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping buffer/capsule bridge extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping buffer/capsule bridge extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_buffer_capsule_bridge");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_buffer_capsule_bridge.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"
#include <stdint.h>

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    const uint8_t payload[] = {10, 20, 30, 40};
    PyrsObjectHandle bytes_obj = api->object_new_bytes(module_ctx, payload, 4);
    if (!bytes_obj) {
        return -2;
    }
    PyrsObjectHandle builtins_mod = 0;
    if (api->module_import(module_ctx, "builtins", &builtins_mod) != 0 || !builtins_mod) {
        return -3;
    }
    PyrsObjectHandle bytearray_cls = 0;
    if (api->module_get_attr(module_ctx, builtins_mod, "bytearray", &bytearray_cls) != 0 || !bytearray_cls) {
        return -4;
    }
    PyrsObjectHandle bytearray_obj = 0;
    if (api->object_call_onearg(module_ctx, bytearray_cls, bytes_obj, &bytearray_obj) != 0 || !bytearray_obj) {
        return -5;
    }
    PyrsBufferViewV1 view;
    if (api->object_get_buffer(module_ctx, bytearray_obj, &view) != 0) {
        return -6;
    }
    if (!view.data || view.len != 4 || view.readonly != 0) {
        return -7;
    }
    PyrsObjectHandle cap = api->capsule_new(module_ctx, (void*)view.data, "bridge.buf");
    if (!cap) {
        return -8;
    }
    if (api->capsule_set_context(module_ctx, cap, (void*)(uintptr_t)view.len) != 0) {
        return -9;
    }
    void* got_ptr = api->capsule_get_pointer(module_ctx, cap, "bridge.buf");
    if (got_ptr != (void*)view.data) {
        return -10;
    }
    void* got_len = api->capsule_get_context(module_ctx, cap);
    if ((uintptr_t)got_len != (uintptr_t)view.len) {
        return -11;
    }
    if (api->object_release_buffer(module_ctx, bytearray_obj) != 0) {
        return -12;
    }
    if (api->module_set_bool(module_ctx, "BUFFER_CAPSULE_BRIDGE_OK", 1) != 0) {
        return -13;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_buffer_capsule_bridge");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_buffer_capsule_bridge.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_buffer_capsule_bridge\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_buffer_capsule_bridge\nassert native_buffer_capsule_bridge.BUFFER_CAPSULE_BRIDGE_OK is True",
    )
    .expect("buffer/capsule bridge extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_query_capabilities() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping capability-query extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping capability-query extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_capabilities");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_capabilities.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    int has_dict = api->api_has_capability(module_ctx, "object_new_dict");
    int has_kw = api->api_has_capability(module_ctx, "module_add_function_kw");
    int has_module_get_object = api->api_has_capability(module_ctx, "module_get_object");
    int has_module_import = api->api_has_capability(module_ctx, "module_import");
    int has_module_get_attr = api->api_has_capability(module_ctx, "module_get_attr");
    int has_object_new_bytearray = api->api_has_capability(module_ctx, "object_new_bytearray");
    int has_object_new_memoryview = api->api_has_capability(module_ctx, "object_new_memoryview");
    int has_module_set_state = api->api_has_capability(module_ctx, "module_set_state");
    int has_module_get_state = api->api_has_capability(module_ctx, "module_get_state");
    int has_module_set_finalize = api->api_has_capability(module_ctx, "module_set_finalize");
    int has_module_set_attr = api->api_has_capability(module_ctx, "module_set_attr");
    int has_module_del_attr = api->api_has_capability(module_ctx, "module_del_attr");
    int has_module_has_attr = api->api_has_capability(module_ctx, "module_has_attr");
    int has_object_len = api->api_has_capability(module_ctx, "object_len");
    int has_object_get_item = api->api_has_capability(module_ctx, "object_get_item");
    int has_object_set_item = api->api_has_capability(module_ctx, "object_set_item");
    int has_object_del_item = api->api_has_capability(module_ctx, "object_del_item");
    int has_object_contains = api->api_has_capability(module_ctx, "object_contains");
    int has_object_dict_keys = api->api_has_capability(module_ctx, "object_dict_keys");
    int has_object_dict_items = api->api_has_capability(module_ctx, "object_dict_items");
    int has_object_get_buffer = api->api_has_capability(module_ctx, "object_get_buffer");
    int has_object_get_writable_buffer = api->api_has_capability(module_ctx, "object_get_writable_buffer");
    int has_object_get_buffer_info = api->api_has_capability(module_ctx, "object_get_buffer_info");
    int has_object_get_buffer_info_v2 = api->api_has_capability(module_ctx, "object_get_buffer_info_v2");
    int has_object_release_buffer = api->api_has_capability(module_ctx, "object_release_buffer");
    int has_capsule_new = api->api_has_capability(module_ctx, "capsule_new");
    int has_capsule_get_pointer = api->api_has_capability(module_ctx, "capsule_get_pointer");
    int has_capsule_set_pointer = api->api_has_capability(module_ctx, "capsule_set_pointer");
    int has_capsule_get_name = api->api_has_capability(module_ctx, "capsule_get_name");
    int has_capsule_set_context = api->api_has_capability(module_ctx, "capsule_set_context");
    int has_capsule_get_context = api->api_has_capability(module_ctx, "capsule_get_context");
    int has_capsule_set_destructor = api->api_has_capability(module_ctx, "capsule_set_destructor");
    int has_capsule_get_destructor = api->api_has_capability(module_ctx, "capsule_get_destructor");
    int has_capsule_set_name = api->api_has_capability(module_ctx, "capsule_set_name");
    int has_capsule_is_valid = api->api_has_capability(module_ctx, "capsule_is_valid");
    int has_capsule_export = api->api_has_capability(module_ctx, "capsule_export");
    int has_capsule_import = api->api_has_capability(module_ctx, "capsule_import");
    int has_get_iter = api->api_has_capability(module_ctx, "object_get_iter");
    int has_iter_next = api->api_has_capability(module_ctx, "object_iter_next");
    int has_list_append = api->api_has_capability(module_ctx, "object_list_append");
    int has_list_set_item = api->api_has_capability(module_ctx, "object_list_set_item");
    int has_dict_contains = api->api_has_capability(module_ctx, "object_dict_contains");
    int has_dict_del_item = api->api_has_capability(module_ctx, "object_dict_del_item");
    int has_get_attr = api->api_has_capability(module_ctx, "object_get_attr");
    int has_set_attr = api->api_has_capability(module_ctx, "object_set_attr");
    int has_del_attr = api->api_has_capability(module_ctx, "object_del_attr");
    int has_has_attr = api->api_has_capability(module_ctx, "object_has_attr");
    int has_is_instance = api->api_has_capability(module_ctx, "object_is_instance");
    int has_is_subclass = api->api_has_capability(module_ctx, "object_is_subclass");
    int has_call_noargs = api->api_has_capability(module_ctx, "object_call_noargs");
    int has_call_onearg = api->api_has_capability(module_ctx, "object_call_onearg");
    int has_object_call = api->api_has_capability(module_ctx, "object_call");
    int has_error_get_message = api->api_has_capability(module_ctx, "error_get_message");
    int has_missing = api->api_has_capability(module_ctx, "does_not_exist");
    if (has_dict != 1 || has_kw != 1 || has_module_get_object != 1 ||
        has_module_import != 1 || has_module_get_attr != 1 || has_object_new_bytearray != 1 || has_object_new_memoryview != 1 ||
        has_module_set_state != 1 || has_module_get_state != 1 || has_module_set_finalize != 1 ||
        has_module_set_attr != 1 || has_module_del_attr != 1 || has_module_has_attr != 1 ||
        has_object_len != 1 || has_object_get_item != 1 ||
        has_object_set_item != 1 || has_object_del_item != 1 ||
        has_object_contains != 1 || has_object_dict_keys != 1 || has_object_dict_items != 1 ||
        has_object_get_buffer != 1 || has_object_get_writable_buffer != 1 ||
        has_object_get_buffer_info != 1 || has_object_get_buffer_info_v2 != 1 ||
        has_object_release_buffer != 1 ||
        has_capsule_new != 1 || has_capsule_get_pointer != 1 || has_capsule_set_pointer != 1 ||
        has_capsule_get_name != 1 ||
        has_capsule_set_context != 1 || has_capsule_get_context != 1 ||
        has_capsule_set_destructor != 1 || has_capsule_get_destructor != 1 ||
        has_capsule_set_name != 1 || has_capsule_is_valid != 1 ||
        has_capsule_export != 1 || has_capsule_import != 1 ||
        has_get_iter != 1 || has_iter_next != 1 ||
        has_list_append != 1 || has_list_set_item != 1 ||
        has_dict_contains != 1 || has_dict_del_item != 1 ||
        has_get_attr != 1 || has_set_attr != 1 || has_del_attr != 1 || has_has_attr != 1 ||
        has_is_instance != 1 || has_is_subclass != 1 ||
        has_call_noargs != 1 || has_call_onearg != 1 ||
        has_object_call != 1 || has_error_get_message != 1 || has_missing != 0) {
        return -2;
    }
    if (api->module_set_bool(module_ctx, "HAS_DICT", has_dict) != 0) {
        return -3;
    }
    if (api->module_set_bool(module_ctx, "HAS_KW", has_kw) != 0) {
        return -4;
    }
    if (api->module_set_bool(module_ctx, "HAS_MODULE_GET_OBJECT", has_module_get_object) != 0) {
        return -14;
    }
    if (api->module_set_bool(module_ctx, "HAS_MODULE_IMPORT", has_module_import) != 0) {
        return -16;
    }
    if (api->module_set_bool(module_ctx, "HAS_MODULE_GET_ATTR", has_module_get_attr) != 0) {
        return -19;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_NEW_BYTEARRAY", has_object_new_bytearray) != 0) {
        return -61;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_NEW_MEMORYVIEW", has_object_new_memoryview) != 0) {
        return -62;
    }
    if (api->module_set_bool(module_ctx, "HAS_MODULE_SET_STATE", has_module_set_state) != 0) {
        return -57;
    }
    if (api->module_set_bool(module_ctx, "HAS_MODULE_GET_STATE", has_module_get_state) != 0) {
        return -58;
    }
    if (api->module_set_bool(module_ctx, "HAS_MODULE_SET_FINALIZE", has_module_set_finalize) != 0) {
        return -60;
    }
    if (api->module_set_bool(module_ctx, "HAS_MODULE_SET_ATTR", has_module_set_attr) != 0) {
        return -27;
    }
    if (api->module_set_bool(module_ctx, "HAS_MODULE_DEL_ATTR", has_module_del_attr) != 0) {
        return -28;
    }
    if (api->module_set_bool(module_ctx, "HAS_MODULE_HAS_ATTR", has_module_has_attr) != 0) {
        return -29;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_LEN", has_object_len) != 0) {
        return -25;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_GET_ITEM", has_object_get_item) != 0) {
        return -26;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_SET_ITEM", has_object_set_item) != 0) {
        return -30;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_DEL_ITEM", has_object_del_item) != 0) {
        return -31;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_CONTAINS", has_object_contains) != 0) {
        return -32;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_DICT_KEYS", has_object_dict_keys) != 0) {
        return -33;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_DICT_ITEMS", has_object_dict_items) != 0) {
        return -34;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_GET_BUFFER", has_object_get_buffer) != 0) {
        return -35;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_GET_WRITABLE_BUFFER", has_object_get_writable_buffer) != 0) {
        return -63;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_GET_BUFFER_INFO", has_object_get_buffer_info) != 0) {
        return -64;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_GET_BUFFER_INFO_V2", has_object_get_buffer_info_v2) != 0) {
        return -65;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_RELEASE_BUFFER", has_object_release_buffer) != 0) {
        return -36;
    }
    if (api->module_set_bool(module_ctx, "HAS_CAPSULE_NEW", has_capsule_new) != 0) {
        return -37;
    }
    if (api->module_set_bool(module_ctx, "HAS_CAPSULE_GET_POINTER", has_capsule_get_pointer) != 0) {
        return -38;
    }
    if (api->module_set_bool(module_ctx, "HAS_CAPSULE_SET_POINTER", has_capsule_set_pointer) != 0) {
        return -59;
    }
    if (api->module_set_bool(module_ctx, "HAS_CAPSULE_GET_NAME", has_capsule_get_name) != 0) {
        return -39;
    }
    if (api->module_set_bool(module_ctx, "HAS_CAPSULE_SET_CONTEXT", has_capsule_set_context) != 0) {
        return -40;
    }
    if (api->module_set_bool(module_ctx, "HAS_CAPSULE_GET_CONTEXT", has_capsule_get_context) != 0) {
        return -41;
    }
    if (api->module_set_bool(module_ctx, "HAS_CAPSULE_SET_DESTRUCTOR", has_capsule_set_destructor) != 0) {
        return -42;
    }
    if (api->module_set_bool(module_ctx, "HAS_CAPSULE_GET_DESTRUCTOR", has_capsule_get_destructor) != 0) {
        return -52;
    }
    if (api->module_set_bool(module_ctx, "HAS_CAPSULE_SET_NAME", has_capsule_set_name) != 0) {
        return -53;
    }
    if (api->module_set_bool(module_ctx, "HAS_CAPSULE_IS_VALID", has_capsule_is_valid) != 0) {
        return -54;
    }
    if (api->module_set_bool(module_ctx, "HAS_CAPSULE_EXPORT", has_capsule_export) != 0) {
        return -55;
    }
    if (api->module_set_bool(module_ctx, "HAS_CAPSULE_IMPORT", has_capsule_import) != 0) {
        return -56;
    }
    if (api->module_set_bool(module_ctx, "HAS_GET_ITER", has_get_iter) != 0) {
        return -23;
    }
    if (api->module_set_bool(module_ctx, "HAS_ITER_NEXT", has_iter_next) != 0) {
        return -24;
    }
    if (api->module_set_bool(module_ctx, "HAS_LIST_APPEND", has_list_append) != 0) {
        return -6;
    }
    if (api->module_set_bool(module_ctx, "HAS_LIST_SET_ITEM", has_list_set_item) != 0) {
        return -7;
    }
    if (api->module_set_bool(module_ctx, "HAS_DICT_CONTAINS", has_dict_contains) != 0) {
        return -8;
    }
    if (api->module_set_bool(module_ctx, "HAS_DICT_DEL_ITEM", has_dict_del_item) != 0) {
        return -9;
    }
    if (api->module_set_bool(module_ctx, "HAS_GET_ATTR", has_get_attr) != 0) {
        return -11;
    }
    if (api->module_set_bool(module_ctx, "HAS_SET_ATTR", has_set_attr) != 0) {
        return -12;
    }
    if (api->module_set_bool(module_ctx, "HAS_DEL_ATTR", has_del_attr) != 0) {
        return -13;
    }
    if (api->module_set_bool(module_ctx, "HAS_HAS_ATTR", has_has_attr) != 0) {
        return -15;
    }
    if (api->module_set_bool(module_ctx, "HAS_IS_INSTANCE", has_is_instance) != 0) {
        return -17;
    }
    if (api->module_set_bool(module_ctx, "HAS_IS_SUBCLASS", has_is_subclass) != 0) {
        return -18;
    }
    if (api->module_set_bool(module_ctx, "HAS_CALL_NOARGS", has_call_noargs) != 0) {
        return -20;
    }
    if (api->module_set_bool(module_ctx, "HAS_CALL_ONEARG", has_call_onearg) != 0) {
        return -21;
    }
    if (api->module_set_bool(module_ctx, "HAS_OBJECT_CALL", has_object_call) != 0) {
        return -10;
    }
    if (api->module_set_bool(module_ctx, "HAS_ERROR_GET_MESSAGE", has_error_get_message) != 0) {
        return -22;
    }
    if (api->module_set_bool(module_ctx, "HAS_MISSING", has_missing) != 0) {
        return -5;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_capabilities");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_capabilities.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_capabilities\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_capabilities\nassert native_capabilities.HAS_DICT is True\nassert native_capabilities.HAS_KW is True\nassert native_capabilities.HAS_MODULE_GET_OBJECT is True\nassert native_capabilities.HAS_MODULE_IMPORT is True\nassert native_capabilities.HAS_MODULE_GET_ATTR is True\nassert native_capabilities.HAS_OBJECT_NEW_BYTEARRAY is True\nassert native_capabilities.HAS_OBJECT_NEW_MEMORYVIEW is True\nassert native_capabilities.HAS_MODULE_SET_STATE is True\nassert native_capabilities.HAS_MODULE_GET_STATE is True\nassert native_capabilities.HAS_MODULE_SET_FINALIZE is True\nassert native_capabilities.HAS_MODULE_SET_ATTR is True\nassert native_capabilities.HAS_MODULE_DEL_ATTR is True\nassert native_capabilities.HAS_MODULE_HAS_ATTR is True\nassert native_capabilities.HAS_OBJECT_LEN is True\nassert native_capabilities.HAS_OBJECT_GET_ITEM is True\nassert native_capabilities.HAS_OBJECT_SET_ITEM is True\nassert native_capabilities.HAS_OBJECT_DEL_ITEM is True\nassert native_capabilities.HAS_OBJECT_CONTAINS is True\nassert native_capabilities.HAS_OBJECT_DICT_KEYS is True\nassert native_capabilities.HAS_OBJECT_DICT_ITEMS is True\nassert native_capabilities.HAS_OBJECT_GET_BUFFER is True\nassert native_capabilities.HAS_OBJECT_GET_WRITABLE_BUFFER is True\nassert native_capabilities.HAS_OBJECT_GET_BUFFER_INFO is True\nassert native_capabilities.HAS_OBJECT_GET_BUFFER_INFO_V2 is True\nassert native_capabilities.HAS_OBJECT_RELEASE_BUFFER is True\nassert native_capabilities.HAS_CAPSULE_NEW is True\nassert native_capabilities.HAS_CAPSULE_GET_POINTER is True\nassert native_capabilities.HAS_CAPSULE_SET_POINTER is True\nassert native_capabilities.HAS_CAPSULE_GET_NAME is True\nassert native_capabilities.HAS_CAPSULE_SET_CONTEXT is True\nassert native_capabilities.HAS_CAPSULE_GET_CONTEXT is True\nassert native_capabilities.HAS_CAPSULE_SET_DESTRUCTOR is True\nassert native_capabilities.HAS_CAPSULE_GET_DESTRUCTOR is True\nassert native_capabilities.HAS_CAPSULE_SET_NAME is True\nassert native_capabilities.HAS_CAPSULE_IS_VALID is True\nassert native_capabilities.HAS_CAPSULE_EXPORT is True\nassert native_capabilities.HAS_CAPSULE_IMPORT is True\nassert native_capabilities.HAS_GET_ITER is True\nassert native_capabilities.HAS_ITER_NEXT is True\nassert native_capabilities.HAS_LIST_APPEND is True\nassert native_capabilities.HAS_LIST_SET_ITEM is True\nassert native_capabilities.HAS_DICT_CONTAINS is True\nassert native_capabilities.HAS_DICT_DEL_ITEM is True\nassert native_capabilities.HAS_GET_ATTR is True\nassert native_capabilities.HAS_SET_ATTR is True\nassert native_capabilities.HAS_DEL_ATTR is True\nassert native_capabilities.HAS_HAS_ATTR is True\nassert native_capabilities.HAS_IS_INSTANCE is True\nassert native_capabilities.HAS_IS_SUBCLASS is True\nassert native_capabilities.HAS_CALL_NOARGS is True\nassert native_capabilities.HAS_CALL_ONEARG is True\nassert native_capabilities.HAS_OBJECT_CALL is True\nassert native_capabilities.HAS_ERROR_GET_MESSAGE is True\nassert native_capabilities.HAS_MISSING is False",
    )
    .expect("capability-query extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_manage_module_state_lifecycle() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping module-state extension smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping module-state extension smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_module_state");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_module_state.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"
#include <stdint.h>

static int g_free_calls = 0;
static uintptr_t g_last_freed = 0;
static int g_finalize_calls = 0;
static uintptr_t g_last_finalized = 0;

static void free_state(void* state) {
    g_free_calls += 1;
    g_last_freed = (uintptr_t)state;
}

static void finalize_state(void* state) {
    g_finalize_calls += 1;
    g_last_finalized = (uintptr_t)state;
}

static int state_ptr(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    (void)argv;
    if (argc != 0 || !result) {
        return -10;
    }
    void* state = api->module_get_state(module_ctx);
    PyrsObjectHandle value = api->object_new_int(module_ctx, (int64_t)(uintptr_t)state);
    if (!value) {
        return -11;
    }
    *result = value;
    return 0;
}

static int free_calls(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    (void)argv;
    if (argc != 0 || !result) {
        return -12;
    }
    PyrsObjectHandle value = api->object_new_int(module_ctx, g_free_calls);
    if (!value) {
        return -13;
    }
    *result = value;
    return 0;
}

static int last_freed(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    (void)argv;
    if (argc != 0 || !result) {
        return -14;
    }
    PyrsObjectHandle value = api->object_new_int(module_ctx, (int64_t)g_last_freed);
    if (!value) {
        return -15;
    }
    *result = value;
    return 0;
}

static int finalize_calls(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    (void)argv;
    if (argc != 0 || !result) {
        return -22;
    }
    PyrsObjectHandle value = api->object_new_int(module_ctx, g_finalize_calls);
    if (!value) {
        return -23;
    }
    *result = value;
    return 0;
}

static int last_finalized(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    (void)argv;
    if (argc != 0 || !result) {
        return -24;
    }
    PyrsObjectHandle value = api->object_new_int(module_ctx, (int64_t)g_last_finalized);
    if (!value) {
        return -25;
    }
    *result = value;
    return 0;
}

static int replace_state(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    (void)argv;
    if (argc != 0 || !result) {
        return -16;
    }
    if (api->module_set_state(module_ctx, (void*)(uintptr_t)0x2222, free_state) != 0) {
        return -17;
    }
    PyrsObjectHandle value = api->object_new_int(module_ctx, g_free_calls);
    if (!value) {
        return -18;
    }
    *result = value;
    return 0;
}

static int clear_state(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    (void)argv;
    if (argc != 0 || !result) {
        return -19;
    }
    if (api->module_set_state(module_ctx, 0, 0) != 0) {
        return -20;
    }
    PyrsObjectHandle value = api->object_new_int(module_ctx, g_free_calls);
    if (!value) {
        return -21;
    }
    *result = value;
    return 0;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_get_state(module_ctx) != 0) {
        return -2;
    }
    if (api->module_set_finalize(module_ctx, finalize_state) != 0) {
        return -26;
    }
    if (api->module_set_state(module_ctx, (void*)(uintptr_t)0x1111, free_state) != 0) {
        return -3;
    }
    if (api->module_get_state(module_ctx) != (void*)(uintptr_t)0x1111) {
        return -4;
    }
    if (api->module_add_function(module_ctx, "state_ptr", state_ptr) != 0) {
        return -5;
    }
    if (api->module_add_function(module_ctx, "free_calls", free_calls) != 0) {
        return -6;
    }
    if (api->module_add_function(module_ctx, "last_freed", last_freed) != 0) {
        return -7;
    }
    if (api->module_add_function(module_ctx, "finalize_calls", finalize_calls) != 0) {
        return -10;
    }
    if (api->module_add_function(module_ctx, "last_finalized", last_finalized) != 0) {
        return -11;
    }
    if (api->module_add_function(module_ctx, "replace_state", replace_state) != 0) {
        return -8;
    }
    if (api->module_add_function(module_ctx, "clear_state", clear_state) != 0) {
        return -9;
    }
    if (api->module_set_bool(module_ctx, "STATE_READY", 1) != 0) {
        return -22;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_module_state");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_module_state.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_module_state\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_module_state as m\nassert m.STATE_READY is True\nassert m.state_ptr() == 0x1111\nassert m.free_calls() == 0\nassert m.finalize_calls() == 0\nassert m.replace_state() == 1\nassert m.last_freed() == 0x1111\nassert m.last_finalized() == 0x1111\nassert m.finalize_calls() == 1\nassert m.state_ptr() == 0x2222\nassert m.clear_state() == 2\nassert m.last_freed() == 0x2222\nassert m.last_finalized() == 0x2222\nassert m.finalize_calls() == 2\nassert m.state_ptr() == 0\nexpected = 2\nfor _ in range(20):\n    assert m.replace_state() == expected\n    expected += 1\n    assert m.clear_state() == expected\nassert m.free_calls() == expected\nassert m.finalize_calls() == expected\nassert m.state_ptr() == 0\nassert m.replace_state() == expected\nassert m.finalize_calls() == expected\nassert m.state_ptr() == 0x2222\nbefore_reimport_free = m.free_calls()\nbefore_reimport_finalize = m.finalize_calls()\nimport sys\ndel sys.modules['native_module_state']\nimport native_module_state as m2\nassert m2.STATE_READY is True\nassert m2.free_calls() == before_reimport_free + 1\nassert m2.finalize_calls() == before_reimport_finalize + 1\nassert m2.last_finalized() == 0x2222\nassert m2.state_ptr() == 0x1111",
    )
    .expect("module-state extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_module_state_drop_runs_finalize_before_free() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping module-state drop smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping module-state drop smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_module_state_drop");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");
    let log_path = temp_root.join("module_state_drop.log");
    let source_path = temp_root.join("native_module_state_drop.c");
    let source = r#"#include "pyrs_capi.h"
#include <stdint.h>
#include <stdio.h>

static const char* g_log_path = "__LOG_PATH__";

static void append_event(const char* event) {
    FILE* log_file = fopen(g_log_path, "a");
    if (!log_file) {
        return;
    }
    fputs(event, log_file);
    fputc('\n', log_file);
    fclose(log_file);
}

static void free_state(void* state) {
    (void)state;
    append_event("free");
}

static void finalize_state(void* state) {
    (void)state;
    append_event("finalize");
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_set_finalize(module_ctx, finalize_state) != 0) {
        return -2;
    }
    if (api->module_set_state(module_ctx, (void*)(uintptr_t)0x99, free_state) != 0) {
        return -3;
    }
    if (api->module_set_bool(module_ctx, "STATE_READY", 1) != 0) {
        return -4;
    }
    return 0;
}
"#
    .replace("__LOG_PATH__", &c_string_literal(&log_path));
    fs::write(&source_path, source).expect("source should be written");

    let library_file = shared_library_filename("native_module_state_drop");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_module_state_drop.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_module_state_drop\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_module_state_drop\nassert native_module_state_drop.STATE_READY is True",
    )
    .expect("module-state drop extension import should succeed");

    let log_content = fs::read_to_string(&log_path).expect("drop callbacks should write log");
    let events: Vec<_> = log_content.lines().collect();
    assert_eq!(events, vec!["finalize", "free"]);

    let _ = fs::remove_file(log_path);
    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_can_disable_module_state_finalize_callback() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping module-state finalize-disable smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping module-state finalize-disable smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_module_state_finalize_disable");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_module_state_finalize_disable.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"
#include <stdint.h>

static int g_free_calls = 0;
static int g_finalize_calls = 0;

static void free_state(void* state) {
    (void)state;
    g_free_calls += 1;
}

static void finalize_state(void* state) {
    (void)state;
    g_finalize_calls += 1;
}

static int free_calls(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    (void)argv;
    if (argc != 0 || !result) {
        return -10;
    }
    PyrsObjectHandle value = api->object_new_int(module_ctx, g_free_calls);
    if (!value) {
        return -11;
    }
    *result = value;
    return 0;
}

static int finalize_calls(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    (void)argv;
    if (argc != 0 || !result) {
        return -12;
    }
    PyrsObjectHandle value = api->object_new_int(module_ctx, g_finalize_calls);
    if (!value) {
        return -13;
    }
    *result = value;
    return 0;
}

static int clear_state(
    const PyrsApiV1* api,
    void* module_ctx,
    uintptr_t argc,
    const PyrsObjectHandle* argv,
    PyrsObjectHandle* result
) {
    (void)argv;
    if (argc != 0 || !result) {
        return -14;
    }
    if (api->module_set_state(module_ctx, 0, 0) != 0) {
        return -15;
    }
    PyrsObjectHandle value = api->object_new_int(module_ctx, g_free_calls);
    if (!value) {
        return -16;
    }
    *result = value;
    return 0;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    if (api->module_set_finalize(module_ctx, finalize_state) != 0) {
        return -2;
    }
    if (api->module_set_state(module_ctx, (void*)(uintptr_t)0x1234, free_state) != 0) {
        return -3;
    }
    if (api->module_set_finalize(module_ctx, 0) != 0) {
        return -4;
    }
    if (api->module_add_function(module_ctx, "free_calls", free_calls) != 0) {
        return -5;
    }
    if (api->module_add_function(module_ctx, "finalize_calls", finalize_calls) != 0) {
        return -6;
    }
    if (api->module_add_function(module_ctx, "clear_state", clear_state) != 0) {
        return -7;
    }
    if (api->module_set_bool(module_ctx, "STATE_READY", 1) != 0) {
        return -8;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_module_state_finalize_disable");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_module_state_finalize_disable.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_module_state_finalize_disable\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_module_state_finalize_disable as m\nassert m.STATE_READY is True\nassert m.free_calls() == 0\nassert m.finalize_calls() == 0\nassert m.clear_state() == 1\nassert m.free_calls() == 1\nassert m.finalize_calls() == 0",
    )
    .expect("module-state finalize-disable extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn dynamic_extension_module_state_apis_guard_null_module_ctx() {
    let Some(bin) = pyrs_bin() else {
        eprintln!("skipping null-module-ctx smoke (pyrs binary not found)");
        return;
    };
    if !has_c_compiler() {
        eprintln!("skipping null-module-ctx smoke (cc not available)");
        return;
    }

    let temp_root = unique_temp_dir("ext_smoke_null_module_ctx");
    fs::create_dir_all(&temp_root).expect("temp dir should be created");

    let source_path = temp_root.join("native_null_module_ctx.c");
    fs::write(
        &source_path,
        r#"#include "pyrs_capi.h"
#include <stdint.h>

static void free_state(void* state) {
    (void)state;
}

static void finalize_state(void* state) {
    (void)state;
}

int pyrs_extension_init_v1(const PyrsApiV1* api, void* module_ctx) {
    if (!api || api->abi_version != PYRS_CAPI_ABI_VERSION) {
        return -1;
    }
    int set_state_result = api->module_set_state(0, (void*)(uintptr_t)0x1, free_state);
    if (set_state_result != -1) {
        return -2;
    }
    if (api->module_get_state(0) != 0) {
        return -3;
    }
    int set_finalize_result = api->module_set_finalize(0, finalize_state);
    if (set_finalize_result != -1) {
        return -4;
    }
    if (api->error_get_message(0) != 0) {
        return -5;
    }
    if (api->module_set_bool(module_ctx, "NULL_CTX_GUARDS", 1) != 0) {
        return -6;
    }
    return 0;
}
"#,
    )
    .expect("source should be written");

    let library_file = shared_library_filename("native_null_module_ctx");
    let library_path = temp_root.join(&library_file);
    compile_shared_extension(&source_path, &library_path)
        .expect("compiled extension library should build");

    let manifest_path = temp_root.join("native_null_module_ctx.pyrs-ext");
    fs::write(
        &manifest_path,
        format!(
            "module=native_null_module_ctx\nabi=pyrs314\nentrypoint=dynamic:pyrs_extension_init_v1\nlibrary={library_file}\n"
        ),
    )
    .expect("manifest should be written");

    run_import_snippet(
        &bin,
        &temp_root,
        "import native_null_module_ctx\nassert native_null_module_ctx.NULL_CTX_GUARDS is True",
    )
    .expect("null-module-ctx extension import should succeed");

    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(library_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(temp_root);
}
