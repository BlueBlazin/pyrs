/* @ts-self-types="./pyrs.d.ts" */

export class WasmCapabilityReport {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmCapabilityReport.prototype);
        obj.__wbg_ptr = ptr;
        WasmCapabilityReportFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmCapabilityReportFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmcapabilityreport_free(ptr, 0);
    }
    /**
     * @returns {boolean}
     */
    get clock_time() {
        const ret = wasm.wasmcapabilityreport_clock_time(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {boolean}
     */
    get dynamic_library_load() {
        const ret = wasm.wasmcapabilityreport_dynamic_library_load(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {boolean}
     */
    get environment_read() {
        const ret = wasm.wasmcapabilityreport_environment_read(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {boolean}
     */
    get filesystem_read() {
        const ret = wasm.wasmcapabilityreport_filesystem_read(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {boolean}
     */
    get filesystem_write() {
        const ret = wasm.wasmcapabilityreport_filesystem_write(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {boolean}
     */
    get interactive_terminal() {
        const ret = wasm.wasmcapabilityreport_interactive_terminal(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {boolean}
     */
    get network_sockets() {
        const ret = wasm.wasmcapabilityreport_network_sockets(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {boolean}
     */
    get process_args() {
        const ret = wasm.wasmcapabilityreport_process_args(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {boolean}
     */
    get process_spawn() {
        const ret = wasm.wasmcapabilityreport_process_spawn(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {boolean}
     */
    get thread_sleep() {
        const ret = wasm.wasmcapabilityreport_thread_sleep(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) WasmCapabilityReport.prototype[Symbol.dispose] = WasmCapabilityReport.prototype.free;

export class WasmCompileResult {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmCompileResult.prototype);
        obj.__wbg_ptr = ptr;
        WasmCompileResultFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmCompileResultFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmcompileresult_free(ptr, 0);
    }
    /**
     * @returns {number}
     */
    get column() {
        const ret = wasm.wasmcompileresult_column(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {string | undefined}
     */
    get error() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmcompileresult_error(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {number}
     */
    get line() {
        const ret = wasm.wasmcompileresult_line(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {boolean}
     */
    get ok() {
        const ret = wasm.wasmcompileresult_ok(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {string}
     */
    get phase() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmcompileresult_phase(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
}
if (Symbol.dispose) WasmCompileResult.prototype[Symbol.dispose] = WasmCompileResult.prototype.free;

export class WasmExecutionBlocker {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmExecutionBlocker.prototype);
        obj.__wbg_ptr = ptr;
        WasmExecutionBlockerFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmExecutionBlockerFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmexecutionblocker_free(ptr, 0);
    }
    /**
     * @returns {string}
     */
    get key() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmexecutionblocker_key(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get message() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmexecutionblocker_message(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
}
if (Symbol.dispose) WasmExecutionBlocker.prototype[Symbol.dispose] = WasmExecutionBlocker.prototype.free;

export class WasmExecutionResult {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmExecutionResult.prototype);
        obj.__wbg_ptr = ptr;
        WasmExecutionResultFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmExecutionResultFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmexecutionresult_free(ptr, 0);
    }
    /**
     * @returns {string | undefined}
     */
    get blocker_key() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmexecutionresult_blocker_key(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {number}
     */
    get column() {
        const ret = wasm.wasmexecutionresult_column(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {string | undefined}
     */
    get error() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmexecutionresult_error(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {number}
     */
    get line() {
        const ret = wasm.wasmexecutionresult_line(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {string}
     */
    get phase() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmexecutionresult_phase(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get stderr() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmexecutionresult_stderr(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get stdout() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmexecutionresult_stdout(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {boolean}
     */
    get success() {
        const ret = wasm.wasmexecutionresult_success(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) WasmExecutionResult.prototype[Symbol.dispose] = WasmExecutionResult.prototype.free;

export class WasmModulePolicyEntry {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmModulePolicyEntry.prototype);
        obj.__wbg_ptr = ptr;
        WasmModulePolicyEntryFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmModulePolicyEntryFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmmodulepolicyentry_free(ptr, 0);
    }
    /**
     * @returns {string}
     */
    get blocker_key() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmmodulepolicyentry_blocker_key(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get module() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmmodulepolicyentry_module(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
}
if (Symbol.dispose) WasmModulePolicyEntry.prototype[Symbol.dispose] = WasmModulePolicyEntry.prototype.free;

export class WasmModuleSupport {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmModuleSupport.prototype);
        obj.__wbg_ptr = ptr;
        WasmModuleSupportFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmModuleSupportFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmmodulesupport_free(ptr, 0);
    }
    /**
     * @returns {string | undefined}
     */
    get blocker_key() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmmodulesupport_blocker_key(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string | undefined}
     */
    get message() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmmodulesupport_message(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string}
     */
    get module() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmmodulesupport_module(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {boolean}
     */
    get supported() {
        const ret = wasm.wasmmodulesupport_supported(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) WasmModuleSupport.prototype[Symbol.dispose] = WasmModuleSupport.prototype.free;

export class WasmReplSession {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmReplSessionFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmreplsession_free(ptr, 0);
    }
    /**
     * @returns {boolean}
     */
    get continuation_prompt() {
        const ret = wasm.wasmreplsession_continuation_prompt(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @param {string} source
     * @returns {WasmExecutionResult}
     */
    execute_input(source) {
        const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmreplsession_execute_input(this.__wbg_ptr, ptr0, len0);
        return WasmExecutionResult.__wrap(ret);
    }
    /**
     * @returns {number}
     */
    get inputs_executed() {
        const ret = wasm.wasmreplsession_inputs_executed(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {string | undefined}
     */
    get last_error() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmreplsession_last_error(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    constructor() {
        const ret = wasm.wasmreplsession_new();
        this.__wbg_ptr = ret >>> 0;
        WasmReplSessionFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    reset() {
        wasm.wasmreplsession_reset(this.__wbg_ptr);
    }
}
if (Symbol.dispose) WasmReplSession.prototype[Symbol.dispose] = WasmReplSession.prototype.free;

export class WasmRuntimeInfo {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmRuntimeInfo.prototype);
        obj.__wbg_ptr = ptr;
        WasmRuntimeInfoFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmRuntimeInfoFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmruntimeinfo_free(ptr, 0);
    }
    /**
     * @returns {number}
     */
    get api_version() {
        const ret = wasm.wasmruntimeinfo_api_version(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {string}
     */
    get cpython_compat_version() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmruntimeinfo_cpython_compat_version(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get execution_backend() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmruntimeinfo_execution_backend(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {number}
     */
    get execution_blocker_count() {
        const ret = wasm.wasmruntimeinfo_execution_blocker_count(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {string}
     */
    get execution_status() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmruntimeinfo_execution_status(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get pyrs_version() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmruntimeinfo_pyrs_version(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {boolean}
     */
    get supports_execution() {
        const ret = wasm.wasmruntimeinfo_supports_execution(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {boolean}
     */
    get supports_parse_compile() {
        const ret = wasm.wasmruntimeinfo_supports_parse_compile(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) WasmRuntimeInfo.prototype[Symbol.dispose] = WasmRuntimeInfo.prototype.free;

export class WasmSession {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmSessionFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmsession_free(ptr, 0);
    }
    /**
     * @param {string} source
     * @returns {WasmCompileResult}
     */
    check_compile(source) {
        const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsession_check_compile(this.__wbg_ptr, ptr0, len0);
        return WasmCompileResult.__wrap(ret);
    }
    /**
     * @param {string} source
     * @returns {WasmSyntaxResult}
     */
    check_syntax(source) {
        const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsession_check_syntax(this.__wbg_ptr, ptr0, len0);
        return WasmSyntaxResult.__wrap(ret);
    }
    /**
     * @param {string} source
     * @returns {WasmExecutionResult}
     */
    execute(source) {
        const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmsession_execute(this.__wbg_ptr, ptr0, len0);
        return WasmExecutionResult.__wrap(ret);
    }
    /**
     * @returns {string | undefined}
     */
    get last_error() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmsession_last_error(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    constructor() {
        const ret = wasm.wasmsession_new();
        this.__wbg_ptr = ret >>> 0;
        WasmSessionFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    reset() {
        wasm.wasmsession_reset(this.__wbg_ptr);
    }
    /**
     * @returns {number}
     */
    get snippets_checked() {
        const ret = wasm.wasmsession_snippets_checked(this.__wbg_ptr);
        return ret >>> 0;
    }
}
if (Symbol.dispose) WasmSession.prototype[Symbol.dispose] = WasmSession.prototype.free;

export class WasmSnippetBlocker {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmSnippetBlocker.prototype);
        obj.__wbg_ptr = ptr;
        WasmSnippetBlockerFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmSnippetBlockerFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmsnippetblocker_free(ptr, 0);
    }
    /**
     * @returns {string}
     */
    get blocker_key() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmsnippetblocker_blocker_key(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get message() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmsnippetblocker_message(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get module() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmsnippetblocker_module(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
}
if (Symbol.dispose) WasmSnippetBlocker.prototype[Symbol.dispose] = WasmSnippetBlocker.prototype.free;

export class WasmSnippetSupport {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmSnippetSupport.prototype);
        obj.__wbg_ptr = ptr;
        WasmSnippetSupportFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmSnippetSupportFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmsnippetsupport_free(ptr, 0);
    }
    /**
     * @returns {number}
     */
    get blocker_count() {
        const ret = wasm.wasmsnippetsupport_blocker_count(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    get column() {
        const ret = wasm.wasmsnippetsupport_column(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {string | undefined}
     */
    get error() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmsnippetsupport_error(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string | undefined}
     */
    get first_blocker_key() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmsnippetsupport_first_blocker_key(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string | undefined}
     */
    get first_blocker_message() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmsnippetsupport_first_blocker_message(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string | undefined}
     */
    get first_blocker_module() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmsnippetsupport_first_blocker_module(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {number}
     */
    get imported_module_count() {
        const ret = wasm.wasmsnippetsupport_imported_module_count(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    get line() {
        const ret = wasm.wasmsnippetsupport_line(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {string}
     */
    get phase() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmsnippetsupport_phase(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {boolean}
     */
    get supported() {
        const ret = wasm.wasmsnippetsupport_supported(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) WasmSnippetSupport.prototype[Symbol.dispose] = WasmSnippetSupport.prototype.free;

export class WasmSyntaxResult {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmSyntaxResult.prototype);
        obj.__wbg_ptr = ptr;
        WasmSyntaxResultFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmSyntaxResultFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmsyntaxresult_free(ptr, 0);
    }
    /**
     * @returns {number}
     */
    get column() {
        const ret = wasm.wasmsyntaxresult_column(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {string | undefined}
     */
    get error() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmsyntaxresult_error(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {number}
     */
    get line() {
        const ret = wasm.wasmsyntaxresult_line(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {boolean}
     */
    get ok() {
        const ret = wasm.wasmsyntaxresult_ok(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) WasmSyntaxResult.prototype[Symbol.dispose] = WasmSyntaxResult.prototype.free;

export class WasmWorkerBlocker {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmWorkerBlocker.prototype);
        obj.__wbg_ptr = ptr;
        WasmWorkerBlockerFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmWorkerBlockerFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmworkerblocker_free(ptr, 0);
    }
    /**
     * @returns {string}
     */
    get key() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerblocker_key(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get message() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerblocker_message(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
}
if (Symbol.dispose) WasmWorkerBlocker.prototype[Symbol.dispose] = WasmWorkerBlocker.prototype.free;

export class WasmWorkerExecutionResult {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmWorkerExecutionResult.prototype);
        obj.__wbg_ptr = ptr;
        WasmWorkerExecutionResultFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmWorkerExecutionResultFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmworkerexecutionresult_free(ptr, 0);
    }
    /**
     * @returns {string | undefined}
     */
    get blocker_key() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerexecutionresult_blocker_key(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {number}
     */
    get column() {
        const ret = wasm.wasmworkerexecutionresult_column(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {string | undefined}
     */
    get error() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerexecutionresult_error(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {number}
     */
    get line() {
        const ret = wasm.wasmworkerexecutionresult_line(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {string}
     */
    get operation_id() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerexecutionresult_operation_id(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get phase() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerexecutionresult_phase(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get state() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerexecutionresult_state(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get stderr() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerexecutionresult_stderr(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get stdout() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerexecutionresult_stdout(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {boolean}
     */
    get success() {
        const ret = wasm.wasmworkerexecutionresult_success(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) WasmWorkerExecutionResult.prototype[Symbol.dispose] = WasmWorkerExecutionResult.prototype.free;

export class WasmWorkerInfo {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmWorkerInfo.prototype);
        obj.__wbg_ptr = ptr;
        WasmWorkerInfoFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmWorkerInfoFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmworkerinfo_free(ptr, 0);
    }
    /**
     * @returns {string}
     */
    get backend() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerinfo_backend(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {number}
     */
    get blocker_count() {
        const ret = wasm.wasmworkerinfo_blocker_count(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {boolean}
     */
    get execute_supported() {
        const ret = wasm.wasmworkerinfo_execute_supported(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {boolean}
     */
    get execution_probe_enabled() {
        const ret = wasm.wasmworkerinfo_execution_probe_enabled(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {string}
     */
    get interruption_model() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerinfo_interruption_model(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {boolean}
     */
    get lifecycle_supported() {
        const ret = wasm.wasmworkerinfo_lifecycle_supported(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {string}
     */
    get state() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerinfo_state(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {boolean}
     */
    get supported() {
        const ret = wasm.wasmworkerinfo_supported(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {boolean}
     */
    get timeout_configuration_supported() {
        const ret = wasm.wasmworkerinfo_timeout_configuration_supported(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {boolean}
     */
    get timeout_enforcement_supported() {
        const ret = wasm.wasmworkerinfo_timeout_enforcement_supported(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) WasmWorkerInfo.prototype[Symbol.dispose] = WasmWorkerInfo.prototype.free;

export class WasmWorkerLifecycleResult {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmWorkerLifecycleResult.prototype);
        obj.__wbg_ptr = ptr;
        WasmWorkerLifecycleResultFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmWorkerLifecycleResultFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmworkerlifecycleresult_free(ptr, 0);
    }
    /**
     * @returns {string | undefined}
     */
    get blocker_key() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerlifecycleresult_blocker_key(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string | undefined}
     */
    get error() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerlifecycleresult_error(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string}
     */
    get operation_id() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerlifecycleresult_operation_id(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get phase() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerlifecycleresult_phase(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get state() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkerlifecycleresult_state(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {boolean}
     */
    get success() {
        const ret = wasm.wasmworkerlifecycleresult_success(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) WasmWorkerLifecycleResult.prototype[Symbol.dispose] = WasmWorkerLifecycleResult.prototype.free;

export class WasmWorkerSession {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmWorkerSessionFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmworkersession_free(ptr, 0);
    }
    /**
     * @param {string} source
     * @returns {WasmExecutionResult}
     */
    execute(source) {
        const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmworkersession_execute(this.__wbg_ptr, ptr0, len0);
        return WasmExecutionResult.__wrap(ret);
    }
    /**
     * @param {string} source
     * @returns {WasmWorkerExecutionResult}
     */
    execute_with_operation(source) {
        const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.wasmworkersession_execute_with_operation(this.__wbg_ptr, ptr0, len0);
        return WasmWorkerExecutionResult.__wrap(ret);
    }
    /**
     * @returns {number}
     */
    get executes_requested() {
        const ret = wasm.wasmworkersession_executes_requested(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {WasmWorkerInfo}
     */
    info() {
        const ret = wasm.wasmworkersession_info(this.__wbg_ptr);
        return WasmWorkerInfo.__wrap(ret);
    }
    /**
     * @returns {string | undefined}
     */
    get last_error() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkersession_last_error(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string | undefined}
     */
    get last_operation_id() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkersession_last_operation_id(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string | undefined}
     */
    get last_phase() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkersession_last_phase(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string | undefined}
     */
    get last_state() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkersession_last_state(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {number | undefined}
     */
    get last_timeout_ms_requested() {
        const ret = wasm.wasmworkersession_last_timeout_ms_requested(this.__wbg_ptr);
        return ret === 0x100000001 ? undefined : ret;
    }
    constructor() {
        const ret = wasm.wasmworkersession_new();
        this.__wbg_ptr = ret >>> 0;
        WasmWorkerSessionFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * @returns {WasmWorkerLifecycleResult}
     */
    recycle() {
        const ret = wasm.wasmworkersession_recycle(this.__wbg_ptr);
        return WasmWorkerLifecycleResult.__wrap(ret);
    }
    /**
     * @returns {number}
     */
    get recycles_requested() {
        const ret = wasm.wasmworkersession_recycles_requested(this.__wbg_ptr);
        return ret >>> 0;
    }
    reset() {
        wasm.wasmworkersession_reset(this.__wbg_ptr);
    }
    /**
     * @param {number} timeout_ms
     * @returns {WasmWorkerTimeoutResult}
     */
    set_timeout_ms(timeout_ms) {
        const ret = wasm.wasmworkersession_set_timeout_ms(this.__wbg_ptr, timeout_ms);
        return WasmWorkerTimeoutResult.__wrap(ret);
    }
    /**
     * @returns {WasmWorkerSessionSnapshot}
     */
    snapshot() {
        const ret = wasm.wasmworkersession_snapshot(this.__wbg_ptr);
        return WasmWorkerSessionSnapshot.__wrap(ret);
    }
    /**
     * @returns {WasmWorkerLifecycleResult}
     */
    start() {
        const ret = wasm.wasmworkersession_start(this.__wbg_ptr);
        return WasmWorkerLifecycleResult.__wrap(ret);
    }
    /**
     * @returns {number}
     */
    get starts_requested() {
        const ret = wasm.wasmworkersession_starts_requested(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {WasmWorkerLifecycleResult}
     */
    terminate() {
        const ret = wasm.wasmworkersession_terminate(this.__wbg_ptr);
        return WasmWorkerLifecycleResult.__wrap(ret);
    }
    /**
     * @returns {number}
     */
    get terminates_requested() {
        const ret = wasm.wasmworkersession_terminates_requested(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    get timeout_updates_requested() {
        const ret = wasm.wasmworkersession_timeout_updates_requested(this.__wbg_ptr);
        return ret >>> 0;
    }
}
if (Symbol.dispose) WasmWorkerSession.prototype[Symbol.dispose] = WasmWorkerSession.prototype.free;

export class WasmWorkerSessionSnapshot {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmWorkerSessionSnapshot.prototype);
        obj.__wbg_ptr = ptr;
        WasmWorkerSessionSnapshotFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmWorkerSessionSnapshotFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmworkersessionsnapshot_free(ptr, 0);
    }
    /**
     * @returns {number}
     */
    get executes_requested() {
        const ret = wasm.wasmworkersessionsnapshot_executes_requested(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {string | undefined}
     */
    get last_error() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkersessionsnapshot_last_error(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string | undefined}
     */
    get last_operation_id() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkersessionsnapshot_last_operation_id(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string | undefined}
     */
    get last_phase() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkersessionsnapshot_last_phase(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string | undefined}
     */
    get last_state() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkersessionsnapshot_last_state(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {number | undefined}
     */
    get last_timeout_ms_requested() {
        const ret = wasm.wasmworkersessionsnapshot_last_timeout_ms_requested(this.__wbg_ptr);
        return ret === 0x100000001 ? undefined : ret;
    }
    /**
     * @returns {number}
     */
    get recycles_requested() {
        const ret = wasm.wasmworkersessionsnapshot_recycles_requested(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    get starts_requested() {
        const ret = wasm.wasmworkersessionsnapshot_starts_requested(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    get terminates_requested() {
        const ret = wasm.wasmworkersessionsnapshot_terminates_requested(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    get timeout_updates_requested() {
        const ret = wasm.wasmworkersessionsnapshot_timeout_updates_requested(this.__wbg_ptr);
        return ret >>> 0;
    }
}
if (Symbol.dispose) WasmWorkerSessionSnapshot.prototype[Symbol.dispose] = WasmWorkerSessionSnapshot.prototype.free;

export class WasmWorkerTimeoutPolicy {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmWorkerTimeoutPolicy.prototype);
        obj.__wbg_ptr = ptr;
        WasmWorkerTimeoutPolicyFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmWorkerTimeoutPolicyFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmworkertimeoutpolicy_free(ptr, 0);
    }
    /**
     * @returns {boolean}
     */
    get configuration_supported() {
        const ret = wasm.wasmworkertimeoutpolicy_configuration_supported(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {number}
     */
    get default_timeout_ms() {
        const ret = wasm.wasmworkertimeoutpolicy_default_timeout_ms(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {boolean}
     */
    get enforcement_supported() {
        const ret = wasm.wasmworkertimeoutpolicy_enforcement_supported(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {number}
     */
    get max_timeout_ms() {
        const ret = wasm.wasmworkertimeoutpolicy_max_timeout_ms(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    get min_timeout_ms() {
        const ret = wasm.wasmworkertimeoutpolicy_min_timeout_ms(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {boolean}
     */
    get recycle_on_timeout() {
        const ret = wasm.wasmworkertimeoutpolicy_recycle_on_timeout(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {string}
     */
    get unsupported_phase() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkertimeoutpolicy_unsupported_phase(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string | undefined}
     */
    get unsupported_reason() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkertimeoutpolicy_unsupported_reason(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
}
if (Symbol.dispose) WasmWorkerTimeoutPolicy.prototype[Symbol.dispose] = WasmWorkerTimeoutPolicy.prototype.free;

export class WasmWorkerTimeoutResult {
    static __wrap(ptr) {
        ptr = ptr >>> 0;
        const obj = Object.create(WasmWorkerTimeoutResult.prototype);
        obj.__wbg_ptr = ptr;
        WasmWorkerTimeoutResultFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmWorkerTimeoutResultFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmworkertimeoutresult_free(ptr, 0);
    }
    /**
     * @returns {string | undefined}
     */
    get blocker_key() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkertimeoutresult_blocker_key(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string | undefined}
     */
    get error() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkertimeoutresult_error(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {string}
     */
    get operation_id() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkertimeoutresult_operation_id(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get phase() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkertimeoutresult_phase(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string}
     */
    get state() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmworkertimeoutresult_state(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {boolean}
     */
    get success() {
        const ret = wasm.wasmworkertimeoutresult_success(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {number}
     */
    get timeout_ms() {
        const ret = wasm.wasmworkertimeoutresult_timeout_ms(this.__wbg_ptr);
        return ret >>> 0;
    }
}
if (Symbol.dispose) WasmWorkerTimeoutResult.prototype[Symbol.dispose] = WasmWorkerTimeoutResult.prototype.free;

/**
 * Parse+compile gate with JS error for web clients.
 * @param {string} source
 */
export function check_compile(source) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
        const len0 = WASM_VECTOR_LEN;
        wasm.check_compile(retptr, ptr0, len0);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        if (r1) {
            throw takeObject(r0);
        }
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Parse+compile validation with structured diagnostics for web clients.
 * @param {string} source
 * @returns {WasmCompileResult}
 */
export function check_compile_result(source) {
    const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.check_compile_result(ptr0, len0);
    return WasmCompileResult.__wrap(ret);
}

/**
 * Parses module source and reports syntax diagnostics with parser-native text.
 * @param {string} source
 */
export function check_syntax(source) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
        const len0 = WASM_VECTOR_LEN;
        wasm.check_syntax(retptr, ptr0, len0);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        if (r1) {
            throw takeObject(r0);
        }
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Parser-backed syntax check with structured diagnostics for web clients.
 * @param {string} source
 * @returns {WasmSyntaxResult}
 */
export function check_syntax_result(source) {
    const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.check_syntax_result(ptr0, len0);
    return WasmSyntaxResult.__wrap(ret);
}

/**
 * Executes a snippet using the current wasm bridge contract.
 *
 * Current milestone behavior:
 * - parse-invalid input returns `phase = "syntax_error"`
 * - parse-valid but compile-invalid input returns `phase = "compile_error"`
 * - parse+compile-valid snippets that import known blocked modules return
 *   `phase = "unsupported_execution"` with capability-specific blocker keys,
 * - default wasm builds return `phase = "unsupported_execution"` for remaining
 *   parse+compile-valid snippets,
 * - `wasm-vm-probe` builds execute remaining snippets through VM and return
 *   `phase = "ok"` or `phase = "runtime_error"`.
 * @param {string} source
 * @returns {WasmExecutionResult}
 */
export function execute(source) {
    const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.execute(ptr0, len0);
    return WasmExecutionResult.__wrap(ret);
}

/**
 * Installs panic hook once so Rust panic payloads surface in browser console.
 */
export function init_wasm_runtime() {
    wasm.init_wasm_runtime();
}

/**
 * Minimal WASM bridge surface used during compile-isolation bring-up.
 * @returns {string}
 */
export function pyrs_version() {
    let deferred1_0;
    let deferred1_1;
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.pyrs_version(retptr);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        deferred1_0 = r0;
        deferred1_1 = r1;
        return getStringFromWasm0(r0, r1);
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
        wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Version of the wasm JS-facing API contract.
 * @returns {number}
 */
export function wasm_api_version() {
    const ret = wasm.wasm_api_version();
    return ret >>> 0;
}

/**
 * Exposes explicit capability support for browser mode.
 * @returns {WasmCapabilityReport}
 */
export function wasm_capabilities() {
    const ret = wasm.wasm_capabilities();
    return WasmCapabilityReport.__wrap(ret);
}

/**
 * Returns a stable unsupported-capability message for browser mode.
 * @param {string} capability_key
 * @returns {string | undefined}
 */
export function wasm_capability_error(capability_key) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passStringToWasm0(capability_key, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
        const len0 = WASM_VECTOR_LEN;
        wasm.wasm_capability_error(retptr, ptr0, len0);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        let v2;
        if (r0 !== 0) {
            v2 = getStringFromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 1, 1);
        }
        return v2;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Returns the canonical capability keys exported by the wasm bridge.
 * @returns {Array<any>}
 */
export function wasm_capability_keys() {
    const ret = wasm.wasm_capability_keys();
    return takeObject(ret);
}

/**
 * Returns a stable blocker message for wasm execution blockers.
 * @param {string} blocker_key
 * @returns {string | undefined}
 */
export function wasm_execution_blocker_error(blocker_key) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passStringToWasm0(blocker_key, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
        const len0 = WASM_VECTOR_LEN;
        wasm.wasm_execution_blocker_error(retptr, ptr0, len0);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        let v2;
        if (r0 !== 0) {
            v2 = getStringFromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 1, 1);
        }
        return v2;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Returns canonical blocker keys that currently prevent wasm execution.
 * @returns {Array<any>}
 */
export function wasm_execution_blocker_keys() {
    const ret = wasm.wasm_execution_blocker_keys();
    return takeObject(ret);
}

/**
 * Returns key+message entries for known execution blockers.
 * @returns {Array<any>}
 */
export function wasm_execution_blockers() {
    const ret = wasm.wasm_execution_blockers();
    return takeObject(ret);
}

/**
 * Returns canonical phase keys for top-level execute() contract responses.
 * @returns {Array<any>}
 */
export function wasm_execution_phase_keys() {
    const ret = wasm.wasm_execution_phase_keys();
    return takeObject(ret);
}

/**
 * Returns module-level blocker policy entries for browser-mode preflight UX.
 * @returns {Array<any>}
 */
export function wasm_module_policy_entries() {
    const ret = wasm.wasm_module_policy_entries();
    return takeObject(ret);
}

/**
 * Reports whether a module is known to require an unsupported wasm capability.
 * @param {string} module_name
 * @returns {WasmModuleSupport}
 */
export function wasm_module_support(module_name) {
    const ptr0 = passStringToWasm0(module_name, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_module_support(ptr0, len0);
    return WasmModuleSupport.__wrap(ret);
}

/**
 * Reports runtime contract status for browser clients.
 * @returns {WasmRuntimeInfo}
 */
export function wasm_runtime_info() {
    const ret = wasm.wasm_runtime_info();
    return WasmRuntimeInfo.__wrap(ret);
}

/**
 * Returns snippet blockers detected from import preflight analysis.
 * @param {string} source
 * @returns {Array<any>}
 */
export function wasm_snippet_blockers(source) {
    const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_snippet_blockers(ptr0, len0);
    return takeObject(ret);
}

/**
 * Returns canonical root imports detected from parse+compile-valid snippet source.
 * @param {string} source
 * @returns {Array<any>}
 */
export function wasm_snippet_import_roots(source) {
    const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_snippet_import_roots(ptr0, len0);
    return takeObject(ret);
}

/**
 * Preflight analysis for snippet viability in wasm mode.
 * @param {string} source
 * @returns {WasmSnippetSupport}
 */
export function wasm_snippet_support(source) {
    const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_snippet_support(ptr0, len0);
    return WasmSnippetSupport.__wrap(ret);
}

/**
 * Clears previously registered virtual stdlib module sources used by wasm VM sessions.
 */
export function wasm_virtual_stdlib_clear() {
    wasm.wasm_virtual_stdlib_clear();
}

/**
 * Returns the number of registered virtual stdlib module sources in wasm runtime.
 * @returns {number}
 */
export function wasm_virtual_stdlib_count() {
    const ret = wasm.wasm_virtual_stdlib_count();
    return ret >>> 0;
}

/**
 * Registers a virtual stdlib module source for wasm VM sessions.
 * @param {string} module_name
 * @param {string} source
 * @param {boolean} is_package
 * @returns {boolean}
 */
export function wasm_virtual_stdlib_register(module_name, source, is_package) {
    const ptr0 = passStringToWasm0(module_name, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_virtual_stdlib_register(ptr0, len0, ptr1, len1, is_package);
    return ret !== 0;
}

/**
 * Returns a stable blocker message for wasm worker blockers.
 * @param {string} blocker_key
 * @returns {string | undefined}
 */
export function wasm_worker_blocker_error(blocker_key) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passStringToWasm0(blocker_key, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
        const len0 = WASM_VECTOR_LEN;
        wasm.wasm_worker_blocker_error(retptr, ptr0, len0);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        let v2;
        if (r0 !== 0) {
            v2 = getStringFromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 1, 1);
        }
        return v2;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Returns canonical blocker keys for worker-mode execution.
 * @returns {Array<any>}
 */
export function wasm_worker_blocker_keys() {
    const ret = wasm.wasm_worker_blocker_keys();
    return takeObject(ret);
}

/**
 * Returns key+message entries for known worker blockers.
 * @returns {Array<any>}
 */
export function wasm_worker_blockers() {
    const ret = wasm.wasm_worker_blockers();
    return takeObject(ret);
}

/**
 * Returns the currently configured worker timeout value in milliseconds.
 * @returns {number}
 */
export function wasm_worker_current_timeout_ms() {
    const ret = wasm.wasm_worker_current_timeout_ms();
    return ret >>> 0;
}

/**
 * Executes a snippet through the wasm worker contract.
 *
 * Current milestone behavior:
 * - parse-invalid input returns `phase = "syntax_error"`,
 * - parse-valid but compile-invalid input returns `phase = "compile_error"`,
 * - parse+compile-valid snippets that import known blocked modules return
 *   `phase = "unsupported_worker_execution"` with capability-specific blocker keys,
 * - default builds return `phase = "unsupported_worker_execution"` for remaining
 *   parse+compile-valid snippets,
 * - `wasm-vm-probe` builds execute remaining snippets through VM and return
 *   `phase = "ok"` or `phase = "runtime_error"`.
 * @param {string} source
 * @returns {WasmExecutionResult}
 */
export function wasm_worker_execute(source) {
    const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_worker_execute(ptr0, len0);
    return WasmExecutionResult.__wrap(ret);
}

/**
 * Returns canonical execute phase keys for wasm worker runtime contracts.
 * @returns {Array<any>}
 */
export function wasm_worker_execute_phase_keys() {
    const ret = wasm.wasm_worker_execute_phase_keys();
    return takeObject(ret);
}

/**
 * Executes a snippet through the worker contract with an operation correlation id.
 * @param {string} source
 * @returns {WasmWorkerExecutionResult}
 */
export function wasm_worker_execute_with_operation(source) {
    const ptr0 = passStringToWasm0(source, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.wasm_worker_execute_with_operation(ptr0, len0);
    return WasmWorkerExecutionResult.__wrap(ret);
}

/**
 * Reports worker-runtime contract state for browser clients.
 * @returns {WasmWorkerInfo}
 */
export function wasm_worker_info() {
    const ret = wasm.wasm_worker_info();
    return WasmWorkerInfo.__wrap(ret);
}

/**
 * Returns canonical lifecycle phase keys for wasm worker runtime contracts.
 * @returns {Array<any>}
 */
export function wasm_worker_lifecycle_phase_keys() {
    const ret = wasm.wasm_worker_lifecycle_phase_keys();
    return takeObject(ret);
}

/**
 * Recycles worker runtime execution state.
 *
 * Current milestone behavior:
 * - `wasm-vm-probe`: returns `phase = "worker_recycled"` and `state = "ready"`,
 * - default builds: unsupported lifecycle result with unwired blocker.
 * @returns {WasmWorkerLifecycleResult}
 */
export function wasm_worker_recycle() {
    const ret = wasm.wasm_worker_recycle();
    return WasmWorkerLifecycleResult.__wrap(ret);
}

/**
 * Applies a requested timeout policy update for worker execution.
 *
 * Current milestone behavior:
 * - out-of-range values report `invalid_worker_timeout`,
 * - default builds report `unsupported_worker_timeout_enforcement` for in-range values,
 * - `wasm-vm-probe` builds report `worker_timeout_configured` for in-range values.
 * @param {number} timeout_ms
 * @returns {WasmWorkerTimeoutResult}
 */
export function wasm_worker_set_timeout(timeout_ms) {
    const ret = wasm.wasm_worker_set_timeout(timeout_ms);
    return WasmWorkerTimeoutResult.__wrap(ret);
}

/**
 * Starts worker runtime execution.
 *
 * Current milestone behavior:
 * - `wasm-vm-probe`: returns `phase = "worker_started"` and `state = "ready"`,
 * - default builds: unsupported lifecycle result with unwired blocker.
 * @returns {WasmWorkerLifecycleResult}
 */
export function wasm_worker_start() {
    const ret = wasm.wasm_worker_start();
    return WasmWorkerLifecycleResult.__wrap(ret);
}

/**
 * Returns canonical worker state keys for wasm worker runtime contracts.
 * @returns {Array<any>}
 */
export function wasm_worker_state_keys() {
    const ret = wasm.wasm_worker_state_keys();
    return takeObject(ret);
}

/**
 * Terminates worker runtime execution.
 *
 * Current milestone behavior:
 * - `wasm-vm-probe`: returns `phase = "worker_terminated"` and `state = "unwired"`,
 * - default builds: unsupported lifecycle result with unwired blocker.
 * @returns {WasmWorkerLifecycleResult}
 */
export function wasm_worker_terminate() {
    const ret = wasm.wasm_worker_terminate();
    return WasmWorkerLifecycleResult.__wrap(ret);
}

/**
 * Returns canonical timeout phase keys for wasm worker timeout contracts.
 * @returns {Array<any>}
 */
export function wasm_worker_timeout_phase_keys() {
    const ret = wasm.wasm_worker_timeout_phase_keys();
    return takeObject(ret);
}

/**
 * Returns timeout/recycle policy contract for wasm worker execution.
 * @returns {WasmWorkerTimeoutPolicy}
 */
export function wasm_worker_timeout_policy() {
    const ret = wasm.wasm_worker_timeout_policy();
    return WasmWorkerTimeoutPolicy.__wrap(ret);
}

function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg___wbindgen_throw_6ddd609b62940d55: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg_error_a6fa202b58aa1cd3: function(arg0, arg1) {
            let deferred0_0;
            let deferred0_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                console.error(getStringFromWasm0(arg0, arg1));
            } finally {
                wasm.__wbindgen_export(deferred0_0, deferred0_1, 1);
            }
        },
        __wbg_getRandomValues_ab1935b403569652: function() { return handleError(function (arg0, arg1) {
            globalThis.crypto.getRandomValues(getArrayU8FromWasm0(arg0, arg1));
        }, arguments); },
        __wbg_new_227d7c05414eb861: function() {
            const ret = new Error();
            return addHeapObject(ret);
        },
        __wbg_new_a70fbab9066b301f: function() {
            const ret = new Array();
            return addHeapObject(ret);
        },
        __wbg_now_16f0c993d5dd6c27: function() {
            const ret = Date.now();
            return ret;
        },
        __wbg_push_e87b0e732085a946: function(arg0, arg1) {
            const ret = getObject(arg0).push(getObject(arg1));
            return ret;
        },
        __wbg_stack_3b0d974bbf31e44f: function(arg0, arg1) {
            const ret = getObject(arg1).stack;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_export3, wasm.__wbindgen_export4);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_wasmexecutionblocker_new: function(arg0) {
            const ret = WasmExecutionBlocker.__wrap(arg0);
            return addHeapObject(ret);
        },
        __wbg_wasmmodulepolicyentry_new: function(arg0) {
            const ret = WasmModulePolicyEntry.__wrap(arg0);
            return addHeapObject(ret);
        },
        __wbg_wasmsnippetblocker_new: function(arg0) {
            const ret = WasmSnippetBlocker.__wrap(arg0);
            return addHeapObject(ret);
        },
        __wbg_wasmworkerblocker_new: function(arg0) {
            const ret = WasmWorkerBlocker.__wrap(arg0);
            return addHeapObject(ret);
        },
        __wbindgen_cast_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return addHeapObject(ret);
        },
        __wbindgen_object_drop_ref: function(arg0) {
            takeObject(arg0);
        },
    };
    return {
        __proto__: null,
        "./pyrs_bg.js": import0,
    };
}

const WasmCapabilityReportFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmcapabilityreport_free(ptr >>> 0, 1));
const WasmCompileResultFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmcompileresult_free(ptr >>> 0, 1));
const WasmExecutionBlockerFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmexecutionblocker_free(ptr >>> 0, 1));
const WasmExecutionResultFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmexecutionresult_free(ptr >>> 0, 1));
const WasmModulePolicyEntryFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmmodulepolicyentry_free(ptr >>> 0, 1));
const WasmModuleSupportFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmmodulesupport_free(ptr >>> 0, 1));
const WasmReplSessionFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmreplsession_free(ptr >>> 0, 1));
const WasmRuntimeInfoFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmruntimeinfo_free(ptr >>> 0, 1));
const WasmSessionFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmsession_free(ptr >>> 0, 1));
const WasmSnippetBlockerFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmsnippetblocker_free(ptr >>> 0, 1));
const WasmSnippetSupportFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmsnippetsupport_free(ptr >>> 0, 1));
const WasmSyntaxResultFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmsyntaxresult_free(ptr >>> 0, 1));
const WasmWorkerBlockerFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmworkerblocker_free(ptr >>> 0, 1));
const WasmWorkerExecutionResultFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmworkerexecutionresult_free(ptr >>> 0, 1));
const WasmWorkerInfoFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmworkerinfo_free(ptr >>> 0, 1));
const WasmWorkerLifecycleResultFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmworkerlifecycleresult_free(ptr >>> 0, 1));
const WasmWorkerSessionFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmworkersession_free(ptr >>> 0, 1));
const WasmWorkerSessionSnapshotFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmworkersessionsnapshot_free(ptr >>> 0, 1));
const WasmWorkerTimeoutPolicyFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmworkertimeoutpolicy_free(ptr >>> 0, 1));
const WasmWorkerTimeoutResultFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmworkertimeoutresult_free(ptr >>> 0, 1));

function addHeapObject(obj) {
    if (heap_next === heap.length) heap.push(heap.length + 1);
    const idx = heap_next;
    heap_next = heap[idx];

    heap[idx] = obj;
    return idx;
}

function dropObject(idx) {
    if (idx < 1028) return;
    heap[idx] = heap_next;
    heap_next = idx;
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

function getStringFromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return decodeText(ptr, len);
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function getObject(idx) { return heap[idx]; }

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        wasm.__wbindgen_export2(addHeapObject(e));
    }
}

let heap = new Array(1024).fill(undefined);
heap.push(undefined, null, true, false);

let heap_next = heap.length;

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeObject(idx) {
    const ret = getObject(idx);
    dropObject(idx);
    return ret;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasm;
function __wbg_finalize_init(instance, module) {
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('pyrs_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
