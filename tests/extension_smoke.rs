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
    if (PyModule_AddIntConstant(module, "CALL_METHOD_VALUE", PyLong_AsLongLong(mapped)) != 0) {
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
    if (PyModule_AddIntConstant(module, "DICT_UPDATE_X", (int)PyLong_AsLongLong(updated_x)) != 0) {
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
    if (PyModule_AddIntConstant(module, "DICT_MERGE_X", (int)PyLong_AsLongLong(merged_x)) != 0) {
        return 0;
    }
    if (PyModule_AddIntConstant(module, "DICT_MERGE_Y", (int)PyLong_AsLongLong(merged_y)) != 0) {
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
    int list_first = (int)PyLong_AsLongLong(PyList_GetItem(list, 0));
    int list_second = (int)PyLong_AsLongLong(PyList_GetItem(list, 1));
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
    long long popped_value = PyLong_AsLongLong(popped);
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
    int noargs_ok = noargs_result && PyLong_AsLongLong(noargs_result) == 42 ? 1 : 0;

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
    int generated_method_nargs = generated_method_call ? (int)PyLong_AsLongLong(generated_method_call) : -1;
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
