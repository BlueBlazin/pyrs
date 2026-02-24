use std::ffi::{c_char, c_int, c_void};

#[repr(C)]
pub(super) struct CpythonTypeObject {
    pub(super) ob_refcnt: isize,
    pub(super) ob_type: *mut c_void,
    pub(super) ob_size: isize,
    pub(super) tp_name: *const c_char,
    pub(super) tp_basicsize: isize,
    pub(super) tp_itemsize: isize,
    pub(super) tp_dealloc: *mut c_void,
    pub(super) tp_vectorcall_offset: isize,
    pub(super) tp_getattr: *mut c_void,
    pub(super) tp_setattr: *mut c_void,
    pub(super) tp_as_async: *mut c_void,
    pub(super) tp_repr: *mut c_void,
    pub(super) tp_as_number: *mut c_void,
    pub(super) tp_as_sequence: *mut c_void,
    pub(super) tp_as_mapping: *mut c_void,
    pub(super) tp_hash: *mut c_void,
    pub(super) tp_call: *mut c_void,
    pub(super) tp_str: *mut c_void,
    pub(super) tp_getattro: *mut c_void,
    pub(super) tp_setattro: *mut c_void,
    pub(super) tp_as_buffer: *mut c_void,
    pub(super) tp_flags: usize,
    pub(super) tp_doc: *const c_char,
    pub(super) tp_traverse: *mut c_void,
    pub(super) tp_clear: *mut c_void,
    pub(super) tp_richcompare: *mut c_void,
    pub(super) tp_weaklistoffset: isize,
    pub(super) tp_iter: *mut c_void,
    pub(super) tp_iternext: *mut c_void,
    pub(super) tp_methods: *mut c_void,
    pub(super) tp_members: *mut c_void,
    pub(super) tp_getset: *mut c_void,
    pub(super) tp_base: *mut CpythonTypeObject,
    pub(super) tp_dict: *mut c_void,
    pub(super) tp_descr_get: *mut c_void,
    pub(super) tp_descr_set: *mut c_void,
    pub(super) tp_dictoffset: isize,
    pub(super) tp_init: *mut c_void,
    pub(super) tp_alloc: *mut c_void,
    pub(super) tp_new: *mut c_void,
    pub(super) tp_free: *mut c_void,
    pub(super) tp_is_gc: *mut c_void,
    pub(super) tp_bases: *mut c_void,
    pub(super) tp_mro: *mut c_void,
    pub(super) tp_cache: *mut c_void,
    pub(super) tp_subclasses: *mut c_void,
    pub(super) tp_weaklist: *mut c_void,
    pub(super) tp_del: *mut c_void,
    pub(super) tp_version_tag: u32,
    pub(super) tp_finalize: *mut c_void,
    pub(super) tp_vectorcall: *mut c_void,
    pub(super) tp_watched: u8,
    pub(super) tp_versions_used: u16,
}

#[repr(C)]
pub(super) struct CpythonBufferProcs {
    pub(super) bf_getbuffer: Option<unsafe extern "C" fn(*mut c_void, *mut c_void, c_int) -> c_int>,
    pub(super) bf_releasebuffer: Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>,
}

#[repr(C)]
pub(super) struct CpythonAsyncMethods {
    pub(super) am_await: *mut c_void,
    pub(super) am_aiter: *mut c_void,
    pub(super) am_anext: *mut c_void,
    pub(super) am_send: *mut c_void,
}

#[repr(C)]
pub(super) struct CpythonNumberMethods {
    pub(super) nb_add: *mut c_void,
    pub(super) nb_subtract: *mut c_void,
    pub(super) nb_multiply: *mut c_void,
    pub(super) nb_remainder: *mut c_void,
    pub(super) nb_divmod: *mut c_void,
    pub(super) nb_power: *mut c_void,
    pub(super) nb_negative: *mut c_void,
    pub(super) nb_positive: *mut c_void,
    pub(super) nb_absolute: *mut c_void,
    pub(super) nb_bool: *mut c_void,
    pub(super) nb_invert: *mut c_void,
    pub(super) nb_lshift: *mut c_void,
    pub(super) nb_rshift: *mut c_void,
    pub(super) nb_and: *mut c_void,
    pub(super) nb_xor: *mut c_void,
    pub(super) nb_or: *mut c_void,
    pub(super) nb_int: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
    pub(super) nb_reserved: *mut c_void,
    pub(super) nb_float: *mut c_void,
    pub(super) nb_inplace_add: *mut c_void,
    pub(super) nb_inplace_subtract: *mut c_void,
    pub(super) nb_inplace_multiply: *mut c_void,
    pub(super) nb_inplace_remainder: *mut c_void,
    pub(super) nb_inplace_power: *mut c_void,
    pub(super) nb_inplace_lshift: *mut c_void,
    pub(super) nb_inplace_rshift: *mut c_void,
    pub(super) nb_inplace_and: *mut c_void,
    pub(super) nb_inplace_xor: *mut c_void,
    pub(super) nb_inplace_or: *mut c_void,
    pub(super) nb_floor_divide: *mut c_void,
    pub(super) nb_true_divide: *mut c_void,
    pub(super) nb_inplace_floor_divide: *mut c_void,
    pub(super) nb_inplace_true_divide: *mut c_void,
    pub(super) nb_index: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
    pub(super) nb_matrix_multiply: *mut c_void,
    pub(super) nb_inplace_matrix_multiply: *mut c_void,
}

#[repr(C)]
pub(super) struct CpythonMappingMethods {
    pub(super) mp_length: *mut c_void,
    pub(super) mp_subscript: *mut c_void,
    pub(super) mp_ass_subscript: *mut c_void,
}

#[repr(C)]
pub(super) struct CpythonSequenceMethods {
    pub(super) sq_length: *mut c_void,
    pub(super) sq_concat: *mut c_void,
    pub(super) sq_repeat: *mut c_void,
    pub(super) sq_item: *mut c_void,
    pub(super) was_sq_slice: *mut c_void,
    pub(super) sq_ass_item: *mut c_void,
    pub(super) was_sq_ass_slice: *mut c_void,
    pub(super) sq_contains: *mut c_void,
    pub(super) sq_inplace_concat: *mut c_void,
    pub(super) sq_inplace_repeat: *mut c_void,
}

#[repr(C)]
pub(super) struct CpythonComplexValue {
    pub(super) real: f64,
    pub(super) imag: f64,
}

#[repr(C)]
pub(super) struct CpythonSpecializationCache {
    pub(super) getitem: *mut c_void,
    pub(super) getitem_version: u32,
    pub(super) _padding: u32,
    pub(super) init: *mut c_void,
}

#[repr(C)]
pub(super) struct CpythonHeapTypeObject {
    pub(super) ht_type: CpythonTypeObject,
    pub(super) as_async: CpythonAsyncMethods,
    pub(super) as_number: CpythonNumberMethods,
    pub(super) as_mapping: CpythonMappingMethods,
    pub(super) as_sequence: CpythonSequenceMethods,
    pub(super) as_buffer: CpythonBufferProcs,
    pub(super) ht_name: *mut c_void,
    pub(super) ht_slots: *mut c_void,
    pub(super) ht_qualname: *mut c_void,
    pub(super) ht_cached_keys: *mut c_void,
    pub(super) ht_module: *mut c_void,
    pub(super) ht_tpname_storage: *mut c_char,
    pub(super) ht_token: *mut c_void,
    pub(super) spec_cache: CpythonSpecializationCache,
}

pub(super) const PY_MEMBER_T_SHORT: c_int = 0;
pub(super) const PY_MEMBER_T_INT: c_int = 1;
pub(super) const PY_MEMBER_T_LONG: c_int = 2;
pub(super) const PY_MEMBER_T_FLOAT: c_int = 3;
pub(super) const PY_MEMBER_T_DOUBLE: c_int = 4;
pub(super) const PY_MEMBER_T_STRING: c_int = 5;
pub(super) const PY_MEMBER_T_OBJECT: c_int = 6;
pub(super) const PY_MEMBER_T_CHAR: c_int = 7;
pub(super) const PY_MEMBER_T_BYTE: c_int = 8;
pub(super) const PY_MEMBER_T_UBYTE: c_int = 9;
pub(super) const PY_MEMBER_T_USHORT: c_int = 10;
pub(super) const PY_MEMBER_T_UINT: c_int = 11;
pub(super) const PY_MEMBER_T_ULONG: c_int = 12;
pub(super) const PY_MEMBER_T_STRING_INPLACE: c_int = 13;
pub(super) const PY_MEMBER_T_BOOL: c_int = 14;
pub(super) const PY_MEMBER_T_OBJECT_EX: c_int = 16;
pub(super) const PY_MEMBER_T_LONGLONG: c_int = 17;
pub(super) const PY_MEMBER_T_ULONGLONG: c_int = 18;
pub(super) const PY_MEMBER_T_PYSSIZET: c_int = 19;
pub(super) const PY_MEMBER_T_NONE: c_int = 20;
pub(super) const PY_MEMBER_READONLY: c_int = 1;
pub(super) const PY_MEMBER_RELATIVE_OFFSET: c_int = 8;
pub(super) const PY_TYPE_SLOT_BF_GETBUFFER: c_int = 1;
pub(super) const PY_TYPE_SLOT_BF_RELEASEBUFFER: c_int = 2;
pub(super) const PY_TYPE_SLOT_MP_ASS_SUBSCRIPT: c_int = 3;
pub(super) const PY_TYPE_SLOT_MP_LENGTH: c_int = 4;
pub(super) const PY_TYPE_SLOT_MP_SUBSCRIPT: c_int = 5;
pub(super) const PY_TYPE_SLOT_NB_ABSOLUTE: c_int = 6;
pub(super) const PY_TYPE_SLOT_NB_ADD: c_int = 7;
pub(super) const PY_TYPE_SLOT_NB_AND: c_int = 8;
pub(super) const PY_TYPE_SLOT_NB_BOOL: c_int = 9;
pub(super) const PY_TYPE_SLOT_NB_DIVMOD: c_int = 10;
pub(super) const PY_TYPE_SLOT_NB_FLOAT: c_int = 11;
pub(super) const PY_TYPE_SLOT_NB_FLOOR_DIVIDE: c_int = 12;
pub(super) const PY_TYPE_SLOT_NB_INDEX: c_int = 13;
pub(super) const PY_TYPE_SLOT_NB_INPLACE_ADD: c_int = 14;
pub(super) const PY_TYPE_SLOT_NB_INPLACE_AND: c_int = 15;
pub(super) const PY_TYPE_SLOT_NB_INPLACE_FLOOR_DIVIDE: c_int = 16;
pub(super) const PY_TYPE_SLOT_NB_INPLACE_LSHIFT: c_int = 17;
pub(super) const PY_TYPE_SLOT_NB_INPLACE_MULTIPLY: c_int = 18;
pub(super) const PY_TYPE_SLOT_NB_INPLACE_OR: c_int = 19;
pub(super) const PY_TYPE_SLOT_NB_INPLACE_POWER: c_int = 20;
pub(super) const PY_TYPE_SLOT_NB_INPLACE_REMAINDER: c_int = 21;
pub(super) const PY_TYPE_SLOT_NB_INPLACE_RSHIFT: c_int = 22;
pub(super) const PY_TYPE_SLOT_NB_INPLACE_SUBTRACT: c_int = 23;
pub(super) const PY_TYPE_SLOT_NB_INPLACE_TRUE_DIVIDE: c_int = 24;
pub(super) const PY_TYPE_SLOT_NB_INPLACE_XOR: c_int = 25;
pub(super) const PY_TYPE_SLOT_NB_INT: c_int = 26;
pub(super) const PY_TYPE_SLOT_NB_INVERT: c_int = 27;
pub(super) const PY_TYPE_SLOT_NB_LSHIFT: c_int = 28;
pub(super) const PY_TYPE_SLOT_NB_MULTIPLY: c_int = 29;
pub(super) const PY_TYPE_SLOT_NB_NEGATIVE: c_int = 30;
pub(super) const PY_TYPE_SLOT_NB_OR: c_int = 31;
pub(super) const PY_TYPE_SLOT_NB_POSITIVE: c_int = 32;
pub(super) const PY_TYPE_SLOT_NB_POWER: c_int = 33;
pub(super) const PY_TYPE_SLOT_NB_REMAINDER: c_int = 34;
pub(super) const PY_TYPE_SLOT_NB_RSHIFT: c_int = 35;
pub(super) const PY_TYPE_SLOT_NB_SUBTRACT: c_int = 36;
pub(super) const PY_TYPE_SLOT_NB_TRUE_DIVIDE: c_int = 37;
pub(super) const PY_TYPE_SLOT_NB_XOR: c_int = 38;
pub(super) const PY_TYPE_SLOT_SQ_ASS_ITEM: c_int = 39;
pub(super) const PY_TYPE_SLOT_SQ_CONCAT: c_int = 40;
pub(super) const PY_TYPE_SLOT_SQ_CONTAINS: c_int = 41;
pub(super) const PY_TYPE_SLOT_SQ_INPLACE_CONCAT: c_int = 42;
pub(super) const PY_TYPE_SLOT_SQ_INPLACE_REPEAT: c_int = 43;
pub(super) const PY_TYPE_SLOT_SQ_ITEM: c_int = 44;
pub(super) const PY_TYPE_SLOT_SQ_LENGTH: c_int = 45;
pub(super) const PY_TYPE_SLOT_SQ_REPEAT: c_int = 46;
pub(super) const PY_TYPE_SLOT_TP_ALLOC: c_int = 47;
pub(super) const PY_TYPE_SLOT_TP_BASE: c_int = 48;
pub(super) const PY_TYPE_SLOT_TP_BASES: c_int = 49;
pub(super) const PY_TYPE_SLOT_TP_CALL: c_int = 50;
pub(super) const PY_TYPE_SLOT_TP_CLEAR: c_int = 51;
pub(super) const PY_TYPE_SLOT_TP_DEALLOC: c_int = 52;
pub(super) const PY_TYPE_SLOT_TP_DEL: c_int = 53;
pub(super) const PY_TYPE_SLOT_TP_DESCR_GET: c_int = 54;
pub(super) const PY_TYPE_SLOT_TP_DESCR_SET: c_int = 55;
pub(super) const PY_TYPE_SLOT_TP_DOC: c_int = 56;
pub(super) const PY_TYPE_SLOT_TP_GETATTR: c_int = 57;
pub(super) const PY_TYPE_SLOT_TP_GETATTRO: c_int = 58;
pub(super) const PY_TYPE_SLOT_TP_HASH: c_int = 59;
pub(super) const PY_TYPE_SLOT_TP_INIT: c_int = 60;
pub(super) const PY_TYPE_SLOT_TP_IS_GC: c_int = 61;
pub(super) const PY_TYPE_SLOT_TP_ITER: c_int = 62;
pub(super) const PY_TYPE_SLOT_TP_ITERNEXT: c_int = 63;
pub(super) const PY_TYPE_SLOT_TP_METHODS: c_int = 64;
pub(super) const PY_TYPE_SLOT_TP_NEW: c_int = 65;
pub(super) const PY_TYPE_SLOT_TP_REPR: c_int = 66;
pub(super) const PY_TYPE_SLOT_TP_RICHCOMPARE: c_int = 67;
pub(super) const PY_TYPE_SLOT_TP_SETATTR: c_int = 68;
pub(super) const PY_TYPE_SLOT_TP_SETATTRO: c_int = 69;
pub(super) const PY_TYPE_SLOT_TP_STR: c_int = 70;
pub(super) const PY_TYPE_SLOT_TP_TRAVERSE: c_int = 71;
pub(super) const PY_TYPE_SLOT_TP_MEMBERS: c_int = 72;
pub(super) const PY_TYPE_SLOT_TP_GETSET: c_int = 73;
pub(super) const PY_TYPE_SLOT_TP_FREE: c_int = 74;
pub(super) const PY_TYPE_SLOT_NB_MATRIX_MULTIPLY: c_int = 75;
pub(super) const PY_TYPE_SLOT_NB_INPLACE_MATRIX_MULTIPLY: c_int = 76;
pub(super) const PY_TYPE_SLOT_AM_AWAIT: c_int = 77;
pub(super) const PY_TYPE_SLOT_AM_AITER: c_int = 78;
pub(super) const PY_TYPE_SLOT_AM_ANEXT: c_int = 79;
pub(super) const PY_TYPE_SLOT_TP_FINALIZE: c_int = 80;
pub(super) const PY_TYPE_SLOT_AM_SEND: c_int = 81;
pub(super) const PY_TYPE_SLOT_TP_VECTORCALL: c_int = 82;
pub(super) const PY_TYPE_SLOT_TP_TOKEN: c_int = 83;
pub(super) const PY_TYPE_SLOT_MAX: c_int = 83;
