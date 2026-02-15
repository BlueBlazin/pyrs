use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let src = "src/vm/capi_variadics.c";
    println!("cargo:rerun-if-changed={src}");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set"));
    let obj = out_dir.join("capi_variadics.o");
    let compiler = env::var("CC").unwrap_or_else(|_| "cc".to_string());

    let status = Command::new(&compiler)
        .arg("-std=c11")
        .arg("-fPIC")
        .arg("-c")
        .arg(src)
        .arg("-o")
        .arg(&obj)
        .status()
        .unwrap_or_else(|err| panic!("failed to invoke C compiler '{compiler}': {err}"));
    assert!(
        status.success(),
        "C compiler '{compiler}' failed building {src}"
    );

    println!("cargo:rustc-link-arg={}", obj.display());
}
