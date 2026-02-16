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
fn exports_abi3_batch2_symbols() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = [
        "PyList_GetItem",
        "PyList_SetItem",
        "PyList_Insert",
        "PyList_GetSlice",
        "PyList_SetSlice",
        "PyList_Sort",
        "PyList_Reverse",
        "PySet_New",
        "PyFrozenSet_New",
        "PySet_Size",
        "PySet_Contains",
        "PySet_Add",
        "PySet_Discard",
        "PySet_Clear",
        "PySet_Pop",
        "PyException_GetTraceback",
        "PyException_GetCause",
        "PyException_GetContext",
        "PyException_GetArgs",
        "PyException_SetArgs",
        "PyGC_Collect",
        "PyGC_Enable",
        "PyGC_Disable",
        "PyGC_IsEnabled",
        "PyFloat_GetMax",
        "PyFloat_GetMin",
        "PyFloat_GetInfo",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing ABI batch2 symbols: {missing:?}"
    );
}

#[test]
fn exports_abi3_batch3_symbols() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = [
        "PyBytes_FromObject",
        "PyBytes_Concat",
        "PyBytes_ConcatAndDel",
        "PyErr_BadArgument",
        "PyErr_BadInternalCall",
        "PyErr_PrintEx",
        "PyErr_Display",
        "PyErr_DisplayException",
        "PyCFunction_Call",
        "PyCFunction_New",
        "PyCFunction_NewEx",
        "PyCMethod_New",
        "PyCFunction_GetFunction",
        "PyCFunction_GetSelf",
        "PyCFunction_GetFlags",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing ABI batch3 symbols: {missing:?}"
    );
}

#[test]
fn exports_abi3_batch4_symbols() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = [
        "PyImport_GetModuleDict",
        "PyImport_AddModuleRef",
        "PyImport_AddModuleObject",
        "PyImport_AddModule",
        "PyImport_GetModule",
        "PyImport_ImportModuleNoBlock",
        "PyImport_ImportModuleLevelObject",
        "PyImport_ImportModuleLevel",
        "PyImport_ReloadModule",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing ABI batch4 symbols: {missing:?}"
    );
}

#[test]
fn exports_abi3_batch5_symbols() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = [
        "PyErr_GetRaisedException",
        "PyErr_SetRaisedException",
        "PyErr_GetHandledException",
        "PyErr_SetHandledException",
        "PyErr_GetExcInfo",
        "PyErr_SetExcInfo",
        "PyFile_GetLine",
        "PyFile_WriteObject",
        "PyFile_WriteString",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing ABI batch5 symbols: {missing:?}"
    );
}

#[test]
fn exports_abi3_batch6_symbols() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = [
        "PyLong_FromSize_t",
        "PyLong_FromInt32",
        "PyLong_FromUInt32",
        "PyLong_FromInt64",
        "PyLong_FromUInt64",
        "PyLong_AsInt",
        "PyLong_AsInt32",
        "PyLong_AsUInt32",
        "PyLong_AsInt64",
        "PyLong_AsUInt64",
        "PyLong_AsSize_t",
        "PyLong_AsDouble",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing ABI batch6 symbols: {missing:?}"
    );
}

#[test]
fn exports_abi3_batch7_symbols() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = [
        "PyLong_AsUnsignedLongMask",
        "PyLong_AsUnsignedLongLongMask",
        "PyLong_FromString",
        "PyLong_GetInfo",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing ABI batch7 symbols: {missing:?}"
    );
}

#[test]
fn exports_abi3_batch8_symbols() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = [
        "PyLong_AsNativeBytes",
        "PyLong_FromNativeBytes",
        "PyLong_FromUnsignedNativeBytes",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing ABI batch8 symbols: {missing:?}"
    );
}

#[test]
fn exports_abi3_batch9_symbols() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = [
        "PyBuffer_IsContiguous",
        "PyBuffer_GetPointer",
        "PyBuffer_SizeFromFormat",
        "PyBuffer_FromContiguous",
        "PyBuffer_ToContiguous",
        "PyBuffer_FillContiguousStrides",
        "PyBuffer_FillInfo",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing ABI batch9 symbols: {missing:?}"
    );
}

#[test]
fn exports_abi3_batch10_symbols() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = [
        "PySequence_Length",
        "PySequence_GetSlice",
        "PySequence_SetItem",
        "PySequence_DelItem",
        "PySequence_SetSlice",
        "PySequence_DelSlice",
        "PySequence_List",
        "PySequence_Count",
        "PySequence_Index",
        "PySequence_In",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing ABI batch10 symbols: {missing:?}"
    );
}

#[test]
fn exports_abi3_batch11_symbols() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = ["PySlice_GetIndices", "PySlice_GetIndicesEx"];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing ABI batch11 symbols: {missing:?}"
    );
}

#[test]
fn exports_abi3_batch12_symbols() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = [
        "PyIter_NextItem",
        "PyIter_Send",
        "PyObject_CheckBuffer",
        "PyMemoryView_FromObject",
        "PyMemoryView_FromMemory",
        "PyMemoryView_FromBuffer",
        "PyMemoryView_GetContiguous",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing ABI batch12 symbols: {missing:?}"
    );
}

#[test]
fn exports_abi3_batch13_symbols() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = [
        "PyObject_CallNoArgs",
        "PyObject_CallMethodObjArgs",
        "PyObject_DelAttr",
        "PyObject_DelAttrString",
        "PyObject_DelItemString",
        "PyObject_Dir",
        "PyObject_GetOptionalAttrString",
        "PyObject_HasAttr",
        "PyObject_HasAttrWithError",
        "PyObject_HasAttrStringWithError",
        "PyObject_Length",
        "PyObject_Repr",
        "PyObject_SetAttr",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing ABI batch13 symbols: {missing:?}"
    );
}

#[test]
fn exports_abi3_batch14_symbols() {
    let symbols = exported_symbols(&pyrs_bin());
    let required = [
        "PyObject_ASCII",
        "PyObject_Calloc",
        "PyObject_CheckReadBuffer",
        "PyObject_AsReadBuffer",
        "PyObject_AsWriteBuffer",
        "PyObject_AsCharBuffer",
        "PyObject_CopyData",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| !symbols.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "missing ABI batch14 symbols: {missing:?}"
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
