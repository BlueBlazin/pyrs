use std::collections::HashMap;
use std::ffi::{c_char, c_void};

use crate::runtime::{BuiltinFunction, Object, Value};
use crate::vm::{InternalCallOutcome, STR_BACKING_STORAGE_ATTR};

use super::{
    _Py_NotImplementedStruct, CpythonMappingMethods, CpythonNumberMethods, CpythonObjectHead,
    CpythonSequenceMethods, CpythonStructSequenceField, CpythonTypeObject, ModuleCapiContext,
    Py_DecRef, PyErr_Occurred, PyExc_RuntimeError, PyExc_TypeError, PyType_IsSubtype,
    c_name_to_string, cpython_call_internal_in_context,
    cpython_exception_name_from_runtime_message, cpython_exception_ptr_for_name,
    cpython_exception_traceback_ptr_for_value, cpython_exception_type_ptr,
    cpython_exception_type_ptr_for_value, cpython_getattr_in_context,
    cpython_safe_object_type_name, cpython_set_error, cpython_set_typed_error,
    cpython_value_debug_tag, with_active_cpython_context_mut,
};

#[repr(C)]
struct DebugMpdT {
    flags: u8,
    _padding: [u8; 7],
    exp: isize,
    digits: isize,
    len: isize,
    alloc: isize,
    data: *mut c_void,
}

#[repr(C)]
struct DebugDecimalObject {
    head: CpythonObjectHead,
    hash: isize,
    dec: DebugMpdT,
}

pub(super) fn cpython_valid_type_ptr(type_ptr: *mut CpythonTypeObject) -> bool {
    const MIN_VALID_PTR: usize = super::MIN_VALID_PTR_THRESHOLD;
    if type_ptr.is_null() {
        return false;
    }
    let addr = type_ptr as usize;
    addr >= MIN_VALID_PTR && addr % std::mem::align_of::<CpythonTypeObject>() == 0
}

unsafe fn cpython_number_binop_slot(
    type_ptr: *mut CpythonTypeObject,
    slot_offset: usize,
) -> Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void> {
    if !cpython_valid_type_ptr(type_ptr) {
        return None;
    }
    // SAFETY: caller guarantees `type_ptr` points to a readable PyTypeObject.
    let as_number = unsafe { (*type_ptr).tp_as_number }.cast::<CpythonNumberMethods>();
    if as_number.is_null() {
        return None;
    }
    // SAFETY: `slot_offset` is an offset within `CpythonNumberMethods`.
    let slot_ptr = unsafe {
        as_number
            .cast::<u8>()
            .add(slot_offset)
            .cast::<*mut c_void>()
    };
    // SAFETY: slot pointer is readable when `tp_as_number` points to a valid table.
    let raw = unsafe { *slot_ptr };
    if raw.is_null() {
        return None;
    }
    // SAFETY: number binary slots all have `binaryfunc` signature.
    Some(unsafe { std::mem::transmute(raw) })
}

pub(super) unsafe fn cpython_mapping_ass_subscript_slot(
    type_ptr: *mut CpythonTypeObject,
) -> Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32> {
    if !cpython_valid_type_ptr(type_ptr) {
        return None;
    }
    // SAFETY: caller guarantees `type_ptr` points to a readable PyTypeObject.
    let as_mapping = unsafe { (*type_ptr).tp_as_mapping }.cast::<CpythonMappingMethods>();
    if as_mapping.is_null() {
        return None;
    }
    // SAFETY: mapping slot table is readable when `tp_as_mapping` is non-null.
    let raw = unsafe { (*as_mapping).mp_ass_subscript };
    if raw.is_null() {
        return None;
    }
    // SAFETY: mapping assign slot follows `objobjargproc` signature.
    Some(unsafe { std::mem::transmute(raw) })
}

pub(super) unsafe fn cpython_mapping_subscript_slot(
    type_ptr: *mut CpythonTypeObject,
) -> Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void> {
    if !cpython_valid_type_ptr(type_ptr) {
        return None;
    }
    // SAFETY: caller guarantees `type_ptr` points to a readable PyTypeObject.
    let as_mapping = unsafe { (*type_ptr).tp_as_mapping }.cast::<CpythonMappingMethods>();
    if as_mapping.is_null() {
        return None;
    }
    // SAFETY: mapping slot table is readable when `tp_as_mapping` is non-null.
    let raw = unsafe { (*as_mapping).mp_subscript };
    if raw.is_null() {
        return None;
    }
    // SAFETY: mapping subscript slot follows `binaryfunc` object/key ABI.
    Some(unsafe { std::mem::transmute(raw) })
}

pub(super) unsafe extern "C" fn cpython_runtime_mp_subscript_slot(
    object: *mut c_void,
    key: *mut c_void,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("runtime mp_subscript missing VM context");
            return std::ptr::null_mut();
        }
        let Some(object_value) = context.cpython_value_from_ptr_or_proxy(object) else {
            context.set_error("runtime mp_subscript received unknown object pointer");
            return std::ptr::null_mut();
        };
        let Some(key_value) = context.cpython_value_from_ptr_or_proxy(key) else {
            context.set_error("runtime mp_subscript received unknown key pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.getitem_value(object_value, key_value) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                if err.exception_name() == Some("TypeError")
                    && err.message.contains("subscript unsupported type")
                {
                    let type_name = cpython_safe_object_type_name(object)
                        .unwrap_or_else(|| "object".to_string());
                    cpython_set_typed_error(
                        unsafe { PyExc_TypeError },
                        format!("'{type_name}' object is not subscriptable"),
                    );
                } else {
                    cpython_set_error(err.message);
                }
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

pub(super) static mut PY_RUNTIME_MAPPING_METHODS: CpythonMappingMethods = CpythonMappingMethods {
    mp_length: std::ptr::null_mut(),
    mp_subscript: cpython_runtime_mp_subscript_slot as *mut c_void,
    mp_ass_subscript: std::ptr::null_mut(),
};

pub(super) unsafe fn cpython_sequence_item_slot(
    type_ptr: *mut CpythonTypeObject,
) -> Option<unsafe extern "C" fn(*mut c_void, isize) -> *mut c_void> {
    if !cpython_valid_type_ptr(type_ptr) {
        return None;
    }
    // SAFETY: caller guarantees `type_ptr` points to a readable PyTypeObject.
    let as_sequence = unsafe { (*type_ptr).tp_as_sequence }.cast::<CpythonSequenceMethods>();
    if as_sequence.is_null() {
        return None;
    }
    // SAFETY: sequence slot table is readable when `tp_as_sequence` is non-null.
    let raw = unsafe { (*as_sequence).sq_item };
    if raw.is_null() {
        return None;
    }
    // SAFETY: `sq_item` follows `ssizeargfunc` ABI.
    Some(unsafe { std::mem::transmute(raw) })
}

unsafe fn cpython_richcompare_slot(
    type_ptr: *mut CpythonTypeObject,
) -> Option<unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> *mut c_void> {
    if !cpython_valid_type_ptr(type_ptr) {
        return None;
    }
    // SAFETY: caller guarantees `type_ptr` points to a readable PyTypeObject.
    let raw = unsafe { (*type_ptr).tp_richcompare };
    if raw.is_null() {
        return None;
    }
    // SAFETY: tp_richcompare ABI matches richcmpfunc.
    Some(unsafe { std::mem::transmute(raw) })
}

fn cpython_swapped_compare_op(op: i32) -> Option<i32> {
    match op {
        0 => Some(4),
        1 => Some(5),
        2 => Some(2),
        3 => Some(3),
        4 => Some(0),
        5 => Some(1),
        _ => None,
    }
}

unsafe fn cpython_find_richcompare_slot(
    type_ptr: *mut CpythonTypeObject,
) -> Option<unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> *mut c_void> {
    let mut current = type_ptr;
    for _ in 0..64 {
        if !cpython_valid_type_ptr(current) {
            return None;
        }
        // SAFETY: `current` is validated at each step above.
        if let Some(slot) = unsafe { cpython_richcompare_slot(current) } {
            return Some(slot);
        }
        // SAFETY: `current` is a valid type pointer and `tp_base` is read-only metadata.
        let next = unsafe { (*current).tp_base };
        if next.is_null() || next == current {
            break;
        }
        current = next;
    }
    None
}

pub(super) fn cpython_try_richcompare_slot(
    left: *mut c_void,
    right: *mut c_void,
    op: i32,
) -> Option<*mut c_void> {
    const MIN_VALID_PTR: usize = super::MIN_VALID_PTR_THRESHOLD;
    if left.is_null() || right.is_null() {
        return None;
    }
    let left_addr = left as usize;
    let right_addr = right as usize;
    if left_addr < MIN_VALID_PTR
        || right_addr < MIN_VALID_PTR
        || left_addr % std::mem::align_of::<usize>() != 0
        || right_addr % std::mem::align_of::<usize>() != 0
    {
        return None;
    }
    // SAFETY: pointer validity checked above for non-null/alignment/min-address.
    let left_type = unsafe {
        left.cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    // SAFETY: pointer validity checked above for non-null/alignment/min-address.
    let right_type = unsafe {
        right
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if left_type.is_null() || right_type.is_null() {
        return None;
    }
    if !cpython_valid_type_ptr(left_type) || !cpython_valid_type_ptr(right_type) {
        return None;
    }
    let trace = super::super::env_var_present_cached("PYRS_TRACE_RICH_SLOT");
    if trace {
        // SAFETY: type pointers validated above.
        let left_name = unsafe {
            c_name_to_string((*left_type).tp_name).unwrap_or_else(|_| "<invalid>".to_string())
        };
        // SAFETY: type pointers validated above.
        let right_name = unsafe {
            c_name_to_string((*right_type).tp_name).unwrap_or_else(|_| "<invalid>".to_string())
        };
        eprintln!(
            "[cpy-rich-slot] begin op={op} left={left:p}({left_name}) right={right:p}({right_name})"
        );
    }
    let swapped_op = cpython_swapped_compare_op(op)?;
    // SAFETY: type pointers are non-null and read-only inspected.
    let slotv = unsafe { cpython_find_richcompare_slot(left_type) };
    // SAFETY: type pointers are non-null and read-only inspected.
    let mut slotw = if right_type != left_type {
        unsafe { cpython_find_richcompare_slot(right_type) }
    } else {
        None
    };
    if matches!((slotv, slotw), (Some(a), Some(b)) if (a as usize) == (b as usize)) {
        slotw = None;
    }
    if trace {
        eprintln!(
            "[cpy-rich-slot] slotv={} slotw={}",
            slotv.is_some(),
            slotw.is_some()
        );
    }
    if slotv.is_none() && slotw.is_none() {
        return None;
    }
    let not_implemented = std::ptr::addr_of_mut!(_Py_NotImplementedStruct).cast::<c_void>();
    let mut checked_reverse = false;
    if let Some(slotv_fn) = slotv {
        if let Some(slotw_fn) = slotw
            // SAFETY: type pointers are valid for subtype test.
            && unsafe { PyType_IsSubtype(right_type.cast(), left_type.cast()) != 0 }
        {
            checked_reverse = true;
            // SAFETY: richcompare slot ABI matches richcmpfunc.
            let result = unsafe { slotw_fn(right, left, swapped_op) };
            if result.is_null() {
                if trace {
                    eprintln!("[cpy-rich-slot] subtype-right slot returned error");
                }
                return Some(std::ptr::null_mut());
            }
            if result != not_implemented {
                if trace {
                    eprintln!(
                        "[cpy-rich-slot] subtype-right slot returned value {:p}",
                        result
                    );
                }
                return Some(result);
            }
            // SAFETY: slot returned new reference to NotImplemented.
            unsafe { Py_DecRef(result) };
            slotw = None;
        }
        // SAFETY: richcompare slot ABI matches richcmpfunc.
        let result = unsafe { slotv_fn(left, right, op) };
        if result.is_null() {
            if trace {
                eprintln!("[cpy-rich-slot] left slot returned error");
            }
            return Some(std::ptr::null_mut());
        }
        if result != not_implemented {
            if trace {
                eprintln!("[cpy-rich-slot] left slot returned value {:p}", result);
            }
            return Some(result);
        }
        // SAFETY: slot returned new reference to NotImplemented.
        unsafe { Py_DecRef(result) };
    }
    if !checked_reverse && let Some(slotw_fn) = slotw {
        // SAFETY: richcompare slot ABI matches richcmpfunc.
        let result = unsafe { slotw_fn(right, left, swapped_op) };
        if result.is_null() {
            if trace {
                eprintln!("[cpy-rich-slot] right slot returned error");
            }
            return Some(std::ptr::null_mut());
        }
        if result != not_implemented {
            if trace {
                eprintln!("[cpy-rich-slot] right slot returned value {:p}", result);
            }
            return Some(result);
        }
        // SAFETY: slot returned new reference to NotImplemented.
        unsafe { Py_DecRef(result) };
    }
    None
}

pub(super) fn cpython_try_binary_number_slot(
    left: *mut c_void,
    right: *mut c_void,
    slot_offset: usize,
) -> Option<*mut c_void> {
    let trace = super::super::env_var_present_cached("PYRS_TRACE_NUMBER_SLOT");
    const MIN_VALID_PTR: usize = super::MIN_VALID_PTR_THRESHOLD;
    if left.is_null() || right.is_null() {
        return None;
    }
    let left_addr = left as usize;
    let right_addr = right as usize;
    if left_addr < MIN_VALID_PTR
        || right_addr < MIN_VALID_PTR
        || left_addr % std::mem::align_of::<usize>() != 0
        || right_addr % std::mem::align_of::<usize>() != 0
    {
        return None;
    }
    // SAFETY: pointer validity checked above for non-null/alignment/min-address.
    let left_type = unsafe {
        left.cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    // SAFETY: pointer validity checked above for non-null/alignment/min-address.
    let right_type = unsafe {
        right
            .cast::<CpythonObjectHead>()
            .as_ref()
            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
            .unwrap_or(std::ptr::null_mut())
    };
    if left_type.is_null() || right_type.is_null() {
        return None;
    }
    if trace {
        // SAFETY: type pointers were validated above for non-null.
        let (left_name, right_name, left_refcnt, right_refcnt, left_basicsize, right_basicsize) = unsafe {
            let left_name =
                c_name_to_string((*left_type).tp_name).unwrap_or_else(|_| "<invalid>".to_string());
            let right_name =
                c_name_to_string((*right_type).tp_name).unwrap_or_else(|_| "<invalid>".to_string());
            let left_refcnt = left
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_refcnt)
                .unwrap_or(-1);
            let right_refcnt = right
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_refcnt)
                .unwrap_or(-1);
            (
                left_name,
                right_name,
                left_refcnt,
                right_refcnt,
                (*left_type).tp_basicsize,
                (*right_type).tp_basicsize,
            )
        };
        eprintln!(
            "[cpy-number-slot] begin slot_off={} left={:p} type={:p}({}) rc={} basicsize={} right={:p} type={:p}({}) rc={} basicsize={}",
            slot_offset,
            left,
            left_type,
            left_name,
            left_refcnt,
            left_basicsize,
            right,
            right_type,
            right_name,
            right_refcnt,
            right_basicsize
        );
        if left_name == "decimal.Decimal" {
            // SAFETY: debug-only memory probe for Decimal-compatible layout.
            unsafe {
                if let Some(left_dec) = left.cast::<DebugDecimalObject>().as_ref() {
                    eprintln!(
                        "[cpy-number-slot] left-dec ptr={:p} flags={} exp={} digits={} len={} alloc={} data={:p}",
                        left,
                        left_dec.dec.flags,
                        left_dec.dec.exp,
                        left_dec.dec.digits,
                        left_dec.dec.len,
                        left_dec.dec.alloc,
                        left_dec.dec.data
                    );
                }
                if let Some(right_dec) = right.cast::<DebugDecimalObject>().as_ref() {
                    eprintln!(
                        "[cpy-number-slot] right-dec ptr={:p} flags={} exp={} digits={} len={} alloc={} data={:p}",
                        right,
                        right_dec.dec.flags,
                        right_dec.dec.exp,
                        right_dec.dec.digits,
                        right_dec.dec.len,
                        right_dec.dec.alloc,
                        right_dec.dec.data
                    );
                }
            }
        }
    }
    // SAFETY: type pointers are non-null and read-only inspected.
    let slotv = unsafe { cpython_number_binop_slot(left_type, slot_offset) };
    // SAFETY: type pointers are non-null and read-only inspected.
    let mut slotw = if right_type != left_type {
        unsafe { cpython_number_binop_slot(right_type, slot_offset) }
    } else {
        None
    };
    if matches!((slotv, slotw), (Some(a), Some(b)) if (a as usize) == (b as usize)) {
        slotw = None;
    }
    if slotv.is_none() && slotw.is_none() {
        return None;
    }
    if trace {
        eprintln!(
            "[cpy-number-slot] dispatch slotv={} slotw={}",
            slotv
                .map(|slot| format!("{:p}", slot as *const c_void))
                .unwrap_or_else(|| "<none>".to_string()),
            slotw
                .map(|slot| format!("{:p}", slot as *const c_void))
                .unwrap_or_else(|| "<none>".to_string())
        );
    }
    let not_implemented = std::ptr::addr_of_mut!(_Py_NotImplementedStruct).cast::<c_void>();
    if let Some(slotv_fn) = slotv {
        if let Some(slotw_fn) = slotw
            // SAFETY: type pointers are valid for subtype test.
            && unsafe { PyType_IsSubtype(right_type.cast(), left_type.cast()) != 0 }
        {
            // SAFETY: binary slot ABI matches `binaryfunc`.
            let result = unsafe { slotw_fn(left, right) };
            if result.is_null() {
                if trace {
                    eprintln!("[cpy-number-slot] subtype-right result=<null>");
                }
                return Some(std::ptr::null_mut());
            }
            if result != not_implemented {
                if trace {
                    eprintln!("[cpy-number-slot] subtype-right result={:p}", result);
                }
                return Some(result);
            }
            // SAFETY: slot returned new reference to NotImplemented.
            unsafe { Py_DecRef(result) };
            slotw = None;
        }
        // SAFETY: binary slot ABI matches `binaryfunc`.
        let result = unsafe { slotv_fn(left, right) };
        if result.is_null() {
            if trace {
                eprintln!("[cpy-number-slot] left result=<null>");
            }
            return Some(std::ptr::null_mut());
        }
        if result != not_implemented {
            if trace {
                eprintln!("[cpy-number-slot] left result={:p}", result);
            }
            return Some(result);
        }
        // SAFETY: slot returned new reference to NotImplemented.
        unsafe { Py_DecRef(result) };
    }
    if let Some(slotw_fn) = slotw {
        // SAFETY: binary slot ABI matches `binaryfunc`.
        let result = unsafe { slotw_fn(left, right) };
        if result.is_null() {
            if trace {
                eprintln!("[cpy-number-slot] right result=<null>");
            }
            return Some(std::ptr::null_mut());
        }
        if result != not_implemented {
            if trace {
                eprintln!("[cpy-number-slot] right result={:p}", result);
            }
            return Some(result);
        }
        // SAFETY: slot returned new reference to NotImplemented.
        unsafe { Py_DecRef(result) };
    }
    None
}

pub(super) fn cpython_call_object(
    callable: *mut c_void,
    args: Vec<Value>,
    kwargs: HashMap<String, Value>,
) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let callable_ptr = callable;
        if context.vm.is_null() {
            context.set_error("missing VM context for object call");
            return std::ptr::null_mut();
        }
        if let Some(result) = context.try_native_tp_call(callable_ptr, &args, &kwargs) {
            if result.is_null() && unsafe { PyErr_Occurred() }.is_null() {
                if super::super::env_var_present_cached("PYRS_TRACE_CPY_NULL_NOERR") {
                    let callable_tag = context
                        .cpython_value_from_borrowed_ptr(callable_ptr)
                        .map(|value| cpython_value_debug_tag(&value))
                        .unwrap_or_else(|| "<unresolved>".to_string());
                    let callable_type = unsafe {
                        callable_ptr
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                            .filter(|ty| !ty.is_null())
                            .and_then(|ty| c_name_to_string((*ty).tp_name).ok())
                            .unwrap_or_else(|| "<unknown>".to_string())
                    };
                    eprintln!(
                        "[cpy-null-noerr] callable_ptr={:p} callable={} type={} args={} kwargs={}",
                        callable_ptr,
                        callable_tag,
                        callable_type,
                        args.len(),
                        kwargs.len()
                    );
                }
                context.set_error("SystemError: NULL result without error in PyObject_Call");
            }
            return result;
        }
        let Some(mut callable) = context.cpython_value_from_borrowed_ptr(callable_ptr) else {
            context.set_error("unknown callable object pointer");
            return std::ptr::null_mut();
        };
        // C-API exception globals resolve to `Value::ExceptionType`; call dispatch
        // expects concrete class objects for constructor invocation.
        // SAFETY: VM pointer is valid for active context lifetime.
        let vm = unsafe { &mut *context.vm };
        if let Value::ExceptionType(name) = callable {
            if super::super::env_var_present_cached("PYRS_TRACE_EXCEPTION_TYPE_FLAGS")
                && (name == "TypeError" || name == "ValueError" || name == "IndexError")
            {
                // SAFETY: exception symbol pointers are process-global type objects.
                unsafe {
                    if let Some(sym) = super::cpython_exception_ptr_for_name(&name) {
                        let ty = sym.cast::<CpythonTypeObject>();
                        eprintln!(
                            "[cpy-exc-flags] name={} sym={:p} ob_type={:p} tp_flags=0x{:x} tp_base={:p}",
                            name,
                            sym,
                            (*ty).ob_type,
                            (*ty).tp_flags,
                            (*ty).tp_base
                        );
                    }
                }
            }
            callable = Value::Class(vm.alloc_synthetic_exception_class(&name));
        }
        let trace_ufunc_errors = super::super::env_var_present_cached("PYRS_TRACE_CPY_UFUNC_ERRORS");
        let callable_tag = if trace_ufunc_errors {
            Some(cpython_value_debug_tag(&callable))
        } else {
            None
        };
        let callable_desc_for_error = cpython_value_debug_tag(&callable);
        let arg_count = args.len();
        let kwarg_count = kwargs.len();
        if super::super::env_var_present_cached("PYRS_TRACE_CPY_API") {
            eprintln!(
                "[cpy-api] cpython_call_object ptr={:p} callable={}",
                callable_ptr,
                cpython_value_debug_tag(&callable)
            );
        }
        match vm.call_internal(callable, args, kwargs) {
            Ok(InternalCallOutcome::Value(value)) => context.alloc_cpython_ptr_for_value(value),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                let active_exception = vm
                    .frames
                    .last()
                    .and_then(|frame| frame.active_exception.clone());
                if let Some(exception_value) = active_exception {
                    if super::super::env_var_present_cached("PYRS_TRACE_CPY_CALL_EXC") {
                        eprintln!(
                            "[cpy-call-exc] value={}",
                            cpython_value_debug_tag(&exception_value)
                        );
                    }
                    let pvalue = context.alloc_cpython_ptr_for_value(exception_value.clone());
                    let ptype = cpython_exception_type_ptr_for_value(context, &exception_value)
                        .or_else(|| {
                            let inferred = cpython_exception_type_ptr(pvalue);
                            if inferred.is_null() {
                                None
                            } else {
                                Some(inferred)
                            }
                        })
                        .unwrap_or_else(|| unsafe { PyExc_RuntimeError });
                    let ptraceback = cpython_exception_traceback_ptr_for_value(
                        context,
                        &exception_value,
                    )
                    .unwrap_or(std::ptr::null_mut());
                    let message = vm
                        .runtime_error_from_active_exception("object call failed")
                        .message;
                    if trace_ufunc_errors
                        && message.contains("_UFunc")
                    {
                        let stack = vm
                            .frames
                            .iter()
                            .rev()
                            .take(8)
                            .map(|frame| {
                                format!("{}@{}", frame.code.name, frame.code.filename)
                            })
                            .collect::<Vec<_>>()
                            .join(" <- ");
                        eprintln!(
                            "[cpy-call-ufunc] callable_ptr={:p} callable={} args={} kwargs={} stack={}",
                            callable_ptr,
                            callable_tag.as_deref().unwrap_or("<unknown>"),
                            arg_count,
                            kwarg_count,
                            stack
                        );
                    }
                    if super::super::env_var_present_cached("PYRS_TRACE_CPY_CTYPES_ERROR")
                        && message.contains("ModuleNotFoundError: module '_ctypes' not found")
                    {
                        let stack = vm
                            .frames
                            .iter()
                            .rev()
                            .take(8)
                            .map(|frame| format!("{}@{}", frame.code.name, frame.code.filename))
                            .collect::<Vec<_>>()
                            .join(" <- ");
                        eprintln!(
                            "[cpy-call-ctypes] callable_ptr={:p} callable={} args={} kwargs={} stack={}",
                            callable_ptr,
                            callable_tag.as_deref().unwrap_or("<unknown>"),
                            arg_count,
                            kwarg_count,
                            stack
                        );
                    }
                    context.set_error_state(ptype, pvalue, ptraceback, message);
                } else {
                    if context.current_error.is_none() {
                        context.set_error(
                            vm.runtime_error_from_active_exception("object call failed").message,
                        );
                    }
                }
                if context.current_error.is_none() {
                    context.set_error("RuntimeError: object call failed without active exception");
                }
                std::ptr::null_mut()
            }
            Err(err) => {
                if context.current_error.is_some() {
                    return std::ptr::null_mut();
                }
                let message = err.message;
                if super::super::env_var_present_cached("PYRS_TRACE_BIND_CALLABLE")
                    && message.contains("argument count mismatch")
                {
                    let callable_type_name = unsafe {
                        callable_ptr
                            .cast::<CpythonObjectHead>()
                            .as_ref()
                            .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                            .filter(|ty| !ty.is_null())
                            .and_then(|ty| c_name_to_string((*ty).tp_name).ok())
                            .unwrap_or_else(|| "<unknown>".to_string())
                    };
                    eprintln!(
                        "[bind-callable] callable_ptr={:p} callable={} callable_type={} args={} kwargs={} msg={}",
                        callable_ptr,
                        callable_desc_for_error,
                        callable_type_name,
                        arg_count,
                        kwarg_count,
                        message
                    );
                }
                if super::super::env_var_present_cached("PYRS_TRACE_PROXY_NOT_CALLABLE")
                    && message.contains("proxy object is not callable")
                {
                    eprintln!(
                        "[proxy-not-callable] callable_ptr={:p} callable={} args={} kwargs={}",
                        callable_ptr, callable_desc_for_error, arg_count, kwarg_count
                    );
                }
                if let Some(exception_name) = cpython_exception_name_from_runtime_message(&message)
                {
                    if trace_ufunc_errors && message.contains("_UFunc") {
                        let map_hit = context
                            .exception_type_ptr_by_name
                            .get(&exception_name)
                            .copied();
                        eprintln!(
                            "[cpy-call-ufunc-err] exception_name={} map_hit={map_hit:?}",
                            exception_name
                        );
                    }
                    let ptype = context
                        .exception_type_ptr_by_name
                        .get(&exception_name)
                        .copied()
                        .map(|ptr| ptr as *mut c_void)
                        .or_else(|| cpython_exception_ptr_for_name(&exception_name))
                        .unwrap_or_else(|| unsafe { PyExc_RuntimeError });
                    let pvalue = context.alloc_cpython_ptr_for_value(Value::Str(message.clone()));
                    context.set_error_state(ptype, pvalue, std::ptr::null_mut(), message);
                } else {
                    context.set_error(message);
                }
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

pub(super) fn cpython_structseq_count_fields(
    fields: *mut CpythonStructSequenceField,
) -> Result<usize, String> {
    if fields.is_null() {
        return Err("PyStructSequence_NewType expected non-null fields".to_string());
    }
    let mut count = 0usize;
    let mut cursor = fields;
    // Field table is null-name terminated per CPython contract.
    while count < 8192 {
        // SAFETY: `cursor` points into caller-owned contiguous field table.
        let name_ptr = unsafe { (*cursor).name };
        if name_ptr.is_null() {
            return Ok(count);
        }
        count += 1;
        // SAFETY: `cursor` advances over contiguous field entries.
        cursor = unsafe { cursor.add(1) };
    }
    Err("PyStructSequence_NewType field table is not terminated".to_string())
}

pub(super) fn cpython_unicode_text_from_value(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text.clone()),
        Value::Instance(instance) => match &*instance.kind() {
            Object::Instance(instance_data) => {
                match instance_data.attrs.get(STR_BACKING_STORAGE_ATTR) {
                    Some(Value::Str(text)) => Some(text.clone()),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn cpython_call_method_for_capi(
    context: &mut ModuleCapiContext,
    receiver: Value,
    method: &str,
    args: Vec<Value>,
    api_name: &str,
) -> Option<Value> {
    let callable = match cpython_getattr_in_context(context, receiver, method) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(format!("{api_name} {err}"));
            return None;
        }
    };
    match cpython_call_internal_in_context(context, callable, args, HashMap::new()) {
        Ok(value) => Some(value),
        Err(err) => {
            context.set_error(format!("{api_name} {err}"));
            None
        }
    }
}

pub(super) fn cpython_codec_name_or_default(
    encoding: *const c_char,
    default_name: &str,
    api_name: &str,
) -> Result<String, String> {
    if encoding.is_null() {
        return Ok(default_name.to_string());
    }
    // SAFETY: caller passes NUL-terminated non-null C string for encoding names.
    unsafe { c_name_to_string(encoding) }.map_err(|err| format!("{api_name} {err}"))
}

pub(super) fn cpython_codec_error_name_optional(
    errors: *const c_char,
    api_name: &str,
) -> Result<Option<String>, String> {
    if errors.is_null() {
        return Ok(None);
    }
    // SAFETY: caller passes NUL-terminated non-null C string for error names.
    unsafe { c_name_to_string(errors) }
        .map(Some)
        .map_err(|err| format!("{api_name} {err}"))
}

pub(super) fn cpython_unicode_decode_with_codec_in_context(
    context: &mut ModuleCapiContext,
    source: Value,
    encoding_name: String,
    errors_name: Option<String>,
    api_name: &str,
) -> Option<Value> {
    let mut args = vec![source, Value::Str(encoding_name.clone())];
    if let Some(errors_name) = errors_name {
        args.push(Value::Str(errors_name));
    }
    let decoded = match cpython_call_internal_in_context(
        context,
        Value::Builtin(BuiltinFunction::CodecsDecode),
        args,
        HashMap::new(),
    ) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(format!("{api_name} {err}"));
            return None;
        }
    };
    if cpython_unicode_text_from_value(&decoded).is_none() {
        let got = if context.vm.is_null() {
            "object".to_string()
        } else {
            // SAFETY: VM pointer is valid for active C-API context lifetime.
            unsafe { (&mut *context.vm).value_type_name_for_error(&decoded) }
        };
        context.set_error(format!(
            "'{encoding_name}' decoder returned '{got}' instead of 'str'; use codecs.decode() to decode to arbitrary types"
        ));
        return None;
    }
    Some(decoded)
}

pub(super) fn cpython_unicode_encode_with_codec_in_context(
    context: &mut ModuleCapiContext,
    source: Value,
    encoding_name: String,
    errors_name: Option<String>,
    api_name: &str,
) -> Option<Value> {
    let mut args = vec![source, Value::Str(encoding_name.clone())];
    if let Some(errors_name) = errors_name {
        args.push(Value::Str(errors_name));
    }
    let encoded = match cpython_call_internal_in_context(
        context,
        Value::Builtin(BuiltinFunction::CodecsEncode),
        args,
        HashMap::new(),
    ) {
        Ok(value) => value,
        Err(err) => {
            context.set_error(format!("{api_name} {err}"));
            return None;
        }
    };
    Some(encoded)
}
