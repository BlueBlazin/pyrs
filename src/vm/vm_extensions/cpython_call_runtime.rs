use std::collections::HashMap;

use crate::runtime::Value;

use super::{InternalCallOutcome, ModuleCapiContext};

pub(in crate::vm::vm_extensions) fn cpython_call_internal_in_context(
    context: &mut ModuleCapiContext,
    callable: Value,
    args: Vec<Value>,
    kwargs: HashMap<String, Value>,
) -> Result<Value, String> {
    if context.vm.is_null() {
        return Err("missing VM context for call".to_string());
    }
    // SAFETY: VM pointer is valid for active context lifetime.
    let vm = unsafe { &mut *context.vm };
    match vm.call_internal(callable, args, kwargs) {
        Ok(InternalCallOutcome::Value(value)) => Ok(value),
        Ok(InternalCallOutcome::CallerExceptionHandled) => Err(vm
            .runtime_error_from_active_exception("call failed")
            .message),
        Err(err) => Err(err.message),
    }
}

pub(in crate::vm::vm_extensions) fn cpython_getattr_in_context(
    context: &mut ModuleCapiContext,
    target: Value,
    attr_name: &str,
) -> Result<Value, String> {
    if context.vm.is_null() {
        return Err("missing VM context for getattr".to_string());
    }
    // SAFETY: VM pointer is valid for active context lifetime.
    let vm = unsafe { &mut *context.vm };
    vm.builtin_getattr(
        vec![target, Value::Str(attr_name.to_string())],
        HashMap::new(),
    )
    .map_err(|err| err.message)
}
