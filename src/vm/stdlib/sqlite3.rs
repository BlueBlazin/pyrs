use super::super::{Vm, Value, RuntimeError, Object, HashMap, InternalCallOutcome, format_repr, value_to_int, is_truthy, BigInt, classify_runtime_error, vm_current_thread_ident, ObjRef, bytes_like_from_value, BuiltinFunction, dict_get_value, dict_set_value_checked, value_to_f64, slice_indices};
use std::cell::Cell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uchar, c_uint, c_void};
use std::ptr::{self, NonNull};
use std::sync::Arc;

#[repr(C)]
pub(in crate::vm) struct Sqlite3Db {
    _private: [u8; 0],
}

#[repr(C)]
struct Sqlite3Stmt {
    _private: [u8; 0],
}

#[repr(C)]
struct Sqlite3Backup {
    _private: [u8; 0],
}

#[repr(C)]
struct Sqlite3Blob {
    _private: [u8; 0],
}

#[repr(C)]
struct Sqlite3Context {
    _private: [u8; 0],
}

#[repr(C)]
struct Sqlite3Value {
    _private: [u8; 0],
}

type SqliteDestructor = Option<unsafe extern "C" fn(*mut c_void)>;
type SqliteExecCallback =
    Option<unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_char, *mut *mut c_char) -> c_int>;
type SqliteFunctionCallback =
    Option<unsafe extern "C" fn(*mut Sqlite3Context, c_int, *mut *mut Sqlite3Value)>;
type SqliteAuthorizerCallback = Option<
    unsafe extern "C" fn(
        *mut c_void,
        c_int,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
    ) -> c_int,
>;
type SqliteProgressCallback = Option<unsafe extern "C" fn(*mut c_void) -> c_int>;
type SqliteTraceCallback =
    Option<unsafe extern "C" fn(c_uint, *mut c_void, *mut c_void, *mut c_void) -> c_int>;
type SqliteCollationCallback =
    Option<unsafe extern "C" fn(*mut c_void, c_int, *const c_void, c_int, *const c_void) -> c_int>;

#[link(name = "sqlite3")]
unsafe extern "C" {
    fn sqlite3_open_v2(
        filename: *const c_char,
        db_out: *mut *mut Sqlite3Db,
        flags: c_int,
        vfs: *const c_char,
    ) -> c_int;
    fn sqlite3_close_v2(db: *mut Sqlite3Db) -> c_int;
    fn sqlite3_errmsg(db: *mut Sqlite3Db) -> *const c_char;
    fn sqlite3_libversion() -> *const c_char;
    fn sqlite3_prepare_v2(
        db: *mut Sqlite3Db,
        sql: *const c_char,
        nbyte: c_int,
        stmt_out: *mut *mut Sqlite3Stmt,
        tail_out: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_step(stmt: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_finalize(stmt: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_column_count(stmt: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_column_type(stmt: *mut Sqlite3Stmt, col: c_int) -> c_int;
    fn sqlite3_column_int64(stmt: *mut Sqlite3Stmt, col: c_int) -> i64;
    fn sqlite3_column_double(stmt: *mut Sqlite3Stmt, col: c_int) -> f64;
    fn sqlite3_column_text(stmt: *mut Sqlite3Stmt, col: c_int) -> *const c_uchar;
    fn sqlite3_column_blob(stmt: *mut Sqlite3Stmt, col: c_int) -> *const c_void;
    fn sqlite3_column_bytes(stmt: *mut Sqlite3Stmt, col: c_int) -> c_int;
    fn sqlite3_column_name(stmt: *mut Sqlite3Stmt, col: c_int) -> *const c_char;
    fn sqlite3_bind_parameter_count(stmt: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_bind_parameter_name(stmt: *mut Sqlite3Stmt, idx: c_int) -> *const c_char;
    fn sqlite3_bind_null(stmt: *mut Sqlite3Stmt, idx: c_int) -> c_int;
    fn sqlite3_bind_int64(stmt: *mut Sqlite3Stmt, idx: c_int, value: i64) -> c_int;
    fn sqlite3_bind_double(stmt: *mut Sqlite3Stmt, idx: c_int, value: f64) -> c_int;
    fn sqlite3_bind_text(
        stmt: *mut Sqlite3Stmt,
        idx: c_int,
        text: *const c_char,
        len: c_int,
        destructor: SqliteDestructor,
    ) -> c_int;
    fn sqlite3_bind_blob(
        stmt: *mut Sqlite3Stmt,
        idx: c_int,
        blob: *const c_void,
        len: c_int,
        destructor: SqliteDestructor,
    ) -> c_int;
    fn sqlite3_backup_init(
        dest: *mut Sqlite3Db,
        dest_name: *const c_char,
        src: *mut Sqlite3Db,
        src_name: *const c_char,
    ) -> *mut Sqlite3Backup;
    fn sqlite3_backup_step(backup: *mut Sqlite3Backup, pages: c_int) -> c_int;
    fn sqlite3_backup_finish(backup: *mut Sqlite3Backup) -> c_int;
    fn sqlite3_backup_remaining(backup: *mut Sqlite3Backup) -> c_int;
    fn sqlite3_backup_pagecount(backup: *mut Sqlite3Backup) -> c_int;
    fn sqlite3_sleep(ms: c_int) -> c_int;
    fn sqlite3_complete(sql: *const c_char) -> c_int;
    fn sqlite3_blob_open(
        db: *mut Sqlite3Db,
        db_name: *const c_char,
        table_name: *const c_char,
        column_name: *const c_char,
        row_id: i64,
        flags: c_int,
        blob_out: *mut *mut Sqlite3Blob,
    ) -> c_int;
    fn sqlite3_blob_close(blob: *mut Sqlite3Blob) -> c_int;
    fn sqlite3_blob_bytes(blob: *mut Sqlite3Blob) -> c_int;
    fn sqlite3_blob_read(
        blob: *mut Sqlite3Blob,
        buf: *mut c_void,
        n: c_int,
        offset: c_int,
    ) -> c_int;
    fn sqlite3_blob_write(
        blob: *mut Sqlite3Blob,
        buf: *const c_void,
        n: c_int,
        offset: c_int,
    ) -> c_int;
    fn sqlite3_exec(
        db: *mut Sqlite3Db,
        sql: *const c_char,
        callback: SqliteExecCallback,
        callback_arg: *mut c_void,
        err_out: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_limit(db: *mut Sqlite3Db, id: c_int, new_val: c_int) -> c_int;
    fn sqlite3_db_config(db: *mut Sqlite3Db, op: c_int, ...) -> c_int;
    fn sqlite3_total_changes(db: *mut Sqlite3Db) -> c_int;
    fn sqlite3_get_autocommit(db: *mut Sqlite3Db) -> c_int;
    fn sqlite3_extended_errcode(db: *mut Sqlite3Db) -> c_int;
    fn sqlite3_interrupt(db: *mut Sqlite3Db);
    fn sqlite3_changes(db: *mut Sqlite3Db) -> c_int;
    fn sqlite3_last_insert_rowid(db: *mut Sqlite3Db) -> i64;
    fn sqlite3_create_function_v2(
        db: *mut Sqlite3Db,
        z_function_name: *const c_char,
        n_arg: c_int,
        e_text_rep: c_int,
        p_app: *mut c_void,
        x_func: SqliteFunctionCallback,
        x_step: SqliteFunctionCallback,
        x_final: SqliteFunctionCallback,
        x_destroy: SqliteDestructor,
    ) -> c_int;
    fn sqlite3_create_collation_v2(
        db: *mut Sqlite3Db,
        z_name: *const c_char,
        e_text_rep: c_int,
        p_arg: *mut c_void,
        x_compare: SqliteCollationCallback,
        x_destroy: SqliteDestructor,
    ) -> c_int;
    fn sqlite3_set_authorizer(
        db: *mut Sqlite3Db,
        x_auth: SqliteAuthorizerCallback,
        p_user_data: *mut c_void,
    ) -> c_int;
    fn sqlite3_progress_handler(
        db: *mut Sqlite3Db,
        n_ops: c_int,
        x_progress: SqliteProgressCallback,
        p_user_data: *mut c_void,
    );
    fn sqlite3_trace_v2(
        db: *mut Sqlite3Db,
        mask: c_uint,
        callback: SqliteTraceCallback,
        p_user_data: *mut c_void,
    ) -> c_int;
    fn sqlite3_expanded_sql(stmt: *mut Sqlite3Stmt) -> *mut c_char;
    fn sqlite3_db_handle(stmt: *mut Sqlite3Stmt) -> *mut Sqlite3Db;
    fn sqlite3_free(ptr: *mut c_void);
    fn sqlite3_user_data(context: *mut Sqlite3Context) -> *mut c_void;
    fn sqlite3_value_type(value: *mut Sqlite3Value) -> c_int;
    fn sqlite3_value_int64(value: *mut Sqlite3Value) -> i64;
    fn sqlite3_value_double(value: *mut Sqlite3Value) -> f64;
    fn sqlite3_value_text(value: *mut Sqlite3Value) -> *const c_uchar;
    fn sqlite3_value_blob(value: *mut Sqlite3Value) -> *const c_void;
    fn sqlite3_value_bytes(value: *mut Sqlite3Value) -> c_int;
    fn sqlite3_result_null(context: *mut Sqlite3Context);
    fn sqlite3_result_int64(context: *mut Sqlite3Context, value: i64);
    fn sqlite3_result_double(context: *mut Sqlite3Context, value: f64);
    fn sqlite3_result_text(
        context: *mut Sqlite3Context,
        value: *const c_char,
        len: c_int,
        destructor: SqliteDestructor,
    );
    fn sqlite3_result_blob(
        context: *mut Sqlite3Context,
        value: *const c_void,
        len: c_int,
        destructor: SqliteDestructor,
    );
    fn sqlite3_result_error(context: *mut Sqlite3Context, value: *const c_char, len: c_int);
}

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;
const SQLITE_INTEGER: c_int = 1;
const SQLITE_FLOAT: c_int = 2;
const SQLITE_TEXT: c_int = 3;
const SQLITE_BLOB: c_int = 4;
const SQLITE_UTF8: c_int = 1;
const SQLITE_DETERMINISTIC: c_int = 0x0000_0800;
const SQLITE_TRACE_STMT: c_uint = 0x01;
const SQLITE_DENY: c_int = 1;
const SQLITE_ERROR: c_int = 1;
const SQLITE_INTERNAL: c_int = 2;
const SQLITE_PERM: c_int = 3;
const SQLITE_ABORT: c_int = 4;
const SQLITE_BUSY: c_int = 5;
const SQLITE_LOCKED: c_int = 6;
const SQLITE_NOMEM: c_int = 7;
const SQLITE_READONLY: c_int = 8;
const SQLITE_INTERRUPT: c_int = 9;
const SQLITE_IOERR: c_int = 10;
const SQLITE_CORRUPT: c_int = 11;
const SQLITE_NOTFOUND: c_int = 12;
const SQLITE_FULL: c_int = 13;
const SQLITE_CANTOPEN: c_int = 14;
const SQLITE_PROTOCOL: c_int = 15;
const SQLITE_EMPTY: c_int = 16;
const SQLITE_SCHEMA: c_int = 17;
const SQLITE_TOOBIG: c_int = 18;
const SQLITE_CONSTRAINT: c_int = 19;
const SQLITE_MISMATCH: c_int = 20;
const SQLITE_MISUSE: c_int = 21;
const SQLITE_RANGE: c_int = 25;
const SQLITE_OPEN_READWRITE: c_int = 0x0000_0002;
const SQLITE_OPEN_CREATE: c_int = 0x0000_0004;
const SQLITE_OPEN_URI: c_int = 0x0000_0040;
const SQLITE_LIMIT_SQL_LENGTH_ID: c_int = 1;
const SQLITE_LIMIT_MAX_CATEGORY: i64 = 11;
const SQLITE_CONNECTION_BASE_INIT_CALLED_ATTR: &str = "__pyrs_sqlite_base_init_called";
const SQLITE_CURSOR_BASE_INIT_CALLED_ATTR: &str = "__pyrs_sqlite_cursor_base_init_called";
const SQLITE_CONNECTION_ISOLATION_LEVEL_ATTR: &str = "isolation_level";
const SQLITE_LEGACY_TRANSACTION_CONTROL: i64 = -1;
const SQLITE_CONNECTION_ISOLATION_LEVEL_VALUE_ERROR: &str =
    "isolation_level string must be '', 'DEFERRED', 'IMMEDIATE', or 'EXCLUSIVE'";
const SQLITE_ROW_DATA_ATTR: &str = "__pyrs_sqlite_row_data";
const SQLITE_ROW_DESCRIPTION_ATTR: &str = "__pyrs_sqlite_row_description";
const SQLITE_DBCONFIG_KNOWN_OPS: &[i64] = &[
    1002, 1003, 1004, 1005, 1006, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014, 1015, 1016, 1017,
];

thread_local! {
    static SQLITE_CALLBACK_VM: Cell<*mut Vm> = const { Cell::new(ptr::null_mut()) };
}

struct SqliteCallbackVmGuard {
    previous: *mut Vm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqliteAutocommitMode {
    Legacy,
    Enabled,
    Disabled,
}

impl SqliteCallbackVmGuard {
    fn enter(vm: &mut Vm) -> Self {
        let previous = SQLITE_CALLBACK_VM.with(|slot| slot.replace(vm as *mut Vm));
        Self { previous }
    }
}

impl Drop for SqliteCallbackVmGuard {
    fn drop(&mut self) {
        SQLITE_CALLBACK_VM.with(|slot| slot.set(self.previous));
    }
}

#[derive(Clone)]
struct SqliteScalarFunctionCallbackState {
    callable: Value,
}

#[derive(Clone)]
struct SqliteVmCallbackState {
    callable: Value,
}

#[derive(Debug)]
pub(in crate::vm) struct SqliteConnectionState {
    handle: Option<NonNull<Sqlite3Db>>,
    check_same_thread: bool,
    creator_thread_ident: i64,
    autocommit_mode: SqliteAutocommitMode,
    trace_callback: Option<NonNull<Arc<SqliteVmCallbackState>>>,
    progress_callback: Option<NonNull<Arc<SqliteVmCallbackState>>>,
    authorizer_callback: Option<NonNull<Arc<SqliteVmCallbackState>>>,
}

impl SqliteConnectionState {
    fn drop_vm_callback_ptr(handle: Option<NonNull<Arc<SqliteVmCallbackState>>>) {
        if let Some(handle) = handle {
            // SAFETY: pointer was allocated with Box::into_raw(Box<Arc<_>>).
            unsafe { drop(Box::from_raw(handle.as_ptr())) };
        }
    }

    fn new(
        handle: *mut Sqlite3Db,
        check_same_thread: bool,
        autocommit_mode: SqliteAutocommitMode,
    ) -> Self {
        Self {
            handle: NonNull::new(handle),
            check_same_thread,
            creator_thread_ident: sqlite_current_thread_ident(),
            autocommit_mode,
            trace_callback: None,
            progress_callback: None,
            authorizer_callback: None,
        }
    }

    pub(in crate::vm) fn db_handle(&self) -> Option<*mut Sqlite3Db> {
        self.handle.map(NonNull::as_ptr)
    }

    fn clear_vm_callbacks(&mut self) {
        Self::drop_vm_callback_ptr(self.trace_callback.take());
        Self::drop_vm_callback_ptr(self.progress_callback.take());
        Self::drop_vm_callback_ptr(self.authorizer_callback.take());
    }

    pub(in crate::vm) fn close(&mut self) -> Result<(), String> {
        let Some(handle) = self.handle.take() else {
            self.clear_vm_callbacks();
            return Ok(());
        };
        // SAFETY: handle is a valid sqlite connection; disabling callbacks before close
        // prevents callback invocations from touching stale VM callback context.
        unsafe {
            let _ = sqlite3_trace_v2(handle.as_ptr(), SQLITE_TRACE_STMT, None, ptr::null_mut());
            sqlite3_progress_handler(handle.as_ptr(), 0, None, ptr::null_mut());
            let _ = sqlite3_set_authorizer(handle.as_ptr(), None, ptr::null_mut());
        }
        self.clear_vm_callbacks();
        // SAFETY: handle was created by sqlite3_open_v2 and is owned by this state.
        let rc = unsafe { sqlite3_close_v2(handle.as_ptr()) };
        if rc == SQLITE_OK {
            Ok(())
        } else {
            Err(format!("sqlite3_close_v2 failed with code {rc}"))
        }
    }

    pub(in crate::vm) fn ensure_thread_affinity(&self) -> Result<(), RuntimeError> {
        if !self.check_same_thread {
            return Ok(());
        }
        let current_thread_ident = sqlite_current_thread_ident();
        if self.creator_thread_ident == current_thread_ident {
            return Ok(());
        }
        Err(sqlite_error(
            "ProgrammingError",
            format!(
                "SQLite objects created in a thread can only be used in that same thread. \
The object was created in thread id {} and this is thread id {}.",
                self.creator_thread_ident, current_thread_ident
            ),
        ))
    }
}

impl Drop for SqliteConnectionState {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[derive(Debug, Clone)]
pub(in crate::vm) struct SqliteCursorState {
    pub(in crate::vm) connection_id: u64,
    pub(in crate::vm) rows: Vec<Value>,
    pub(in crate::vm) next_row: usize,
    pub(in crate::vm) description: Option<Value>,
    pub(in crate::vm) closed: bool,
}

impl SqliteCursorState {
    fn new(connection_id: u64) -> Self {
        Self {
            connection_id,
            rows: Vec::new(),
            next_row: 0,
            description: None,
            closed: false,
        }
    }
}

#[derive(Debug)]
struct SqliteQueryResult {
    rows: Vec<Value>,
    description: Option<Value>,
}

enum SqliteParams {
    Positional(Vec<Value>),
    Named(Value),
}

#[derive(Debug)]
pub(in crate::vm) struct SqliteBlobState {
    handle: Option<NonNull<Sqlite3Blob>>,
    pub(in crate::vm) connection_id: u64,
    offset: usize,
}

impl SqliteBlobState {
    fn new(handle: *mut Sqlite3Blob, connection_id: u64) -> Self {
        Self {
            handle: NonNull::new(handle),
            connection_id,
            offset: 0,
        }
    }

    fn handle(&self) -> Option<*mut Sqlite3Blob> {
        self.handle.map(NonNull::as_ptr)
    }

    fn close(&mut self) -> Result<(), String> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        // SAFETY: handle was created by sqlite3_blob_open and is owned by this state.
        let _ = unsafe { sqlite3_blob_close(handle.as_ptr()) };
        // CPython close path intentionally ignores sqlite3_blob_close return code.
        Ok(())
    }
}

impl Drop for SqliteBlobState {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

struct PreparedStatement {
    raw: NonNull<Sqlite3Stmt>,
}

impl PreparedStatement {
    fn as_ptr(&self) -> *mut Sqlite3Stmt {
        self.raw.as_ptr()
    }
}

impl Drop for PreparedStatement {
    fn drop(&mut self) {
        // SAFETY: statement pointer is valid and owned by this wrapper.
        unsafe {
            let _ = sqlite3_finalize(self.raw.as_ptr());
        }
    }
}

enum SqliteBlobSetOp {
    Slice {
        lower: Option<i64>,
        upper: Option<i64>,
        step: Option<i64>,
        payload: Vec<u8>,
    },
    Index(i64, u8),
}

fn sqlite_error(kind: &str, message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(format!("{kind}: {}", message.into()))
}

fn sqlite_transient_destructor() -> SqliteDestructor {
    // SAFETY: sqlite3 defines SQLITE_TRANSIENT as (sqlite3_destructor_type)-1.
    unsafe { std::mem::transmute::<isize, SqliteDestructor>(-1isize) }
}

unsafe fn sqlite_result_error_message(context: *mut Sqlite3Context, message: &str) {
    if let Ok(c_message) = CString::new(message) {
        // SAFETY: context is provided by sqlite and c_message is null-terminated.
        unsafe { sqlite3_result_error(context, c_message.as_ptr(), -1) };
    } else {
        let fallback = CString::new("sqlite callback error").expect("static string is valid");
        // SAFETY: context is provided by sqlite and fallback is null-terminated.
        unsafe { sqlite3_result_error(context, fallback.as_ptr(), -1) };
    }
}

unsafe fn sqlite_value_to_vm_value(vm: &mut Vm, value: *mut Sqlite3Value) -> Value {
    // SAFETY: value pointer is provided by sqlite for the current callback frame.
    let value_type = unsafe { sqlite3_value_type(value) };
    match value_type {
        SQLITE_INTEGER => {
            // SAFETY: sqlite conversion is valid for integer-typed value.
            Value::Int(unsafe { sqlite3_value_int64(value) })
        }
        SQLITE_FLOAT => {
            // SAFETY: sqlite conversion is valid for float-typed value.
            Value::Float(unsafe { sqlite3_value_double(value) })
        }
        SQLITE_TEXT => {
            // SAFETY: sqlite returns UTF-8 text pointer for SQLITE_TEXT.
            let ptr = unsafe { sqlite3_value_text(value) };
            if ptr.is_null() {
                Value::None
            } else {
                // SAFETY: sqlite returns byte count for the same value pointer.
                let len = unsafe { sqlite3_value_bytes(value) };
                if len <= 0 {
                    Value::Str(String::new())
                } else {
                    let len = usize::try_from(len).unwrap_or(0);
                    // SAFETY: sqlite guarantees at least len bytes valid.
                    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
                    Value::Str(String::from_utf8_lossy(bytes).to_string())
                }
            }
        }
        SQLITE_BLOB => {
            // SAFETY: sqlite returns blob pointer and size for the same value.
            let ptr = unsafe { sqlite3_value_blob(value) };
            // SAFETY: sqlite conversion for byte length is valid.
            let len = unsafe { sqlite3_value_bytes(value) };
            if ptr.is_null() || len <= 0 {
                vm.heap.alloc_bytes(Vec::new())
            } else {
                let len = usize::try_from(len).unwrap_or(0);
                // SAFETY: sqlite guarantees at least len bytes valid.
                let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
                vm.heap.alloc_bytes(bytes.to_vec())
            }
        }
        _ => Value::None,
    }
}

unsafe fn sqlite_result_from_vm_value(
    vm: &mut Vm,
    context: *mut Sqlite3Context,
    value: Value,
) -> Result<(), String> {
    match value {
        Value::None => {
            // SAFETY: sqlite callback context is valid for result emission.
            unsafe { sqlite3_result_null(context) };
            Ok(())
        }
        Value::Bool(flag) => {
            // SAFETY: sqlite callback context is valid for result emission.
            unsafe { sqlite3_result_int64(context, if flag { 1 } else { 0 }) };
            Ok(())
        }
        Value::Int(int_value) => {
            // SAFETY: sqlite callback context is valid for result emission.
            unsafe { sqlite3_result_int64(context, int_value) };
            Ok(())
        }
        Value::BigInt(bigint_obj) => {
            let int_value = bigint_obj
                .to_i64()
                .ok_or_else(|| "Python int too large to convert to SQLite INTEGER".to_string())?;
            // SAFETY: sqlite callback context is valid for result emission.
            unsafe { sqlite3_result_int64(context, int_value) };
            Ok(())
        }
        Value::Float(float_value) => {
            // SAFETY: sqlite callback context is valid for result emission.
            unsafe { sqlite3_result_double(context, float_value) };
            Ok(())
        }
        Value::Str(text) => {
            let bytes = text.as_bytes();
            let len = sqlite_len_to_c_int(bytes.len(), "sqlite callback text")
                .map_err(|err| err.message)?;
            // SAFETY: sqlite copies bytes because SQLITE_TRANSIENT is used.
            unsafe {
                sqlite3_result_text(
                    context,
                    bytes.as_ptr() as *const c_char,
                    len,
                    sqlite_transient_destructor(),
                )
            };
            Ok(())
        }
        Value::Bytes(bytes_obj) => {
            let Object::Bytes(bytes) = &*bytes_obj.kind() else {
                return Err("user-defined function returned unsupported value".to_string());
            };
            let len = sqlite_len_to_c_int(bytes.len(), "sqlite callback blob")
                .map_err(|err| err.message)?;
            // SAFETY: sqlite copies bytes because SQLITE_TRANSIENT is used.
            unsafe {
                sqlite3_result_blob(
                    context,
                    bytes.as_ptr() as *const c_void,
                    len,
                    sqlite_transient_destructor(),
                )
            };
            Ok(())
        }
        Value::ByteArray(bytearray_obj) => {
            let Object::ByteArray(bytes) = &*bytearray_obj.kind() else {
                return Err("user-defined function returned unsupported value".to_string());
            };
            let len = sqlite_len_to_c_int(bytes.len(), "sqlite callback blob")
                .map_err(|err| err.message)?;
            // SAFETY: sqlite copies bytes because SQLITE_TRANSIENT is used.
            unsafe {
                sqlite3_result_blob(
                    context,
                    bytes.as_ptr() as *const c_void,
                    len,
                    sqlite_transient_destructor(),
                )
            };
            Ok(())
        }
        Value::MemoryView(_) => {
            let bytes = vm
                .value_to_bytes_payload(value)
                .map_err(|_| "user-defined function returned unsupported value".to_string())?;
            let len = sqlite_len_to_c_int(bytes.len(), "sqlite callback blob")
                .map_err(|err| err.message)?;
            // SAFETY: sqlite copies bytes because SQLITE_TRANSIENT is used.
            unsafe {
                sqlite3_result_blob(
                    context,
                    bytes.as_ptr() as *const c_void,
                    len,
                    sqlite_transient_destructor(),
                )
            };
            Ok(())
        }
        _ => Err("user-defined function returned unsupported value".to_string()),
    }
}

unsafe extern "C" fn sqlite_scalar_function_destroy(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: sqlite invokes destroy exactly once for the pointer provided
    // in sqlite3_create_function_v2 registration.
    unsafe {
        drop(Box::from_raw(ptr as *mut SqliteScalarFunctionCallbackState));
    }
}

unsafe extern "C" fn sqlite_scalar_function_callback(
    context: *mut Sqlite3Context,
    argc: c_int,
    argv: *mut *mut Sqlite3Value,
) {
    if context.is_null() {
        return;
    }
    // SAFETY: sqlite provided user data pointer for this callback registration.
    let callback_state =
        unsafe { sqlite3_user_data(context) as *mut SqliteScalarFunctionCallbackState };
    if callback_state.is_null() {
        // SAFETY: context is valid for result emission.
        unsafe { sqlite3_result_null(context) };
        return;
    }

    let vm_ptr = SQLITE_CALLBACK_VM.with(|slot| slot.get());
    if vm_ptr.is_null() {
        // SAFETY: context is valid for result emission.
        unsafe {
            sqlite_result_error_message(context, "sqlite callback VM context is unavailable")
        };
        return;
    }

    // SAFETY: callback executes while VM guard keeps pointer valid.
    let vm = unsafe { &mut *vm_ptr };
    let callback_state = unsafe { &*callback_state };
    let argc = usize::try_from(argc).unwrap_or(0);
    let mut args = Vec::with_capacity(argc);
    for index in 0..argc {
        // SAFETY: sqlite guarantees argv has argc elements.
        let value_ptr = unsafe { *argv.add(index) };
        // SAFETY: pointer originates from sqlite callback argument array.
        args.push(unsafe { sqlite_value_to_vm_value(vm, value_ptr) });
    }

    let outcome =
        vm.call_internal_preserving_caller(callback_state.callable.clone(), args, HashMap::new());
    match outcome {
        Ok(InternalCallOutcome::Value(value)) => {
            // SAFETY: context is valid for result emission.
            if let Err(message) = unsafe { sqlite_result_from_vm_value(vm, context, value) } {
                // SAFETY: context is valid for result emission.
                unsafe { sqlite_result_error_message(context, &message) };
            }
        }
        Ok(InternalCallOutcome::CallerExceptionHandled) => {
            let message = vm
                .runtime_error_from_active_exception("sqlite callback failed")
                .message;
            vm.clear_active_exception();
            // SAFETY: context is valid for result emission.
            unsafe { sqlite_result_error_message(context, &message) };
        }
        Err(err) => {
            // SAFETY: context is valid for result emission.
            unsafe { sqlite_result_error_message(context, &err.message) };
        }
    }
}

fn sqlite_callback_state_from_ptr(ctx: *mut c_void) -> Option<Arc<SqliteVmCallbackState>> {
    let ptr = ctx as *const Arc<SqliteVmCallbackState>;
    if ptr.is_null() {
        None
    } else {
        // SAFETY: pointer originates from Box<Arc<_>> registration and remains valid for callback.
        Some(unsafe { (*ptr).clone() })
    }
}

fn sqlite_opt_cstr_to_value(text: *const c_char) -> Value {
    if text.is_null() {
        Value::None
    } else {
        // SAFETY: sqlite callback arguments are valid NUL-terminated strings when non-null.
        let text = unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned();
        Value::Str(text)
    }
}

fn sqlite_callback_error_to_exception(vm: &mut Vm, err: RuntimeError) -> Value {
    for frame in vm.frames.iter_mut().rev() {
        if let Some(active) = frame.active_exception.take() {
            return active;
        }
    }
    vm.runtime_error_to_exception_value(err)
}

fn sqlite_callback_display_name(callable: &Value) -> Option<String> {
    match callable {
        Value::Function(function) => match &*function.kind() {
            Object::Function(function_data) => Some(function_data.code.name.clone()),
            _ => None,
        },
        Value::BoundMethod(method) => match &*method.kind() {
            Object::BoundMethod(method_data) => match &*method_data.function.kind() {
                Object::Function(function_data) => Some(function_data.code.name.clone()),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

fn sqlite_callback_argument_mismatch_error(
    callable: &Value,
    given_positional: usize,
) -> Option<RuntimeError> {
    let (name, required_positional) = match callable {
        Value::Function(function) => match &*function.kind() {
            Object::Function(function_data) => {
                let total_positional =
                    function_data.code.posonly_params.len() + function_data.code.params.len();
                let default_count = function_data.defaults.len();
                let required = total_positional.saturating_sub(default_count);
                (function_data.code.name.clone(), required)
            }
            _ => return None,
        },
        Value::BoundMethod(method) => match &*method.kind() {
            Object::BoundMethod(method_data) => match &*method_data.function.kind() {
                Object::Function(function_data) => {
                    let total_positional =
                        function_data.code.posonly_params.len() + function_data.code.params.len();
                    let default_count = function_data.defaults.len();
                    let required = total_positional.saturating_sub(default_count);
                    (function_data.code.name.clone(), required)
                }
                _ => return None,
            },
            _ => return None,
        },
        _ => return None,
    };
    if given_positional < required_positional {
        let missing = required_positional - given_positional;
        Some(RuntimeError::new(format!(
            "TypeError: {name}() missing {missing} required positional arguments"
        )))
    } else {
        None
    }
}

fn sqlite_report_callback_exception(vm: &mut Vm, callable: &Value, err: RuntimeError) {
    let exception = sqlite_callback_error_to_exception(vm, err);
    if vm.sqlite_callback_tracebacks_enabled {
        let callback_label = sqlite_callback_display_name(callable)
            .map(|name| format!("<function {name}>"))
            .unwrap_or_else(|| format_repr(callable));
        let message = format!("Exception ignored on sqlite3 callback {callback_label}");
        vm.emit_unraisable_exception(exception, Some(callable.clone()), Some(message.as_str()));
    }
    for frame in &mut vm.frames {
        frame.active_exception = None;
    }
}

unsafe extern "C" fn sqlite_vm_callback_destroy(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: sqlite callback userdata was allocated as Box<Arc<SqliteVmCallbackState>>.
    unsafe {
        drop(Box::from_raw(ptr as *mut Arc<SqliteVmCallbackState>));
    }
}

unsafe extern "C" fn sqlite_authorizer_callback(
    ctx: *mut c_void,
    action: c_int,
    arg1: *const c_char,
    arg2: *const c_char,
    dbname: *const c_char,
    source: *const c_char,
) -> c_int {
    let Some(state) = sqlite_callback_state_from_ptr(ctx) else {
        return SQLITE_DENY;
    };
    let vm_ptr = SQLITE_CALLBACK_VM.with(|slot| slot.get());
    if vm_ptr.is_null() {
        return SQLITE_DENY;
    }
    // SAFETY: callback executes under SqliteCallbackVmGuard.
    let vm = unsafe { &mut *vm_ptr };
    let args = vec![
        Value::Int(action as i64),
        sqlite_opt_cstr_to_value(arg1),
        sqlite_opt_cstr_to_value(arg2),
        sqlite_opt_cstr_to_value(dbname),
        sqlite_opt_cstr_to_value(source),
    ];
    match vm.call_internal_preserving_caller(state.callable.clone(), args, HashMap::new()) {
        Ok(InternalCallOutcome::Value(value)) => match value_to_int(value) {
            Ok(result) => match i32::try_from(result) {
                Ok(result) => result,
                Err(_) => {
                    sqlite_report_callback_exception(
                        vm,
                        &state.callable,
                        RuntimeError::new(
                            "OverflowError: Python int too large to convert to SQLite integer",
                        ),
                    );
                    SQLITE_DENY
                }
            },
            Err(err) => {
                let err = if err.message.contains("integer overflow") {
                    RuntimeError::new(
                        "OverflowError: Python int too large to convert to SQLite integer",
                    )
                } else {
                    err
                };
                sqlite_report_callback_exception(vm, &state.callable, err);
                SQLITE_DENY
            }
        },
        Ok(InternalCallOutcome::CallerExceptionHandled) => {
            let err = vm.runtime_error_from_active_exception("authorizer callback failed");
            sqlite_report_callback_exception(vm, &state.callable, err);
            SQLITE_DENY
        }
        Err(err) => {
            sqlite_report_callback_exception(vm, &state.callable, err);
            SQLITE_DENY
        }
    }
}

unsafe extern "C" fn sqlite_progress_callback(ctx: *mut c_void) -> c_int {
    let Some(state) = sqlite_callback_state_from_ptr(ctx) else {
        return -1;
    };
    let vm_ptr = SQLITE_CALLBACK_VM.with(|slot| slot.get());
    if vm_ptr.is_null() {
        return -1;
    }
    // SAFETY: callback executes under SqliteCallbackVmGuard.
    let vm = unsafe { &mut *vm_ptr };
    match vm.call_internal_preserving_caller(state.callable.clone(), Vec::new(), HashMap::new()) {
        Ok(InternalCallOutcome::Value(value)) => {
            let truthy = match &value {
                Value::None
                | Value::Bool(_)
                | Value::Int(_)
                | Value::BigInt(_)
                | Value::Float(_)
                | Value::Complex { .. }
                | Value::Str(_)
                | Value::List(_)
                | Value::Tuple(_)
                | Value::Dict(_)
                | Value::DictKeys(_)
                | Value::Set(_)
                | Value::FrozenSet(_)
                | Value::Bytes(_)
                | Value::ByteArray(_)
                | Value::MemoryView(_)
                | Value::Cell(_)
                | Value::Iterator(_)
                | Value::Generator(_)
                | Value::Slice { .. } => is_truthy(&value),
                _ => match vm.lookup_bound_special_method(&value, "__bool__") {
                    Ok(Some(bool_method)) => {
                        match vm.call_internal_preserving_caller(
                            bool_method,
                            Vec::new(),
                            HashMap::new(),
                        ) {
                            Ok(InternalCallOutcome::Value(Value::Bool(flag))) => flag,
                            Ok(InternalCallOutcome::Value(non_bool)) => {
                                sqlite_report_callback_exception(
                                    vm,
                                    &state.callable,
                                    RuntimeError::new(format!(
                                        "TypeError: __bool__ should return bool, returned {}",
                                        vm.value_type_name_for_error(&non_bool)
                                    )),
                                );
                                return -1;
                            }
                            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                                let err =
                                    vm.runtime_error_from_active_exception("__bool__() failed");
                                sqlite_report_callback_exception(vm, &state.callable, err);
                                return -1;
                            }
                            Err(err) => {
                                sqlite_report_callback_exception(vm, &state.callable, err);
                                return -1;
                            }
                        }
                    }
                    Ok(None) => is_truthy(&value),
                    Err(err) => {
                        sqlite_report_callback_exception(vm, &state.callable, err);
                        return -1;
                    }
                },
            };
            i32::from(truthy)
        }
        Ok(InternalCallOutcome::CallerExceptionHandled) => {
            let err = vm.runtime_error_from_active_exception("progress callback failed");
            sqlite_report_callback_exception(vm, &state.callable, err);
            -1
        }
        Err(err) => {
            sqlite_report_callback_exception(vm, &state.callable, err);
            -1
        }
    }
}

unsafe extern "C" fn sqlite_trace_callback(
    callback_type: c_uint,
    ctx: *mut c_void,
    stmt: *mut c_void,
    sql: *mut c_void,
) -> c_int {
    if callback_type != SQLITE_TRACE_STMT {
        return 0;
    }
    let Some(state) = sqlite_callback_state_from_ptr(ctx) else {
        return 0;
    };
    let vm_ptr = SQLITE_CALLBACK_VM.with(|slot| slot.get());
    if vm_ptr.is_null() {
        return 0;
    }
    // SAFETY: callback executes under SqliteCallbackVmGuard.
    let vm = unsafe { &mut *vm_ptr };

    let statement = if stmt.is_null() {
        sqlite_opt_cstr_to_value(sql as *const c_char)
    } else {
        // SAFETY: sqlite returns either an owned expanded SQL buffer (to free with sqlite3_free)
        // or null; statement/sql pointers are provided by sqlite trace callback contract.
        let expanded = unsafe { sqlite3_expanded_sql(stmt as *mut Sqlite3Stmt) };
        if expanded.is_null() {
            // SAFETY: stmt points to sqlite3_stmt in TRACE_STMT callbacks.
            let db = unsafe { sqlite3_db_handle(stmt as *mut Sqlite3Stmt) };
            if !db.is_null() {
                // SAFETY: db is valid for error-code inspection.
                let code = unsafe { sqlite3_extended_errcode(db) };
                if code == SQLITE_NOMEM {
                    sqlite_report_callback_exception(
                        vm,
                        &state.callable,
                        RuntimeError::new("MemoryError"),
                    );
                } else {
                    sqlite_report_callback_exception(
                        vm,
                        &state.callable,
                        RuntimeError::new(
                            "DataError: Expanded SQL string exceeds the maximum string length",
                        ),
                    );
                }
            } else {
                sqlite_report_callback_exception(
                    vm,
                    &state.callable,
                    RuntimeError::new(
                        "DataError: Expanded SQL string exceeds the maximum string length",
                    ),
                );
            }
            sqlite_opt_cstr_to_value(sql as *const c_char)
        } else {
            // SAFETY: expanded is a NUL-terminated UTF-8-ish buffer owned by sqlite.
            let text = unsafe { CStr::from_ptr(expanded) }
                .to_string_lossy()
                .into_owned();
            // SAFETY: expanded came from sqlite3_expanded_sql.
            unsafe { sqlite3_free(expanded as *mut c_void) };
            Value::Str(text)
        }
    };
    match vm.call_internal_preserving_caller(
        state.callable.clone(),
        vec![statement],
        HashMap::new(),
    ) {
        Ok(InternalCallOutcome::Value(_)) => 0,
        Ok(InternalCallOutcome::CallerExceptionHandled) => {
            let err = vm.runtime_error_from_active_exception("trace callback failed");
            sqlite_report_callback_exception(vm, &state.callable, err);
            0
        }
        Err(err) => {
            let err = if err.message.contains("argument count mismatch") {
                sqlite_callback_argument_mismatch_error(&state.callable, 1).unwrap_or(err)
            } else {
                err
            };
            sqlite_report_callback_exception(vm, &state.callable, err);
            0
        }
    }
}

unsafe extern "C" fn sqlite_collation_callback(
    ctx: *mut c_void,
    text1_len: c_int,
    text1_data: *const c_void,
    text2_len: c_int,
    text2_data: *const c_void,
) -> c_int {
    let Some(state) = sqlite_callback_state_from_ptr(ctx) else {
        return 0;
    };
    let vm_ptr = SQLITE_CALLBACK_VM.with(|slot| slot.get());
    if vm_ptr.is_null() {
        return 0;
    }
    // SAFETY: callback executes under SqliteCallbackVmGuard.
    let vm = unsafe { &mut *vm_ptr };

    let text1_len = usize::try_from(text1_len.max(0)).unwrap_or(0);
    let text2_len = usize::try_from(text2_len.max(0)).unwrap_or(0);
    let s1 = if text1_data.is_null() {
        String::new()
    } else {
        // SAFETY: sqlite passes valid pointer/length pairs.
        let bytes = unsafe { std::slice::from_raw_parts(text1_data as *const u8, text1_len) };
        String::from_utf8_lossy(bytes).into_owned()
    };
    let s2 = if text2_data.is_null() {
        String::new()
    } else {
        // SAFETY: sqlite passes valid pointer/length pairs.
        let bytes = unsafe { std::slice::from_raw_parts(text2_data as *const u8, text2_len) };
        String::from_utf8_lossy(bytes).into_owned()
    };
    let args = vec![Value::Str(s1), Value::Str(s2)];
    match vm.call_internal_preserving_caller(state.callable.clone(), args, HashMap::new()) {
        Ok(InternalCallOutcome::Value(result)) => match value_to_int(result) {
            Ok(n) if n > 0 => 1,
            Ok(n) if n < 0 => -1,
            Ok(_) => 0,
            Err(_) => 0,
        },
        Ok(InternalCallOutcome::CallerExceptionHandled) => {
            let err = vm.runtime_error_from_active_exception("collation callback failed");
            sqlite_report_callback_exception(vm, &state.callable, err);
            0
        }
        Err(err) => {
            sqlite_report_callback_exception(vm, &state.callable, err);
            0
        }
    }
}

fn sqlite_error_name_for_code(code: c_int) -> &'static str {
    match code {
        SQLITE_OK => "SQLITE_OK",
        SQLITE_ERROR => "SQLITE_ERROR",
        SQLITE_INTERNAL => "SQLITE_INTERNAL",
        SQLITE_PERM => "SQLITE_PERM",
        SQLITE_ABORT => "SQLITE_ABORT",
        SQLITE_BUSY => "SQLITE_BUSY",
        SQLITE_LOCKED => "SQLITE_LOCKED",
        SQLITE_NOMEM => "SQLITE_NOMEM",
        SQLITE_READONLY => "SQLITE_READONLY",
        SQLITE_INTERRUPT => "SQLITE_INTERRUPT",
        SQLITE_IOERR => "SQLITE_IOERR",
        SQLITE_CORRUPT => "SQLITE_CORRUPT",
        SQLITE_NOTFOUND => "SQLITE_NOTFOUND",
        SQLITE_FULL => "SQLITE_FULL",
        SQLITE_CANTOPEN => "SQLITE_CANTOPEN",
        SQLITE_PROTOCOL => "SQLITE_PROTOCOL",
        SQLITE_EMPTY => "SQLITE_EMPTY",
        SQLITE_SCHEMA => "SQLITE_SCHEMA",
        SQLITE_TOOBIG => "SQLITE_TOOBIG",
        SQLITE_CONSTRAINT => "SQLITE_CONSTRAINT",
        SQLITE_MISMATCH => "SQLITE_MISMATCH",
        SQLITE_MISUSE => "SQLITE_MISUSE",
        SQLITE_RANGE => "SQLITE_RANGE",
        275 => "SQLITE_CONSTRAINT_CHECK",
        531 => "SQLITE_CONSTRAINT_COMMITHOOK",
        787 => "SQLITE_CONSTRAINT_FOREIGNKEY",
        1043 => "SQLITE_CONSTRAINT_FUNCTION",
        1299 => "SQLITE_CONSTRAINT_NOTNULL",
        1555 => "SQLITE_CONSTRAINT_PRIMARYKEY",
        1811 => "SQLITE_CONSTRAINT_TRIGGER",
        2067 => "SQLITE_CONSTRAINT_UNIQUE",
        2323 => "SQLITE_CONSTRAINT_VTAB",
        2579 => "SQLITE_CONSTRAINT_ROWID",
        526 => "SQLITE_CANTOPEN_ISDIR",
        270 => "SQLITE_CANTOPEN_NOTEMPDIR",
        782 => "SQLITE_CANTOPEN_FULLPATH",
        1038 => "SQLITE_CANTOPEN_CONVPATH",
        1294 => "SQLITE_CANTOPEN_DIRTYWAL",
        1550 => "SQLITE_CANTOPEN_SYMLINK",
        _ => "SQLITE_ERROR",
    }
}

fn sqlite_error_with_code(kind: &str, message: impl Into<String>, code: c_int) -> RuntimeError {
    let message = message.into();
    let name = sqlite_error_name_for_code(code);
    RuntimeError::new(format!(
        "{kind}: {message}\n__pyrs_sqlite_meta__:{code}:{name}"
    ))
}

fn sqlite_error_kind_for_code(code: c_int, default_kind: &str) -> &'static str {
    match code {
        SQLITE_INTERNAL | SQLITE_NOTFOUND => "InternalError",
        SQLITE_ERROR | SQLITE_PERM | SQLITE_ABORT | SQLITE_BUSY | SQLITE_LOCKED | SQLITE_NOMEM
        | SQLITE_READONLY | SQLITE_INTERRUPT | SQLITE_IOERR | SQLITE_FULL | SQLITE_CANTOPEN
        | SQLITE_PROTOCOL | SQLITE_EMPTY | SQLITE_SCHEMA => "OperationalError",
        SQLITE_CORRUPT => "DatabaseError",
        SQLITE_TOOBIG => "DataError",
        SQLITE_CONSTRAINT | SQLITE_MISMATCH => "IntegrityError",
        SQLITE_MISUSE | SQLITE_RANGE => "InterfaceError",
        _ => match default_kind {
            "InternalError" => "InternalError",
            "InterfaceError" => "InterfaceError",
            "DataError" => "DataError",
            "DatabaseError" => "DatabaseError",
            "OperationalError" => "OperationalError",
            "IntegrityError" => "IntegrityError",
            "ProgrammingError" => "ProgrammingError",
            "NotSupportedError" => "NotSupportedError",
            _ => "DatabaseError",
        },
    }
}

fn sqlite_error_from_db_status(db: *mut Sqlite3Db, default_kind: &str) -> RuntimeError {
    // SAFETY: sqlite3_extended_errcode accepts a valid sqlite3* handle.
    let code = unsafe { sqlite3_extended_errcode(db) };
    let message = sqlite_last_error_message(db);
    let primary = code & 0xff;
    let kind = sqlite_error_kind_for_code(primary, default_kind);
    sqlite_error_with_code(kind, message, code)
}

fn sqlite_last_error_message(db: *mut Sqlite3Db) -> String {
    if db.is_null() {
        return "sqlite backend error".to_string();
    }
    // SAFETY: sqlite3_errmsg accepts a valid sqlite3* and returns a null-terminated string.
    unsafe {
        let ptr = sqlite3_errmsg(db);
        if ptr.is_null() {
            "sqlite backend error".to_string()
        } else {
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
}

fn sqlite_len_to_c_int(len: usize, context: &str) -> Result<c_int, RuntimeError> {
    i32::try_from(len)
        .map_err(|_| sqlite_error("OverflowError", format!("{context} length is too large")))
}

fn sqlite_has_extra_sql(tail: *const c_char) -> bool {
    if tail.is_null() {
        return false;
    }
    // SAFETY: tail points into the SQL text buffer passed to sqlite3_prepare_v2.
    let tail_bytes = unsafe { CStr::from_ptr(tail).to_bytes() };
    let tail_text = String::from_utf8_lossy(tail_bytes);
    sqlite_lstrip_sql(&tail_text).is_some()
}

fn sqlite_lstrip_sql(mut sql: &str) -> Option<&str> {
    while !sql.is_empty() {
        let bytes = sql.as_bytes();
        match bytes[0] {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0c => {
                sql = &sql[1..];
            }
            b'-' if bytes.len() >= 2 && bytes[1] == b'-' => {
                let Some(newline) = sql.find('\n') else {
                    return None;
                };
                sql = &sql[(newline + 1)..];
            }
            b'/' if bytes.len() >= 2 && bytes[1] == b'*' => {
                let Some(end_comment) = sql.find("*/") else {
                    return None;
                };
                sql = &sql[(end_comment + 2)..];
            }
            _ => return Some(sql),
        }
    }
    None
}

fn sqlite_is_dml_statement(sql: &str) -> bool {
    let Some(head) = sqlite_lstrip_sql(sql) else {
        return false;
    };
    (head.len() >= 6 && head[..6].eq_ignore_ascii_case("insert"))
        || (head.len() >= 6 && head[..6].eq_ignore_ascii_case("update"))
        || (head.len() >= 6 && head[..6].eq_ignore_ascii_case("delete"))
        || (head.len() >= 7 && head[..7].eq_ignore_ascii_case("replace"))
}

fn sqlite_normalize_isolation_level(level: Value) -> Result<Value, RuntimeError> {
    match level {
        Value::None => Ok(Value::None),
        Value::Str(text) => {
            let normalized = text.to_ascii_uppercase();
            match normalized.as_str() {
                "" | "DEFERRED" | "IMMEDIATE" | "EXCLUSIVE" => Ok(Value::Str(normalized)),
                _ => Err(sqlite_error(
                    "ValueError",
                    SQLITE_CONNECTION_ISOLATION_LEVEL_VALUE_ERROR,
                )),
            }
        }
        _ => Err(sqlite_error(
            "TypeError",
            "isolation_level must be str or None",
        )),
    }
}

fn sqlite_normalize_autocommit(value: Option<Value>) -> Result<SqliteAutocommitMode, RuntimeError> {
    let Some(value) = value else {
        return Ok(SqliteAutocommitMode::Legacy);
    };
    match value {
        Value::Bool(true) => Ok(SqliteAutocommitMode::Enabled),
        Value::Bool(false) => Ok(SqliteAutocommitMode::Disabled),
        Value::Int(number) if number == SQLITE_LEGACY_TRANSACTION_CONTROL => {
            Ok(SqliteAutocommitMode::Legacy)
        }
        Value::BigInt(number)
            if number.as_ref() == &BigInt::from_i64(SQLITE_LEGACY_TRANSACTION_CONTROL) =>
        {
            Ok(SqliteAutocommitMode::Legacy)
        }
        _ => Err(sqlite_error(
            "ValueError",
            "autocommit must be True, False, or sqlite3.LEGACY_TRANSACTION_CONTROL",
        )),
    }
}

fn sqlite_autocommit_mode_to_value(mode: SqliteAutocommitMode) -> Value {
    match mode {
        SqliteAutocommitMode::Legacy => Value::Int(SQLITE_LEGACY_TRANSACTION_CONTROL),
        SqliteAutocommitMode::Enabled => Value::Bool(true),
        SqliteAutocommitMode::Disabled => Value::Bool(false),
    }
}

fn sqlite_non_negative_u32(
    value: Value,
    type_message: &str,
    value_message: &str,
    overflow_message: &str,
) -> Result<i64, RuntimeError> {
    let number = match value_to_int(value) {
        Ok(number) => number,
        Err(err)
            if err.message.contains("integer overflow")
                || classify_runtime_error(&err.message) == "OverflowError" =>
        {
            return Err(sqlite_error("OverflowError", overflow_message));
        }
        Err(_) => return Err(sqlite_error("TypeError", type_message)),
    };
    if number < 0 {
        return Err(sqlite_error("ValueError", value_message));
    }
    if number > i64::from(u32::MAX) {
        return Err(sqlite_error("OverflowError", overflow_message));
    }
    Ok(number)
}

fn sqlite_connection_readonly_attr_error(name: &str) -> RuntimeError {
    sqlite_error(
        "AttributeError",
        format!("attribute '{name}' of 'sqlite3.Connection' objects is not writable"),
    )
}

fn sqlite_current_thread_ident() -> i64 {
    vm_current_thread_ident()
}

const SQLITE_CONNECT_POSITIONAL_DEPRECATION: &str = "Passing more than 1 positional argument to sqlite3.connect() is deprecated. \
Parameters 'timeout', 'detect_types', 'isolation_level', 'check_same_thread', \
'factory', 'cached_statements' and 'uri' will become keyword-only parameters in Python 3.15.";
const SQLITE_CONNECTION_POSITIONAL_DEPRECATION: &str = "Passing more than 1 positional argument to _sqlite3.Connection() is deprecated. \
Parameters 'timeout', 'detect_types', 'isolation_level', 'check_same_thread', \
'factory', 'cached_statements' and 'uri' will become keyword-only parameters in Python 3.15.";

impl Vm {
    pub(in crate::vm) fn sqlite_libversion_string(&self) -> String {
        // SAFETY: sqlite3_libversion returns a static null-terminated string.
        unsafe {
            let ptr = sqlite3_libversion();
            if ptr.is_null() {
                "0.0.0".to_string()
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        }
    }

    fn sqlite_module_global(&self, name: &str) -> Result<Value, RuntimeError> {
        let module = self
            .modules
            .get("_sqlite3")
            .ok_or_else(|| RuntimeError::new("module '_sqlite3' not found"))?;
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("invalid _sqlite3 module object"));
        };
        module_data
            .globals
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::new(format!("_sqlite3 missing '{name}'")))
    }

    fn sqlite_module_dict(&self, name: &str) -> Result<ObjRef, RuntimeError> {
        match self.sqlite_module_global(name)? {
            Value::Dict(dict) => Ok(dict),
            _ => Err(RuntimeError::new(format!("_sqlite3.{name} must be a dict"))),
        }
    }

    fn sqlite_connection_class(&self) -> Result<ObjRef, RuntimeError> {
        match self.sqlite_module_global("Connection")? {
            Value::Class(class_ref) => Ok(class_ref),
            _ => Err(RuntimeError::new("_sqlite3.Connection must be a class")),
        }
    }

    fn sqlite_cursor_class(&self) -> Result<ObjRef, RuntimeError> {
        match self.sqlite_module_global("Cursor")? {
            Value::Class(class_ref) => Ok(class_ref),
            _ => Err(RuntimeError::new("_sqlite3.Cursor must be a class")),
        }
    }

    fn sqlite_blob_class(&self) -> Result<ObjRef, RuntimeError> {
        match self.sqlite_module_global("Blob")? {
            Value::Class(class_ref) => Ok(class_ref),
            _ => Err(RuntimeError::new("_sqlite3.Blob must be a class")),
        }
    }

    fn sqlite_default_text_factory(&self) -> Value {
        self.builtins.get("str").cloned().unwrap_or(Value::None)
    }

    fn sqlite_value_is_connection_instance(&self, value: &Value) -> bool {
        let receiver = match self.receiver_from_value(value) {
            Ok(receiver) => receiver,
            Err(_) => return false,
        };
        let receiver_class = match &*receiver.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return false,
        };
        let connection_class = match self.sqlite_connection_class() {
            Ok(class_ref) => class_ref,
            Err(_) => return false,
        };
        if receiver_class.id() == connection_class.id() {
            return true;
        }
        self.class_mro_entries(&receiver_class)
            .iter()
            .any(|entry| entry.id() == connection_class.id())
    }

    fn sqlite_value_is_cursor_instance(&self, value: &Value) -> bool {
        let receiver = match self.receiver_from_value(value) {
            Ok(receiver) => receiver,
            Err(_) => return false,
        };
        let receiver_class = match &*receiver.kind() {
            Object::Instance(instance_data) => instance_data.class.clone(),
            _ => return false,
        };
        let cursor_class = match self.sqlite_cursor_class() {
            Ok(class_ref) => class_ref,
            Err(_) => return false,
        };
        if receiver_class.id() == cursor_class.id() {
            return true;
        }
        self.class_mro_entries(&receiver_class)
            .iter()
            .any(|entry| entry.id() == cursor_class.id())
    }

    fn sqlite_initialize_cursor_instance(
        &mut self,
        cursor: &ObjRef,
        connection: &Value,
        connection_id: u64,
    ) {
        if let Object::Instance(instance_data) = &mut *cursor.kind_mut() {
            instance_data
                .attrs
                .insert("rowcount".to_string(), Value::Int(-1));
            instance_data
                .attrs
                .insert("arraysize".to_string(), Value::Int(1));
            instance_data
                .attrs
                .insert("description".to_string(), Value::None);
            instance_data
                .attrs
                .insert("row_factory".to_string(), Value::None);
            instance_data
                .attrs
                .insert("lastrowid".to_string(), Value::None);
            instance_data
                .attrs
                .insert("connection".to_string(), connection.clone());
            instance_data.attrs.insert(
                SQLITE_CURSOR_BASE_INIT_CALLED_ATTR.to_string(),
                Value::Bool(true),
            );
        }
        self.sqlite_cursors
            .insert(cursor.id(), SqliteCursorState::new(connection_id));
    }

    fn sqlite_connection_id_from_value(
        &self,
        value: &Value,
        method_name: &str,
    ) -> Result<u64, RuntimeError> {
        let receiver = self.receiver_from_value(value)?;
        let receiver_id = receiver.id();
        let receiver_is_sqlite_connection = self.sqlite_value_is_connection_instance(value);
        if matches!(
            Self::instance_attr_get(&receiver, SQLITE_CONNECTION_BASE_INIT_CALLED_ATTR),
            Some(Value::Bool(false))
        ) {
            return Err(sqlite_error(
                "ProgrammingError",
                "Base Connection.__init__ not called.",
            ));
        }
        if self.sqlite_connections.contains_key(&receiver_id) {
            Ok(receiver_id)
        } else if receiver_is_sqlite_connection {
            Err(sqlite_error(
                "ProgrammingError",
                "Base Connection.__init__ not called.",
            ))
        } else {
            Err(sqlite_error(
                "ProgrammingError",
                format!("{method_name}() called on non-Connection object"),
            ))
        }
    }

    fn sqlite_cursor_id_from_value(
        &self,
        value: &Value,
        method_name: &str,
    ) -> Result<u64, RuntimeError> {
        let receiver = self.receiver_from_value(value)?;
        let receiver_id = receiver.id();
        let receiver_is_sqlite_cursor = self.sqlite_value_is_cursor_instance(value);
        if matches!(
            Self::instance_attr_get(&receiver, SQLITE_CURSOR_BASE_INIT_CALLED_ATTR),
            Some(Value::Bool(false))
        ) {
            return Err(sqlite_error(
                "ProgrammingError",
                "Base Cursor.__init__ not called.",
            ));
        }
        if self.sqlite_cursors.contains_key(&receiver_id) {
            Ok(receiver_id)
        } else if receiver_is_sqlite_cursor {
            Err(sqlite_error(
                "ProgrammingError",
                "Base Cursor.__init__ not called.",
            ))
        } else {
            Err(sqlite_error(
                "ProgrammingError",
                format!("{method_name}() called on non-Cursor object"),
            ))
        }
    }

    fn sqlite_blob_id_from_value(
        &self,
        value: &Value,
        method_name: &str,
    ) -> Result<u64, RuntimeError> {
        let receiver = self.receiver_from_value(value)?;
        let receiver_id = receiver.id();
        if self.sqlite_blobs.contains_key(&receiver_id) {
            Ok(receiver_id)
        } else {
            Err(sqlite_error(
                "ProgrammingError",
                format!("{method_name}() called on non-Blob object"),
            ))
        }
    }

    fn sqlite_open_db_handle(&self, connection_id: u64) -> Result<*mut Sqlite3Db, RuntimeError> {
        let state = self
            .sqlite_connections
            .get(&connection_id)
            .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite connection"))?;
        state.ensure_thread_affinity()?;
        state
            .db_handle()
            .ok_or_else(|| sqlite_error("ProgrammingError", "Cannot operate on a closed database."))
    }

    fn sqlite_cursor_closed_runtime_error(&self, connection_id: u64) -> RuntimeError {
        let is_connection_closed = self
            .sqlite_connections
            .get(&connection_id)
            .and_then(|state| state.db_handle())
            .is_none();
        if is_connection_closed {
            sqlite_error("ProgrammingError", "Cannot operate on a closed database.")
        } else {
            sqlite_error("ProgrammingError", "Cannot operate on a closed cursor.")
        }
    }

    fn sqlite_cursor_ensure_thread_affinity(&self, cursor_id: u64) -> Result<u64, RuntimeError> {
        let connection_id = self
            .sqlite_cursors
            .get(&cursor_id)
            .map(|state| state.connection_id)
            .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite cursor"))?;
        let connection_state = self
            .sqlite_connections
            .get(&connection_id)
            .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite connection"))?;
        connection_state.ensure_thread_affinity()?;
        Ok(connection_id)
    }

    fn sqlite_maybe_begin_legacy_transaction(
        &mut self,
        connection_id: u64,
        sql: &str,
    ) -> Result<(), RuntimeError> {
        if self.sqlite_connection_autocommit_mode(connection_id)? != SqliteAutocommitMode::Legacy {
            return Ok(());
        }
        if !sqlite_is_dml_statement(sql) {
            return Ok(());
        }
        let connection = self
            .heap
            .find_object_by_id(connection_id)
            .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite connection"))?;
        let isolation_level =
            Self::instance_attr_get(&connection, SQLITE_CONNECTION_ISOLATION_LEVEL_ATTR)
                .unwrap_or_else(|| Value::Str(String::new()));
        let Value::Str(isolation_level) = isolation_level else {
            return Ok(());
        };
        let db = self.sqlite_open_db_handle(connection_id)?;
        // SAFETY: db is a valid sqlite handle.
        if unsafe { sqlite3_get_autocommit(db) == 0 } {
            return Ok(());
        }

        let begin_sql = if isolation_level.is_empty() {
            "BEGIN ".to_string()
        } else {
            format!("BEGIN {isolation_level}")
        };
        let _ = self.sqlite_execute_query(
            connection_id,
            &begin_sql,
            SqliteParams::Positional(Vec::new()),
        )?;
        Ok(())
    }

    fn sqlite_connection_autocommit_mode(
        &self,
        connection_id: u64,
    ) -> Result<SqliteAutocommitMode, RuntimeError> {
        self.sqlite_connections
            .get(&connection_id)
            .map(|state| state.autocommit_mode)
            .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite connection"))
    }

    fn sqlite_set_connection_autocommit_mode(
        &mut self,
        connection_id: u64,
        mode: SqliteAutocommitMode,
    ) {
        if let Some(state) = self.sqlite_connections.get_mut(&connection_id) {
            state.autocommit_mode = mode;
        }
    }

    fn sqlite_emit_manual_trace_callback(
        &mut self,
        connection_id: u64,
        statement: &str,
    ) -> Result<(), RuntimeError> {
        let callback_state = self
            .sqlite_connections
            .get(&connection_id)
            .and_then(|state| state.trace_callback)
            .map(|ptr| {
                // SAFETY: pointer originates from Box<Arc<...>> callback registration.
                unsafe { (*ptr.as_ptr()).clone() }
            });
        let Some(callback_state) = callback_state else {
            return Ok(());
        };
        match self.call_internal_preserving_caller(
            callback_state.callable.clone(),
            vec![Value::Str(statement.to_string())],
            HashMap::new(),
        ) {
            Ok(InternalCallOutcome::Value(_)) => Ok(()),
            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                let err = self.runtime_error_from_active_exception("trace callback failed");
                sqlite_report_callback_exception(self, &callback_state.callable, err);
                Ok(())
            }
            Err(err) => {
                sqlite_report_callback_exception(self, &callback_state.callable, err);
                Ok(())
            }
        }
    }

    fn sqlite_transition_autocommit_mode(
        &mut self,
        connection_id: u64,
        new_mode: SqliteAutocommitMode,
    ) -> Result<(), RuntimeError> {
        let old_mode = self.sqlite_connection_autocommit_mode(connection_id)?;
        if old_mode == new_mode {
            return Ok(());
        }
        let db = self.sqlite_open_db_handle(connection_id)?;
        // SAFETY: db is a valid sqlite handle.
        let in_transaction = unsafe { sqlite3_get_autocommit(db) == 0 };
        match new_mode {
            SqliteAutocommitMode::Enabled => {
                if in_transaction {
                    let _ = self.sqlite_execute_query(
                        connection_id,
                        "COMMIT",
                        SqliteParams::Positional(Vec::new()),
                    )?;
                }
            }
            SqliteAutocommitMode::Disabled => {
                if !in_transaction {
                    let _ = self.sqlite_execute_query(
                        connection_id,
                        "BEGIN",
                        SqliteParams::Positional(Vec::new()),
                    )?;
                }
            }
            SqliteAutocommitMode::Legacy => {}
        }
        self.sqlite_set_connection_autocommit_mode(connection_id, new_mode);
        Ok(())
    }

    fn sqlite_limit_category(value: Value) -> Result<c_int, RuntimeError> {
        let category = value_to_int(value)
            .map_err(|_| sqlite_error("TypeError", "'category' must be an integer"))?;
        if !(0..=SQLITE_LIMIT_MAX_CATEGORY).contains(&category) {
            return Err(sqlite_error(
                "ProgrammingError",
                "'category' is out of bounds",
            ));
        }
        i32::try_from(category)
            .map_err(|_| sqlite_error("ProgrammingError", "'category' is out of bounds"))
    }

    fn sqlite_dbconfig_operation(value: Value) -> Result<c_int, RuntimeError> {
        let operation = value_to_int(value)
            .map_err(|_| sqlite_error("TypeError", "'op' must be an integer"))?;
        if !SQLITE_DBCONFIG_KNOWN_OPS.contains(&operation) {
            return Err(sqlite_error("ValueError", "unknown config operation"));
        }
        i32::try_from(operation).map_err(|_| sqlite_error("ValueError", "unknown config operation"))
    }

    fn sqlite_blob_state_and_db(
        &mut self,
        blob_id: u64,
    ) -> Result<(&mut SqliteBlobState, *mut Sqlite3Db), RuntimeError> {
        let connection_id = self
            .sqlite_blobs
            .get(&blob_id)
            .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite blob"))?
            .connection_id;
        let db = self.sqlite_open_db_handle(connection_id)?;
        let state = self
            .sqlite_blobs
            .get_mut(&blob_id)
            .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite blob"))?;
        Ok((state, db))
    }

    fn sqlite_blob_len(blob: *mut Sqlite3Blob) -> Result<usize, RuntimeError> {
        // SAFETY: blob is an open sqlite blob handle.
        let len = unsafe { sqlite3_blob_bytes(blob) };
        if len < 0 {
            return Err(sqlite_error(
                "OperationalError",
                "sqlite3_blob_bytes returned a negative length",
            ));
        }
        usize::try_from(len).map_err(|_| sqlite_error("OverflowError", "blob length is too large"))
    }

    fn sqlite_blob_adjust_index(len: usize, index: i64) -> Option<usize> {
        if index >= 0 {
            let idx = usize::try_from(index).ok()?;
            (idx < len).then_some(idx)
        } else {
            let abs = usize::try_from(index.unsigned_abs()).ok()?;
            if abs > len { None } else { Some(len - abs) }
        }
    }

    fn sqlite_blob_error(db: *mut Sqlite3Db, rc: c_int) -> RuntimeError {
        let mut message = sqlite_last_error_message(db);
        if message.is_empty() {
            message = format!("sqlite3 blob operation failed with code {rc}");
        }
        sqlite_error("OperationalError", message)
    }

    fn sqlite_warn_connect_positional_deprecation(
        &mut self,
        from_connection_factory: bool,
    ) -> Result<(), RuntimeError> {
        let message = if from_connection_factory {
            SQLITE_CONNECTION_POSITIONAL_DEPRECATION
        } else {
            SQLITE_CONNECT_POSITIONAL_DEPRECATION
        };
        let stacklevel = if from_connection_factory { 2 } else { 1 };
        let _ = self.builtin_warnings_warn(
            vec![
                Value::Str(message.to_string()),
                Value::ExceptionType("DeprecationWarning".to_string()),
                Value::Int(stacklevel),
            ],
            HashMap::new(),
        )?;
        Ok(())
    }

    fn sqlite_warn_keyword_deprecation(
        &mut self,
        method_name: &str,
        parameter_name: &str,
    ) -> Result<(), RuntimeError> {
        let message = format!(
            "Passing keyword argument '{parameter_name}' to _sqlite3.Connection.{method_name}() is deprecated. \
Parameter '{parameter_name}' will become positional-only in Python 3.15."
        );
        let _ = self.builtin_warnings_warn(
            vec![
                Value::Str(message),
                Value::ExceptionType("DeprecationWarning".to_string()),
                Value::Int(1),
            ],
            HashMap::new(),
        )?;
        Ok(())
    }

    fn sqlite_blob_index_arg(&mut self, value: Value) -> Result<i64, RuntimeError> {
        self.io_index_arg_to_int(value).map_err(|err| {
            if err.message.contains("integer overflow") {
                sqlite_error("IndexError", "cannot fit 'int' into an index-sized integer")
            } else if err.message.contains("unsupported operand type")
                || err.message.contains("cannot be interpreted as an integer")
            {
                sqlite_error("TypeError", "Blob indices must be integers")
            } else {
                err
            }
        })
    }

    fn sqlite_extract_database(&mut self, value: Value) -> Result<Vec<u8>, RuntimeError> {
        let normalized = match value {
            Value::Str(_) | Value::Bytes(_) | Value::ByteArray(_) => value,
            candidate => self.builtin_os_fspath(vec![candidate], HashMap::new())?,
        };
        match normalized {
            Value::Str(text) => Ok(text.into_bytes()),
            Value::Bytes(obj) | Value::ByteArray(obj) => match &*obj.kind() {
                Object::Bytes(bytes) | Object::ByteArray(bytes) => Ok(bytes.clone()),
                _ => Err(sqlite_error(
                    "TypeError",
                    "database argument must be str or bytes-like",
                )),
            },
            _ => Err(sqlite_error(
                "TypeError",
                "database argument must be str or bytes-like",
            )),
        }
    }

    fn sqlite_extract_params(&mut self, value: Value) -> Result<SqliteParams, RuntimeError> {
        match value {
            Value::None => Ok(SqliteParams::Positional(Vec::new())),
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(items) => Ok(SqliteParams::Positional(items.clone())),
                _ => Err(sqlite_error(
                    "ProgrammingError",
                    "parameters are of unsupported type",
                )),
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(items) => Ok(SqliteParams::Positional(items.clone())),
                _ => Err(sqlite_error(
                    "ProgrammingError",
                    "parameters are of unsupported type",
                )),
            },
            Value::Dict(obj) => Ok(SqliteParams::Named(Value::Dict(obj))),
            candidate => {
                if matches!(
                    candidate,
                    Value::Bool(_)
                        | Value::Int(_)
                        | Value::BigInt(_)
                        | Value::Float(_)
                        | Value::Str(_)
                        | Value::Bytes(_)
                        | Value::ByteArray(_)
                ) {
                    Err(sqlite_error(
                        "ProgrammingError",
                        "parameters are of unsupported type",
                    ))
                } else {
                    if let Value::Instance(instance) = &candidate
                        && self.instance_backing_dict(instance).is_some() {
                            return Ok(SqliteParams::Named(candidate));
                        }
                    let has_keys = matches!(
                        self.builtin_hasattr(
                            vec![candidate.clone(), Value::Str("keys".to_string())],
                            HashMap::new(),
                        )?,
                        Value::Bool(true)
                    );
                    if has_keys {
                        return Ok(SqliteParams::Named(candidate));
                    }
                    let length = match self.builtin_len(vec![candidate.clone()], HashMap::new()) {
                        Ok(value) => value_to_int(value).map_err(|_| {
                            sqlite_error("ProgrammingError", "parameters are of unsupported type")
                        })?,
                        Err(err) if classify_runtime_error(&err.message) == "TypeError" => {
                            return Err(sqlite_error(
                                "ProgrammingError",
                                "parameters are of unsupported type",
                            ));
                        }
                        Err(err) => return Err(err),
                    };
                    if length < 0 {
                        return Err(sqlite_error(
                            "ProgrammingError",
                            "parameters are of unsupported type",
                        ));
                    }
                    let mut values = Vec::with_capacity(length as usize);
                    for idx in 0..length {
                        values.push(
                            self.getitem_value(candidate.clone(), Value::Int(idx))
                                .map_err(|_| {
                                    sqlite_error(
                                        "ProgrammingError",
                                        "parameters are of unsupported type",
                                    )
                                })?,
                        );
                    }
                    Ok(SqliteParams::Positional(values))
                }
            }
        }
    }

    fn sqlite_bind_value(
        &self,
        db: *mut Sqlite3Db,
        stmt: *mut Sqlite3Stmt,
        index: usize,
        value: &Value,
        text_buffers: &mut Vec<Vec<u8>>,
        blob_buffers: &mut Vec<Vec<u8>>,
    ) -> Result<(), RuntimeError> {
        let idx = i32::try_from(index + 1)
            .map_err(|_| sqlite_error("ProgrammingError", "too many SQL parameters"))?;
        let rc = match value {
            Value::None => {
                // SAFETY: stmt is a valid prepared statement and idx is in range.
                unsafe { sqlite3_bind_null(stmt, idx) }
            }
            Value::Bool(flag) => {
                // SAFETY: stmt is a valid prepared statement and idx is in range.
                unsafe { sqlite3_bind_int64(stmt, idx, if *flag { 1 } else { 0 }) }
            }
            Value::Int(number) => {
                // SAFETY: stmt is a valid prepared statement and idx is in range.
                unsafe { sqlite3_bind_int64(stmt, idx, *number) }
            }
            Value::BigInt(number) => {
                if let Some(int_value) = number.to_i64() {
                    // SAFETY: stmt is a valid prepared statement and idx is in range.
                    unsafe { sqlite3_bind_int64(stmt, idx, int_value) }
                } else {
                    let bytes = number.to_string().into_bytes();
                    let len = sqlite_len_to_c_int(bytes.len(), "text parameter")?;
                    let ptr = bytes.as_ptr() as *const c_char;
                    text_buffers.push(bytes);
                    // SAFETY: pointer remains valid because text_buffers owns it until execute ends.
                    unsafe { sqlite3_bind_text(stmt, idx, ptr, len, None) }
                }
            }
            Value::Float(number) => {
                // SAFETY: stmt is a valid prepared statement and idx is in range.
                unsafe { sqlite3_bind_double(stmt, idx, *number) }
            }
            Value::Str(text) => {
                let bytes = text.as_bytes().to_vec();
                let len = sqlite_len_to_c_int(bytes.len(), "text parameter")?;
                let ptr = bytes.as_ptr() as *const c_char;
                text_buffers.push(bytes);
                // SAFETY: pointer remains valid because text_buffers owns it until execute ends.
                unsafe { sqlite3_bind_text(stmt, idx, ptr, len, None) }
            }
            Value::Bytes(_) | Value::ByteArray(_) => {
                let bytes = bytes_like_from_value(value.clone()).map_err(|_| {
                    sqlite_error("ProgrammingError", "parameters are of unsupported type")
                })?;
                let len = sqlite_len_to_c_int(bytes.len(), "blob parameter")?;
                let ptr = bytes.as_ptr() as *const c_void;
                blob_buffers.push(bytes);
                // SAFETY: pointer remains valid because blob_buffers owns it until execute ends.
                unsafe { sqlite3_bind_blob(stmt, idx, ptr, len, None) }
            }
            _ => {
                return Err(sqlite_error(
                    "ProgrammingError",
                    "parameters are of unsupported type",
                ));
            }
        };
        if rc == SQLITE_OK {
            Ok(())
        } else {
            Err(sqlite_error_from_db_status(db, "OperationalError"))
        }
    }

    fn sqlite_collect_row(
        &mut self,
        stmt: *mut Sqlite3Stmt,
        column_count: i32,
    ) -> Result<Value, RuntimeError> {
        let mut row = Vec::with_capacity(column_count as usize);
        for col in 0..column_count {
            // SAFETY: stmt is valid and column index is in range [0, column_count).
            let value = unsafe {
                match sqlite3_column_type(stmt, col) {
                    SQLITE_INTEGER => Value::Int(sqlite3_column_int64(stmt, col)),
                    SQLITE_FLOAT => Value::Float(sqlite3_column_double(stmt, col)),
                    SQLITE_TEXT => {
                        let text_ptr = sqlite3_column_text(stmt, col);
                        let len = sqlite3_column_bytes(stmt, col);
                        if text_ptr.is_null() || len <= 0 {
                            self.heap.alloc_bytearray(Vec::new())
                        } else {
                            let slice = std::slice::from_raw_parts(
                                text_ptr,
                                usize::try_from(len).unwrap_or(0),
                            );
                            self.heap.alloc_bytearray(slice.to_vec())
                        }
                    }
                    SQLITE_BLOB => {
                        let blob_ptr = sqlite3_column_blob(stmt, col);
                        let len = sqlite3_column_bytes(stmt, col);
                        if blob_ptr.is_null() || len <= 0 {
                            self.heap.alloc_bytes(Vec::new())
                        } else {
                            let slice = std::slice::from_raw_parts(
                                blob_ptr as *const u8,
                                usize::try_from(len).unwrap_or(0),
                            );
                            self.heap.alloc_bytes(slice.to_vec())
                        }
                    }
                    _ => Value::None,
                }
            };
            row.push(value);
        }
        Ok(self.heap.alloc_tuple(row))
    }

    fn sqlite_collect_description(
        &mut self,
        stmt: *mut Sqlite3Stmt,
        column_count: i32,
    ) -> Option<Value> {
        if column_count <= 0 {
            return None;
        }
        let mut description = Vec::with_capacity(column_count as usize);
        for col in 0..column_count {
            // SAFETY: stmt is valid and col is in bounds.
            let name = unsafe {
                let name_ptr = sqlite3_column_name(stmt, col);
                if name_ptr.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(name_ptr).to_string_lossy().into_owned()
                }
            };
            description.push(self.heap.alloc_tuple(vec![
                Value::Str(name),
                Value::None,
                Value::None,
                Value::None,
                Value::None,
                Value::None,
                Value::None,
            ]));
        }
        Some(self.heap.alloc_tuple(description))
    }

    fn sqlite_text_factory_is_str(value: &Value) -> bool {
        match value {
            Value::Builtin(BuiltinFunction::Str) => true,
            Value::Class(class_ref) => match &*class_ref.kind() {
                Object::Class(class_data) => class_data.name == "str",
                _ => false,
            },
            _ => false,
        }
    }

    fn sqlite_apply_text_factory(
        &mut self,
        connection: &ObjRef,
        value: Value,
    ) -> Result<Value, RuntimeError> {
        let Value::ByteArray(raw_text) = value else {
            return Ok(value);
        };
        let bytes = match &*raw_text.kind() {
            Object::ByteArray(data) => data.clone(),
            _ => Vec::new(),
        };
        let text_factory = Self::instance_attr_get(connection, "text_factory");
        match text_factory {
            None | Some(Value::None) => {
                Ok(Value::Str(String::from_utf8_lossy(&bytes).into_owned()))
            }
            Some(factory) if Self::sqlite_text_factory_is_str(&factory) => {
                Ok(Value::Str(String::from_utf8_lossy(&bytes).into_owned()))
            }
            Some(factory) => {
                let payload = self.heap.alloc_bytes(bytes);
                match self.call_internal(factory, vec![payload], HashMap::new())? {
                    InternalCallOutcome::Value(value) => Ok(value),
                    InternalCallOutcome::CallerExceptionHandled => Err(self
                        .runtime_error_from_active_exception(
                            "sqlite text_factory() raised an exception",
                        )),
                }
            }
        }
    }

    fn sqlite_materialize_row_for_cursor(
        &mut self,
        cursor_value: &Value,
        raw_row: Value,
    ) -> Result<Value, RuntimeError> {
        let cursor_obj = self.receiver_from_value(cursor_value)?;
        let cursor_id = cursor_obj.id();
        let connection_id = self
            .sqlite_cursors
            .get(&cursor_id)
            .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite cursor"))?
            .connection_id;
        let connection_obj = self
            .heap
            .find_object_by_id(connection_id)
            .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite connection"))?;

        let row_tuple = match raw_row {
            Value::Tuple(tuple_obj) => {
                let raw_items = match &*tuple_obj.kind() {
                    Object::Tuple(items) => items.clone(),
                    _ => Vec::new(),
                };
                let mut converted = Vec::with_capacity(raw_items.len());
                for item in raw_items {
                    converted.push(self.sqlite_apply_text_factory(&connection_obj, item)?);
                }
                self.heap.alloc_tuple(converted)
            }
            other => other,
        };

        let row_factory =
            Self::instance_attr_get(&cursor_obj, "row_factory").unwrap_or(Value::None);
        if matches!(row_factory, Value::None) {
            Ok(row_tuple)
        } else {
            match self.call_internal(
                row_factory,
                vec![cursor_value.clone(), row_tuple],
                HashMap::new(),
            )? {
                InternalCallOutcome::Value(value) => Ok(value),
                InternalCallOutcome::CallerExceptionHandled => Err(self
                    .runtime_error_from_active_exception(
                        "sqlite row_factory() raised an exception",
                    )),
            }
        }
    }

    fn sqlite_execute_query(
        &mut self,
        connection_id: u64,
        sql: &str,
        params: SqliteParams,
    ) -> Result<SqliteQueryResult, RuntimeError> {
        let _callback_vm_guard = SqliteCallbackVmGuard::enter(self);
        let db = self.sqlite_open_db_handle(connection_id)?;
        // Match CPython statement.c preflight: SQL length over the sqlite limit
        // is surfaced as DataError with a stable message.
        // SAFETY: db is valid and the category id is a valid sqlite constant.
        let max_sql_length = unsafe { sqlite3_limit(db, SQLITE_LIMIT_SQL_LENGTH_ID, -1) };
        if max_sql_length >= 0 && sql.len() > max_sql_length as usize {
            return Err(sqlite_error("DataError", "query string is too large"));
        }
        let sql_c = CString::new(sql.as_bytes())
            .map_err(|_| sqlite_error("ProgrammingError", "SQL contains embedded NUL"))?;
        let mut raw_stmt: *mut Sqlite3Stmt = ptr::null_mut();
        let mut tail: *const c_char = ptr::null();
        // SAFETY: db is a live sqlite handle, sql_c is a valid C string, and output pointers are valid.
        let prepare_rc = unsafe {
            sqlite3_prepare_v2(
                db,
                sql_c.as_ptr(),
                -1,
                &mut raw_stmt as *mut *mut Sqlite3Stmt,
                &mut tail as *mut *const c_char,
            )
        };
        if prepare_rc != SQLITE_OK {
            return Err(sqlite_error_from_db_status(db, "OperationalError"));
        }
        let Some(stmt_ptr) = NonNull::new(raw_stmt) else {
            return Ok(SqliteQueryResult {
                rows: Vec::new(),
                description: None,
            });
        };
        if sqlite_has_extra_sql(tail) {
            return Err(sqlite_error(
                "ProgrammingError",
                "You can only execute one statement at a time.",
            ));
        }
        let statement = PreparedStatement { raw: stmt_ptr };
        // SAFETY: statement pointer is valid while statement wrapper is alive.
        let expected_params = unsafe { sqlite3_bind_parameter_count(statement.as_ptr()) };

        let mut text_buffers = Vec::new();
        let mut blob_buffers = Vec::new();
        match params {
            SqliteParams::Positional(values) => {
                for idx in 1..=expected_params {
                    // SAFETY: statement pointer is valid while statement wrapper is alive.
                    let raw_name = unsafe { sqlite3_bind_parameter_name(statement.as_ptr(), idx) };
                    if raw_name.is_null() {
                        continue;
                    }
                    // SAFETY: sqlite returns a valid null-terminated parameter name for this index.
                    let raw_name = unsafe { CStr::from_ptr(raw_name) }.to_string_lossy();
                    if !raw_name.starts_with('?') {
                        return Err(sqlite_error(
                            "ProgrammingError",
                            format!(
                                "Binding {idx} ('{raw_name}') is a named parameter, but you supplied a sequence."
                            ),
                        ));
                    }
                }
                if expected_params != values.len() as i32 {
                    return Err(sqlite_error(
                        "ProgrammingError",
                        format!(
                            "Incorrect number of bindings supplied. The current statement uses {expected_params}, and there are {} supplied.",
                            values.len()
                        ),
                    ));
                }
                for (index, value) in values.iter().enumerate() {
                    self.sqlite_bind_value(
                        db,
                        statement.as_ptr(),
                        index,
                        value,
                        &mut text_buffers,
                        &mut blob_buffers,
                    )?;
                }
            }
            SqliteParams::Named(mapping) => {
                for idx in 1..=expected_params {
                    // SAFETY: statement pointer is valid while statement wrapper is alive.
                    let raw_name = unsafe { sqlite3_bind_parameter_name(statement.as_ptr(), idx) };
                    if raw_name.is_null() {
                        return Err(sqlite_error(
                            "ProgrammingError",
                            format!(
                                "Binding {idx} has no name, but you supplied a dictionary (which has only names)."
                            ),
                        ));
                    }
                    // SAFETY: sqlite returns a valid null-terminated parameter name for this index.
                    let raw_name = unsafe { CStr::from_ptr(raw_name) }.to_string_lossy();
                    let key = raw_name
                        .strip_prefix(':')
                        .or_else(|| raw_name.strip_prefix('@'))
                        .or_else(|| raw_name.strip_prefix('$'))
                        .unwrap_or(raw_name.as_ref());
                    let value = if let Value::Dict(dict_obj) = &mapping {
                        dict_get_value(dict_obj, &Value::Str(key.to_string()))
                    } else if let Some(getitem) =
                        self.lookup_bound_special_method(&mapping, "__getitem__")?
                    {
                        match self.call_internal(
                            getitem,
                            vec![Value::Str(key.to_string())],
                            HashMap::new(),
                        ) {
                            Ok(InternalCallOutcome::Value(value)) => Some(value),
                            Ok(InternalCallOutcome::CallerExceptionHandled) => {
                                if self.active_exception_is("KeyError") {
                                    self.clear_active_exception();
                                    None
                                } else {
                                    return Err(self.runtime_error_from_active_exception(
                                        "__getitem__() failed",
                                    ));
                                }
                            }
                            Err(err) if classify_runtime_error(&err.message) == "KeyError" => {
                                None
                            }
                            Err(err) => return Err(err),
                        }
                    } else if let Value::Instance(instance) = &mapping {
                        if let Some(backing_dict) = self.instance_backing_dict(instance) {
                            if let Some(value) =
                                dict_get_value(&backing_dict, &Value::Str(key.to_string()))
                            {
                                Some(value)
                            } else if let Some(missing) =
                                self.lookup_bound_special_method(&mapping, "__missing__")?
                            {
                                match self.call_internal(
                                    missing,
                                    vec![Value::Str(key.to_string())],
                                    HashMap::new(),
                                ) {
                                    Ok(InternalCallOutcome::Value(value)) => Some(value),
                                    Ok(InternalCallOutcome::CallerExceptionHandled) => {
                                        if self.active_exception_is("KeyError") {
                                            self.clear_active_exception();
                                            None
                                        } else {
                                            return Err(self
                                                .runtime_error_from_active_exception(
                                                    "__missing__() failed",
                                                ));
                                        }
                                    }
                                    Err(err)
                                        if classify_runtime_error(&err.message)
                                            == "KeyError" =>
                                    {
                                        None
                                    }
                                    Err(err) => return Err(err),
                                }
                            } else {
                                None
                            }
                        } else {
                            return Err(sqlite_error(
                                "ProgrammingError",
                                "parameters are of unsupported type",
                            ));
                        }
                    } else {
                        return Err(sqlite_error(
                            "ProgrammingError",
                            "parameters are of unsupported type",
                        ));
                    };
                    let Some(value) = value else {
                        return Err(sqlite_error(
                            "ProgrammingError",
                            format!("You did not supply a value for binding parameter {raw_name}."),
                        ));
                    };
                    self.sqlite_bind_value(
                        db,
                        statement.as_ptr(),
                        usize::try_from(idx - 1).expect("sqlite bind index should be non-negative"),
                        &value,
                        &mut text_buffers,
                        &mut blob_buffers,
                    )?;
                }
            }
        }

        // SAFETY: statement pointer is valid while statement wrapper is alive.
        let column_count = unsafe { sqlite3_column_count(statement.as_ptr()) };
        let description = self.sqlite_collect_description(statement.as_ptr(), column_count);
        let mut rows = Vec::new();
        loop {
            // SAFETY: statement pointer is valid while statement wrapper is alive.
            let step_rc = unsafe { sqlite3_step(statement.as_ptr()) };
            match step_rc {
                SQLITE_ROW => {
                    rows.push(self.sqlite_collect_row(statement.as_ptr(), column_count)?);
                }
                SQLITE_DONE => break,
                _ => {
                    return Err(sqlite_error_from_db_status(db, "OperationalError"));
                }
            }
        }
        Ok(SqliteQueryResult { rows, description })
    }

    fn sqlite_execute_script(
        &mut self,
        connection_id: u64,
        script: &str,
    ) -> Result<(), RuntimeError> {
        let db = self.sqlite_open_db_handle(connection_id)?;
        // Match CPython statement.c preflight for oversized script payloads.
        // SAFETY: db is valid and the category id is a valid sqlite constant.
        let max_sql_length = unsafe { sqlite3_limit(db, SQLITE_LIMIT_SQL_LENGTH_ID, -1) };
        if max_sql_length >= 0 && script.len() > max_sql_length as usize {
            return Err(sqlite_error("DataError", "query string is too large"));
        }
        if script.chars().any(|ch| ch == '\u{fffd}') {
            return Err(sqlite_error("UnicodeEncodeError", "surrogates not allowed"));
        }
        // CPython executescript() commits an active transaction in legacy
        // transaction mode before running the script payload.
        // SAFETY: db is valid for autocommit checks.
        if self.sqlite_connection_autocommit_mode(connection_id)? == SqliteAutocommitMode::Legacy
            && unsafe { sqlite3_get_autocommit(db) == 0 }
        {
            self.sqlite_emit_manual_trace_callback(connection_id, "COMMIT")?;
            let commit_sql = CString::new("COMMIT").expect("static SQL should be valid C string");
            let mut err_out: *mut c_char = ptr::null_mut();
            // SAFETY: db is live and commit_sql is a valid C string.
            let commit_rc = unsafe {
                sqlite3_exec(db, commit_sql.as_ptr(), None, ptr::null_mut(), &mut err_out)
            };
            if commit_rc != SQLITE_OK {
                return Err(sqlite_error_from_db_status(db, "OperationalError"));
            }
        }
        let sql_c = CString::new(script.as_bytes())
            .map_err(|_| sqlite_error("ValueError", "embedded null character"))?;
        let mut err_out: *mut c_char = ptr::null_mut();
        let _callback_vm_guard = SqliteCallbackVmGuard::enter(self);
        // SAFETY: db is live, sql_c is valid, callback is null, and err_out is a valid out pointer.
        let rc = unsafe { sqlite3_exec(db, sql_c.as_ptr(), None, ptr::null_mut(), &mut err_out) };
        if rc == SQLITE_OK {
            return Ok(());
        }
        Err(sqlite_error_from_db_status(db, "OperationalError"))
    }

    pub(in crate::vm) fn builtin_sqlite_connect(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let positional_count = args.len();
        if args.len() > 8 {
            return Err(sqlite_error(
                "TypeError",
                format!(
                    "connect() takes at most 8 positional arguments ({} given)",
                    args.len()
                ),
            ));
        }
        let database = if args.is_empty() {
            kwargs.remove("database").ok_or_else(|| {
                sqlite_error(
                    "TypeError",
                    "connect() missing required argument 'database'",
                )
            })?
        } else {
            args.remove(0)
        };
        let optional_positional_names = [
            "timeout",
            "detect_types",
            "isolation_level",
            "check_same_thread",
            "factory",
            "cached_statements",
            "uri",
        ];
        for (name, value) in optional_positional_names.iter().zip(args.into_iter()) {
            if kwargs.contains_key(*name) {
                return Err(sqlite_error(
                    "TypeError",
                    format!("connect() got multiple values for argument '{name}'"),
                ));
            }
            kwargs.insert((*name).to_string(), value);
        }
        let timeout_arg = kwargs.remove("timeout");
        let detect_types_arg = kwargs.remove("detect_types");
        let isolation_level_arg = kwargs.remove("isolation_level");
        let isolation_level = sqlite_normalize_isolation_level(
            isolation_level_arg
                .clone()
                .unwrap_or_else(|| Value::Str(String::new())),
        )?;
        let check_same_thread_arg = kwargs.remove("check_same_thread");
        let check_same_thread = check_same_thread_arg
            .as_ref()
            .map(is_truthy)
            .unwrap_or(true);
        let factory = kwargs.remove("factory");
        let cached_statements_arg = kwargs.remove("cached_statements");
        let uri_arg = kwargs.remove("uri");
        let uri = uri_arg.as_ref().map(is_truthy).unwrap_or(false);
        let autocommit_arg = kwargs.remove("autocommit");
        let autocommit_mode = sqlite_normalize_autocommit(autocommit_arg.clone())?;
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(sqlite_error(
                "TypeError",
                format!("connect() got an unexpected keyword argument '{unexpected}'"),
            ));
        }

        let default_connection_class = self.sqlite_connection_class()?;
        if positional_count > 1 {
            let from_connection_factory = match factory.as_ref() {
                Some(Value::Class(class_ref)) => class_ref.id() != default_connection_class.id(),
                _ => false,
            };
            self.sqlite_warn_connect_positional_deprecation(from_connection_factory)?;
        }

        if let Some(factory_callable) = factory.clone() {
            if let Value::Class(class_ref) = &factory_callable {
                if class_ref.id() != default_connection_class.id() {
                    let mut factory_kwargs = HashMap::new();
                    if let Some(value) = timeout_arg {
                        factory_kwargs.insert("timeout".to_string(), value);
                    }
                    if let Some(value) = detect_types_arg {
                        factory_kwargs.insert("detect_types".to_string(), value);
                    }
                    if let Some(value) = isolation_level_arg {
                        factory_kwargs.insert("isolation_level".to_string(), value);
                    }
                    if let Some(value) = check_same_thread_arg {
                        factory_kwargs.insert("check_same_thread".to_string(), value);
                    }
                    if let Some(value) = cached_statements_arg {
                        factory_kwargs.insert("cached_statements".to_string(), value);
                    }
                    if let Some(value) = uri_arg {
                        factory_kwargs.insert("uri".to_string(), value);
                    }
                    if let Some(value) = autocommit_arg {
                        factory_kwargs.insert("autocommit".to_string(), value);
                    }
                    return match self.call_internal_preserving_caller(
                        factory_callable,
                        vec![database],
                        factory_kwargs,
                    )? {
                        InternalCallOutcome::Value(value) => Ok(value),
                        InternalCallOutcome::CallerExceptionHandled => Err(self
                            .runtime_error_from_active_exception(
                                "sqlite connect factory callable failed",
                            )),
                    };
                }
            } else {
                if !self.is_callable_value(&factory_callable) {
                    return Err(sqlite_error(
                        "TypeError",
                        "factory must be a callable or Connection subclass",
                    ));
                }
                return match self.call_internal_preserving_caller(
                    factory_callable,
                    vec![database],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(value) => Ok(value),
                    InternalCallOutcome::CallerExceptionHandled => Err(self
                        .runtime_error_from_active_exception(
                            "sqlite connect factory callable failed",
                        )),
                };
            }
        }

        let database = self.sqlite_extract_database(database)?;
        let db_path = CString::new(database)
            .map_err(|_| sqlite_error("ProgrammingError", "database path contains embedded NUL"))?;

        let mut handle: *mut Sqlite3Db = ptr::null_mut();
        let mut flags = SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE;
        if uri {
            flags |= SQLITE_OPEN_URI;
        }
        // SAFETY: db_path is a valid C string and handle out pointer is valid.
        let open_rc = unsafe {
            sqlite3_open_v2(
                db_path.as_ptr(),
                &mut handle as *mut *mut Sqlite3Db,
                flags,
                ptr::null(),
            )
        };
        if open_rc != SQLITE_OK {
            let message = sqlite_last_error_message(handle);
            if !handle.is_null() {
                // SAFETY: handle was returned by sqlite3_open_v2 and may be partially initialized.
                unsafe {
                    let _ = sqlite3_close_v2(handle);
                }
            }
            return Err(sqlite_error_with_code("OperationalError", message, open_rc));
        }

        let class = match factory {
            Some(Value::Class(class_ref)) => class_ref,
            Some(_) => unreachable!("non-class factory is handled before sqlite open"),
            None => default_connection_class,
        };
        let connection = self.alloc_instance_for_class(&class);
        if let Object::Instance(instance_data) = &mut *connection.kind_mut() {
            instance_data
                .attrs
                .insert("in_transaction".to_string(), Value::Bool(false));
            instance_data
                .attrs
                .insert("row_factory".to_string(), Value::None);
            instance_data.attrs.insert(
                "text_factory".to_string(),
                self.sqlite_default_text_factory(),
            );
            instance_data.attrs.insert(
                SQLITE_CONNECTION_ISOLATION_LEVEL_ATTR.to_string(),
                isolation_level,
            );
            instance_data.attrs.insert(
                SQLITE_CONNECTION_BASE_INIT_CALLED_ATTR.to_string(),
                Value::Bool(true),
            );
        }
        self.sqlite_connections.insert(
            connection.id(),
            SqliteConnectionState::new(handle, check_same_thread, autocommit_mode),
        );
        if autocommit_mode == SqliteAutocommitMode::Disabled {
            let _ = self.sqlite_execute_query(
                connection.id(),
                "BEGIN",
                SqliteParams::Positional(Vec::new()),
            )?;
        }
        Ok(Value::Instance(connection))
    }

    pub(in crate::vm) fn builtin_sqlite_connection_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(sqlite_error(
                "TypeError",
                "Connection.__init__() missing self",
            ));
        }
        if args.len() > 9 {
            return Err(sqlite_error(
                "TypeError",
                format!(
                    "Connection.__init__() takes at most 8 positional arguments ({} given)",
                    args.len() - 1
                ),
            ));
        }
        let receiver_value = args.remove(0);
        let receiver = self.receiver_from_value(&receiver_value)?;
        let receiver_id = receiver.id();

        let database = if args.is_empty() {
            kwargs.remove("database").ok_or_else(|| {
                sqlite_error("TypeError", "Connection.__init__() missing 'database'")
            })?
        } else {
            args.remove(0)
        };
        let optional_positional_names = [
            "timeout",
            "detect_types",
            "isolation_level",
            "check_same_thread",
            "factory",
            "cached_statements",
            "uri",
        ];
        for (name, value) in optional_positional_names.iter().zip(args.into_iter()) {
            if kwargs.contains_key(*name) {
                return Err(sqlite_error(
                    "TypeError",
                    format!("Connection.__init__() got multiple values for argument '{name}'"),
                ));
            }
            kwargs.insert((*name).to_string(), value);
        }
        let uri = kwargs
            .remove("uri")
            .map(|value| is_truthy(&value))
            .unwrap_or(false);
        let _ = kwargs.remove("timeout");
        let _ = kwargs.remove("detect_types");
        let isolation_level = sqlite_normalize_isolation_level(
            kwargs
                .remove("isolation_level")
                .unwrap_or_else(|| Value::Str(String::new())),
        )?;
        let check_same_thread = kwargs
            .remove("check_same_thread")
            .map(|value| is_truthy(&value))
            .unwrap_or(true);
        let _ = kwargs.remove("factory");
        let _ = kwargs.remove("cached_statements");
        let autocommit_mode = sqlite_normalize_autocommit(kwargs.remove("autocommit"))?;
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(sqlite_error(
                "TypeError",
                format!("Connection.__init__() got an unexpected keyword argument '{unexpected}'"),
            ));
        }

        if let Some(state) = self.sqlite_connections.get_mut(&receiver_id) {
            let _ = state.close();
        } else {
            self.sqlite_connections.insert(
                receiver_id,
                SqliteConnectionState::new(ptr::null_mut(), check_same_thread, autocommit_mode),
            );
        }

        let database = self.sqlite_extract_database(database)?;
        let db_path = CString::new(database)
            .map_err(|_| sqlite_error("ProgrammingError", "database path contains embedded NUL"))?;
        let mut handle: *mut Sqlite3Db = ptr::null_mut();
        let mut flags = SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE;
        if uri {
            flags |= SQLITE_OPEN_URI;
        }
        // SAFETY: db_path is a valid C string and handle out pointer is valid.
        let open_rc = unsafe {
            sqlite3_open_v2(
                db_path.as_ptr(),
                &mut handle as *mut *mut Sqlite3Db,
                flags,
                ptr::null(),
            )
        };
        if open_rc != SQLITE_OK {
            let message = sqlite_last_error_message(handle);
            if !handle.is_null() {
                // SAFETY: handle was returned by sqlite3_open_v2 and may be partially initialized.
                unsafe {
                    let _ = sqlite3_close_v2(handle);
                }
            }
            let _ = Self::instance_attr_set(
                &receiver,
                SQLITE_CONNECTION_BASE_INIT_CALLED_ATTR,
                Value::Bool(false),
            );
            return Err(sqlite_error_with_code("OperationalError", message, open_rc));
        }

        self.sqlite_connections.insert(
            receiver_id,
            SqliteConnectionState::new(handle, check_same_thread, autocommit_mode),
        );
        if autocommit_mode == SqliteAutocommitMode::Disabled {
            let _ = self.sqlite_execute_query(
                receiver_id,
                "BEGIN",
                SqliteParams::Positional(Vec::new()),
            )?;
        }
        let _ = Self::instance_attr_set(
            &receiver,
            SQLITE_CONNECTION_BASE_INIT_CALLED_ATTR,
            Value::Bool(true),
        );
        let _ = Self::instance_attr_set(
            &receiver,
            SQLITE_CONNECTION_ISOLATION_LEVEL_ATTR,
            isolation_level,
        );
        let _ = Self::instance_attr_set(&receiver, "row_factory", Value::None);
        let _ = Self::instance_attr_set(
            &receiver,
            "text_factory",
            self.sqlite_default_text_factory(),
        );
        let _ = Self::instance_attr_set(&receiver, "in_transaction", Value::Bool(false));
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_complete_statement(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "complete_statement() expects exactly one argument",
            ));
        }
        let statement = match args.remove(0) {
            Value::Str(text) => text,
            _ => {
                return Err(sqlite_error(
                    "TypeError",
                    "complete_statement() argument must be str",
                ));
            }
        };
        let statement_c = CString::new(statement.as_bytes())
            .map_err(|_| sqlite_error("ProgrammingError", "statement contains embedded NUL"))?;
        // SAFETY: statement_c is a valid C string.
        Ok(Value::Bool(unsafe {
            sqlite3_complete(statement_c.as_ptr()) != 0
        }))
    }

    pub(in crate::vm) fn builtin_sqlite_register_adapter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error(
                "TypeError",
                "register_adapter() expects type and callable",
            ));
        }
        let mut args = args;
        let adapter = args.pop().expect("adapter arg");
        let type_key = args.pop().expect("type arg");
        let adapters = self.sqlite_module_dict("adapters")?;
        dict_set_value_checked(&adapters, type_key, adapter)?;
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_register_converter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error(
                "TypeError",
                "register_converter() expects name and callable",
            ));
        }
        let mut args = args;
        let converter = args.pop().expect("converter arg");
        let name = match args.pop().expect("name arg") {
            Value::Str(text) => text,
            Value::Bytes(obj) | Value::ByteArray(obj) => match &*obj.kind() {
                Object::Bytes(bytes) | Object::ByteArray(bytes) => {
                    String::from_utf8_lossy(bytes).into_owned()
                }
                _ => {
                    return Err(sqlite_error(
                        "TypeError",
                        "register_converter() name must be str",
                    ));
                }
            },
            _ => {
                return Err(sqlite_error(
                    "TypeError",
                    "register_converter() name must be str",
                ));
            }
        };
        let converters = self.sqlite_module_dict("converters")?;
        dict_set_value_checked(
            &converters,
            Value::Str(name.to_ascii_uppercase()),
            converter,
        )?;
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_enable_callback_tracebacks(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "enable_callback_tracebacks() expects exactly one argument",
            ));
        }
        let enabled = self.truthy_from_value(&args.remove(0))?;
        self.sqlite_callback_tracebacks_enabled = enabled;
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_del(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.__del__() expects no arguments",
            ));
        }
        let receiver = self.receiver_from_value(&args[0])?;
        let receiver_id = receiver.id();
        let should_warn = self
            .sqlite_connections
            .get(&receiver_id)
            .and_then(|state| state.db_handle())
            .is_some();
        if should_warn {
            let _ = self.builtin_warnings_warn(
                vec![
                    Value::Str("unclosed sqlite3.Connection".to_string()),
                    Value::ExceptionType("ResourceWarning".to_string()),
                    Value::Int(1),
                ],
                HashMap::new(),
            );
            if let Some(state) = self.sqlite_connections.get_mut(&receiver_id) {
                let _ = state.close();
            }
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_cursor(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(sqlite_error(
                "TypeError",
                "Connection.cursor() missing self",
            ));
        }
        if args.len() > 2 {
            return Err(sqlite_error(
                "TypeError",
                format!(
                    "cursor() takes at most 1 argument ({} given)",
                    args.len() - 1
                ),
            ));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "cursor")?;
        let _ = self.sqlite_open_db_handle(connection_id)?;
        let factory = if args.is_empty() {
            kwargs
                .remove("factory")
                .unwrap_or(Value::Class(self.sqlite_cursor_class()?))
        } else if kwargs.contains_key("factory") {
            return Err(sqlite_error(
                "TypeError",
                "cursor() got multiple values for argument 'factory'",
            ));
        } else {
            args.remove(0)
        };
        if !kwargs.is_empty() {
            return Err(sqlite_error(
                "TypeError",
                format!(
                    "cursor() takes at most 1 keyword argument ({} given)",
                    kwargs.len()
                ),
            ));
        }
        let default_cursor_class = self.sqlite_cursor_class()?;
        let cursor = match factory {
            Value::Class(class_ref) if class_ref.id() == default_cursor_class.id() => {
                let cursor_obj = self.alloc_instance_for_class(&class_ref);
                self.sqlite_initialize_cursor_instance(&cursor_obj, &receiver, connection_id);
                Value::Instance(cursor_obj)
            }
            factory_callable => match self.call_internal_preserving_caller(
                factory_callable,
                vec![receiver.clone()],
                HashMap::new(),
            )? {
                InternalCallOutcome::Value(value) => value,
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(self.runtime_error_from_active_exception(
                        "sqlite cursor factory callable failed",
                    ));
                }
            },
        };
        if !self.sqlite_value_is_cursor_instance(&cursor) {
            return Err(sqlite_error(
                "TypeError",
                format!(
                    "factory must return a cursor, not {}",
                    self.value_type_name_for_error(&cursor)
                ),
            ));
        }
        let row_factory = self
            .heap
            .find_object_by_id(connection_id)
            .and_then(|connection| Self::instance_attr_get(&connection, "row_factory"))
            .unwrap_or(Value::None);
        if row_factory != Value::None {
            let cursor_obj = self.receiver_from_value(&cursor)?;
            let _ = Self::instance_attr_set(&cursor_obj, "row_factory", row_factory);
        }
        Ok(cursor)
    }

    pub(in crate::vm) fn builtin_sqlite_cursor_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(sqlite_error("TypeError", "Cursor.__init__() missing self"));
        }
        if args.len() > 2 {
            return Err(sqlite_error(
                "TypeError",
                format!("Cursor expected 1 argument, got {}", args.len() - 1),
            ));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let connection = if args.is_empty() {
            kwargs
                .remove("connection")
                .ok_or_else(|| sqlite_error("TypeError", "Cursor expected 1 argument, got 0"))?
        } else {
            args.remove(0)
        };
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(sqlite_error(
                "TypeError",
                format!("Cursor.__init__() got an unexpected keyword argument '{unexpected}'"),
            ));
        }
        if !self.sqlite_value_is_connection_instance(&connection) {
            return Err(sqlite_error(
                "TypeError",
                format!(
                    "Cursor() argument 1 must be sqlite3.Connection, not {}",
                    self.value_type_name_for_error(&connection)
                ),
            ));
        }
        let connection_id = self.sqlite_connection_id_from_value(&connection, "Cursor.__init__")?;
        let _ = self.sqlite_open_db_handle(connection_id)?;
        self.sqlite_initialize_cursor_instance(&receiver, &connection, connection_id);
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_getattribute(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.__getattribute__() expects two arguments",
            ));
        }
        let receiver = args.remove(0);
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(sqlite_error("TypeError", "attribute name must be string")),
        };

        match name.as_str() {
            SQLITE_CONNECTION_ISOLATION_LEVEL_ATTR => {
                let connection_id =
                    self.sqlite_connection_id_from_value(&receiver, "isolation_level")?;
                let _ = self.sqlite_open_db_handle(connection_id)?;
                let receiver_obj = self.receiver_from_value(&receiver)?;
                Ok(
                    Self::instance_attr_get(&receiver_obj, SQLITE_CONNECTION_ISOLATION_LEVEL_ATTR)
                        .unwrap_or_else(|| Value::Str(String::new())),
                )
            }
            "autocommit" => {
                let connection_id =
                    self.sqlite_connection_id_from_value(&receiver, "autocommit")?;
                let _ = self.sqlite_open_db_handle(connection_id)?;
                let mode = self.sqlite_connection_autocommit_mode(connection_id)?;
                Ok(sqlite_autocommit_mode_to_value(mode))
            }
            "total_changes" => {
                let connection_id =
                    self.sqlite_connection_id_from_value(&receiver, "total_changes")?;
                let db = self.sqlite_open_db_handle(connection_id)?;
                // SAFETY: db is a valid sqlite handle.
                Ok(Value::Int(unsafe { sqlite3_total_changes(db) as i64 }))
            }
            "in_transaction" => {
                let connection_id =
                    self.sqlite_connection_id_from_value(&receiver, "in_transaction")?;
                let db = self.sqlite_open_db_handle(connection_id)?;
                // SAFETY: db is a valid sqlite handle.
                Ok(Value::Bool(unsafe { sqlite3_get_autocommit(db) == 0 }))
            }
            "__text_signature__" => Ok(Value::Str("(sql, /)".to_string())),
            _ => self.builtin_object_getattribute(vec![receiver, Value::Str(name)], HashMap::new()),
        }
    }

    pub(in crate::vm) fn builtin_sqlite_connection_setattr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 3 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.__setattr__() expects three arguments",
            ));
        }
        let receiver = args.remove(0);
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(sqlite_error("TypeError", "attribute name must be string")),
        };
        let value = args.remove(0);

        match name.as_str() {
            SQLITE_CONNECTION_ISOLATION_LEVEL_ATTR => {
                let connection_id =
                    self.sqlite_connection_id_from_value(&receiver, "isolation_level")?;
                let _ = self.sqlite_open_db_handle(connection_id)?;
                let receiver_obj = self.receiver_from_value(&receiver)?;
                let normalized = sqlite_normalize_isolation_level(value)?;
                if matches!(normalized, Value::None) {
                    self.builtin_sqlite_connection_commit(vec![receiver.clone()], HashMap::new())?;
                }
                Self::instance_attr_set(
                    &receiver_obj,
                    SQLITE_CONNECTION_ISOLATION_LEVEL_ATTR,
                    normalized,
                )?;
                Ok(Value::None)
            }
            "autocommit" => {
                let connection_id =
                    self.sqlite_connection_id_from_value(&receiver, "autocommit")?;
                let _ = self.sqlite_open_db_handle(connection_id)?;
                let mode = sqlite_normalize_autocommit(Some(value))?;
                self.sqlite_transition_autocommit_mode(connection_id, mode)?;
                Ok(Value::None)
            }
            "in_transaction" | "total_changes" => Err(sqlite_connection_readonly_attr_error(&name)),
            _ => {
                self.builtin_object_setattr(vec![receiver, Value::Str(name), value], HashMap::new())
            }
        }
    }

    pub(in crate::vm) fn builtin_sqlite_connection_delattr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.__delattr__() expects two arguments",
            ));
        }
        let receiver = args.remove(0);
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(sqlite_error("TypeError", "attribute name must be string")),
        };
        match name.as_str() {
            SQLITE_CONNECTION_ISOLATION_LEVEL_ATTR
            | "autocommit"
            | "in_transaction"
            | "total_changes" => Err(sqlite_error("AttributeError", "cannot delete attribute")),
            _ => self.builtin_object_delattr(vec![receiver, Value::Str(name)], HashMap::new()),
        }
    }

    pub(in crate::vm) fn builtin_sqlite_connection_close(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.close() expects no arguments",
            ));
        }
        let connection_id = self.sqlite_connection_id_from_value(&args[0], "close")?;
        if let Some(state) = self.sqlite_connections.get(&connection_id) {
            state.ensure_thread_affinity()?;
        }
        let is_open = self
            .sqlite_connections
            .get(&connection_id)
            .and_then(|state| state.db_handle())
            .is_some();
        if is_open
            && self.sqlite_connection_autocommit_mode(connection_id)?
                == SqliteAutocommitMode::Disabled
        {
            let db = self.sqlite_open_db_handle(connection_id)?;
            // SAFETY: db is a valid sqlite handle.
            if unsafe { sqlite3_get_autocommit(db) == 0 } {
                let _ = self.sqlite_execute_query(
                    connection_id,
                    "ROLLBACK",
                    SqliteParams::Positional(Vec::new()),
                )?;
            }
        }
        if let Some(state) = self.sqlite_connections.get_mut(&connection_id) {
            state
                .close()
                .map_err(|message| sqlite_error("OperationalError", message))?;
        }
        for cursor in self.sqlite_cursors.values_mut() {
            if cursor.connection_id == connection_id {
                cursor.closed = true;
                cursor.rows.clear();
                cursor.next_row = 0;
                cursor.description = None;
            }
        }
        for blob in self.sqlite_blobs.values_mut() {
            if blob.connection_id == connection_id {
                let _ = blob.close();
            }
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_enter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.__enter__() expects no arguments",
            ));
        }
        let connection_id = self.sqlite_connection_id_from_value(&args[0], "__enter__")?;
        let _ = self.sqlite_open_db_handle(connection_id)?;
        Ok(args.remove(0))
    }

    pub(in crate::vm) fn builtin_sqlite_connection_exit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 4 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.__exit__() expects exception triple",
            ));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "__exit__")?;
        if self.sqlite_connection_autocommit_mode(connection_id)? == SqliteAutocommitMode::Enabled {
            return Ok(Value::Bool(false));
        }
        let exc_type = args.remove(0);
        if matches!(exc_type, Value::None) {
            self.builtin_sqlite_connection_commit(vec![receiver], HashMap::new())?;
        } else {
            self.builtin_sqlite_connection_rollback(vec![receiver], HashMap::new())?;
        }
        Ok(Value::Bool(false))
    }

    pub(in crate::vm) fn builtin_sqlite_connection_execute(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(sqlite_error(
                "TypeError",
                "Connection.execute() missing self",
            ));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "execute")?;
        let _ = self.sqlite_open_db_handle(connection_id)?;
        if args.is_empty() {
            return Err(sqlite_error(
                "TypeError",
                "Connection.execute() missing SQL argument",
            ));
        }
        let cursor = self.builtin_sqlite_connection_cursor(vec![receiver], HashMap::new())?;
        let mut cursor_args = vec![cursor.clone()];
        cursor_args.extend(args);
        self.builtin_sqlite_cursor_execute(cursor_args, kwargs)?;
        Ok(cursor)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_executemany(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(sqlite_error(
                "TypeError",
                "Connection.executemany() missing self",
            ));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "executemany")?;
        let _ = self.sqlite_open_db_handle(connection_id)?;
        if args.len() < 2 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.executemany() missing SQL or parameters",
            ));
        }
        let cursor = self.builtin_sqlite_connection_cursor(vec![receiver], HashMap::new())?;
        let mut cursor_args = vec![cursor.clone()];
        cursor_args.extend(args);
        self.builtin_sqlite_cursor_executemany(cursor_args, kwargs)?;
        Ok(cursor)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_executescript(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(sqlite_error(
                "TypeError",
                "Connection.executescript() missing self",
            ));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "executescript")?;
        let _ = self.sqlite_open_db_handle(connection_id)?;
        if args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.executescript() expects one SQL script argument",
            ));
        }
        let cursor = self.builtin_sqlite_connection_cursor(vec![receiver], HashMap::new())?;
        let mut cursor_args = vec![cursor.clone()];
        cursor_args.extend(args);
        self.builtin_sqlite_cursor_executescript(cursor_args, kwargs)?;
        Ok(cursor)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_commit(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.commit() expects no arguments",
            ));
        }
        let connection_id = self.sqlite_connection_id_from_value(&args[0], "commit")?;
        let autocommit_mode = self.sqlite_connection_autocommit_mode(connection_id)?;
        if autocommit_mode == SqliteAutocommitMode::Enabled {
            return Ok(Value::None);
        }
        let db = self.sqlite_open_db_handle(connection_id)?;
        // SAFETY: db is a valid sqlite handle.
        let in_transaction = unsafe { sqlite3_get_autocommit(db) == 0 };
        if !in_transaction {
            if autocommit_mode == SqliteAutocommitMode::Disabled {
                let _ = self.sqlite_execute_query(
                    connection_id,
                    "BEGIN",
                    SqliteParams::Positional(Vec::new()),
                )?;
            }
            return Ok(Value::None);
        }
        match self.sqlite_execute_query(
            connection_id,
            "COMMIT",
            SqliteParams::Positional(Vec::new()),
        ) {
            Ok(_) => {
                if autocommit_mode == SqliteAutocommitMode::Disabled {
                    let db = self.sqlite_open_db_handle(connection_id)?;
                    // SAFETY: db is a valid sqlite handle.
                    if unsafe { sqlite3_get_autocommit(db) != 0 } {
                        let _ = self.sqlite_execute_query(
                            connection_id,
                            "BEGIN",
                            SqliteParams::Positional(Vec::new()),
                        )?;
                    }
                }
                Ok(Value::None)
            }
            Err(err) if err.message.contains("no transaction is active") => {
                if autocommit_mode == SqliteAutocommitMode::Disabled {
                    let db = self.sqlite_open_db_handle(connection_id)?;
                    // SAFETY: db is a valid sqlite handle.
                    if unsafe { sqlite3_get_autocommit(db) != 0 } {
                        let _ = self.sqlite_execute_query(
                            connection_id,
                            "BEGIN",
                            SqliteParams::Positional(Vec::new()),
                        )?;
                    }
                }
                Ok(Value::None)
            }
            Err(err) => Err(err),
        }
    }

    pub(in crate::vm) fn builtin_sqlite_connection_rollback(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.rollback() expects no arguments",
            ));
        }
        let connection_id = self.sqlite_connection_id_from_value(&args[0], "rollback")?;
        let autocommit_mode = self.sqlite_connection_autocommit_mode(connection_id)?;
        if autocommit_mode == SqliteAutocommitMode::Enabled {
            return Ok(Value::None);
        }
        let db = self.sqlite_open_db_handle(connection_id)?;
        // SAFETY: db is a valid sqlite handle.
        let in_transaction = unsafe { sqlite3_get_autocommit(db) == 0 };
        if !in_transaction {
            if autocommit_mode == SqliteAutocommitMode::Disabled {
                let _ = self.sqlite_execute_query(
                    connection_id,
                    "BEGIN",
                    SqliteParams::Positional(Vec::new()),
                )?;
            }
            return Ok(Value::None);
        }
        match self.sqlite_execute_query(
            connection_id,
            "ROLLBACK",
            SqliteParams::Positional(Vec::new()),
        ) {
            Ok(_) => {
                if autocommit_mode == SqliteAutocommitMode::Disabled {
                    let db = self.sqlite_open_db_handle(connection_id)?;
                    // SAFETY: db is a valid sqlite handle.
                    if unsafe { sqlite3_get_autocommit(db) != 0 } {
                        let _ = self.sqlite_execute_query(
                            connection_id,
                            "BEGIN",
                            SqliteParams::Positional(Vec::new()),
                        )?;
                    }
                }
                Ok(Value::None)
            }
            Err(err) if err.message.contains("no transaction is active") => {
                if autocommit_mode == SqliteAutocommitMode::Disabled {
                    let db = self.sqlite_open_db_handle(connection_id)?;
                    // SAFETY: db is a valid sqlite handle.
                    if unsafe { sqlite3_get_autocommit(db) != 0 } {
                        let _ = self.sqlite_execute_query(
                            connection_id,
                            "BEGIN",
                            SqliteParams::Positional(Vec::new()),
                        )?;
                    }
                }
                Ok(Value::None)
            }
            Err(err) => Err(err),
        }
    }

    pub(in crate::vm) fn builtin_sqlite_connection_interrupt(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.interrupt() expects no arguments",
            ));
        }
        let connection_id = self.sqlite_connection_id_from_value(&args[0], "interrupt")?;
        let db = self.sqlite_open_db_handle(connection_id)?;
        // SAFETY: db is a valid sqlite handle and sqlite3_interrupt has no return value.
        unsafe {
            sqlite3_interrupt(db);
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_backup(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(sqlite_error("TypeError", "backup() missing self"));
        }
        if args.len() > 2 {
            return Err(sqlite_error(
                "TypeError",
                "backup() takes exactly one positional argument",
            ));
        }
        let receiver = args.remove(0);
        let target = if args.is_empty() {
            kwargs.remove("target").ok_or_else(|| {
                sqlite_error("TypeError", "backup() missing required argument 'target'")
            })?
        } else {
            args.remove(0)
        };
        let pages_arg = kwargs.remove("pages").unwrap_or(Value::Int(-1));
        let progress = kwargs.remove("progress").unwrap_or(Value::None);
        let name = match kwargs
            .remove("name")
            .unwrap_or_else(|| Value::Str("main".to_string()))
        {
            Value::Str(name) => name,
            other => {
                return Err(sqlite_error(
                    "TypeError",
                    format!(
                        "name must be str, not {}",
                        self.value_type_name_for_error(&other)
                    ),
                ));
            }
        };
        let sleep = match kwargs.remove("sleep").unwrap_or(Value::Float(0.25)) {
            Value::Float(value) => value,
            Value::Int(value) => value as f64,
            Value::Bool(flag) => {
                if flag {
                    1.0
                } else {
                    0.0
                }
            }
            other => value_to_f64(other).map_err(|_| {
                sqlite_error("TypeError", "sleep must be a real number (int or float)")
            })?,
        };
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(sqlite_error(
                "TypeError",
                format!("backup() got an unexpected keyword argument '{unexpected}'"),
            ));
        }
        if !self.sqlite_value_is_connection_instance(&target) {
            return Err(sqlite_error(
                "TypeError",
                "target must be a sqlite3.Connection",
            ));
        }
        if !matches!(progress, Value::None) && !self.is_callable_value(&progress) {
            return Err(sqlite_error(
                "TypeError",
                "progress argument must be a callable",
            ));
        }

        let mut pages = value_to_int(pages_arg)
            .map_err(|_| sqlite_error("TypeError", "pages must be an integer"))?;
        if pages == 0 {
            pages = -1;
        }
        let pages_c = c_int::try_from(pages)
            .map_err(|_| sqlite_error("OverflowError", "pages out of range"))?;
        let sleep_ms = if sleep <= 0.0 {
            0
        } else {
            let millis = sleep * 1000.0;
            if millis > c_int::MAX as f64 {
                c_int::MAX
            } else {
                millis as c_int
            }
        };

        let source_id = self.sqlite_connection_id_from_value(&receiver, "backup")?;
        let target_id = self.sqlite_connection_id_from_value(&target, "backup")?;
        if source_id == target_id {
            return Err(sqlite_error(
                "ValueError",
                "target cannot be the same connection instance",
            ));
        }
        let source_db = self.sqlite_open_db_handle(source_id)?;
        let target_db = self.sqlite_open_db_handle(target_id)?;
        let dest_name = CString::new("main").expect("valid static sqlite db name");
        let source_name = CString::new(name)
            .map_err(|_| sqlite_error("ValueError", "embedded null character"))?;

        let _callback_vm_guard = SqliteCallbackVmGuard::enter(self);
        // SAFETY: handles and C strings are valid for backup initialization.
        let backup = unsafe {
            sqlite3_backup_init(
                target_db,
                dest_name.as_ptr(),
                source_db,
                source_name.as_ptr(),
            )
        };
        if backup.is_null() {
            return Err(sqlite_error_from_db_status(target_db, "OperationalError"));
        }

        let mut rc: c_int;
        loop {
            // SAFETY: backup handle is valid until backup_finish is called.
            rc = unsafe { sqlite3_backup_step(backup, pages_c) };
            if !matches!(progress, Value::None) {
                // SAFETY: backup handle is valid and functions are read-only.
                let remaining = unsafe { sqlite3_backup_remaining(backup) } as i64;
                // SAFETY: backup handle is valid and functions are read-only.
                let total = unsafe { sqlite3_backup_pagecount(backup) } as i64;
                match self.call_internal_preserving_caller(
                    progress.clone(),
                    vec![
                        Value::Int(rc as i64),
                        Value::Int(remaining),
                        Value::Int(total),
                    ],
                    HashMap::new(),
                )? {
                    InternalCallOutcome::Value(_) => {}
                    InternalCallOutcome::CallerExceptionHandled => {
                        // SAFETY: backup handle is valid and must be finalized exactly once.
                        unsafe {
                            let _ = sqlite3_backup_finish(backup);
                        }
                        return Err(self
                            .runtime_error_from_active_exception("sqlite backup progress failed"));
                    }
                }
            }
            if rc == SQLITE_OK || rc == SQLITE_BUSY || rc == SQLITE_LOCKED {
                if rc == SQLITE_BUSY || rc == SQLITE_LOCKED {
                    // SAFETY: sqlite3_sleep does not require additional invariants.
                    unsafe {
                        sqlite3_sleep(sleep_ms);
                    }
                }
                continue;
            }
            break;
        }

        // SAFETY: backup handle is valid and must be finalized exactly once.
        let finish_rc = unsafe { sqlite3_backup_finish(backup) };
        if finish_rc != SQLITE_OK {
            return Err(sqlite_error_from_db_status(target_db, "OperationalError"));
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_iterdump(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(sqlite_error("TypeError", "iterdump() missing self"));
        }
        if args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "iterdump() takes no positional arguments",
            ));
        }
        let receiver = args.remove(0);
        let filter = kwargs.remove("filter");
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(sqlite_error(
                "TypeError",
                format!("iterdump() got an unexpected keyword argument '{unexpected}'"),
            ));
        }
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "iterdump")?;
        let _ = self.sqlite_open_db_handle(connection_id)?;
        let dump_module = self
            .builtin_import_module(vec![Value::Str("sqlite3.dump".to_string())], HashMap::new())?;
        let iterdump_callable = self.builtin_getattr(
            vec![dump_module, Value::Str("_iterdump".to_string())],
            HashMap::new(),
        )?;
        let mut call_kwargs = HashMap::new();
        if let Some(filter_value) = filter {
            call_kwargs.insert("filter".to_string(), filter_value);
        }
        match self.call_internal_preserving_caller(
            iterdump_callable,
            vec![receiver],
            call_kwargs,
        )? {
            InternalCallOutcome::Value(value) => Ok(value),
            InternalCallOutcome::CallerExceptionHandled => {
                Err(self.runtime_error_from_active_exception("iterdump() failed"))
            }
        }
    }

    pub(in crate::vm) fn builtin_sqlite_connection_create_function(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 4 {
            return Err(sqlite_error(
                "TypeError",
                "create_function() expects name, num_params, func",
            ));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "create_function")?;
        let db = self.sqlite_open_db_handle(connection_id)?;
        let name = args.remove(0);
        let num_params = args.remove(0);
        let func = args.remove(0);
        if !args.is_empty() {
            return Err(sqlite_error(
                "TypeError",
                "create_function() takes 3 positional arguments",
            ));
        }
        let name = match name {
            Value::Str(name) => name,
            _ => {
                return Err(sqlite_error(
                    "TypeError",
                    "create_function() name must be str",
                ));
            }
        };
        let num_params = value_to_int(num_params).map_err(|_| {
            sqlite_error(
                "TypeError",
                "create_function() num_params must be an integer",
            )
        })?;
        if !(-1..=127).contains(&num_params) {
            return Err(sqlite_error(
                "ProgrammingError",
                "create_function() parameter count out of range",
            ));
        }
        if !matches!(func, Value::None) && !self.is_callable_value(&func) {
            return Err(sqlite_error(
                "TypeError",
                "create_function() expected a callable or None",
            ));
        }
        let deterministic = kwargs
            .remove("deterministic")
            .map(|value| is_truthy(&value))
            .unwrap_or(false);
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(sqlite_error(
                "TypeError",
                format!("create_function() got an unexpected keyword argument '{unexpected}'"),
            ));
        }
        let name_c = CString::new(name.as_bytes())
            .map_err(|_| sqlite_error("ProgrammingError", "function name contains embedded NUL"))?;
        let mut text_rep = SQLITE_UTF8;
        if deterministic {
            text_rep |= SQLITE_DETERMINISTIC;
        }

        let rc = if matches!(func, Value::None) {
            // SAFETY: db is valid and name_c points to a valid C string.
            unsafe {
                sqlite3_create_function_v2(
                    db,
                    name_c.as_ptr(),
                    num_params as c_int,
                    text_rep,
                    ptr::null_mut(),
                    None,
                    None,
                    None,
                    None,
                )
            }
        } else {
            let callback_state = Box::new(SqliteScalarFunctionCallbackState { callable: func });
            let callback_ptr = Box::into_raw(callback_state) as *mut c_void;
            // SAFETY: db is valid and callback pointer remains valid until sqlite invokes destroy.
            let rc = unsafe {
                sqlite3_create_function_v2(
                    db,
                    name_c.as_ptr(),
                    num_params as c_int,
                    text_rep,
                    callback_ptr,
                    Some(sqlite_scalar_function_callback),
                    None,
                    None,
                    Some(sqlite_scalar_function_destroy),
                )
            };
            if rc != SQLITE_OK {
                // SAFETY: sqlite did not take ownership on failed registration.
                unsafe { sqlite_scalar_function_destroy(callback_ptr) };
            }
            rc
        };
        if rc != SQLITE_OK {
            return Err(sqlite_error_from_db_status(db, "OperationalError"));
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_create_aggregate(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 4 {
            return Err(sqlite_error(
                "TypeError",
                "create_aggregate() expects name, num_params, aggregate_class",
            ));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "create_aggregate")?;
        let _ = self.sqlite_open_db_handle(connection_id)?;
        match args.remove(0) {
            Value::Str(_) => {}
            _ => {
                return Err(sqlite_error(
                    "TypeError",
                    "create_aggregate() name must be str",
                ));
            }
        }
        let _ = value_to_int(args.remove(0)).map_err(|_| {
            sqlite_error(
                "TypeError",
                "create_aggregate() num_params must be an integer",
            )
        })?;
        let aggregate_class = args.remove(0);
        if !matches!(aggregate_class, Value::None)
            && !matches!(aggregate_class, Value::Class(_))
            && !self.is_callable_value(&aggregate_class)
        {
            return Err(sqlite_error(
                "TypeError",
                "create_aggregate() expected class or None",
            ));
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_create_window_function(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 4 {
            return Err(sqlite_error(
                "TypeError",
                "create_window_function() expects name, num_params, aggregate_class",
            ));
        }
        let receiver = args.remove(0);
        let connection_id =
            self.sqlite_connection_id_from_value(&receiver, "create_window_function")?;
        let _ = self.sqlite_open_db_handle(connection_id)?;
        match args.remove(0) {
            Value::Str(_) => {}
            _ => {
                return Err(sqlite_error(
                    "TypeError",
                    "create_window_function() name must be str",
                ));
            }
        }
        let _ = value_to_int(args.remove(0)).map_err(|_| {
            sqlite_error(
                "TypeError",
                "create_window_function() num_params must be an integer",
            )
        })?;
        let aggregate_class = args.remove(0);
        if !matches!(aggregate_class, Value::None)
            && !matches!(aggregate_class, Value::Class(_))
            && !self.is_callable_value(&aggregate_class)
        {
            return Err(sqlite_error(
                "TypeError",
                "create_window_function() expected class or None",
            ));
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_set_trace_callback(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(sqlite_error(
                "TypeError",
                "set_trace_callback() expects trace_callback",
            ));
        }
        let receiver = args.remove(0);
        let connection_id =
            self.sqlite_connection_id_from_value(&receiver, "set_trace_callback")?;
        let db = self.sqlite_open_db_handle(connection_id)?;

        let callback = if !args.is_empty() {
            args.remove(0)
        } else if let Some(value) = kwargs.remove("trace_callback") {
            self.sqlite_warn_keyword_deprecation("set_trace_callback", "trace_callback")?;
            value
        } else {
            return Err(sqlite_error(
                "TypeError",
                "set_trace_callback() expects trace_callback",
            ));
        };
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(sqlite_error(
                "TypeError",
                format!("set_trace_callback() got an unexpected keyword argument '{unexpected}'"),
            ));
        }
        if !matches!(callback, Value::None) && !self.is_callable_value(&callback) {
            return Err(sqlite_error(
                "TypeError",
                "set_trace_callback() expected a callable or None",
            ));
        }

        let replacement = if matches!(callback, Value::None) {
            // SAFETY: db handle is valid and callbacks are disabled by sqlite.
            let rc = unsafe { sqlite3_trace_v2(db, SQLITE_TRACE_STMT, None, ptr::null_mut()) };
            if rc != SQLITE_OK {
                return Err(sqlite_error_from_db_status(db, "OperationalError"));
            }
            None
        } else {
            let callback_state = Box::new(Arc::new(SqliteVmCallbackState { callable: callback }));
            let callback_ptr = Box::into_raw(callback_state);
            // SAFETY: db is valid and callback_ptr remains alive until we drop it on reconfigure/close.
            let rc = unsafe {
                sqlite3_trace_v2(
                    db,
                    SQLITE_TRACE_STMT,
                    Some(sqlite_trace_callback),
                    callback_ptr as *mut c_void,
                )
            };
            if rc != SQLITE_OK {
                // SAFETY: sqlite did not take ownership because registration failed.
                unsafe { drop(Box::from_raw(callback_ptr)) };
                return Err(sqlite_error_from_db_status(db, "OperationalError"));
            }
            Some(
                NonNull::new(callback_ptr)
                    .expect("sqlite trace callback pointer should never be null"),
            )
        };
        if let Some(state) = self.sqlite_connections.get_mut(&connection_id) {
            let previous = state.trace_callback.take();
            state.trace_callback = replacement;
            SqliteConnectionState::drop_vm_callback_ptr(previous);
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_create_collation(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 3 {
            return Err(sqlite_error(
                "TypeError",
                "create_collation() expects name and callback",
            ));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "create_collation")?;
        let db = self.sqlite_open_db_handle(connection_id)?;
        let name = args.remove(0);
        let callback = args.remove(0);
        let name = if let Value::Str(text) = name {
            text
        } else if let Value::Instance(_) = &name {
            match self.builtin_str(vec![name], HashMap::new())? {
                Value::Str(text) => text,
                _ => {
                    return Err(sqlite_error(
                        "TypeError",
                        "create_collation() name must be str",
                    ));
                }
            }
        } else {
            return Err(sqlite_error(
                "TypeError",
                "create_collation() name must be str",
            ));
        };
        if !matches!(callback, Value::None) && !self.is_callable_value(&callback) {
            return Err(sqlite_error("TypeError", "parameter must be callable"));
        }
        let name_c = CString::new(name.as_bytes())
            .map_err(|_| sqlite_error("ProgrammingError", "function name contains embedded NUL"))?;
        let rc = if matches!(callback, Value::None) {
            // SAFETY: db/name are valid and this unregisters any prior collation callback.
            unsafe {
                sqlite3_create_collation_v2(
                    db,
                    name_c.as_ptr(),
                    SQLITE_UTF8,
                    ptr::null_mut(),
                    None,
                    None,
                )
            }
        } else {
            let callback_state = Box::new(Arc::new(SqliteVmCallbackState { callable: callback }));
            let callback_ptr = Box::into_raw(callback_state);
            // SAFETY: db/name are valid and callback_ptr is owned by sqlite via destroy callback.
            let rc = unsafe {
                sqlite3_create_collation_v2(
                    db,
                    name_c.as_ptr(),
                    SQLITE_UTF8,
                    callback_ptr as *mut c_void,
                    Some(sqlite_collation_callback),
                    Some(sqlite_vm_callback_destroy),
                )
            };
            if rc != SQLITE_OK {
                // SAFETY: sqlite docs: collation destroy callback is not called on failed registration.
                unsafe { drop(Box::from_raw(callback_ptr)) };
            }
            rc
        };
        if rc != SQLITE_OK {
            return Err(sqlite_error_from_db_status(db, "OperationalError"));
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_set_authorizer(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error(
                "TypeError",
                "set_authorizer() expects callback",
            ));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "set_authorizer")?;
        let db = self.sqlite_open_db_handle(connection_id)?;
        let callback = args.remove(0);
        if !matches!(callback, Value::None) && !self.is_callable_value(&callback) {
            return Err(sqlite_error(
                "TypeError",
                "set_authorizer() expected a callable or None",
            ));
        }
        if matches!(callback, Value::None) {
            // SAFETY: db is valid and callback removal is explicit.
            let rc = unsafe { sqlite3_set_authorizer(db, None, ptr::null_mut()) };
            if rc != SQLITE_OK {
                return Err(sqlite_error(
                    "OperationalError",
                    "Error setting authorizer callback",
                ));
            }
            if let Some(state) = self.sqlite_connections.get_mut(&connection_id) {
                SqliteConnectionState::drop_vm_callback_ptr(state.authorizer_callback.take());
            }
            return Ok(Value::None);
        }
        let callback_state = Box::new(Arc::new(SqliteVmCallbackState { callable: callback }));
        let callback_ptr = Box::into_raw(callback_state);
        // SAFETY: db is valid and callback_ptr is retained in connection state.
        let rc = unsafe {
            sqlite3_set_authorizer(
                db,
                Some(sqlite_authorizer_callback),
                callback_ptr as *mut c_void,
            )
        };
        if rc != SQLITE_OK {
            // SAFETY: sqlite did not take ownership on failed registration.
            unsafe { drop(Box::from_raw(callback_ptr)) };
            return Err(sqlite_error(
                "OperationalError",
                "Error setting authorizer callback",
            ));
        }
        if let Some(state) = self.sqlite_connections.get_mut(&connection_id) {
            let replacement = NonNull::new(callback_ptr)
                .expect("sqlite authorizer callback pointer should never be null");
            SqliteConnectionState::drop_vm_callback_ptr(
                state.authorizer_callback.replace(replacement),
            );
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_set_progress_handler(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 3 {
            return Err(sqlite_error(
                "TypeError",
                "set_progress_handler() expects callback and n",
            ));
        }
        let receiver = args.remove(0);
        let connection_id =
            self.sqlite_connection_id_from_value(&receiver, "set_progress_handler")?;
        let db = self.sqlite_open_db_handle(connection_id)?;

        let callback = if !args.is_empty() {
            args.remove(0)
        } else if let Some(value) = kwargs.remove("progress_handler") {
            self.sqlite_warn_keyword_deprecation("set_progress_handler", "progress_handler")?;
            value
        } else {
            return Err(sqlite_error(
                "TypeError",
                "set_progress_handler() expects callback and n",
            ));
        };
        let n_value = if !args.is_empty() {
            args.remove(0)
        } else if let Some(value) = kwargs.remove("n") {
            self.sqlite_warn_keyword_deprecation("set_progress_handler", "n")?;
            value
        } else {
            return Err(sqlite_error(
                "TypeError",
                "set_progress_handler() expects callback and n",
            ));
        };
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(sqlite_error(
                "TypeError",
                format!("set_progress_handler() got an unexpected keyword argument '{unexpected}'"),
            ));
        }
        let n = value_to_int(n_value).map_err(|_| {
            sqlite_error("TypeError", "set_progress_handler() n must be an integer")
        })?;
        if !matches!(callback, Value::None) && !self.is_callable_value(&callback) {
            return Err(sqlite_error(
                "TypeError",
                "set_progress_handler() expected a callable or None",
            ));
        }
        let disable = matches!(callback, Value::None) || n == 0;
        if disable {
            // SAFETY: db is valid and disabling progress callback is explicit.
            unsafe { sqlite3_progress_handler(db, 0, None, ptr::null_mut()) };
            if let Some(state) = self.sqlite_connections.get_mut(&connection_id) {
                SqliteConnectionState::drop_vm_callback_ptr(state.progress_callback.take());
            }
            return Ok(Value::None);
        }
        let n = i32::try_from(n)
            .map_err(|_| sqlite_error("OverflowError", "signed integer is greater than maximum"))?;
        let callback_state = Box::new(Arc::new(SqliteVmCallbackState { callable: callback }));
        let callback_ptr = Box::into_raw(callback_state);
        // SAFETY: db is valid and callback_ptr is retained in connection state.
        unsafe {
            sqlite3_progress_handler(
                db,
                n,
                Some(sqlite_progress_callback),
                callback_ptr as *mut c_void,
            );
        }
        if let Some(state) = self.sqlite_connections.get_mut(&connection_id) {
            let replacement = NonNull::new(callback_ptr)
                .expect("sqlite progress callback pointer should never be null");
            SqliteConnectionState::drop_vm_callback_ptr(
                state.progress_callback.replace(replacement),
            );
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_getlimit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error("TypeError", "getlimit() expects category"));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "getlimit")?;
        let category = Self::sqlite_limit_category(args.remove(0))?;
        let db = self.sqlite_open_db_handle(connection_id)?;
        // SAFETY: db is a valid sqlite handle and category is validated.
        let value = unsafe { sqlite3_limit(db, category, -1) };
        Ok(Value::Int(value as i64))
    }

    pub(in crate::vm) fn builtin_sqlite_connection_setlimit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 3 {
            return Err(sqlite_error(
                "TypeError",
                "setlimit() expects category and limit",
            ));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "setlimit")?;
        let category = Self::sqlite_limit_category(args.remove(0))?;
        let new_limit = value_to_int(args.remove(0))
            .map_err(|_| sqlite_error("TypeError", "'limit' must be an integer"))?;
        let new_limit = i32::try_from(new_limit)
            .map_err(|_| sqlite_error("OverflowError", "limit out of range"))?;
        let db = self.sqlite_open_db_handle(connection_id)?;
        // SAFETY: db is a valid sqlite handle, category/new_limit are validated integers.
        let previous = unsafe { sqlite3_limit(db, category, new_limit) };
        Ok(Value::Int(previous as i64))
    }

    pub(in crate::vm) fn builtin_sqlite_connection_getconfig(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error(
                "TypeError",
                "getconfig() expects one operation argument",
            ));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "getconfig")?;
        let op = Self::sqlite_dbconfig_operation(args.remove(0))?;
        let db = self.sqlite_open_db_handle(connection_id)?;
        let mut current: c_int = 0;
        // SAFETY: db is valid and sqlite3_db_config expects op, -1 to query, and output pointer.
        let rc = unsafe { sqlite3_db_config(db, op, -1, &mut current as *mut c_int) };
        if rc != SQLITE_OK {
            return Err(sqlite_error("ValueError", "unknown config operation"));
        }
        Ok(Value::Bool(current != 0))
    }

    pub(in crate::vm) fn builtin_sqlite_connection_setconfig(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(sqlite_error(
                "TypeError",
                "setconfig() expects operation and optional enabled flag",
            ));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "setconfig")?;
        let op = Self::sqlite_dbconfig_operation(args.remove(0))?;
        let enabled = if !args.is_empty() {
            is_truthy(&args.remove(0))
        } else if let Some(value) = kwargs.remove("enabled") {
            is_truthy(&value)
        } else {
            true
        };
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(sqlite_error(
                "TypeError",
                format!("setconfig() got an unexpected keyword argument '{unexpected}'"),
            ));
        }
        let db = self.sqlite_open_db_handle(connection_id)?;
        let mut current: c_int = 0;
        let enabled_int: c_int = if enabled { 1 } else { 0 };
        // SAFETY: db is valid and sqlite3_db_config expects op, enable flag, and output pointer.
        let rc = unsafe { sqlite3_db_config(db, op, enabled_int, &mut current as *mut c_int) };
        if rc != SQLITE_OK {
            return Err(sqlite_error("ValueError", "unknown config operation"));
        }
        Ok(Value::Bool(current != 0))
    }

    pub(in crate::vm) fn builtin_sqlite_connection_blobopen(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 4 || args.len() > 5 {
            return Err(sqlite_error(
                "TypeError",
                "blobopen() takes 4 positional arguments",
            ));
        }
        let receiver = args.remove(0);
        let connection_id = self.sqlite_connection_id_from_value(&receiver, "blobopen")?;
        let db = self.sqlite_open_db_handle(connection_id)?;

        let table = match args.remove(0) {
            Value::Str(value) => value,
            _ => {
                return Err(sqlite_error(
                    "TypeError",
                    "blobopen() argument 1 must be str",
                ));
            }
        };
        let column = match args.remove(0) {
            Value::Str(value) => value,
            _ => {
                return Err(sqlite_error(
                    "TypeError",
                    "blobopen() argument 2 must be str",
                ));
            }
        };
        let row_id = match args.remove(0) {
            Value::Int(value) => value,
            Value::Bool(value) => {
                if value {
                    1
                } else {
                    0
                }
            }
            Value::BigInt(value) => value.to_i64().ok_or_else(|| {
                sqlite_error(
                    "OverflowError",
                    "row id too large to fit into 64-bit integer",
                )
            })?,
            _ => {
                return Err(sqlite_error(
                    "TypeError",
                    "blobopen() argument 3 must be int",
                ));
            }
        };
        let readonly = kwargs
            .remove("readonly")
            .map(|value| is_truthy(&value))
            .unwrap_or(false);
        let name = match kwargs.remove("name") {
            Some(Value::Str(value)) => value,
            Some(_) => {
                return Err(sqlite_error(
                    "TypeError",
                    "blobopen() argument 'name' must be str",
                ));
            }
            None => "main".to_string(),
        };
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(sqlite_error(
                "TypeError",
                format!("blobopen() got an unexpected keyword argument '{unexpected}'"),
            ));
        }

        let db_name_c = CString::new(name.as_bytes()).map_err(|_| {
            sqlite_error(
                "ProgrammingError",
                "blob database name contains embedded NUL",
            )
        })?;
        let table_c = CString::new(table.as_bytes())
            .map_err(|_| sqlite_error("ProgrammingError", "blob table contains embedded NUL"))?;
        let column_c = CString::new(column.as_bytes())
            .map_err(|_| sqlite_error("ProgrammingError", "blob column contains embedded NUL"))?;
        let flags = if readonly { 0 } else { 1 };
        let mut blob_handle: *mut Sqlite3Blob = ptr::null_mut();
        // SAFETY: db and C-string pointers are valid for this call and blob_handle is an out pointer.
        let rc = unsafe {
            sqlite3_blob_open(
                db,
                db_name_c.as_ptr(),
                table_c.as_ptr(),
                column_c.as_ptr(),
                row_id,
                flags,
                &mut blob_handle as *mut *mut Sqlite3Blob,
            )
        };
        if rc != SQLITE_OK {
            return Err(Self::sqlite_blob_error(db, rc));
        }

        let class = self.sqlite_blob_class()?;
        let blob = self.alloc_instance_for_class(&class);
        self.sqlite_blobs
            .insert(blob.id(), SqliteBlobState::new(blob_handle, connection_id));
        Ok(Value::Instance(blob))
    }

    pub(in crate::vm) fn builtin_sqlite_blob_close(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Blob.close() expects no arguments",
            ));
        }
        let blob_id = self.sqlite_blob_id_from_value(&args[0], "close")?;
        if let Some(state) = self.sqlite_blobs.get_mut(&blob_id) {
            state
                .close()
                .map_err(|message| sqlite_error("OperationalError", message))?;
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_blob_read(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(sqlite_error(
                "TypeError",
                "Blob.read() expects optional length argument",
            ));
        }
        let blob_id = self.sqlite_blob_id_from_value(&args[0], "read")?;
        let requested_len = if args.len() == 2 {
            self.io_index_arg_to_int(args.remove(1))?
        } else {
            -1
        };
        let out = {
            let (state, db) = self.sqlite_blob_state_and_db(blob_id)?;
            let Some(handle) = state.handle() else {
                return Err(sqlite_error(
                    "ProgrammingError",
                    "Cannot operate on a closed blob.",
                ));
            };
            let total_len = Self::sqlite_blob_len(handle)?;
            if state.offset >= total_len {
                Vec::new()
            } else {
                let remaining = total_len - state.offset;
                let read_len = if requested_len < 0 {
                    remaining
                } else {
                    let requested = usize::try_from(requested_len).map_err(|_| {
                        sqlite_error("OverflowError", "read length must fit in machine word")
                    })?;
                    requested.min(remaining)
                };
                if read_len == 0 {
                    Vec::new()
                } else {
                    let read_len_c = sqlite_len_to_c_int(read_len, "blob read length")?;
                    let offset_c = sqlite_len_to_c_int(state.offset, "blob read offset")?;
                    let mut out = vec![0u8; read_len];
                    // SAFETY: handle is open, out points to valid writable memory, and size/offset are checked.
                    let rc = unsafe {
                        sqlite3_blob_read(
                            handle,
                            out.as_mut_ptr() as *mut c_void,
                            read_len_c,
                            offset_c,
                        )
                    };
                    if rc != SQLITE_OK {
                        return Err(Self::sqlite_blob_error(db, rc));
                    }
                    state.offset += read_len;
                    out
                }
            }
        };
        Ok(self.heap.alloc_bytes(out))
    }

    pub(in crate::vm) fn builtin_sqlite_blob_write(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error(
                "TypeError",
                "Blob.write() expects bytes-like data",
            ));
        }
        let blob_id = self.sqlite_blob_id_from_value(&args[0], "write")?;
        let payload = bytes_like_from_value(args.remove(1))
            .map_err(|_| sqlite_error("TypeError", "a bytes-like object is required"))?;
        let (state, db) = self.sqlite_blob_state_and_db(blob_id)?;
        let Some(handle) = state.handle() else {
            return Err(sqlite_error(
                "ProgrammingError",
                "Cannot operate on a closed blob.",
            ));
        };
        let total_len = Self::sqlite_blob_len(handle)?;
        let remaining = total_len.saturating_sub(state.offset);
        if payload.len() > remaining {
            return Err(sqlite_error("ValueError", "data longer than blob length"));
        }
        if payload.is_empty() {
            return Ok(Value::None);
        }
        let payload_len_c = sqlite_len_to_c_int(payload.len(), "blob write length")?;
        let offset_c = sqlite_len_to_c_int(state.offset, "blob write offset")?;
        // SAFETY: handle is open, payload pointer stays valid for call duration, and bounds are checked.
        let rc = unsafe {
            sqlite3_blob_write(
                handle,
                payload.as_ptr() as *const c_void,
                payload_len_c,
                offset_c,
            )
        };
        if rc != SQLITE_OK {
            return Err(Self::sqlite_blob_error(db, rc));
        }
        state.offset += payload.len();
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_blob_seek(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 3 {
            return Err(sqlite_error(
                "TypeError",
                "Blob.seek() expects offset and optional origin",
            ));
        }
        let blob_id = self.sqlite_blob_id_from_value(&args[0], "seek")?;
        let offset = if args.len() >= 2 {
            self.io_index_arg_to_int(args.remove(1))?
        } else {
            0
        };
        let origin = if args.len() == 2 {
            self.io_index_arg_to_int(args.remove(1))?
        } else {
            0
        };
        let (state, _) = self.sqlite_blob_state_and_db(blob_id)?;
        let Some(handle) = state.handle() else {
            return Err(sqlite_error(
                "ProgrammingError",
                "Cannot operate on a closed blob.",
            ));
        };
        let len = Self::sqlite_blob_len(handle)?;
        let offset_i32 = i32::try_from(offset)
            .map_err(|_| sqlite_error("OverflowError", "seek offset results in overflow"))?;
        let base_i32 = match origin {
            0 => 0i32,
            1 => i32::try_from(state.offset)
                .map_err(|_| sqlite_error("OverflowError", "seek offset results in overflow"))?,
            2 => i32::try_from(len)
                .map_err(|_| sqlite_error("OverflowError", "seek offset results in overflow"))?,
            _ => {
                return Err(sqlite_error(
                    "ValueError",
                    "'origin' should be os.SEEK_SET, os.SEEK_CUR, or os.SEEK_END",
                ));
            }
        };
        let Some(new_offset_i32) = base_i32.checked_add(offset_i32) else {
            return Err(sqlite_error(
                "OverflowError",
                "seek offset results in overflow",
            ));
        };
        let len_i32 = i32::try_from(len)
            .map_err(|_| sqlite_error("OverflowError", "seek offset results in overflow"))?;
        if new_offset_i32 < 0 || new_offset_i32 > len_i32 {
            return Err(sqlite_error("ValueError", "offset out of blob range"));
        }
        state.offset = usize::try_from(new_offset_i32)
            .map_err(|_| sqlite_error("OverflowError", "seek offset results in overflow"))?;
        Ok(Value::Int(new_offset_i32 as i64))
    }

    pub(in crate::vm) fn builtin_sqlite_blob_tell(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Blob.tell() expects no arguments",
            ));
        }
        let blob_id = self.sqlite_blob_id_from_value(&args[0], "tell")?;
        let (state, _) = self.sqlite_blob_state_and_db(blob_id)?;
        if state.handle().is_none() {
            return Err(sqlite_error(
                "ProgrammingError",
                "Cannot operate on a closed blob.",
            ));
        }
        Ok(Value::Int(i64::try_from(state.offset).map_err(|_| {
            sqlite_error("OverflowError", "offset too large")
        })?))
    }

    pub(in crate::vm) fn builtin_sqlite_blob_enter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Blob.__enter__() expects no arguments",
            ));
        }
        let blob_id = self.sqlite_blob_id_from_value(&args[0], "__enter__")?;
        let (state, _) = self.sqlite_blob_state_and_db(blob_id)?;
        if state.handle().is_none() {
            return Err(sqlite_error(
                "ProgrammingError",
                "Cannot operate on a closed blob.",
            ));
        }
        Ok(args.remove(0))
    }

    pub(in crate::vm) fn builtin_sqlite_blob_iter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Blob.__iter__() expects no arguments",
            ));
        }
        Err(sqlite_error(
            "TypeError",
            "argument of type 'Blob' is not iterable",
        ))
    }

    pub(in crate::vm) fn builtin_sqlite_blob_exit(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 4 {
            return Err(sqlite_error(
                "TypeError",
                "Blob.__exit__() expects exception triple",
            ));
        }
        let blob_id = self.sqlite_blob_id_from_value(&args[0], "__exit__")?;
        {
            let (state, _) = self.sqlite_blob_state_and_db(blob_id)?;
            if state.handle().is_none() {
                return Err(sqlite_error(
                    "ProgrammingError",
                    "Cannot operate on a closed blob.",
                ));
            }
        }
        self.builtin_sqlite_blob_close(vec![args[0].clone()], HashMap::new())?;
        Ok(Value::Bool(false))
    }

    pub(in crate::vm) fn builtin_sqlite_blob_len(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Blob.__len__() expects no arguments",
            ));
        }
        let blob_id = self.sqlite_blob_id_from_value(&args[0], "__len__")?;
        let (state, _) = self.sqlite_blob_state_and_db(blob_id)?;
        let Some(handle) = state.handle() else {
            return Err(sqlite_error(
                "ProgrammingError",
                "Cannot operate on a closed blob.",
            ));
        };
        Ok(Value::Int(
            i64::try_from(Self::sqlite_blob_len(handle)?)
                .map_err(|_| sqlite_error("OverflowError", "blob length too large"))?,
        ))
    }

    pub(in crate::vm) fn builtin_sqlite_blob_getitem(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error(
                "TypeError",
                "Blob.__getitem__() expects an index or slice",
            ));
        }
        let blob_id = self.sqlite_blob_id_from_value(&args[0], "__getitem__")?;
        let key = args.remove(1);
        let parsed_index = match &key {
            Value::Slice(_) => None,
            other => Some(self.sqlite_blob_index_arg(other.clone())?),
        };
        let blob_bytes = {
            let (state, db) = self.sqlite_blob_state_and_db(blob_id)?;
            let Some(handle) = state.handle() else {
                return Err(sqlite_error(
                    "ProgrammingError",
                    "Cannot operate on a closed blob.",
                ));
            };
            let len = Self::sqlite_blob_len(handle)?;
            match key {
                Value::Slice(slice) => {
                    let indices = slice_indices(len, slice.lower, slice.upper, slice.step)?;
                    if indices.is_empty() {
                        Some(Vec::new())
                    } else {
                        let mut out = Vec::with_capacity(indices.len());
                        for index in indices {
                            let mut byte = [0u8; 1];
                            let index_c = sqlite_len_to_c_int(index, "blob index")?;
                            // SAFETY: handle is open and byte points to one writable byte.
                            let rc = unsafe {
                                sqlite3_blob_read(
                                    handle,
                                    byte.as_mut_ptr() as *mut c_void,
                                    1,
                                    index_c,
                                )
                            };
                            if rc != SQLITE_OK {
                                return Err(Self::sqlite_blob_error(db, rc));
                            }
                            out.push(byte[0]);
                        }
                        Some(out)
                    }
                }
                _ => {
                    let index = parsed_index.expect("non-slice branch precomputes index");
                    let Some(index) = Self::sqlite_blob_adjust_index(len, index) else {
                        return Err(sqlite_error("IndexError", "Blob index out of range"));
                    };
                    let mut byte = [0u8; 1];
                    let index_c = sqlite_len_to_c_int(index, "blob index")?;
                    // SAFETY: handle is open and byte points to one writable byte.
                    let rc = unsafe {
                        sqlite3_blob_read(handle, byte.as_mut_ptr() as *mut c_void, 1, index_c)
                    };
                    if rc != SQLITE_OK {
                        return Err(Self::sqlite_blob_error(db, rc));
                    }
                    return Ok(Value::Int(byte[0] as i64));
                }
            }
        };
        Ok(self
            .heap
            .alloc_bytes(blob_bytes.expect("slice branch always returns bytes")))
    }

    pub(in crate::vm) fn builtin_sqlite_blob_setitem(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 3 {
            return Err(sqlite_error(
                "TypeError",
                "Blob.__setitem__() expects index/slice and value",
            ));
        }
        let blob_id = self.sqlite_blob_id_from_value(&args[0], "__setitem__")?;
        {
            let (state, _) = self.sqlite_blob_state_and_db(blob_id)?;
            if state.handle().is_none() {
                return Err(sqlite_error(
                    "ProgrammingError",
                    "Cannot operate on a closed blob.",
                ));
            }
        }
        let key = args.remove(1);
        let replacement = args.remove(1);
        let op = match key {
            Value::Slice(slice) => {
                let payload = match replacement {
                    Value::MemoryView(obj) => {
                        if let Object::MemoryView(view) = &*obj.kind()
                            && !view.contiguous {
                                return Err(RuntimeError::new(
                                    "BufferError: underlying buffer is not C-contiguous",
                                ));
                            }
                        bytes_like_from_value(Value::MemoryView(obj)).map_err(|_| {
                            sqlite_error("TypeError", "a bytes-like object is required")
                        })?
                    }
                    other => bytes_like_from_value(other).map_err(|_| {
                        sqlite_error("TypeError", "a bytes-like object is required")
                    })?,
                };
                SqliteBlobSetOp::Slice {
                    lower: slice.lower,
                    upper: slice.upper,
                    step: slice.step,
                    payload,
                }
            }
            other => {
                let index = self.sqlite_blob_index_arg(other)?;
                let byte_value = match replacement {
                    Value::Int(_) | Value::Bool(_) | Value::BigInt(_) => value_to_int(replacement)
                        .map_err(|_| sqlite_error("ValueError", "byte must be in range(0, 256)"))?,
                    other => {
                        return Err(sqlite_error(
                            "TypeError",
                            format!(
                                "'{}' object cannot be interpreted as an integer",
                                self.value_type_name_for_error(&other)
                            ),
                        ));
                    }
                };
                let Ok(byte) = u8::try_from(byte_value) else {
                    return Err(sqlite_error("ValueError", "byte must be in range(0, 256)"));
                };
                SqliteBlobSetOp::Index(index, byte)
            }
        };
        let (state, db) = self.sqlite_blob_state_and_db(blob_id)?;
        let Some(handle) = state.handle() else {
            return Err(sqlite_error(
                "ProgrammingError",
                "Cannot operate on a closed blob.",
            ));
        };
        let len = Self::sqlite_blob_len(handle)?;
        match op {
            SqliteBlobSetOp::Slice {
                lower,
                upper,
                step,
                payload,
            } => {
                let indices = slice_indices(len, lower, upper, step)?;
                if payload.len() != indices.len() {
                    return Err(sqlite_error(
                        "IndexError",
                        "Blob slice assignment is wrong size",
                    ));
                }
                if payload.is_empty() {
                    return Ok(Value::None);
                }
                if indices
                    .windows(2)
                    .all(|window| window[1] == window[0].saturating_add(1))
                {
                    let start = indices[0];
                    let start_c = sqlite_len_to_c_int(start, "blob slice index")?;
                    let payload_len_c = sqlite_len_to_c_int(payload.len(), "blob write length")?;
                    // SAFETY: handle is open and payload pointer stays valid for this call.
                    let rc = unsafe {
                        sqlite3_blob_write(
                            handle,
                            payload.as_ptr() as *const c_void,
                            payload_len_c,
                            start_c,
                        )
                    };
                    if rc != SQLITE_OK {
                        return Err(Self::sqlite_blob_error(db, rc));
                    }
                } else {
                    for (index, byte) in indices.into_iter().zip(payload.into_iter()) {
                        let index_c = sqlite_len_to_c_int(index, "blob index")?;
                        let data = [byte];
                        // SAFETY: handle is open and data points to one readable byte.
                        let rc = unsafe {
                            sqlite3_blob_write(handle, data.as_ptr() as *const c_void, 1, index_c)
                        };
                        if rc != SQLITE_OK {
                            return Err(Self::sqlite_blob_error(db, rc));
                        }
                    }
                }
                Ok(Value::None)
            }
            SqliteBlobSetOp::Index(index, byte) => {
                let Some(index) = Self::sqlite_blob_adjust_index(len, index) else {
                    return Err(sqlite_error("IndexError", "Blob index out of range"));
                };
                let index_c = sqlite_len_to_c_int(index, "blob index")?;
                let data = [byte];
                // SAFETY: handle is open and data points to one readable byte.
                let rc = unsafe {
                    sqlite3_blob_write(handle, data.as_ptr() as *const c_void, 1, index_c)
                };
                if rc != SQLITE_OK {
                    return Err(Self::sqlite_blob_error(db, rc));
                }
                Ok(Value::None)
            }
        }
    }

    pub(in crate::vm) fn builtin_sqlite_blob_delitem(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error(
                "TypeError",
                "Blob.__delitem__() expects exactly one index argument",
            ));
        }
        match &args[1] {
            Value::Slice(_) => Err(sqlite_error(
                "TypeError",
                "Blob doesn't support slice deletion",
            )),
            _ => Err(sqlite_error(
                "TypeError",
                "Blob doesn't support item deletion",
            )),
        }
    }

    fn sqlite_row_data_tuple(instance: &ObjRef) -> Result<Value, RuntimeError> {
        let data = Self::instance_attr_get(instance, SQLITE_ROW_DATA_ATTR)
            .ok_or_else(|| sqlite_error("TypeError", "uninitialized Row object"))?;
        match data {
            Value::Tuple(_) => Ok(data),
            _ => Err(sqlite_error("TypeError", "Row data must be a tuple")),
        }
    }

    fn sqlite_row_description_value(instance: &ObjRef) -> Value {
        Self::instance_attr_get(instance, SQLITE_ROW_DESCRIPTION_ATTR).unwrap_or(Value::None)
    }

    fn sqlite_row_description_keys(description: &Value) -> Vec<String> {
        let Value::Tuple(columns) = description else {
            return Vec::new();
        };
        match &*columns.kind() {
            Object::Tuple(items) => items
                .iter()
                .filter_map(|item| match item {
                    Value::Tuple(entry) => match &*entry.kind() {
                        Object::Tuple(values) => match values.first() {
                            Some(Value::Str(name)) => Some(name.clone()),
                            _ => None,
                        },
                        _ => None,
                    },
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(in crate::vm) fn builtin_sqlite_row_init(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 3 {
            return Err(sqlite_error(
                "TypeError",
                "Row.__init__() expects cursor and tuple data",
            ));
        }
        let receiver = self.receiver_from_value(&args.remove(0))?;
        let cursor = args.remove(0);
        let data = args.remove(0);

        let cursor_obj = match cursor {
            Value::Instance(instance) => {
                if self.sqlite_cursors.contains_key(&instance.id()) {
                    instance
                } else {
                    return Err(sqlite_error(
                        "TypeError",
                        format!(
                            "Row() argument 1 must be sqlite3.Cursor, not {}",
                            self.value_type_name_for_error(&Value::Instance(instance))
                        ),
                    ));
                }
            }
            other => {
                return Err(sqlite_error(
                    "TypeError",
                    format!(
                        "Row() argument 1 must be sqlite3.Cursor, not {}",
                        self.value_type_name_for_error(&other)
                    ),
                ));
            }
        };

        let data_tuple = match data {
            Value::Tuple(tuple_obj) => Value::Tuple(tuple_obj),
            other => {
                return Err(sqlite_error(
                    "TypeError",
                    format!(
                        "Row() argument 2 must be tuple, not {}",
                        self.value_type_name_for_error(&other)
                    ),
                ));
            }
        };
        let description =
            Self::instance_attr_get(&cursor_obj, "description").unwrap_or(Value::None);
        Self::instance_attr_set(&receiver, SQLITE_ROW_DATA_ATTR, data_tuple)?;
        Self::instance_attr_set(&receiver, SQLITE_ROW_DESCRIPTION_ATTR, description)?;
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_row_keys(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error("TypeError", "Row.keys() expects no arguments"));
        }
        let receiver = self.receiver_from_value(&args[0])?;
        let description = Self::sqlite_row_description_value(&receiver);
        if matches!(description, Value::None) {
            return Ok(self.heap.alloc_list(Vec::new()));
        }
        let keys = Self::sqlite_row_description_keys(&description)
            .into_iter()
            .map(Value::Str)
            .collect();
        Ok(self.heap.alloc_list(keys))
    }

    pub(in crate::vm) fn builtin_sqlite_row_len(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Row.__len__() expects no arguments",
            ));
        }
        let receiver = self.receiver_from_value(&args[0])?;
        let data = Self::sqlite_row_data_tuple(&receiver)?;
        self.builtin_len(vec![data], HashMap::new())
    }

    pub(in crate::vm) fn builtin_sqlite_row_getitem(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error(
                "TypeError",
                "Row.__getitem__() expects one key argument",
            ));
        }
        let receiver = self.receiver_from_value(&args[0])?;
        let data = Self::sqlite_row_data_tuple(&receiver)?;
        let key = args[1].clone();

        match &key {
            Value::Int(_) | Value::Slice(_) => {
                self.builtin_operator_getitem(vec![data, key], HashMap::new())
            }
            Value::Str(name) => {
                let description = Self::sqlite_row_description_value(&receiver);
                if matches!(description, Value::None) {
                    return Err(sqlite_error(
                        "IndexError",
                        format!("No item with key {name:?}"),
                    ));
                }
                let keys = Self::sqlite_row_description_keys(&description);
                if let Some(index) = keys
                    .iter()
                    .position(|candidate| candidate.eq_ignore_ascii_case(name))
                {
                    self.builtin_operator_getitem(
                        vec![data, Value::Int(index as i64)],
                        HashMap::new(),
                    )
                } else {
                    Err(sqlite_error("IndexError", "No item with that key"))
                }
            }
            _ => Err(sqlite_error("IndexError", "Index must be int or string")),
        }
    }

    pub(in crate::vm) fn builtin_sqlite_row_iter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Row.__iter__() expects no arguments",
            ));
        }
        let receiver = self.receiver_from_value(&args[0])?;
        let data = Self::sqlite_row_data_tuple(&receiver)?;
        self.builtin_iter(vec![data], HashMap::new())
    }

    pub(in crate::vm) fn builtin_sqlite_row_eq(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error(
                "TypeError",
                "Row.__eq__() expects one argument",
            ));
        }
        let left = self.receiver_from_value(&args[0])?;
        let right = match &args[1] {
            Value::Instance(instance) => instance.clone(),
            _ => return Ok(Value::Bool(false)),
        };
        let left_data = Self::sqlite_row_data_tuple(&left)?;
        let right_data = match Self::sqlite_row_data_tuple(&right) {
            Ok(value) => value,
            Err(_) => return Ok(Value::Bool(false)),
        };
        let left_desc = Self::sqlite_row_description_value(&left);
        let right_desc = Self::sqlite_row_description_value(&right);
        let desc_eq_value = self.compare_eq_runtime(left_desc, right_desc)?;
        let desc_equal = self.truthy_from_value(&desc_eq_value)?;
        let data_eq_value = self.compare_eq_runtime(left_data, right_data)?;
        let data_equal = self.truthy_from_value(&data_eq_value)?;
        Ok(Value::Bool(desc_equal && data_equal))
    }

    pub(in crate::vm) fn builtin_sqlite_row_hash(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Row.__hash__() expects no arguments",
            ));
        }
        let receiver = self.receiver_from_value(&args[0])?;
        let description = Self::sqlite_row_description_value(&receiver);
        let data = Self::sqlite_row_data_tuple(&receiver)?;
        let description_hash = match self.builtin_hash(vec![description], HashMap::new())? {
            Value::Int(value) => value,
            other => value_to_int(other)?,
        };
        let data_hash = match self.builtin_hash(vec![data], HashMap::new())? {
            Value::Int(value) => value,
            other => value_to_int(other)?,
        };
        Ok(Value::Int(description_hash ^ data_hash))
    }

    pub(in crate::vm) fn builtin_sqlite_cursor_setattr(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 3 {
            return Err(sqlite_error(
                "TypeError",
                "Cursor.__setattr__() expects three arguments",
            ));
        }
        let receiver = args.remove(0);
        let name = match args.remove(0) {
            Value::Str(name) => name,
            _ => return Err(sqlite_error("TypeError", "attribute name must be string")),
        };
        let value = args.remove(0);
        if name == "arraysize" {
            let parsed = sqlite_non_negative_u32(
                value,
                "arraysize must be an integer",
                "arraysize must be non-negative",
                "arraysize value is too large",
            )?;
            return self.builtin_object_setattr(
                vec![receiver, Value::Str(name), Value::Int(parsed)],
                HashMap::new(),
            );
        }
        self.builtin_object_setattr(vec![receiver, Value::Str(name), value], HashMap::new())
    }

    pub(in crate::vm) fn builtin_sqlite_cursor_setinputsizes(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error(
                "TypeError",
                "Cursor.setinputsizes() expects one argument",
            ));
        }
        let cursor_id = self.sqlite_cursor_id_from_value(&args[0], "setinputsizes")?;
        let connection_id = self.sqlite_cursor_ensure_thread_affinity(cursor_id)?;
        if let Some(state) = self.sqlite_cursors.get(&cursor_id)
            && state.closed {
                return Err(self.sqlite_cursor_closed_runtime_error(connection_id));
            }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_cursor_setoutputsize(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() < 2 || args.len() > 3 {
            return Err(sqlite_error(
                "TypeError",
                "Cursor.setoutputsize() expects one or two arguments",
            ));
        }
        let cursor_id = self.sqlite_cursor_id_from_value(&args[0], "setoutputsize")?;
        let connection_id = self.sqlite_cursor_ensure_thread_affinity(cursor_id)?;
        if let Some(state) = self.sqlite_cursors.get(&cursor_id)
            && state.closed {
                return Err(self.sqlite_cursor_closed_runtime_error(connection_id));
            }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_cursor_execute(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(sqlite_error(
                "TypeError",
                "Cursor.execute() does not accept keyword arguments",
            ));
        }
        if args.len() < 2 || args.len() > 3 {
            return Err(sqlite_error(
                "TypeError",
                "Cursor.execute() expects SQL and optional parameters",
            ));
        }
        let receiver = args.remove(0);
        let cursor_id = self.sqlite_cursor_id_from_value(&receiver, "execute")?;
        let sql = match args.remove(0) {
            Value::Str(text) => text,
            _ => {
                return Err(sqlite_error(
                    "TypeError",
                    "execute() argument 1 must be str",
                ));
            }
        };
        let params = if args.is_empty() {
            SqliteParams::Positional(Vec::new())
        } else {
            self.sqlite_extract_params(args.remove(0))?
        };
        let connection_id = self.sqlite_cursor_ensure_thread_affinity(cursor_id)?;
        {
            let state = self
                .sqlite_cursors
                .get(&cursor_id)
                .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite cursor"))?;
            if state.closed {
                return Err(self.sqlite_cursor_closed_runtime_error(state.connection_id));
            }
        }
        let is_dml = sqlite_is_dml_statement(&sql);
        self.sqlite_maybe_begin_legacy_transaction(connection_id, &sql)?;
        let query_result = self.sqlite_execute_query(connection_id, &sql, params)?;
        let db = self.sqlite_open_db_handle(connection_id)?;
        // SAFETY: db is a valid sqlite handle.
        let rowcount = if is_dml {
            unsafe { sqlite3_changes(db) as i64 }
        } else {
            -1
        };
        // SAFETY: db is a valid sqlite handle.
        let lastrowid = unsafe { sqlite3_last_insert_rowid(db) };
        if let Some(state) = self.sqlite_cursors.get_mut(&cursor_id) {
            state.rows = query_result.rows;
            state.next_row = 0;
            state.description = query_result.description.clone();
            state.closed = false;
        }
        let receiver_obj = self.receiver_from_value(&receiver)?;
        let _ = Self::instance_attr_set(
            &receiver_obj,
            "description",
            query_result.description.unwrap_or(Value::None),
        );
        let _ = Self::instance_attr_set(&receiver_obj, "rowcount", Value::Int(rowcount));
        let _ = Self::instance_attr_set(&receiver_obj, "lastrowid", Value::Int(lastrowid));
        Ok(receiver)
    }

    pub(in crate::vm) fn builtin_sqlite_cursor_executemany(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(sqlite_error(
                "TypeError",
                "Cursor.executemany() does not accept keyword arguments",
            ));
        }
        if args.len() != 3 {
            return Err(sqlite_error(
                "TypeError",
                "Cursor.executemany() expects SQL and parameter iterable",
            ));
        }
        let receiver = args.remove(0);
        let cursor_id = self.sqlite_cursor_id_from_value(&receiver, "executemany")?;
        let sql = match args.remove(0) {
            Value::Str(text) => text,
            _ => {
                return Err(sqlite_error(
                    "TypeError",
                    "executemany() argument 1 must be str",
                ));
            }
        };
        if !sqlite_is_dml_statement(&sql) {
            return Err(sqlite_error(
                "ProgrammingError",
                "executemany() can only execute DML statements.",
            ));
        }
        let parameter_sets = self.collect_iterable_values(args.remove(0)).map_err(|_| {
            sqlite_error(
                "TypeError",
                "executemany() second argument must be iterable",
            )
        })?;
        let connection_id = self.sqlite_cursor_ensure_thread_affinity(cursor_id)?;
        {
            let state = self
                .sqlite_cursors
                .get(&cursor_id)
                .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite cursor"))?;
            if state.closed {
                return Err(self.sqlite_cursor_closed_runtime_error(state.connection_id));
            }
        }
        self.sqlite_maybe_begin_legacy_transaction(connection_id, &sql)?;
        let mut rowcount_total: i64 = 0;

        let mut last_result = SqliteQueryResult {
            rows: Vec::new(),
            description: None,
        };
        for param_set in parameter_sets {
            let params = self.sqlite_extract_params(param_set)?;
            last_result = self.sqlite_execute_query(connection_id, &sql, params)?;
            let db = self.sqlite_open_db_handle(connection_id)?;
            // SAFETY: db is a valid sqlite handle.
            rowcount_total += unsafe { sqlite3_changes(db) as i64 };
        }
        if let Some(state) = self.sqlite_cursors.get_mut(&cursor_id) {
            state.rows = last_result.rows;
            state.next_row = 0;
            state.description = last_result.description.clone();
            state.closed = false;
        }
        let receiver_obj = self.receiver_from_value(&receiver)?;
        let _ = Self::instance_attr_set(
            &receiver_obj,
            "description",
            last_result.description.unwrap_or(Value::None),
        );
        let _ = Self::instance_attr_set(&receiver_obj, "rowcount", Value::Int(rowcount_total));
        Ok(receiver)
    }

    pub(in crate::vm) fn builtin_sqlite_cursor_executescript(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(sqlite_error(
                "TypeError",
                "Cursor.executescript() expects one SQL script argument",
            ));
        }
        let receiver = args.remove(0);
        let cursor_id = self.sqlite_cursor_id_from_value(&receiver, "executescript")?;
        let script = match args.remove(0) {
            Value::Str(text) => text,
            _ => {
                return Err(sqlite_error(
                    "TypeError",
                    "executescript() argument must be str",
                ));
            }
        };
        let connection_id = self.sqlite_cursor_ensure_thread_affinity(cursor_id)?;
        {
            let state = self
                .sqlite_cursors
                .get(&cursor_id)
                .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite cursor"))?;
            if state.closed {
                return Err(self.sqlite_cursor_closed_runtime_error(state.connection_id));
            }
        }
        self.sqlite_execute_script(connection_id, &script)?;
        if let Some(state) = self.sqlite_cursors.get_mut(&cursor_id) {
            state.rows.clear();
            state.next_row = 0;
            state.description = None;
            state.closed = false;
        }
        let receiver_obj = self.receiver_from_value(&receiver)?;
        let _ = Self::instance_attr_set(&receiver_obj, "description", Value::None);
        Ok(receiver)
    }

    pub(in crate::vm) fn builtin_sqlite_cursor_fetchmany(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(sqlite_error(
                "TypeError",
                "Cursor.fetchmany() expects optional size argument",
            ));
        }
        let cursor_id = self.sqlite_cursor_id_from_value(&args[0], "fetchmany")?;
        let size = if args.len() == 2 {
            sqlite_non_negative_u32(
                args.remove(1),
                "fetchmany() size argument must be integer",
                "fetchmany() size must be non-negative",
                "fetchmany() size is too large",
            )?
        } else if let Some(size_kw) = kwargs.remove("size") {
            sqlite_non_negative_u32(
                size_kw,
                "fetchmany() size must be integer",
                "fetchmany() size must be non-negative",
                "fetchmany() size is too large",
            )?
        } else {
            let receiver = self.receiver_from_value(&args[0])?;
            let arraysize =
                Self::instance_attr_get(&receiver, "arraysize").unwrap_or(Value::Int(1));
            sqlite_non_negative_u32(
                arraysize,
                "arraysize must be an integer",
                "arraysize must be non-negative",
                "arraysize value is too large",
            )
            .unwrap_or(1)
        };
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(sqlite_error(
                "TypeError",
                format!("fetchmany() got an unexpected keyword argument '{unexpected}'"),
            ));
        }
        let _connection_id = self.sqlite_cursor_ensure_thread_affinity(cursor_id)?;
        let raw_rows = {
            let state = self
                .sqlite_cursors
                .get_mut(&cursor_id)
                .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite cursor"))?;
            if state.closed {
                let connection_id = state.connection_id;
                let _ = state;
                return Err(self.sqlite_cursor_closed_runtime_error(connection_id));
            }
            if state.next_row >= state.rows.len() {
                return Ok(self.heap.alloc_list(Vec::new()));
            }
            let take = usize::try_from(size).unwrap_or(usize::MAX);
            let end = state.next_row.saturating_add(take).min(state.rows.len());
            let out = state.rows[state.next_row..end].to_vec();
            state.next_row = end;
            out
        };
        let mut materialized = Vec::with_capacity(raw_rows.len());
        for raw_row in raw_rows {
            materialized.push(self.sqlite_materialize_row_for_cursor(&args[0], raw_row)?);
        }
        Ok(self.heap.alloc_list(materialized))
    }

    pub(in crate::vm) fn builtin_sqlite_cursor_fetchone(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Cursor.fetchone() expects no arguments",
            ));
        }
        let cursor_id = self.sqlite_cursor_id_from_value(&args[0], "fetchone")?;
        let _connection_id = self.sqlite_cursor_ensure_thread_affinity(cursor_id)?;
        let raw_row = {
            let state = self
                .sqlite_cursors
                .get_mut(&cursor_id)
                .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite cursor"))?;
            if state.closed {
                let connection_id = state.connection_id;
                let _ = state;
                return Err(self.sqlite_cursor_closed_runtime_error(connection_id));
            }
            if state.next_row >= state.rows.len() {
                return Ok(Value::None);
            }
            let value = state.rows[state.next_row].clone();
            state.next_row += 1;
            value
        };
        self.sqlite_materialize_row_for_cursor(&args[0], raw_row)
    }

    pub(in crate::vm) fn builtin_sqlite_cursor_fetchall(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Cursor.fetchall() expects no arguments",
            ));
        }
        let cursor_id = self.sqlite_cursor_id_from_value(&args[0], "fetchall")?;
        let _connection_id = self.sqlite_cursor_ensure_thread_affinity(cursor_id)?;
        let raw_rows = {
            let state = self
                .sqlite_cursors
                .get_mut(&cursor_id)
                .ok_or_else(|| sqlite_error("ProgrammingError", "invalid sqlite cursor"))?;
            if state.closed {
                let connection_id = state.connection_id;
                let _ = state;
                return Err(self.sqlite_cursor_closed_runtime_error(connection_id));
            }
            if state.next_row >= state.rows.len() {
                return Ok(self.heap.alloc_list(Vec::new()));
            }
            let remaining = state.rows[state.next_row..].to_vec();
            state.next_row = state.rows.len();
            remaining
        };
        let mut materialized = Vec::with_capacity(raw_rows.len());
        for raw_row in raw_rows {
            materialized.push(self.sqlite_materialize_row_for_cursor(&args[0], raw_row)?);
        }
        Ok(self.heap.alloc_list(materialized))
    }

    pub(in crate::vm) fn builtin_sqlite_cursor_close(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Cursor.close() expects no arguments",
            ));
        }
        let cursor_id = self.sqlite_cursor_id_from_value(&args[0], "close")?;
        let _connection_id = self.sqlite_cursor_ensure_thread_affinity(cursor_id)?;
        if let Some(state) = self.sqlite_cursors.get_mut(&cursor_id) {
            state.closed = true;
            state.rows.clear();
            state.next_row = 0;
            state.description = None;
        }
        let receiver = self.receiver_from_value(&args[0])?;
        let _ = Self::instance_attr_set(&receiver, "description", Value::None);
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_cursor_iter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Cursor.__iter__() expects no arguments",
            ));
        }
        Ok(args.remove(0))
    }

    pub(in crate::vm) fn builtin_sqlite_cursor_next(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let value = self.builtin_sqlite_cursor_fetchone(args, kwargs)?;
        if matches!(value, Value::None) {
            return Err(RuntimeError::new("StopIteration"));
        }
        Ok(value)
    }
}
