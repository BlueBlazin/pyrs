use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn pyrs_bin() -> PathBuf {
    let debug = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/pyrs");
    if debug.is_file() {
        return debug;
    }
    let release = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/pyrs");
    assert!(release.is_file(), "missing pyrs binary at {release:?}");
    release
}

fn exported_symbols(bin: &PathBuf) -> HashSet<String> {
    let nm_commands = vec![
        vec!["-gU".to_string(), bin.to_string_lossy().to_string()],
        vec!["-g".to_string(), bin.to_string_lossy().to_string()],
    ];
    let mut output = None;
    for args in nm_commands {
        let result = Command::new("nm")
            .args(args)
            .output()
            .expect("failed to invoke nm");
        if result.status.success() {
            output = Some(result.stdout);
            break;
        }
    }
    let stdout = output.expect("unable to read exported symbols with nm");
    let mut symbols = HashSet::new();
    for line in String::from_utf8_lossy(&stdout).lines() {
        let mut parts = line.split_whitespace();
        let symbol = match parts.next_back() {
            Some(name) => name,
            None => continue,
        };
        let normalized = if symbol.starts_with('_')
            && symbol.len() > 1
            && symbol.as_bytes()[1].is_ascii_alphabetic()
        {
            symbol[1..].to_string()
        } else {
            symbol.to_string()
        };
        symbols.insert(normalized);
    }
    symbols
}

#[test]
fn exports_first_abi3_symbol_slice() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = [
        "Py_IncRef",
        "Py_DecRef",
        "PyErr_SetString",
        "PyErr_Occurred",
        "PyModule_Create2",
        "PyObject_GetAttrString",
        "PyLong_FromLong",
        "PyLong_AsLong",
        "PyUnicode_FromString",
        "PyBytes_FromStringAndSize",
        "PyByteArray_Type",
        "PyByteArray_FromStringAndSize",
        "PyByteArray_AsString",
        "PyByteArray_Size",
        "PyCapsule_New",
        "PyCapsule_GetPointer",
        "PyCapsule_GetName",
        "PyCapsule_SetPointer",
        "PyCapsule_GetDestructor",
        "PyCapsule_SetDestructor",
        "PyDict_Keys",
        "PyDict_Values",
        "PyDict_Items",
        "PyDict_Clear",
        "PyDict_Update",
        "PyExc_RuntimeError",
        "PyExc_TypeError",
        "PyExc_ImportError",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing required ABI surface symbols: {missing:?}"
    );
}

#[test]
fn generates_abi3_manifest_snapshot() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    let out_path = std::env::temp_dir().join(format!("pyrs_abi3_manifest_{stamp}.json"));
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/generate_abi3_manifest.py");
    let status = Command::new("python3")
        .arg(script)
        .arg("--binary")
        .arg(pyrs_bin())
        .arg("--out")
        .arg(&out_path)
        .status()
        .expect("failed to run abi3 manifest script");
    assert!(status.success(), "abi3 manifest script failed: {status}");
    let payload = fs::read_to_string(&out_path).expect("failed to read generated manifest");
    assert!(
        payload.contains("\"function_count\"") && payload.contains("\"data_count\""),
        "manifest missing stable abi summary fields"
    );
    assert!(
        payload.contains("\"Py_IncRef\"") && payload.contains("\"PyExc_RuntimeError\""),
        "manifest missing expected core symbols"
    );
}
