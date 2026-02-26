fn main() {
    let src = "src/vm/capi_variadics.c";
    println!("cargo:rerun-if-changed={src}");

    let mut build = cc::Build::new();
    build.file(src);
    build.flag_if_supported("-std=c11");

    if cfg!(unix) {
        build.flag_if_supported("-fPIC");
    }
    if cfg!(all(unix, not(target_vendor = "apple"))) {
        build.define("_POSIX_C_SOURCE", Some("200809L"));
    }
    if cfg!(target_os = "linux") {
        build.define("_GNU_SOURCE", Some("1"));
    }

    build.compile("pyrs_capi_variadics");
}
