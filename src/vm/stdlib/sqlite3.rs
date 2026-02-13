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
