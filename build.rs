fn main() {
    let src = "src/vm/capi_variadics.c";
    println!("cargo:rerun-if-changed={src}");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_ARCH");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_FAMILY");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_VENDOR");

    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if target_arch == "wasm32" {
        // Browser/WebAssembly builds do not support the native C-ABI bridge.
        return;
    }

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_family = std::env::var("CARGO_CFG_TARGET_FAMILY").unwrap_or_default();
    let target_vendor = std::env::var("CARGO_CFG_TARGET_VENDOR").unwrap_or_default();

    let mut build = cc::Build::new();
    build.file(src);
    build.flag_if_supported("-std=c11");

    if target_family == "unix" {
        build.flag_if_supported("-fPIC");
    }
    if target_family == "unix" && target_vendor != "apple" {
        build.define("_POSIX_C_SOURCE", Some("200809L"));
    }
    if target_os == "linux" {
        build.define("_GNU_SOURCE", Some("1"));
    }

    build.compile("pyrs_capi_variadics");

    if target_os == "linux" {
        // Export C-API symbols from the executable so dlopen'd extension modules
        // can resolve references back into the running pyrs process.
        println!("cargo:rustc-link-arg-bin=pyrs=-Wl,--export-dynamic");
    }
}
