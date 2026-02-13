use super::super::*;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uchar, c_void};
use std::ptr::{self, NonNull};

#[repr(C)]
pub(in crate::vm) struct Sqlite3Db {
    _private: [u8; 0],
}

#[repr(C)]
struct Sqlite3Stmt {
    _private: [u8; 0],
}

#[repr(C)]
struct Sqlite3Blob {
    _private: [u8; 0],
}

type SqliteDestructor = Option<unsafe extern "C" fn(*mut c_void)>;

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
    fn sqlite3_bind_parameter_count(stmt: *mut Sqlite3Stmt) -> c_int;
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
    fn sqlite3_blob_read(blob: *mut Sqlite3Blob, buf: *mut c_void, n: c_int, offset: c_int)
        -> c_int;
    fn sqlite3_blob_write(
        blob: *mut Sqlite3Blob,
        buf: *const c_void,
        n: c_int,
        offset: c_int,
    ) -> c_int;
}

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;
const SQLITE_INTEGER: c_int = 1;
const SQLITE_FLOAT: c_int = 2;
const SQLITE_TEXT: c_int = 3;
const SQLITE_BLOB: c_int = 4;
const SQLITE_OPEN_READWRITE: c_int = 0x0000_0002;
const SQLITE_OPEN_CREATE: c_int = 0x0000_0004;
const SQLITE_OPEN_URI: c_int = 0x0000_0040;

#[derive(Debug)]
pub(in crate::vm) struct SqliteConnectionState {
    handle: Option<NonNull<Sqlite3Db>>,
}

impl SqliteConnectionState {
    pub(in crate::vm) fn new(handle: *mut Sqlite3Db) -> Self {
        Self {
            handle: NonNull::new(handle),
        }
    }

    pub(in crate::vm) fn db_handle(&self) -> Option<*mut Sqlite3Db> {
        self.handle.map(NonNull::as_ptr)
    }

    pub(in crate::vm) fn close(&mut self) -> Result<(), String> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        // SAFETY: handle was created by sqlite3_open_v2 and is owned by this state.
        let rc = unsafe { sqlite3_close_v2(handle.as_ptr()) };
        if rc == SQLITE_OK {
            Ok(())
        } else {
            Err(format!("sqlite3_close_v2 failed with code {rc}"))
        }
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
    pub(in crate::vm) closed: bool,
}

impl SqliteCursorState {
    fn new(connection_id: u64) -> Self {
        Self {
            connection_id,
            rows: Vec::new(),
            next_row: 0,
            closed: false,
        }
    }
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
        let rc = unsafe { sqlite3_blob_close(handle.as_ptr()) };
        if rc == SQLITE_OK {
            Ok(())
        } else {
            Err(format!("sqlite3_blob_close failed with code {rc}"))
        }
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
    unsafe { CStr::from_ptr(tail).to_bytes() }
        .iter()
        .any(|byte| !byte.is_ascii_whitespace())
}

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
            _ => Err(RuntimeError::new(format!(
                "_sqlite3.{name} must be a dict"
            ))),
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

    fn sqlite_connection_id_from_value(
        &self,
        value: &Value,
        method_name: &str,
    ) -> Result<u64, RuntimeError> {
        let receiver = self.receiver_from_value(value)?;
        let receiver_id = receiver.id();
        if self.sqlite_connections.contains_key(&receiver_id) {
            Ok(receiver_id)
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
        if self.sqlite_cursors.contains_key(&receiver_id) {
            Ok(receiver_id)
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
        state.db_handle().ok_or_else(|| {
            sqlite_error(
                "ProgrammingError",
                "Cannot operate on a closed database.",
            )
        })
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
            if abs > len {
                None
            } else {
                Some(len - abs)
            }
        }
    }

    fn sqlite_blob_error(db: *mut Sqlite3Db, rc: c_int) -> RuntimeError {
        let mut message = sqlite_last_error_message(db);
        if message.is_empty() {
            message = format!("sqlite3 blob operation failed with code {rc}");
        }
        sqlite_error("OperationalError", message)
    }

    fn sqlite_extract_database(value: Value) -> Result<String, RuntimeError> {
        match value {
            Value::Str(text) => Ok(text),
            Value::Bytes(obj) | Value::ByteArray(obj) => match &*obj.kind() {
                Object::Bytes(bytes) | Object::ByteArray(bytes) => {
                    Ok(String::from_utf8_lossy(bytes).into_owned())
                }
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

    fn sqlite_extract_params(&self, value: Value) -> Result<Vec<Value>, RuntimeError> {
        match value {
            Value::None => Ok(Vec::new()),
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(items) => Ok(items.clone()),
                _ => Err(sqlite_error(
                    "ProgrammingError",
                    "parameters are of unsupported type",
                )),
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(items) => Ok(items.clone()),
                _ => Err(sqlite_error(
                    "ProgrammingError",
                    "parameters are of unsupported type",
                )),
            },
            Value::Dict(_) => Err(sqlite_error(
                "ProgrammingError",
                "named parameters are not supported yet",
            )),
            _ => Err(sqlite_error(
                "ProgrammingError",
                "parameters are of unsupported type",
            )),
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
            Err(sqlite_error(
                "OperationalError",
                sqlite_last_error_message(db),
            ))
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
                            Value::Str(String::new())
                        } else {
                            let slice =
                                std::slice::from_raw_parts(text_ptr, usize::try_from(len).unwrap_or(0));
                            Value::Str(String::from_utf8_lossy(slice).into_owned())
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

    fn sqlite_execute_query(
        &mut self,
        connection_id: u64,
        sql: &str,
        params: Vec<Value>,
    ) -> Result<Vec<Value>, RuntimeError> {
        let db = self.sqlite_open_db_handle(connection_id)?;
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
            return Err(sqlite_error(
                "OperationalError",
                sqlite_last_error_message(db),
            ));
        }
        let Some(stmt_ptr) = NonNull::new(raw_stmt) else {
            return Ok(Vec::new());
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
        if expected_params != params.len() as i32 {
            return Err(sqlite_error(
                "ProgrammingError",
                format!(
                    "Incorrect number of bindings supplied. The current statement uses {expected_params}, and there are {} supplied.",
                    params.len()
                ),
            ));
        }

        let mut text_buffers = Vec::new();
        let mut blob_buffers = Vec::new();
        for (index, value) in params.iter().enumerate() {
            self.sqlite_bind_value(
                db,
                statement.as_ptr(),
                index,
                value,
                &mut text_buffers,
                &mut blob_buffers,
            )?;
        }

        // SAFETY: statement pointer is valid while statement wrapper is alive.
        let column_count = unsafe { sqlite3_column_count(statement.as_ptr()) };
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
                    return Err(sqlite_error(
                        "OperationalError",
                        sqlite_last_error_message(db),
                    ));
                }
            }
        }
        Ok(rows)
    }

    pub(in crate::vm) fn builtin_sqlite_connect(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let database = if args.is_empty() {
            kwargs
                .remove("database")
                .ok_or_else(|| sqlite_error("TypeError", "connect() missing required argument 'database'"))?
        } else {
            args.remove(0)
        };
        if !args.is_empty() {
            return Err(sqlite_error(
                "TypeError",
                format!(
                    "connect() takes 1 positional argument but {} were given",
                    args.len() + 1
                ),
            ));
        }

        let _timeout = kwargs.remove("timeout");
        let _detect_types = kwargs.remove("detect_types");
        let _isolation_level = kwargs.remove("isolation_level");
        let _check_same_thread = kwargs.remove("check_same_thread");
        let factory = kwargs.remove("factory");
        let _cached_statements = kwargs.remove("cached_statements");
        let uri = kwargs.remove("uri").map(|value| is_truthy(&value)).unwrap_or(false);
        let _autocommit = kwargs.remove("autocommit");
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(sqlite_error(
                "TypeError",
                format!("connect() got an unexpected keyword argument '{unexpected}'"),
            ));
        }

        let database = Self::sqlite_extract_database(database)?;
        let db_path = CString::new(database.as_bytes())
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
            return Err(sqlite_error("OperationalError", message));
        }

        let class = match factory {
            Some(Value::Class(class_ref)) => class_ref,
            Some(_) => {
                if !handle.is_null() {
                    // SAFETY: handle is live and owned by this method before state insertion.
                    unsafe {
                        let _ = sqlite3_close_v2(handle);
                    }
                }
                return Err(sqlite_error(
                    "TypeError",
                    "factory must be a Connection subclass",
                ));
            }
            None => self.sqlite_connection_class()?,
        };
        let connection = self.alloc_instance_for_class(&class);
        if let Object::Instance(instance_data) = &mut *connection.kind_mut() {
            instance_data
                .attrs
                .insert("in_transaction".to_string(), Value::Bool(false));
        }
        self.sqlite_connections
            .insert(connection.id(), SqliteConnectionState::new(handle));
        Ok(Value::Instance(connection))
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
        Ok(Value::Bool(unsafe { sqlite3_complete(statement_c.as_ptr()) != 0 }))
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
        dict_set_value_checked(&converters, Value::Str(name.to_ascii_uppercase()), converter)?;
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_enable_callback_tracebacks(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "enable_callback_tracebacks() expects exactly one argument",
            ));
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_cursor(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.cursor() expects no positional arguments",
            ));
        }
        let connection_id = self.sqlite_connection_id_from_value(&args[0], "cursor")?;
        let _ = self.sqlite_open_db_handle(connection_id)?;
        let class = match kwargs.remove("factory") {
            Some(Value::Class(class_ref)) => class_ref,
            Some(_) => {
                return Err(sqlite_error(
                    "TypeError",
                    "cursor() factory must be a Cursor subclass",
                ));
            }
            None => self.sqlite_cursor_class()?,
        };
        if let Some(unexpected) = kwargs.keys().next() {
            return Err(sqlite_error(
                "TypeError",
                format!("cursor() got an unexpected keyword argument '{unexpected}'"),
            ));
        }
        let cursor = self.alloc_instance_for_class(&class);
        if let Object::Instance(instance_data) = &mut *cursor.kind_mut() {
            instance_data
                .attrs
                .insert("rowcount".to_string(), Value::Int(-1));
        }
        self.sqlite_cursors
            .insert(cursor.id(), SqliteCursorState::new(connection_id));
        Ok(Value::Instance(cursor))
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
        if let Some(state) = self.sqlite_connections.get_mut(&connection_id) {
            state.close().map_err(|message| {
                sqlite_error("OperationalError", message)
            })?;
        }
        for cursor in self.sqlite_cursors.values_mut() {
            if cursor.connection_id == connection_id {
                cursor.closed = true;
                cursor.rows.clear();
                cursor.next_row = 0;
            }
        }
        for blob in self.sqlite_blobs.values_mut() {
            if blob.connection_id == connection_id {
                let _ = blob.close();
            }
        }
        Ok(Value::None)
    }

    pub(in crate::vm) fn builtin_sqlite_connection_execute(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 {
            return Err(sqlite_error(
                "TypeError",
                "Connection.execute() missing SQL argument",
            ));
        }
        let receiver = args.remove(0);
        let cursor = self.builtin_sqlite_connection_cursor(vec![receiver], HashMap::new())?;
        let mut cursor_args = vec![cursor.clone()];
        cursor_args.extend(args);
        self.builtin_sqlite_cursor_execute(cursor_args, kwargs)?;
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
        match self.sqlite_execute_query(connection_id, "COMMIT", Vec::new()) {
            Ok(_) => Ok(Value::None),
            Err(err) if err.message.contains("no transaction is active") => Ok(Value::None),
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
        match self.sqlite_execute_query(connection_id, "ROLLBACK", Vec::new()) {
            Ok(_) => Ok(Value::None),
            Err(err) if err.message.contains("no transaction is active") => Ok(Value::None),
            Err(err) => Err(err),
        }
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
                sqlite_error("OverflowError", "row id too large to fit into 64-bit integer")
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

        let db_name_c = CString::new(name.as_bytes())
            .map_err(|_| sqlite_error("ProgrammingError", "blob database name contains embedded NUL"))?;
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
            return Err(sqlite_error("TypeError", "Blob.close() expects no arguments"));
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
            return Err(sqlite_error("TypeError", "Blob.write() expects bytes-like data"));
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
        let base = match origin {
            0 => 0i64,
            1 => i64::try_from(state.offset)
                .map_err(|_| sqlite_error("OverflowError", "seek offset results in overflow"))?,
            2 => i64::try_from(len)
                .map_err(|_| sqlite_error("OverflowError", "seek offset results in overflow"))?,
            _ => {
                return Err(sqlite_error(
                    "ValueError",
                    "'origin' should be os.SEEK_SET, os.SEEK_CUR, or os.SEEK_END",
                ));
            }
        };
        let Some(new_offset) = base.checked_add(offset) else {
            return Err(sqlite_error(
                "OverflowError",
                "seek offset results in overflow",
            ));
        };
        if new_offset < 0
            || usize::try_from(new_offset)
                .ok()
                .is_none_or(|position| position > len)
        {
            return Err(sqlite_error("ValueError", "offset out of blob range"));
        }
        state.offset = usize::try_from(new_offset)
            .map_err(|_| sqlite_error("OverflowError", "seek offset results in overflow"))?;
        Ok(Value::Int(new_offset))
    }

    pub(in crate::vm) fn builtin_sqlite_blob_tell(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error("TypeError", "Blob.tell() expects no arguments"));
        }
        let blob_id = self.sqlite_blob_id_from_value(&args[0], "tell")?;
        let (state, _) = self.sqlite_blob_state_and_db(blob_id)?;
        if state.handle().is_none() {
            return Err(sqlite_error(
                "ProgrammingError",
                "Cannot operate on a closed blob.",
            ));
        }
        Ok(Value::Int(
            i64::try_from(state.offset).map_err(|_| sqlite_error("OverflowError", "offset too large"))?,
        ))
    }

    pub(in crate::vm) fn builtin_sqlite_blob_enter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(sqlite_error("TypeError", "Blob.__enter__() expects no arguments"));
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
            return Err(sqlite_error("TypeError", "Blob.__len__() expects no arguments"));
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
            other => Some(self.io_index_arg_to_int(other.clone())?),
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
                return Ok(Value::Int(byte[0] as i64));
            }
            }
        };
        Ok(self.heap.alloc_bytes(
            blob_bytes.expect("slice branch always returns bytes"),
        ))
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
                let payload = bytes_like_from_value(replacement)
                    .map_err(|_| sqlite_error("TypeError", "a bytes-like object is required"))?;
                SqliteBlobSetOp::Slice {
                    lower: slice.lower,
                    upper: slice.upper,
                    step: slice.step,
                    payload,
                }
            }
            other => {
                let index = self.io_index_arg_to_int(other)?;
                let byte_value = match replacement {
                    Value::Int(_) | Value::Bool(_) | Value::BigInt(_) => {
                        value_to_int(replacement).map_err(|_| {
                            sqlite_error("ValueError", "byte must be in range(0, 256)")
                        })?
                    }
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
                            sqlite3_blob_write(
                                handle,
                                data.as_ptr() as *const c_void,
                                1,
                                index_c,
                            )
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
                    sqlite3_blob_write(
                        handle,
                        data.as_ptr() as *const c_void,
                        1,
                        index_c,
                    )
                };
                if rc != SQLITE_OK {
                    return Err(Self::sqlite_blob_error(db, rc));
                }
                Ok(Value::None)
            }
        }
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
            Vec::new()
        } else {
            self.sqlite_extract_params(args.remove(0))?
        };
        let connection_id = {
            let state = self.sqlite_cursors.get(&cursor_id).ok_or_else(|| {
                sqlite_error("ProgrammingError", "invalid sqlite cursor")
            })?;
            if state.closed {
                return Err(sqlite_error(
                    "ProgrammingError",
                    "Cannot operate on a closed cursor.",
                ));
            }
            state.connection_id
        };
        let rows = self.sqlite_execute_query(connection_id, &sql, params)?;
        if let Some(state) = self.sqlite_cursors.get_mut(&cursor_id) {
            state.rows = rows;
            state.next_row = 0;
            state.closed = false;
        }
        Ok(receiver)
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
        let state = self.sqlite_cursors.get_mut(&cursor_id).ok_or_else(|| {
            sqlite_error("ProgrammingError", "invalid sqlite cursor")
        })?;
        if state.closed {
            return Err(sqlite_error(
                "ProgrammingError",
                "Cannot operate on a closed cursor.",
            ));
        }
        if state.next_row >= state.rows.len() {
            return Ok(Value::None);
        }
        let value = state.rows[state.next_row].clone();
        state.next_row += 1;
        Ok(value)
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
        let state = self.sqlite_cursors.get_mut(&cursor_id).ok_or_else(|| {
            sqlite_error("ProgrammingError", "invalid sqlite cursor")
        })?;
        if state.closed {
            return Err(sqlite_error(
                "ProgrammingError",
                "Cannot operate on a closed cursor.",
            ));
        }
        if state.next_row >= state.rows.len() {
            return Ok(self.heap.alloc_list(Vec::new()));
        }
        let remaining = state.rows[state.next_row..].to_vec();
        state.next_row = state.rows.len();
        Ok(self.heap.alloc_list(remaining))
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
        if let Some(state) = self.sqlite_cursors.get_mut(&cursor_id) {
            state.closed = true;
            state.rows.clear();
            state.next_row = 0;
        }
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
