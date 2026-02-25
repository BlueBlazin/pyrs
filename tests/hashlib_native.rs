use std::path::PathBuf;

use pyrs::{compiler, parser, runtime::Value, vm::Vm};

fn run_script(vm: &mut Vm, source: &str) {
    let module = parser::parse_module(source).expect("source should parse");
    let code = compiler::compile_module(&module).expect("source should compile");
    vm.execute(&code).expect("execution should succeed");
}

fn detect_cpython_lib() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PYRS_CPYTHON_LIB") {
        let path = PathBuf::from(path);
        if path.join("hashlib.py").is_file() {
            return Some(path);
        }
    }
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        workspace.join(".local/Python-3.14.3/Lib"),
        PathBuf::from("/Library/Frameworks/Python.framework/Versions/3.14/lib/python3.14"),
    ];
    for candidate in candidates {
        if candidate.join("hashlib.py").is_file() {
            return Some(candidate);
        }
    }
    None
}

#[test]
fn native_md5_and_sha2_backends_match_known_vectors() {
    let mut vm = Vm::new();
    run_script(
        &mut vm,
        r#"
import _md5, _sha2
m = _md5.md5(b"abc")
md5_abc = m.hexdigest()
m.update(b"def")
md5_abcdef = m.hexdigest()
md5_name = m.name
md5_digest_size = m.digest_size
md5_block_size = m.block_size
m_copy = m.copy()
copy_matches = m_copy.hexdigest() == md5_abcdef
m.update(b"g")
copy_stays = m_copy.hexdigest() == md5_abcdef

sha224_abc = _sha2.sha224(b"abc").hexdigest()
sha256_obj = _sha2.sha256(b"abc")
sha256_abc = sha256_obj.hexdigest()
sha256_name = sha256_obj.name
sha256_digest_size = sha256_obj.digest_size
sha256_block_size = sha256_obj.block_size
sha384_abc = _sha2.sha384(b"abc").hexdigest()
sha512_abc = _sha2.sha512(b"abc").hexdigest()
import _sha1, _sha3, _blake2
sha1_abc = _sha1.sha1(b"abc").hexdigest()
sha3_256_abc = _sha3.sha3_256(b"abc").hexdigest()
shake128_8 = _sha3.shake_128(b"abc").hexdigest(8)
blake2b_abc = _blake2.blake2b(b"abc").hexdigest()
blake2s_abc = _blake2.blake2s(b"abc").hexdigest()
blake2b_salt_size = _blake2.blake2b.SALT_SIZE
blake2b_person_size = _blake2.blake2b.PERSON_SIZE
blake2b_max_digest_size = _blake2.blake2b.MAX_DIGEST_SIZE
blake2b_max_key_size = _blake2.blake2b.MAX_KEY_SIZE
blake2s_salt_size = _blake2.blake2s.SALT_SIZE
blake2s_person_size = _blake2.blake2s.PERSON_SIZE
blake2s_max_digest_size = _blake2.blake2s.MAX_DIGEST_SIZE
blake2s_max_key_size = _blake2.blake2s.MAX_KEY_SIZE
"#,
    );

    assert_eq!(
        vm.get_global("md5_abc"),
        Some(Value::Str("900150983cd24fb0d6963f7d28e17f72".to_string()))
    );
    assert_eq!(
        vm.get_global("md5_abcdef"),
        Some(Value::Str("e80b5017098950fc58aad83c8c14978e".to_string()))
    );
    assert_eq!(
        vm.get_global("md5_name"),
        Some(Value::Str("md5".to_string()))
    );
    assert_eq!(vm.get_global("md5_digest_size"), Some(Value::Int(16)));
    assert_eq!(vm.get_global("md5_block_size"), Some(Value::Int(64)));
    assert_eq!(vm.get_global("copy_matches"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("copy_stays"), Some(Value::Bool(true)));

    assert_eq!(
        vm.get_global("sha224_abc"),
        Some(Value::Str(
            "23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7".to_string()
        ))
    );
    assert_eq!(
        vm.get_global("sha256_abc"),
        Some(Value::Str(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_string()
        ))
    );
    assert_eq!(
        vm.get_global("sha384_abc"),
        Some(Value::Str(
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded163\
1a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
                .to_string()
        ))
    );
    assert_eq!(
        vm.get_global("sha512_abc"),
        Some(Value::Str(
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20\
a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643\
ce80e2a9ac94fa54ca49f"
                .to_string()
        ))
    );
    assert_eq!(
        vm.get_global("sha256_name"),
        Some(Value::Str("sha256".to_string()))
    );
    assert_eq!(vm.get_global("sha256_digest_size"), Some(Value::Int(32)));
    assert_eq!(vm.get_global("sha256_block_size"), Some(Value::Int(64)));
    assert_eq!(
        vm.get_global("sha1_abc"),
        Some(Value::Str(
            "a9993e364706816aba3e25717850c26c9cd0d89d".to_string()
        ))
    );
    assert_eq!(
        vm.get_global("sha3_256_abc"),
        Some(Value::Str(
            "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532".to_string()
        ))
    );
    assert_eq!(
        vm.get_global("shake128_8"),
        Some(Value::Str("5881092dd818bf5c".to_string()))
    );
    assert_eq!(
        vm.get_global("blake2b_abc"),
        Some(Value::Str("ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923".to_string()))
    );
    assert_eq!(
        vm.get_global("blake2s_abc"),
        Some(Value::Str(
            "508c5e8c327c14e2e1a72ba34eeb452f37458b209ed63a294d999b4c86675982".to_string()
        ))
    );
    assert_eq!(vm.get_global("blake2b_salt_size"), Some(Value::Int(16)));
    assert_eq!(vm.get_global("blake2b_person_size"), Some(Value::Int(16)));
    assert_eq!(
        vm.get_global("blake2b_max_digest_size"),
        Some(Value::Int(64))
    );
    assert_eq!(vm.get_global("blake2b_max_key_size"), Some(Value::Int(64)));
    assert_eq!(vm.get_global("blake2s_salt_size"), Some(Value::Int(8)));
    assert_eq!(vm.get_global("blake2s_person_size"), Some(Value::Int(8)));
    assert_eq!(
        vm.get_global("blake2s_max_digest_size"),
        Some(Value::Int(32))
    );
    assert_eq!(vm.get_global("blake2s_max_key_size"), Some(Value::Int(32)));
}

#[test]
fn native_hashlib_constructors_match_cpython_argument_errors() {
    let mut vm = Vm::new();
    run_script(
        &mut vm,
        r#"
import _md5
def capture(cb):
    try:
        cb()
    except Exception as exc:
        return f"{type(exc).__name__}:{exc}"
    return "ok"

err_str = capture(lambda: _md5.md5("abc"))
err_conflict = capture(lambda: _md5.md5(data=b"a", string=b"b"))
err_keyword = capture(lambda: _md5.md5(foo=1))
err_blake2_keyword_precedes_conflict = capture(lambda: __import__("_blake2").blake2b(data=b"a", string=b"b", foo=1))
obj = _md5.md5()
err_update = capture(lambda: obj.update("abc"))
"#,
    );

    assert_eq!(
        vm.get_global("err_str"),
        Some(Value::Str(
            "TypeError:Strings must be encoded before hashing".to_string()
        ))
    );
    assert_eq!(
        vm.get_global("err_conflict"),
        Some(Value::Str(
            "TypeError:'data' and 'string' are mutually exclusive and support for 'string' keyword parameter is slated for removal in a future version.".to_string()
        ))
    );
    assert_eq!(
        vm.get_global("err_keyword"),
        Some(Value::Str(
            "TypeError:md5() got an unexpected keyword argument 'foo'".to_string()
        ))
    );
    assert_eq!(
        vm.get_global("err_blake2_keyword_precedes_conflict"),
        Some(Value::Str(
            "TypeError:blake2b() got an unexpected keyword argument 'foo'".to_string()
        ))
    );
    assert_eq!(
        vm.get_global("err_update"),
        Some(Value::Str(
            "TypeError:Strings must be encoded before hashing".to_string()
        ))
    );
}

#[test]
fn native_blake2_supports_full_constructor_parameter_block() {
    let mut vm = Vm::new();
    run_script(
        &mut vm,
        r#"
import _blake2
b2b = _blake2.blake2b(
    b"foo",
    digest_size=16,
    key=b"bar",
    salt=b"baz",
    person=b"bing",
    fanout=2,
    depth=3,
    leaf_size=4,
    node_offset=5,
    node_depth=6,
    inner_size=7,
    last_node=True,
).hexdigest()
b2s = _blake2.blake2s(
    b"foo",
    digest_size=16,
    key=b"bar",
    salt=b"baz",
    person=b"bing",
    fanout=2,
    depth=3,
    leaf_size=4,
    node_offset=5,
    node_depth=6,
    inner_size=7,
    last_node=True,
).hexdigest()
"#,
    );

    assert_eq!(
        vm.get_global("b2b"),
        Some(Value::Str("920568b0c5873b2f0ab67bedb6cf1b2b".to_string()))
    );
    assert_eq!(
        vm.get_global("b2s"),
        Some(Value::Str("bf2a8f7fe3c555012a6f8046e646bc75".to_string()))
    );
}

#[test]
fn native_hashlib_update_rejects_non_buffer_inputs_and_exposes_gil_threshold_constants() {
    let mut vm = Vm::new();
    run_script(
        &mut vm,
        r#"
import _hashlib, _sha2
def capture(cb):
    try:
        cb()
    except Exception as exc:
        return f"{type(exc).__name__}:{exc}"
    return "ok"

h = _hashlib.hmac_new(b"key", b"msg", "sha256")
err_update = capture(lambda: h.update([]))
err_digest = capture(lambda: _hashlib.hmac_digest(b"key", [], "sha256"))
gil_hashlib = _hashlib._GIL_MINSIZE
gil_sha2 = _sha2._GIL_MINSIZE
fromhex_ok = bytes.fromhex("00 ff").hex()
"#,
    );

    assert_eq!(
        vm.get_global("err_update"),
        Some(Value::Str(
            "TypeError:object supporting the buffer API required".to_string()
        ))
    );
    assert_eq!(
        vm.get_global("err_digest"),
        Some(Value::Str(
            "TypeError:object supporting the buffer API required".to_string()
        ))
    );
    assert_eq!(vm.get_global("gil_hashlib"), Some(Value::Int(2048)));
    assert_eq!(vm.get_global("gil_sha2"), Some(Value::Int(2048)));
    assert_eq!(
        vm.get_global("fromhex_ok"),
        Some(Value::Str("00ff".to_string()))
    );
}

#[test]
fn native_hashlib_exposes_constructors_mapping_for_clinic_signature_paths() {
    let mut vm = Vm::new();
    run_script(
        &mut vm,
        r#"
import _hashlib
has_constructors = hasattr(_hashlib, "_constructors")
contains_sha256 = "sha256" in _hashlib._constructors
md5_entry = _hashlib._constructors.get(_hashlib.openssl_md5)
"#,
    );

    assert_eq!(vm.get_global("has_constructors"), Some(Value::Bool(true)));
    // CPython stores callable keys in _constructors; string-membership is false.
    assert_eq!(vm.get_global("contains_sha256"), Some(Value::Bool(false)));
    assert_eq!(
        vm.get_global("md5_entry"),
        Some(Value::Str("md5".to_string()))
    );
}

#[test]
fn native_shake_digest_rejects_very_large_lengths() {
    let mut vm = Vm::new();
    run_script(
        &mut vm,
        r#"
import _sha3
def capture(cb):
    try:
        cb()
    except Exception as exc:
        return f"{type(exc).__name__}:{exc}"
    return "ok"
h = _sha3.shake_128(b"abc")
err_digest = capture(lambda: h.digest(2**29))
err_hexdigest = capture(lambda: h.hexdigest(2**29))
"#,
    );

    assert_eq!(
        vm.get_global("err_digest"),
        Some(Value::Str("ValueError:length is too large".to_string()))
    );
    assert_eq!(
        vm.get_global("err_hexdigest"),
        Some(Value::Str("ValueError:length is too large".to_string()))
    );
}

#[test]
fn native_sha3_objects_expose_capacity_rate_and_suffix_metadata() {
    let mut vm = Vm::new();
    run_script(
        &mut vm,
        r#"
import _sha3
s = _sha3.sha3_224()
k = _sha3.shake_128()
s_cap = s._capacity_bits
s_rate = s._rate_bits
s_suffix = s._suffix.hex()
k_cap = k._capacity_bits
k_rate = k._rate_bits
k_suffix = k._suffix.hex()
"#,
    );

    assert_eq!(vm.get_global("s_cap"), Some(Value::Int(448)));
    assert_eq!(vm.get_global("s_rate"), Some(Value::Int(1152)));
    assert_eq!(
        vm.get_global("s_suffix"),
        Some(Value::Str("06".to_string()))
    );
    assert_eq!(vm.get_global("k_cap"), Some(Value::Int(256)));
    assert_eq!(vm.get_global("k_rate"), Some(Value::Int(1344)));
    assert_eq!(
        vm.get_global("k_suffix"),
        Some(Value::Str("1f".to_string()))
    );
}

#[test]
fn native_hash_types_are_immutable() {
    let mut vm = Vm::new();
    run_script(
        &mut vm,
        r#"
import _md5
def capture(cb):
    try:
        cb()
    except Exception as exc:
        return f"{type(exc).__name__}:{exc}"
    return "ok"
t = type(_md5.md5())
err = capture(lambda: setattr(t, "value", False))
"#,
    );

    assert_eq!(
        vm.get_global("err"),
        Some(Value::Str(
            "TypeError:cannot set attribute 'value' of immutable type 'md5'".to_string()
        ))
    );
}

#[test]
fn native_pbkdf2_hmac_accepts_explicit_dklen_none() {
    let mut vm = Vm::new();
    run_script(
        &mut vm,
        r#"
import _hashlib
out = _hashlib.pbkdf2_hmac(hash_name="sha1", password=b"password", salt=b"salt", iterations=1, dklen=None)
hex_out = out.hex()
size = len(out)
"#,
    );

    assert_eq!(vm.get_global("size"), Some(Value::Int(20)));
    assert_eq!(
        vm.get_global("hex_out"),
        Some(Value::Str(
            "0c60c80f961f0e71f3a9b524af6012062fe037a6".to_string()
        ))
    );
}

#[test]
fn native_scrypt_rejects_invalid_maxmem_values() {
    let mut vm = Vm::new();
    run_script(
        &mut vm,
        r#"
import _hashlib
def capture(cb):
    try:
        cb()
    except Exception as exc:
        return f"{type(exc).__name__}:{exc}"
    return "ok"
err_neg = capture(lambda: _hashlib.scrypt(b"password", salt=b"salt", n=2, r=8, p=1, maxmem=-1))
err_none = capture(lambda: _hashlib.scrypt(b"password", salt=b"salt", n=2, r=8, p=1, maxmem=None))
"#,
    );

    assert_eq!(
        vm.get_global("err_neg"),
        Some(Value::Str(
            "ValueError:maxmem must be positive and smaller than 2147483647".to_string()
        ))
    );
    assert_eq!(
        vm.get_global("err_none"),
        Some(Value::Str("TypeError:unsupported operand type".to_string()))
    );
}

#[test]
fn hashlib_module_uses_native_md5_and_sha256_backends() {
    let Some(cpython_lib) = detect_cpython_lib() else {
        eprintln!("skipping hashlib stdlib path test (CPython Lib not found)");
        return;
    };
    let handle = std::thread::Builder::new()
        .name("hashlib-native-stdlib".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let mut vm = Vm::new();
            vm.add_module_path_front(cpython_lib);
            run_script(
                &mut vm,
                r#"
import hashlib
import binascii
md5_hex = hashlib.md5(b"abc", usedforsecurity=False).hexdigest()
sha256_hex = hashlib.sha256(b"abc").hexdigest()
sha1_hex = hashlib.sha1(b"abc").hexdigest()
sha3_hex = hashlib.sha3_256(b"abc").hexdigest()
b2b_hex = hashlib.blake2b(b"abc").hexdigest()
shake_hex = hashlib.shake_128(b"abc").hexdigest(8)
pbkdf2_hex = binascii.hexlify(hashlib.pbkdf2_hmac("sha256", b"password", b"salt", 1, 32)).decode()
scrypt_len = len(hashlib.scrypt(b"password", salt=b"salt", n=16, r=8, p=1, dklen=64))
has_md5 = hasattr(hashlib, "md5")
has_sha256 = hasattr(hashlib, "sha256")
has_pbkdf2 = hasattr(hashlib, "pbkdf2_hmac")
has_scrypt = hasattr(hashlib, "scrypt")
"#,
            );

            assert_eq!(
                vm.get_global("md5_hex"),
                Some(Value::Str("900150983cd24fb0d6963f7d28e17f72".to_string()))
            );
            assert_eq!(
                vm.get_global("sha256_hex"),
                Some(Value::Str(
                    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_string()
                ))
            );
            assert_eq!(
                vm.get_global("sha1_hex"),
                Some(Value::Str(
                    "a9993e364706816aba3e25717850c26c9cd0d89d".to_string()
                ))
            );
            assert_eq!(
                vm.get_global("sha3_hex"),
                Some(Value::Str(
                    "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532"
                        .to_string()
                ))
            );
            assert_eq!(
                vm.get_global("b2b_hex"),
                Some(Value::Str("ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923".to_string()))
            );
            assert_eq!(
                vm.get_global("shake_hex"),
                Some(Value::Str("5881092dd818bf5c".to_string()))
            );
            assert_eq!(
                vm.get_global("pbkdf2_hex"),
                Some(Value::Str(
                    "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
                        .to_string()
                ))
            );
            assert_eq!(vm.get_global("scrypt_len"), Some(Value::Int(64)));
            assert_eq!(vm.get_global("has_md5"), Some(Value::Bool(true)));
            assert_eq!(vm.get_global("has_sha256"), Some(Value::Bool(true)));
            assert_eq!(vm.get_global("has_pbkdf2"), Some(Value::Bool(true)));
            assert_eq!(vm.get_global("has_scrypt"), Some(Value::Bool(true)));
        })
        .expect("spawn hashlib stdlib regression thread");
    handle
        .join()
        .expect("hashlib stdlib regression thread should complete");
}

#[test]
fn native_hashlib_hmac_surface_matches_expected_vectors() {
    let mut vm = Vm::new();
    run_script(
        &mut vm,
        r#"
import _hashlib
h = _hashlib.hmac_new(b"key", b"The quick brown fox jumps over the lazy dog", "sha256")
hmac_hex = h.hexdigest()
hmac_name = h.name
hmac_digest_size = h.digest_size
hmac_block_size = h.block_size
h_copy = h.copy()
h.update(b"!")
h_copy_stable = h_copy.hexdigest() == hmac_hex
single_eq = _hashlib.hmac_digest(b"key", b"data", "sha256") == _hashlib.hmac_new(b"key", b"data", "sha256").digest()
single_eq_kw = _hashlib.hmac_digest(key=b"key", msg=b"data", digest="sha256") == _hashlib.hmac_new(b"key", b"data", "sha256").digest()
try:
    _hashlib.hmac_new(b"key", b"data", object())
except Exception as exc:
    unsupported_exc = type(exc).__name__
unsupported_is_value_error = issubclass(_hashlib.UnsupportedDigestmodError, ValueError)
try:
    _hashlib.hmac_new(b"key", b"data", "shake_128")
except Exception as exc:
    shake_exc = type(exc).__name__
try:
    _hashlib.HMAC.value = None
except Exception as exc:
    immutable_exc = type(exc).__name__
class B(bytes): pass
class S(str): pass
bytes_subclass_compare = _hashlib.compare_digest(B(b"abc"), B(b"abc"))
str_subclass_compare = _hashlib.compare_digest(S("abc"), S("abc"))
"#,
    );

    assert_eq!(
        vm.get_global("hmac_hex"),
        Some(Value::Str(
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8".to_string()
        ))
    );
    assert_eq!(
        vm.get_global("hmac_name"),
        Some(Value::Str("hmac-sha256".to_string()))
    );
    assert_eq!(vm.get_global("hmac_digest_size"), Some(Value::Int(32)));
    assert_eq!(vm.get_global("hmac_block_size"), Some(Value::Int(64)));
    assert_eq!(vm.get_global("h_copy_stable"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("single_eq"), Some(Value::Bool(true)));
    assert_eq!(vm.get_global("single_eq_kw"), Some(Value::Bool(true)));
    assert_eq!(
        vm.get_global("unsupported_exc"),
        Some(Value::Str("UnsupportedDigestmodError".to_string()))
    );
    assert_eq!(
        vm.get_global("unsupported_is_value_error"),
        Some(Value::Bool(true))
    );
    assert_eq!(
        vm.get_global("shake_exc"),
        Some(Value::Str("ValueError".to_string()))
    );
    assert_eq!(
        vm.get_global("immutable_exc"),
        Some(Value::Str("TypeError".to_string()))
    );
    assert_eq!(
        vm.get_global("bytes_subclass_compare"),
        Some(Value::Bool(true))
    );
    assert_eq!(
        vm.get_global("str_subclass_compare"),
        Some(Value::Bool(true))
    );
}
