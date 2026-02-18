use std::ffi::{c_char, c_void};

use super::{
    CpythonCompatObject, CpythonNumberMethods, CpythonTypeObject, EMPTY_TYPE_FLAGS,
    cpython_long_nb_add_slot,
};

const fn empty_type(name: *const c_char) -> CpythonTypeObject {
    CpythonTypeObject {
        ob_refcnt: 1,
        ob_type: std::ptr::null_mut(),
        ob_size: 0,
        tp_name: name,
        tp_basicsize: std::mem::size_of::<CpythonCompatObject>() as isize,
        tp_itemsize: 0,
        tp_dealloc: std::ptr::null_mut(),
        tp_vectorcall_offset: 0,
        tp_getattr: std::ptr::null_mut(),
        tp_setattr: std::ptr::null_mut(),
        tp_as_async: std::ptr::null_mut(),
        tp_repr: std::ptr::null_mut(),
        tp_as_number: std::ptr::null_mut(),
        tp_as_sequence: std::ptr::null_mut(),
        tp_as_mapping: std::ptr::null_mut(),
        tp_hash: std::ptr::null_mut(),
        tp_call: std::ptr::null_mut(),
        tp_str: std::ptr::null_mut(),
        tp_getattro: std::ptr::null_mut(),
        tp_setattro: std::ptr::null_mut(),
        tp_as_buffer: std::ptr::null_mut(),
        tp_flags: EMPTY_TYPE_FLAGS,
        tp_doc: std::ptr::null(),
        tp_traverse: std::ptr::null_mut(),
        tp_clear: std::ptr::null_mut(),
        tp_richcompare: std::ptr::null_mut(),
        tp_weaklistoffset: 0,
        tp_iter: std::ptr::null_mut(),
        tp_iternext: std::ptr::null_mut(),
        tp_methods: std::ptr::null_mut(),
        tp_members: std::ptr::null_mut(),
        tp_getset: std::ptr::null_mut(),
        tp_base: std::ptr::null_mut(),
        tp_dict: std::ptr::null_mut(),
        tp_descr_get: std::ptr::null_mut(),
        tp_descr_set: std::ptr::null_mut(),
        tp_dictoffset: 0,
        tp_init: std::ptr::null_mut(),
        tp_alloc: std::ptr::null_mut(),
        tp_new: std::ptr::null_mut(),
        tp_free: std::ptr::null_mut(),
        tp_is_gc: std::ptr::null_mut(),
        tp_bases: std::ptr::null_mut(),
        tp_mro: std::ptr::null_mut(),
        tp_cache: std::ptr::null_mut(),
        tp_subclasses: std::ptr::null_mut(),
        tp_weaklist: std::ptr::null_mut(),
        tp_del: std::ptr::null_mut(),
        tp_version_tag: 0,
        tp_finalize: std::ptr::null_mut(),
        tp_vectorcall: std::ptr::null_mut(),
        tp_watched: 0,
        tp_versions_used: 0,
    }
}

pub(super) static mut PY_LONG_NUMBER_METHODS: CpythonNumberMethods = CpythonNumberMethods {
    nb_add: cpython_long_nb_add_slot as *mut c_void,
    nb_subtract: std::ptr::null_mut(),
    nb_multiply: std::ptr::null_mut(),
    nb_remainder: std::ptr::null_mut(),
    nb_divmod: std::ptr::null_mut(),
    nb_power: std::ptr::null_mut(),
    nb_negative: std::ptr::null_mut(),
    nb_positive: std::ptr::null_mut(),
    nb_absolute: std::ptr::null_mut(),
    nb_bool: std::ptr::null_mut(),
    nb_invert: std::ptr::null_mut(),
    nb_lshift: std::ptr::null_mut(),
    nb_rshift: std::ptr::null_mut(),
    nb_and: std::ptr::null_mut(),
    nb_xor: std::ptr::null_mut(),
    nb_or: std::ptr::null_mut(),
    nb_int: None,
    nb_reserved: std::ptr::null_mut(),
    nb_float: std::ptr::null_mut(),
    nb_inplace_add: std::ptr::null_mut(),
    nb_inplace_subtract: std::ptr::null_mut(),
    nb_inplace_multiply: std::ptr::null_mut(),
    nb_inplace_remainder: std::ptr::null_mut(),
    nb_inplace_power: std::ptr::null_mut(),
    nb_inplace_lshift: std::ptr::null_mut(),
    nb_inplace_rshift: std::ptr::null_mut(),
    nb_inplace_and: std::ptr::null_mut(),
    nb_inplace_xor: std::ptr::null_mut(),
    nb_inplace_or: std::ptr::null_mut(),
    nb_floor_divide: std::ptr::null_mut(),
    nb_true_divide: std::ptr::null_mut(),
    nb_inplace_floor_divide: std::ptr::null_mut(),
    nb_inplace_true_divide: std::ptr::null_mut(),
    nb_index: None,
    nb_matrix_multiply: std::ptr::null_mut(),
    nb_inplace_matrix_multiply: std::ptr::null_mut(),
};

static PY_TYPE_NAME_OBJECT: &[u8; 7] = b"object\0";
static PY_TYPE_NAME_TYPE: &[u8; 5] = b"type\0";
static PY_TYPE_NAME_BOOL: &[u8; 5] = b"bool\0";
static PY_TYPE_NAME_BYTEARRAY: &[u8; 10] = b"bytearray\0";
static PY_TYPE_NAME_BYTEARRAY_ITER: &[u8; 19] = b"bytearray_iterator\0";
static PY_TYPE_NAME_BYTES: &[u8; 6] = b"bytes\0";
static PY_TYPE_NAME_BYTES_ITER: &[u8; 15] = b"bytes_iterator\0";
static PY_TYPE_NAME_CALL_ITER: &[u8; 18] = b"callable_iterator\0";
static PY_TYPE_NAME_CFUNCTION: &[u8; 27] = b"builtin_function_or_method\0";
static PY_TYPE_NAME_CAPSULE: &[u8; 8] = b"capsule\0";
static PY_TYPE_NAME_CLASSMETHOD_DESCR: &[u8; 18] = b"classmethod_descr\0";
static PY_TYPE_NAME_COMPLEX: &[u8; 8] = b"complex\0";
static PY_TYPE_NAME_DICT_PROXY: &[u8; 10] = b"dictproxy\0";
static PY_TYPE_NAME_DICT_ITEMS: &[u8; 11] = b"dict_items\0";
static PY_TYPE_NAME_DICT_ITER_ITEM: &[u8; 18] = b"dict_itemiterator\0";
static PY_TYPE_NAME_DICT_ITER_KEY: &[u8; 17] = b"dict_keyiterator\0";
static PY_TYPE_NAME_DICT_ITER_VALUE: &[u8; 19] = b"dict_valueiterator\0";
static PY_TYPE_NAME_DICT_KEYS: &[u8; 10] = b"dict_keys\0";
static PY_TYPE_NAME_DICT_REV_ITER_ITEM: &[u8; 25] = b"dict_reverseitemiterator\0";
static PY_TYPE_NAME_DICT_REV_ITER_KEY: &[u8; 24] = b"dict_reversekeyiterator\0";
static PY_TYPE_NAME_DICT_REV_ITER_VALUE: &[u8; 26] = b"dict_reversevalueiterator\0";
static PY_TYPE_NAME_DICT: &[u8; 5] = b"dict\0";
static PY_TYPE_NAME_DICT_VALUES: &[u8; 12] = b"dict_values\0";
static PY_TYPE_NAME_ELLIPSIS: &[u8; 9] = b"ellipsis\0";
static PY_TYPE_NAME_ENUM: &[u8; 5] = b"enum\0";
static PY_TYPE_NAME_FILTER: &[u8; 7] = b"filter\0";
static PY_TYPE_NAME_FLOAT: &[u8; 6] = b"float\0";
static PY_TYPE_NAME_FROZENSET: &[u8; 10] = b"frozenset\0";
static PY_TYPE_NAME_GETSET_DESCR: &[u8; 13] = b"getset_descr\0";
static PY_TYPE_NAME_GENERIC_ALIAS: &[u8; 19] = b"types.GenericAlias\0";
static PY_TYPE_NAME_LIST: &[u8; 5] = b"list\0";
static PY_TYPE_NAME_LIST_ITER: &[u8; 14] = b"list_iterator\0";
static PY_TYPE_NAME_LIST_REV_ITER: &[u8; 21] = b"list_reverseiterator\0";
static PY_TYPE_NAME_LONG: &[u8; 4] = b"int\0";
static PY_TYPE_NAME_LONG_RANGE_ITER: &[u8; 19] = b"longrange_iterator\0";
static PY_TYPE_NAME_MAP: &[u8; 4] = b"map\0";
static PY_TYPE_NAME_MEMBER_DESCR: &[u8; 13] = b"member_descr\0";
static PY_TYPE_NAME_MEMORYVIEW: &[u8; 11] = b"memoryview\0";
static PY_TYPE_NAME_METHOD: &[u8; 7] = b"method\0";
static PY_TYPE_NAME_METHOD_DESCR: &[u8; 13] = b"method_descr\0";
static PY_TYPE_NAME_MODULE_DEF: &[u8; 10] = b"moduledef\0";
static PY_TYPE_NAME_MODULE: &[u8; 7] = b"module\0";
static PY_TYPE_NAME_NONE: &[u8; 9] = b"NoneType\0";
static PY_TYPE_NAME_PROPERTY: &[u8; 9] = b"property\0";
static PY_TYPE_NAME_RANGE_ITER: &[u8; 15] = b"range_iterator\0";
static PY_TYPE_NAME_RANGE: &[u8; 6] = b"range\0";
static PY_TYPE_NAME_REVERSED: &[u8; 9] = b"reversed\0";
static PY_TYPE_NAME_SEQ_ITER: &[u8; 9] = b"iterator\0";
static PY_TYPE_NAME_SET: &[u8; 4] = b"set\0";
static PY_TYPE_NAME_SET_ITER: &[u8; 13] = b"set_iterator\0";
static PY_TYPE_NAME_SLICE: &[u8; 6] = b"slice\0";
static PY_TYPE_NAME_SUPER: &[u8; 6] = b"super\0";
static PY_TYPE_NAME_TRACEBACK: &[u8; 10] = b"traceback\0";
static PY_TYPE_NAME_TUPLE: &[u8; 6] = b"tuple\0";
static PY_TYPE_NAME_TUPLE_ITER: &[u8; 15] = b"tuple_iterator\0";
static PY_TYPE_NAME_UNICODE: &[u8; 4] = b"str\0";
static PY_TYPE_NAME_UNICODE_ITER: &[u8; 13] = b"str_iterator\0";
static PY_TYPE_NAME_WEAKREF_CALLABLE_PROXY: &[u8; 26] = b"weakref.CallableProxyType\0";
static PY_TYPE_NAME_WEAKREF_PROXY: &[u8; 18] = b"weakref.ProxyType\0";
static PY_TYPE_NAME_WEAKREF_REF: &[u8; 22] = b"weakref.ReferenceType\0";
static PY_TYPE_NAME_WRAPPER_DESCR: &[u8; 19] = b"wrapper_descriptor\0";
static PY_TYPE_NAME_ZIP: &[u8; 4] = b"zip\0";

#[unsafe(no_mangle)]
#[used]
pub static mut PyBaseObject_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_OBJECT.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyType_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_TYPE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyBool_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_BOOL.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyByteArrayIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_BYTEARRAY_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyByteArray_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_BYTEARRAY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyBytesIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_BYTES_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyBytes_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_BYTES.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyCallIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_CALL_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyCFunction_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_CFUNCTION.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyCapsule_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_CAPSULE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyClassMethodDescr_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_CLASSMETHOD_DESCR.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyComplex_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_COMPLEX.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictItems_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_ITEMS.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictIterItem_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_ITER_ITEM.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictIterKey_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_ITER_KEY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictIterValue_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_ITER_VALUE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictKeys_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_KEYS.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictProxy_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_PROXY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictRevIterItem_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_REV_ITER_ITEM.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictRevIterKey_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_REV_ITER_KEY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictRevIterValue_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_REV_ITER_VALUE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDict_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_DICT.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyDictValues_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_DICT_VALUES.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyEllipsis_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_ELLIPSIS.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyEnum_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_ENUM.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyFilter_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_FILTER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyFloat_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_FLOAT.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyFrozenSet_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_FROZENSET.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyGetSetDescr_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_GETSET_DESCR.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut Py_GenericAliasType: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_GENERIC_ALIAS.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyList_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_LIST.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyListIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_LIST_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyListRevIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_LIST_REV_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyLong_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_LONG.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyLongRangeIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_LONG_RANGE_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyMap_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_MAP.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyMemberDescr_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_MEMBER_DESCR.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyMemoryView_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_MEMORYVIEW.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyMethod_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_METHOD.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyMethodDescr_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_METHOD_DESCR.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyModuleDef_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_MODULE_DEF.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyModule_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_MODULE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyNone_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_NONE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyProperty_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_PROPERTY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyRangeIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_RANGE_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyRange_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_RANGE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyReversed_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_REVERSED.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PySeqIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_SEQ_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PySet_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_SET.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PySetIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_SET_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PySlice_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_SLICE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PySuper_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_SUPER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyTraceBack_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_TRACEBACK.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyTuple_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_TUPLE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyTupleIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_TUPLE_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyUnicode_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_UNICODE.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyUnicodeIter_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_UNICODE_ITER.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut _PyWeakref_CallableProxyType: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_WEAKREF_CALLABLE_PROXY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut _PyWeakref_ProxyType: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_WEAKREF_PROXY.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut _PyWeakref_RefType: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_WEAKREF_REF.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyWrapperDescr_Type: CpythonTypeObject =
    empty_type(PY_TYPE_NAME_WRAPPER_DESCR.as_ptr().cast());
#[unsafe(no_mangle)]
#[used]
pub static mut PyZip_Type: CpythonTypeObject = empty_type(PY_TYPE_NAME_ZIP.as_ptr().cast());
