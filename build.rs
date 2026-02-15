use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let src = PathBuf::from("src/vm/cpython_varargs_shim.c");
    println!("cargo:rerun-if-changed={}", src.display());

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR should be set"));
    let object = out_dir.join("cpython_varargs_shim.o");
    let lib = out_dir.join("libcpython_varargs_shim.a");

    let status = Command::new("cc")
        .arg("-c")
        .arg(&src)
        .arg("-o")
        .arg(&object)
        .status()
        .expect("failed to invoke cc for cpython_varargs_shim.c");
    assert!(status.success(), "cc failed for cpython_varargs_shim.c");

    let status = Command::new("ar")
        .arg("crus")
        .arg(&lib)
        .arg(&object)
        .status()
        .expect("failed to invoke ar for cpython_varargs_shim.o");
    assert!(status.success(), "ar failed for cpython_varargs_shim.o");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=cpython_varargs_shim");
}
