use std::backtrace::Backtrace;
use std::ffi::c_void;

use crate::runtime::{BigInt, BuiltinFunction, Object, RuntimeError, Value};
use crate::vm::{
    and_values, floor_div_values, invert_value, lshift_values, mod_values, neg_value, or_values,
    pos_value, pow_values, rshift_values, xor_values,
};

use super::{
    CPY_PROXY_PTR_ATTR, CpythonNumberMethods, CpythonObjectHead, CpythonTypeObject,
    ModuleCapiContext, PyErr_Occurred, PyFloat_AsDouble, PyLong_AsSsize_t,
    c_name_to_string, cpython_call_builtin, cpython_new_ptr_for_value, cpython_set_error,
    cpython_try_binary_number_slot, cpython_value_debug_tag, cpython_value_from_ptr,
    is_cpython_proxy_class, value_to_int, with_active_cpython_context_mut,
};
use super::cpython_numeric_runtime::{
    cpython_binary_numeric_op, cpython_binary_numeric_op_with_heap, cpython_unary_numeric_op,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Check(object: *mut c_void) -> i32 {
    match cpython_value_from_ptr(object) {
        Ok(
            Value::Bool(_)
            | Value::Int(_)
            | Value::BigInt(_)
            | Value::Float(_)
            | Value::Complex { .. },
        ) => 1,
        Ok(_) => 0,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Absolute(object: *mut c_void) -> *mut c_void {
    cpython_unary_numeric_op(object, |value| match value {
        Value::Complex { real, imag } => Ok(Value::Float((real * real + imag * imag).sqrt())),
        Value::Int(value) => Ok(Value::Int(value.saturating_abs())),
        Value::Bool(value) => Ok(Value::Int(if value { 1 } else { 0 })),
        Value::Float(value) => Ok(Value::Float(value.abs())),
        Value::BigInt(value) => {
            if value.is_negative() {
                neg_value(Value::BigInt(value))
            } else {
                Ok(Value::BigInt(value))
            }
        }
        other => Err(RuntimeError::new(format!(
            "bad operand type for abs(): {:?}",
            other
        ))),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Add(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    if let Some(result) = cpython_try_binary_number_slot(
        left,
        right,
        std::mem::offset_of!(CpythonNumberMethods, nb_add),
    ) {
        return result;
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyNumber_Add missing VM context");
            return std::ptr::null_mut();
        }
        let Some(left_value) = context.cpython_value_from_ptr(left) else {
            context.set_error("PyNumber_Add unknown left operand pointer");
            return std::ptr::null_mut();
        };
        let Some(right_value) = context.cpython_value_from_ptr(right) else {
            context.set_error("PyNumber_Add unknown right operand pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.binary_add_runtime(left_value, right_value) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err.message);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Subtract(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    if let Some(result) = cpython_try_binary_number_slot(
        left,
        right,
        std::mem::offset_of!(CpythonNumberMethods, nb_subtract),
    ) {
        return result;
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyNumber_Subtract missing VM context");
            return std::ptr::null_mut();
        }
        let Some(left_value) = context.cpython_value_from_ptr(left) else {
            context.set_error("PyNumber_Subtract unknown left operand pointer");
            return std::ptr::null_mut();
        };
        let Some(right_value) = context.cpython_value_from_ptr(right) else {
            context.set_error("PyNumber_Subtract unknown right operand pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.binary_sub_runtime(left_value, right_value) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err.message);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Multiply(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    if let Some(result) = cpython_try_binary_number_slot(
        left,
        right,
        std::mem::offset_of!(CpythonNumberMethods, nb_multiply),
    ) {
        return result;
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyNumber_Multiply missing VM context");
            return std::ptr::null_mut();
        }
        let Some(left_value) = context.cpython_value_from_ptr(left) else {
            context.set_error("PyNumber_Multiply unknown left operand pointer");
            return std::ptr::null_mut();
        };
        let Some(right_value) = context.cpython_value_from_ptr(right) else {
            context.set_error("PyNumber_Multiply unknown right operand pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.binary_mul_runtime(left_value, right_value) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err.message);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_TrueDivide(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    if let Some(result) = cpython_try_binary_number_slot(
        left,
        right,
        std::mem::offset_of!(CpythonNumberMethods, nb_true_divide),
    ) {
        return result;
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyNumber_TrueDivide missing VM context");
            return std::ptr::null_mut();
        }
        let Some(left_value) = context.cpython_value_from_ptr(left) else {
            context.set_error("PyNumber_TrueDivide unknown left operand pointer");
            return std::ptr::null_mut();
        };
        let Some(right_value) = context.cpython_value_from_ptr(right) else {
            context.set_error("PyNumber_TrueDivide unknown right operand pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.binary_div_runtime(left_value, right_value) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err.message);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_FloorDivide(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    if let Some(result) = cpython_try_binary_number_slot(
        left,
        right,
        std::mem::offset_of!(CpythonNumberMethods, nb_floor_divide),
    ) {
        return result;
    }
    cpython_binary_numeric_op(left, right, floor_div_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Remainder(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    if let Some(result) = cpython_try_binary_number_slot(
        left,
        right,
        std::mem::offset_of!(CpythonNumberMethods, nb_remainder),
    ) {
        return result;
    }
    cpython_binary_numeric_op_with_heap(left, right, mod_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Divmod(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    let quotient = cpython_binary_numeric_op(left, right, floor_div_values);
    if quotient.is_null() {
        return std::ptr::null_mut();
    }
    let remainder = cpython_binary_numeric_op_with_heap(left, right, mod_values);
    if remainder.is_null() {
        return std::ptr::null_mut();
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyNumber_Divmod missing VM context");
            return std::ptr::null_mut();
        }
        let Some(q) = context.cpython_value_from_ptr(quotient) else {
            context.set_error("PyNumber_Divmod missing quotient value");
            return std::ptr::null_mut();
        };
        let Some(r) = context.cpython_value_from_ptr(remainder) else {
            context.set_error("PyNumber_Divmod missing remainder value");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for context lifetime.
        let vm = unsafe { &mut *context.vm };
        let tuple = vm.heap.alloc(Object::Tuple(vec![q, r]));
        context.alloc_cpython_ptr_for_value(Value::Tuple(tuple))
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Power(
    left: *mut c_void,
    right: *mut c_void,
    _modulo: *mut c_void,
) -> *mut c_void {
    if let Some(result) = cpython_try_binary_number_slot(
        left,
        right,
        std::mem::offset_of!(CpythonNumberMethods, nb_power),
    ) {
        return result;
    }
    cpython_binary_numeric_op(left, right, pow_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_MatrixMultiply(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    if let Some(result) = cpython_try_binary_number_slot(
        left,
        right,
        std::mem::offset_of!(CpythonNumberMethods, nb_matrix_multiply),
    ) {
        return result;
    }
    with_active_cpython_context_mut(|context| {
        if context.vm.is_null() {
            context.set_error("PyNumber_MatrixMultiply missing VM context");
            return std::ptr::null_mut();
        }
        let Some(left_value) = context.cpython_value_from_ptr(left) else {
            context.set_error("PyNumber_MatrixMultiply unknown left operand pointer");
            return std::ptr::null_mut();
        };
        let Some(right_value) = context.cpython_value_from_ptr(right) else {
            context.set_error("PyNumber_MatrixMultiply unknown right operand pointer");
            return std::ptr::null_mut();
        };
        // SAFETY: VM pointer is valid for active C-API context lifetime.
        let vm = unsafe { &mut *context.vm };
        match vm.binary_matmul_runtime(left_value, right_value) {
            Ok(value) => context.alloc_cpython_ptr_for_value(value),
            Err(err) => {
                context.set_error(err.message);
                std::ptr::null_mut()
            }
        }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Lshift(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    if let Some(result) = cpython_try_binary_number_slot(
        left,
        right,
        std::mem::offset_of!(CpythonNumberMethods, nb_lshift),
    ) {
        return result;
    }
    cpython_binary_numeric_op(left, right, lshift_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Rshift(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    if let Some(result) = cpython_try_binary_number_slot(
        left,
        right,
        std::mem::offset_of!(CpythonNumberMethods, nb_rshift),
    ) {
        return result;
    }
    cpython_binary_numeric_op(left, right, rshift_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_And(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    if let Some(result) = cpython_try_binary_number_slot(
        left,
        right,
        std::mem::offset_of!(CpythonNumberMethods, nb_and),
    ) {
        return result;
    }
    cpython_binary_numeric_op_with_heap(left, right, and_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Or(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    if let Some(result) = cpython_try_binary_number_slot(
        left,
        right,
        std::mem::offset_of!(CpythonNumberMethods, nb_or),
    ) {
        return result;
    }
    cpython_binary_numeric_op_with_heap(left, right, or_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Xor(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    if let Some(result) = cpython_try_binary_number_slot(
        left,
        right,
        std::mem::offset_of!(CpythonNumberMethods, nb_xor),
    ) {
        return result;
    }
    cpython_binary_numeric_op_with_heap(left, right, xor_values)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceAdd(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    unsafe { PyNumber_Add(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceSubtract(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    unsafe { PyNumber_Subtract(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceMultiply(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    unsafe { PyNumber_Multiply(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceMatrixMultiply(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    unsafe { PyNumber_MatrixMultiply(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceFloorDivide(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    unsafe { PyNumber_FloorDivide(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceTrueDivide(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    unsafe { PyNumber_TrueDivide(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceRemainder(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    unsafe { PyNumber_Remainder(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlacePower(
    left: *mut c_void,
    right: *mut c_void,
    modulo: *mut c_void,
) -> *mut c_void {
    unsafe { PyNumber_Power(left, right, modulo) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceLshift(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    unsafe { PyNumber_Lshift(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceRshift(
    left: *mut c_void,
    right: *mut c_void,
) -> *mut c_void {
    unsafe { PyNumber_Rshift(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceAnd(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    unsafe { PyNumber_And(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceOr(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    unsafe { PyNumber_Or(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceXor(left: *mut c_void, right: *mut c_void) -> *mut c_void {
    unsafe { PyNumber_Xor(left, right) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Negative(object: *mut c_void) -> *mut c_void {
    cpython_unary_numeric_op(object, neg_value)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Positive(object: *mut c_void) -> *mut c_void {
    cpython_unary_numeric_op(object, pos_value)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Invert(object: *mut c_void) -> *mut c_void {
    cpython_unary_numeric_op(object, invert_value)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Long(object: *mut c_void) -> *mut c_void {
    with_active_cpython_context_mut(|context| {
        let mapped_value = context.cpython_value_from_ptr(object);
        let mut slot_object = object;
        let mut proxy_slot_fallback = false;
        if let Some(value) = mapped_value.clone() {
            if std::env::var_os("PYRS_TRACE_CPY_LONG").is_some()
                && let Value::Instance(instance_obj) = &value
            {
                let (class_name, has_proxy_marker, has_proxy_attr) =
                    if let Object::Instance(instance_data) = &*instance_obj.kind() {
                        if let Object::Class(class_data) = &*instance_data.class.kind() {
                            (
                                class_data.name.clone(),
                                is_cpython_proxy_class(class_data),
                                instance_data.attrs.contains_key(CPY_PROXY_PTR_ATTR),
                            )
                        } else {
                            (
                                "<non-class>".to_string(),
                                false,
                                instance_data.attrs.contains_key(CPY_PROXY_PTR_ATTR),
                            )
                        }
                    } else {
                        ("<invalid-instance>".to_string(), false, false)
                    };
                eprintln!(
                    "[cpy-long-map] object={:p} class={} proxy_marker={} proxy_attr={}",
                    object, class_name, has_proxy_marker, has_proxy_attr
                );
                if class_name == "_NoValueType"
                    && std::env::var_os("PYRS_TRACE_CPY_LONG_BACKTRACE").is_some()
                {
                    eprintln!(
                        "[cpy-long-map-bt] object={:p}\n{}",
                        object,
                        Backtrace::capture()
                    );
                }
            }
            if let Ok(int_value) = value_to_int(value.clone()) {
                return context.alloc_cpython_ptr_for_value(Value::Int(int_value));
            }
            if let Ok(converted) =
                cpython_call_builtin(BuiltinFunction::Int, vec![value.clone()])
            {
                return match converted {
                    Value::Int(int_value) => context.alloc_cpython_ptr_for_value(Value::Int(int_value)),
                    Value::BigInt(bigint) => {
                        context.alloc_cpython_ptr_for_value(Value::BigInt(bigint))
                    }
                    _ => {
                        context.set_error("PyNumber_Long requires int-compatible object");
                        std::ptr::null_mut()
                    }
                };
            }
            if let Some(proxy_raw_ptr) = ModuleCapiContext::cpython_proxy_raw_ptr_from_value(&value)
            {
                proxy_slot_fallback = true;
                if !proxy_raw_ptr.is_null() {
                    slot_object = proxy_raw_ptr;
                }
            }
            if let Err(err) = value_to_int(value)
                && !proxy_slot_fallback
                && !err.message.contains("unsupported operand type")
            {
                context.set_error(err.message);
                return std::ptr::null_mut();
            }
        }
        if slot_object.is_null() {
            context.set_error("PyNumber_Long expected object");
            return std::ptr::null_mut();
        }
        // SAFETY: `slot_object` is a foreign PyObject* from extension code.
        let type_ptr = unsafe {
            slot_object
                .cast::<CpythonObjectHead>()
                .as_ref()
                .map(|head| head.ob_type.cast::<CpythonTypeObject>())
                .unwrap_or(std::ptr::null_mut())
        };
        if type_ptr.is_null() {
            if std::env::var_os("PYRS_TRACE_UNKNOWN_PTR").is_some() {
                eprintln!("[cpy-unknown-ptr] PyNumber_Long object={:p}", slot_object);
            }
            context.set_error("unknown PyObject pointer");
            return std::ptr::null_mut();
        }
        // SAFETY: `type_ptr` is non-null and points to a type object.
        let number_methods = unsafe {
            (*type_ptr)
                .tp_as_number
                .cast::<CpythonNumberMethods>()
                .as_ref()
        };
        let Some(number_methods) = number_methods else {
            if std::env::var_os("PYRS_TRACE_CPY_LONG").is_some() {
                // SAFETY: `type_ptr` is non-null and points to a CPython-compatible type object.
                let type_name = unsafe {
                    c_name_to_string((*type_ptr).tp_name)
                        .unwrap_or_else(|_| "<invalid>".to_string())
                };
                let mapped_tag = mapped_value
                    .as_ref()
                    .map(cpython_value_debug_tag)
                    .unwrap_or_else(|| "<none>".to_string());
                eprintln!(
                    "[cpy-long-debug] tp_as_number missing object={:p} slot_object={:p} mapped={} type_ptr={:p} type_name={}",
                    object, slot_object, mapped_tag, type_ptr, type_name
                );
            }
            context.set_error("PyNumber_Long requires int-compatible object");
            return std::ptr::null_mut();
        };
        let converter = number_methods.nb_int.or(number_methods.nb_index);
        let Some(converter) = converter else {
            if std::env::var_os("PYRS_TRACE_CPY_ERRORS").is_some() {
                // SAFETY: `type_ptr` is non-null and points to a CPython-compatible type object.
                let type_name = unsafe {
                    c_name_to_string((*type_ptr).tp_name)
                        .unwrap_or_else(|_| "<invalid>".to_string())
                };
                let mapped_tag = mapped_value
                    .as_ref()
                    .map(cpython_value_debug_tag)
                    .unwrap_or_else(|| "<none>".to_string());
                eprintln!(
                    "[cpy-long-debug] nb_int/index missing object={:p} slot_object={:p} mapped={} type_ptr={:p} type_name={}",
                    object, slot_object, mapped_tag, type_ptr, type_name
                );
            }
            context.set_error("PyNumber_Long requires int-compatible object");
            return std::ptr::null_mut();
        };
        // SAFETY: `converter` is a valid nb_int/nb_index slot for this object type.
        unsafe { converter(slot_object) }
    })
    .unwrap_or_else(|err| {
        cpython_set_error(err);
        std::ptr::null_mut()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Float(object: *mut c_void) -> *mut c_void {
    let value = unsafe { PyFloat_AsDouble(object) };
    if value == -1.0 && !unsafe { PyErr_Occurred() }.is_null() {
        return std::ptr::null_mut();
    }
    cpython_new_ptr_for_value(Value::Float(value))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Index(object: *mut c_void) -> *mut c_void {
    unsafe { PyNumber_Long(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_AsSsize_t(object: *mut c_void, _exc: *mut c_void) -> isize {
    unsafe { PyLong_AsSsize_t(object) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_ToBase(object: *mut c_void, base: i32) -> *mut c_void {
    let result = with_active_cpython_context_mut(|context| {
        let value = context
            .cpython_value_from_ptr_or_proxy(object)
            .ok_or_else(|| "PyNumber_ToBase requires an integer object".to_string())?;
        let value = match value {
            Value::Int(int_value) => BigInt::from_i64(int_value),
            Value::Bool(flag) => BigInt::from_i64(if flag { 1 } else { 0 }),
            Value::BigInt(bigint) => *bigint,
            _ => return Err("PyNumber_ToBase requires an integer object".to_string()),
        };
        let (radix, prefix) = match base {
            2 => (2, "0b"),
            8 => (8, "0o"),
            10 => (10, ""),
            16 => (16, "0x"),
            _ => {
                return Err("PyNumber_ToBase base must be 2, 8, 10 or 16".to_string());
            }
        };
        let is_negative = value.is_negative();
        let magnitude = if is_negative { value.abs() } else { value };
        let digits = magnitude
            .to_str_radix(radix)
            .ok_or_else(|| "PyNumber_ToBase failed integer formatting".to_string())?;
        let text = if radix == 10 {
            if is_negative {
                format!("-{digits}")
            } else {
                digits
            }
        } else if is_negative {
            format!("-{prefix}{digits}")
        } else {
            format!("{prefix}{digits}")
        };
        Ok(context.alloc_cpython_ptr_for_value(Value::Str(text)))
    })
    .unwrap_or_else(|err| Err(err.to_string()));
    match result {
        Ok(ptr) => ptr,
        Err(err) => {
            cpython_set_error(err);
            std::ptr::null_mut()
        }
    }
}
