#![cfg(not(target_arch = "wasm32"))]

use pyrs::vm::Vm;

unsafe extern "C" {
    static _Py_ctype_tolower: [u8; 256];
    static _Py_ctype_toupper: [u8; 256];
    static _Py_HashSecret: [u8; 24];
}

#[test]
fn vm_new_initializes_capi_runtime_tables_before_extensions_use_them() {
    let _vm = Vm::new();
    unsafe {
        assert_eq!(_Py_ctype_tolower[b'A' as usize], b'a');
        assert_eq!(_Py_ctype_toupper[b'a' as usize], b'A');
        assert!(_Py_HashSecret.iter().any(|&byte| byte != 0));
    }
}
