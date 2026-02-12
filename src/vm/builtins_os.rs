use super::*;

const CODECS_ATTR_ENCODING: &str = "__pyrs_codec_encoding__";
const CODECS_ATTR_ERRORS: &str = "__pyrs_codec_errors__";
const CODECS_ATTR_PENDING: &str = "__pyrs_codec_pending__";
const CODECS_ATTR_STATE_FLAG: &str = "__pyrs_codec_state_flag__";

impl Vm {
    pub(super) fn builtin_os_getcwd(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("getcwd() expects no arguments"));
        }
        let cwd = std::env::current_dir()
            .map_err(|err| RuntimeError::new(format!("getcwd failed: {err}")))?;
        Ok(Value::Str(cwd.to_string_lossy().to_string()))
    }

    pub(super) fn builtin_os_getpid(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("getpid() expects no arguments"));
        }
        Ok(Value::Int(std::process::id() as i64))
    }

    pub(super) fn builtin_os_getenv(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::new(
                "os.getenv expects key and optional default",
            ));
        }
        let key = if let Some(value) = kwargs.remove("key") {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "os.getenv() got multiple values for argument 'key'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new(
                "os.getenv() missing required argument 'key'",
            ));
        };
        let default = if let Some(value) = kwargs.remove("default") {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "os.getenv() got multiple values for argument 'default'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            Value::None
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "os.getenv() got unexpected keyword argument",
            ));
        }
        let key = match key {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("os.getenv() key must be str")),
        };
        // Mirror CPython behavior: getenv() must observe os.environ mutations
        // performed in-process by Python code.
        let lookup = Value::Str(key.clone());
        for module_name in ["os", "posix"] {
            let Some(module_obj) = self.modules.get(module_name) else {
                continue;
            };
            let module_kind = module_obj.kind();
            let Object::Module(module_data) = &*module_kind else {
                continue;
            };
            let Some(Value::Dict(environ_obj)) = module_data.globals.get("environ") else {
                continue;
            };
            if let Some(value) = dict_get_value(environ_obj, &lookup) {
                return Ok(value);
            }
        }
        match std::env::var(&key) {
            Ok(value) => Ok(Value::Str(value)),
            Err(_) => Ok(default),
        }
    }

    pub(super) fn make_os_terminal_size(&self, columns: i64, lines: i64) -> Value {
        let module = match self
            .heap
            .alloc_module(ModuleObject::new("__os_terminal_size__".to_string()))
        {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *module.kind_mut() {
            module_data
                .globals
                .insert("columns".to_string(), Value::Int(columns));
            module_data
                .globals
                .insert("lines".to_string(), Value::Int(lines));
        }
        Value::Module(module)
    }

    pub(super) fn builtin_os_terminal_size(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("terminal_size() expects one argument"));
        }
        let values = match &args[0] {
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => {
                    return Err(RuntimeError::new(
                        "terminal_size() expects a 2-item sequence",
                    ));
                }
            },
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => values.clone(),
                _ => {
                    return Err(RuntimeError::new(
                        "terminal_size() expects a 2-item sequence",
                    ));
                }
            },
            _ => {
                return Err(RuntimeError::new(
                    "terminal_size() expects a 2-item sequence",
                ));
            }
        };
        if values.len() != 2 {
            return Err(RuntimeError::new(
                "terminal_size() expects a 2-item sequence",
            ));
        }
        let columns = value_to_int(values[0].clone())?;
        let lines = value_to_int(values[1].clone())?;
        Ok(self.make_os_terminal_size(columns, lines))
    }

    pub(super) fn builtin_os_get_terminal_size(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new(
                "get_terminal_size() expects at most one argument",
            ));
        }
        if let Some(fd) = args.first() {
            let _ = value_to_int(fd.clone())?;
        }
        Ok(self.make_os_terminal_size(80, 24))
    }

    pub(super) fn alloc_open_fd(&mut self, file: fs::File) -> i64 {
        let fd = self.next_fd;
        self.next_fd += 1;
        self.open_files.insert(fd, file);
        self.fd_inheritable.insert(fd, true);
        fd
    }

    pub(super) fn find_open_file_mut(&mut self, fd: i64) -> Option<&mut fs::File> {
        if self.open_files.contains_key(&fd) {
            return self.open_files.get_mut(&fd);
        }
        #[cfg(unix)]
        {
            let raw_fd = i32::try_from(fd).ok()?;
            return self
                .open_files
                .values_mut()
                .find(|file| file.as_raw_fd() == raw_fd);
        }
        #[allow(unreachable_code)]
        None
    }

    pub(super) fn find_open_file(&self, fd: i64) -> Option<&fs::File> {
        if let Some(file) = self.open_files.get(&fd) {
            return Some(file);
        }
        #[cfg(unix)]
        {
            let raw_fd = i32::try_from(fd).ok()?;
            return self
                .open_files
                .values()
                .find(|file| file.as_raw_fd() == raw_fd);
        }
        #[allow(unreachable_code)]
        None
    }

    pub(super) fn resolve_open_file_fd(&self, fd: i64) -> Option<i64> {
        if self.open_files.contains_key(&fd) {
            return Some(fd);
        }
        #[cfg(unix)]
        {
            let raw_fd = i32::try_from(fd).ok()?;
            return self.open_files.iter().find_map(|(virtual_fd, file)| {
                (file.as_raw_fd() == raw_fd).then_some(*virtual_fd)
            });
        }
        #[allow(unreachable_code)]
        None
    }

    pub(super) fn cloned_open_file_for_fd(&self, fd: i64) -> Result<fs::File, RuntimeError> {
        self.find_open_file(fd)
            .ok_or_else(|| RuntimeError::new("bad file descriptor"))?
            .try_clone()
            .map_err(|err| RuntimeError::new(format!("fd clone failed: {err}")))
    }

    #[cfg(unix)]
    pub(super) fn stdio_from_vm_fd(&self, fd: i64, fallback: Stdio) -> Result<Stdio, RuntimeError> {
        if fd < 0 {
            return Ok(fallback);
        }
        if let Some(file) = self.find_open_file(fd) {
            let cloned = file
                .try_clone()
                .map_err(|err| RuntimeError::new(format!("fd clone failed: {err}")))?;
            return Ok(Stdio::from(cloned));
        }
        if (0..=2).contains(&fd) {
            return Ok(match fd {
                0 => Stdio::inherit(),
                1 => Stdio::inherit(),
                2 => Stdio::inherit(),
                _ => unreachable!(),
            });
        }
        Err(RuntimeError::new("bad file descriptor"))
    }

    #[cfg(unix)]
    pub(super) fn status_to_wait_status(status: std::process::ExitStatus) -> i64 {
        if let Some(code) = status.code() {
            return ((code & 0xff) << 8) as i64;
        }
        if let Some(signal) = status.signal() {
            return signal as i64;
        }
        0
    }

    pub(super) fn builtin_os_pipe(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("pipe() expects no arguments"));
        }
        #[cfg(unix)]
        {
            let (read_end, write_end) = UnixStream::pair()
                .map_err(|err| RuntimeError::new(format!("pipe failed: {err}")))?;
            let read_fd =
                self.alloc_open_fd(unsafe { fs::File::from_raw_fd(read_end.into_raw_fd()) });
            let write_fd =
                self.alloc_open_fd(unsafe { fs::File::from_raw_fd(write_end.into_raw_fd()) });
            return Ok(self
                .heap
                .alloc_tuple(vec![Value::Int(read_fd), Value::Int(write_fd)]));
        }
        #[cfg(not(unix))]
        {
            Err(RuntimeError::new(
                "os.pipe() is unsupported on this platform",
            ))
        }
    }

    pub(super) fn builtin_os_open(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "open() expects path, flags, and optional mode",
            ));
        }
        let path = self.path_arg_to_string(args[0].clone())?;
        let flags = value_to_int(args.remove(1))?;
        let mode = if args.len() == 2 {
            value_to_int(args.remove(1))?
        } else if let Some(mode) = kwargs.remove("mode") {
            value_to_int(mode)?
        } else {
            0o777
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "open() got an unexpected keyword argument",
            ));
        }

        let access_mode = flags & 0x3;
        let mut options = fs::OpenOptions::new();
        match access_mode {
            0 => {
                options.read(true);
            }
            1 => {
                options.write(true);
            }
            2 => {
                options.read(true).write(true);
            }
            _ => {
                return Err(RuntimeError::new("invalid open flags"));
            }
        }
        let create = (flags & 64) != 0;
        let excl = (flags & 128) != 0;
        let trunc = (flags & 512) != 0;
        let append = (flags & 1024) != 0;
        if create {
            options.create(true);
        }
        if create && excl {
            options.create_new(true);
        }
        if trunc {
            options.truncate(true);
        }
        if append {
            options.append(true);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode as u32);
        }

        let file = options
            .open(path)
            .map_err(|err| RuntimeError::new(format!("open failed: {err}")))?;
        let fd = self.alloc_open_fd(file);
        Ok(Value::Int(fd))
    }

    pub(super) fn builtin_os_close(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("close() expects one argument"));
        }
        let fd = value_to_int(args[0].clone())?;
        if (0..=2).contains(&fd) {
            return Ok(Value::None);
        }
        if self.open_files.remove(&fd).is_some() {
            self.fd_inheritable.remove(&fd);
            return Ok(Value::None);
        }
        #[cfg(unix)]
        {
            if let Some((key, _)) = self
                .open_files
                .iter()
                .find(|(_, file)| file.as_raw_fd() == fd as i32)
            {
                let key = *key;
                self.open_files.remove(&key);
                self.fd_inheritable.remove(&key);
                return Ok(Value::None);
            }
        }
        Err(RuntimeError::new("bad file descriptor"))
    }

    pub(super) fn builtin_os_read(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("read() expects fd and size"));
        }
        let fd = value_to_int(args[0].clone())?;
        let size = value_to_int(args[1].clone())?;
        if size < 0 {
            return Err(RuntimeError::new("negative size not allowed"));
        }
        let mut buffer = vec![0u8; size as usize];
        let read_len = if fd == 0 {
            std::io::stdin()
                .read(&mut buffer)
                .map_err(|err| RuntimeError::new(format!("read failed: {err}")))?
        } else {
            let file = self
                .find_open_file_mut(fd)
                .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
            file.read(&mut buffer)
                .map_err(|err| RuntimeError::new(format!("read failed: {err}")))?
        };
        buffer.truncate(read_len);
        Ok(self.heap.alloc_bytes(buffer))
    }

    pub(super) fn builtin_os_readinto(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("readinto() expects fd and buffer"));
        }
        let fd = value_to_int(args[0].clone())?;
        let target = args[1].clone();
        let request = self.io_writable_buffer_len(&target)?;
        if request == 0 {
            return Ok(Value::Int(0));
        }
        let mut buffer = vec![0u8; request];
        let read_len = if fd == 0 {
            std::io::stdin()
                .read(&mut buffer)
                .map_err(|err| RuntimeError::new(format!("readinto failed: {err}")))?
        } else {
            let file = self
                .find_open_file_mut(fd)
                .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
            file.read(&mut buffer)
                .map_err(|err| RuntimeError::new(format!("readinto failed: {err}")))?
        };
        let copied = self.io_copy_into_writable_buffer(target, &buffer[..read_len])?;
        Ok(Value::Int(copied as i64))
    }

    pub(super) fn builtin_os_dup(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("dup() expects one argument"));
        }
        let fd = value_to_int(args[0].clone())?;
        let cloned = self.cloned_open_file_for_fd(fd)?;
        let new_fd = self.alloc_open_fd(cloned);
        let inheritable = self
            .resolve_open_file_fd(fd)
            .and_then(|resolved| self.fd_inheritable.get(&resolved).copied())
            .unwrap_or(true);
        self.fd_inheritable.insert(new_fd, inheritable);
        Ok(Value::Int(new_fd))
    }

    pub(super) fn builtin_os_lseek(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 3 {
            return Err(RuntimeError::new(
                "lseek() expects fd, position, and whence",
            ));
        }
        let fd = value_to_int(args[0].clone())?;
        let position = value_to_int(args[1].clone())?;
        let whence = value_to_int(args[2].clone())?;
        let seek_from = match whence {
            0 => {
                if position < 0 {
                    return Err(RuntimeError::new("OSError: [Errno 22] Invalid argument"));
                }
                SeekFrom::Start(position as u64)
            }
            1 => SeekFrom::Current(position),
            2 => SeekFrom::End(position),
            _ => return Err(RuntimeError::new("OSError: [Errno 22] Invalid argument")),
        };
        let file = self
            .find_open_file_mut(fd)
            .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
        let offset = file
            .seek(seek_from)
            .map_err(|_| RuntimeError::new("OSError: [Errno 22] Invalid argument"))?;
        Ok(Value::Int(offset as i64))
    }

    pub(super) fn builtin_os_ftruncate(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("ftruncate() expects fd and length"));
        }
        let fd = value_to_int(args[0].clone())?;
        let length = value_to_int(args[1].clone())?;
        if length < 0 {
            return Err(RuntimeError::new("OSError: [Errno 22] Invalid argument"));
        }
        let file = self
            .find_open_file_mut(fd)
            .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
        file.set_len(length as u64)
            .map_err(|err| RuntimeError::new(format!("OSError: {err}")))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_os_kill(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("kill() expects pid and signal"));
        }
        let pid = value_to_int(args[0].clone())?;
        let signal = value_to_int(args[1].clone())?;
        if signal == 0 {
            return Ok(Value::None);
        }
        if let Some(child) = self.child_processes.get_mut(&pid) {
            child
                .kill()
                .map_err(|err| RuntimeError::new(format!("kill failed: {err}")))?;
            return Ok(Value::None);
        }
        Err(RuntimeError::new("No such process"))
    }

    pub(super) fn builtin_os_write(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("write() expects fd and data"));
        }
        let fd = value_to_int(args.remove(0))?;
        let payload = self.value_to_bytes_payload(args.remove(0))?;
        if let Some(file) = self.find_open_file_mut(fd) {
            use std::io::Write;
            file.write_all(&payload)
                .map_err(|err| RuntimeError::new(format!("write failed: {err}")))?;
        } else if fd == 1 {
            std::io::stdout()
                .write_all(&payload)
                .map_err(|err| RuntimeError::new(format!("write failed: {err}")))?;
        } else if fd == 2 {
            std::io::stderr()
                .write_all(&payload)
                .map_err(|err| RuntimeError::new(format!("write failed: {err}")))?;
        } else {
            return Err(RuntimeError::new("bad file descriptor"));
        }
        Ok(Value::Int(payload.len() as i64))
    }

    pub(super) fn builtin_os_isatty(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("isatty() expects one argument"));
        }
        let fd = value_to_int(args[0].clone())?;
        let isatty = match fd {
            0 => std::io::stdin().is_terminal(),
            1 => std::io::stdout().is_terminal(),
            2 => std::io::stderr().is_terminal(),
            _ => false,
        };
        Ok(Value::Bool(isatty))
    }

    pub(super) fn builtin_os_set_inheritable(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "set_inheritable() expects fd and inheritable flag",
            ));
        }
        let fd = value_to_int(args[0].clone())?;
        let inheritable = is_truthy(&args[1]);
        let resolved = self
            .resolve_open_file_fd(fd)
            .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
        self.fd_inheritable.insert(resolved, inheritable);
        Ok(Value::None)
    }

    pub(super) fn builtin_os_get_inheritable(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("get_inheritable() expects one argument"));
        }
        let fd = value_to_int(args[0].clone())?;
        if (0..=2).contains(&fd) {
            return Ok(Value::Bool(true));
        }
        let resolved = self
            .resolve_open_file_fd(fd)
            .ok_or_else(|| RuntimeError::new("bad file descriptor"))?;
        Ok(Value::Bool(
            self.fd_inheritable.get(&resolved).copied().unwrap_or(true),
        ))
    }

    pub(super) fn builtin_os_urandom(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("urandom() expects one argument"));
        }
        let size = value_to_int(args[0].clone())?;
        if size < 0 {
            return Err(RuntimeError::new("negative argument not allowed"));
        }
        let size = size as usize;
        let mut out = vec![0u8; size];

        // Use system entropy where available; fall back to VM RNG only if unavailable.
        let os_fill_ok = fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut out))
            .is_ok();
        if !os_fill_ok {
            for chunk in out.chunks_mut(4) {
                let bytes = self.random.next_u32().to_le_bytes();
                let len = chunk.len();
                chunk.copy_from_slice(&bytes[..len]);
            }
        }
        Ok(self.heap.alloc_bytes(out))
    }

    pub(super) fn builtin_os_stat(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("stat() expects one argument"));
        }
        let metadata = match &args[0] {
            Value::Int(fd) => {
                if let Some(file) = self.open_files.get(fd) {
                    file.metadata()
                        .map_err(|err| RuntimeError::new(format!("fstat failed: {err}")))?
                } else {
                    let fd_path = format!("/proc/self/fd/{fd}");
                    let fallback_fd_path = format!("/dev/fd/{fd}");
                    fs::metadata(&fd_path)
                        .or_else(|_| fs::metadata(&fallback_fd_path))
                        .map_err(|err| RuntimeError::new(format!("fstat failed: {err}")))?
                }
            }
            Value::Bool(flag) => {
                let fd = if *flag { 1 } else { 0 };
                if let Some(file) = self.open_files.get(&fd) {
                    file.metadata()
                        .map_err(|err| RuntimeError::new(format!("fstat failed: {err}")))?
                } else {
                    let fd_path = format!("/proc/self/fd/{fd}");
                    let fallback_fd_path = format!("/dev/fd/{fd}");
                    fs::metadata(&fd_path)
                        .or_else(|_| fs::metadata(&fallback_fd_path))
                        .map_err(|err| RuntimeError::new(format!("fstat failed: {err}")))?
                }
            }
            _ => {
                let path = self.path_arg_to_string(args[0].clone())?;
                fs::metadata(path)
                    .map_err(|err| RuntimeError::new(format!("stat failed: {err}")))?
            }
        };
        self.build_stat_result(metadata, false)
    }

    pub(super) fn builtin_os_lstat(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("lstat() expects one argument"));
        }
        let path = self.path_arg_to_string(args[0].clone())?;
        let metadata = fs::symlink_metadata(path)
            .map_err(|err| RuntimeError::new(format!("lstat failed: {err}")))?;
        self.build_stat_result(metadata, true)
    }

    pub(super) fn builtin_os_mkdir(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("mkdir() expects path and optional mode"));
        }
        let path = self.path_arg_to_string(args.remove(0))?;
        let mode = if !args.is_empty() {
            value_to_int(args.remove(0))?
        } else if let Some(value) = kwargs.remove("mode") {
            value_to_int(value)?
        } else {
            0o777
        };
        if let Some(dir_fd) = kwargs.remove("dir_fd") {
            if !matches!(dir_fd, Value::None) {
                return Err(RuntimeError::new("mkdir() dir_fd is unsupported"));
            }
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "mkdir() got an unexpected keyword argument",
            ));
        }
        fs::create_dir(&path).map_err(|err| RuntimeError::new(format!("mkdir failed: {err}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = fs::Permissions::from_mode((mode & 0o777) as u32);
            fs::set_permissions(&path, permissions)
                .map_err(|err| RuntimeError::new(format!("mkdir failed: {err}")))?;
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_os_chmod(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "chmod() expects path, mode, and optional dir_fd/follow_symlinks",
            ));
        }
        let path = self.path_arg_to_string(args.remove(0))?;
        let mode = value_to_int(args.remove(0))?;
        if let Some(dir_fd) = kwargs.remove("dir_fd") {
            if !matches!(dir_fd, Value::None) {
                return Err(RuntimeError::new("chmod() dir_fd is unsupported"));
            }
        }
        if let Some(follow_symlinks) = kwargs.remove("follow_symlinks") {
            if !is_truthy(&follow_symlinks) {
                return Err(RuntimeError::new("chmod() follow_symlinks is unsupported"));
            }
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "chmod() got an unexpected keyword argument",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = fs::Permissions::from_mode((mode & 0o7777) as u32);
            fs::set_permissions(&path, permissions)
                .map_err(|err| RuntimeError::new(format!("chmod failed: {err}")))?;
        }
        #[cfg(not(unix))]
        {
            let readonly = (mode & 0o222) == 0;
            let metadata = fs::metadata(&path)
                .map_err(|err| RuntimeError::new(format!("chmod failed: {err}")))?;
            let mut permissions = metadata.permissions();
            permissions.set_readonly(readonly);
            fs::set_permissions(&path, permissions)
                .map_err(|err| RuntimeError::new(format!("chmod failed: {err}")))?;
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_os_rmdir(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("rmdir() expects one argument"));
        }
        let path = self.path_arg_to_string(args[0].clone())?;
        fs::remove_dir(path).map_err(|err| RuntimeError::new(format!("rmdir failed: {err}")))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_os_utime(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "utime() expects path and optional times tuple",
            ));
        }
        let path = self.path_arg_to_string(args.remove(0))?;
        let mut times = args.pop();
        if let Some(value) = kwargs.remove("times") {
            if times.is_some() {
                return Err(RuntimeError::new("utime() got multiple values for times"));
            }
            times = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "utime() got an unexpected keyword argument",
            ));
        }

        let file = fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|err| RuntimeError::new(format!("utime failed: {err}")))?;
        let (atime, mtime) = if let Some(spec) = times {
            match spec {
                Value::Tuple(obj) => match &*obj.kind() {
                    Object::Tuple(values) if values.len() == 2 => (
                        value_to_f64(values[0].clone())?,
                        value_to_f64(values[1].clone())?,
                    ),
                    _ => return Err(RuntimeError::new("utime() times must be a 2-tuple")),
                },
                Value::List(obj) => match &*obj.kind() {
                    Object::List(values) if values.len() == 2 => (
                        value_to_f64(values[0].clone())?,
                        value_to_f64(values[1].clone())?,
                    ),
                    _ => return Err(RuntimeError::new("utime() times must be a 2-sequence")),
                },
                _ => return Err(RuntimeError::new("utime() times must be a 2-sequence")),
            }
        } else {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| RuntimeError::new("system time before epoch"))?
                .as_secs_f64();
            (now, now)
        };

        let atime = seconds_to_system_time(atime)?;
        let mtime = seconds_to_system_time(mtime)?;
        let times = fs::FileTimes::new().set_accessed(atime).set_modified(mtime);
        file.set_times(times)
            .map_err(|err| RuntimeError::new(format!("utime failed: {err}")))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_os_scandir(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("scandir() expects at most one argument"));
        }
        let path = if args.is_empty() {
            ".".to_string()
        } else {
            self.path_arg_to_string(args[0].clone())?
        };
        let direntry_class = match self
            .heap
            .alloc_class(ClassObject::new("DirEntry".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *direntry_class.kind_mut() {
            class_data.attrs.insert(
                "is_dir".to_string(),
                Value::Builtin(BuiltinFunction::OsDirEntryIsDir),
            );
            class_data.attrs.insert(
                "is_file".to_string(),
                Value::Builtin(BuiltinFunction::OsDirEntryIsFile),
            );
            class_data.attrs.insert(
                "is_symlink".to_string(),
                Value::Builtin(BuiltinFunction::OsDirEntryIsSymlink),
            );
        }
        let scandir_class = match self
            .heap
            .alloc_class(ClassObject::new("ScandirIterator".to_string(), Vec::new()))
        {
            Value::Class(class) => class,
            _ => unreachable!(),
        };
        if let Object::Class(class_data) = &mut *scandir_class.kind_mut() {
            class_data.attrs.insert(
                "__iter__".to_string(),
                Value::Builtin(BuiltinFunction::OsScandirIter),
            );
            class_data.attrs.insert(
                "__next__".to_string(),
                Value::Builtin(BuiltinFunction::OsScandirNext),
            );
            class_data.attrs.insert(
                "__enter__".to_string(),
                Value::Builtin(BuiltinFunction::OsScandirEnter),
            );
            class_data.attrs.insert(
                "__exit__".to_string(),
                Value::Builtin(BuiltinFunction::OsScandirExit),
            );
            class_data.attrs.insert(
                "close".to_string(),
                Value::Builtin(BuiltinFunction::OsScandirClose),
            );
        }

        let mut rows = Vec::new();
        let entries = fs::read_dir(&path)
            .map_err(|err| RuntimeError::new(format!("scandir failed: {err}")))?;
        for entry in entries {
            let entry = entry.map_err(|err| RuntimeError::new(format!("scandir failed: {err}")))?;
            let file_type = entry
                .file_type()
                .map_err(|err| RuntimeError::new(format!("scandir failed: {err}")))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let full_path = entry.path().to_string_lossy().to_string();
            let direntry = self.alloc_instance_for_class(&direntry_class);
            if let Object::Instance(instance_data) = &mut *direntry.kind_mut() {
                instance_data
                    .attrs
                    .insert("name".to_string(), Value::Str(name.clone()));
                instance_data
                    .attrs
                    .insert("path".to_string(), Value::Str(full_path));
                instance_data
                    .attrs
                    .insert("_is_dir".to_string(), Value::Bool(file_type.is_dir()));
                instance_data
                    .attrs
                    .insert("_is_file".to_string(), Value::Bool(file_type.is_file()));
                instance_data.attrs.insert(
                    "_is_symlink".to_string(),
                    Value::Bool(file_type.is_symlink()),
                );
            }
            rows.push(Value::Instance(direntry));
        }

        let entries_list = self.heap.alloc_list(rows);
        let iterator = self.alloc_instance_for_class(&scandir_class);
        if let Object::Instance(instance_data) = &mut *iterator.kind_mut() {
            instance_data
                .attrs
                .insert("_entries".to_string(), entries_list);
            instance_data
                .attrs
                .insert("_index".to_string(), Value::Int(0));
            instance_data
                .attrs
                .insert("_closed".to_string(), Value::Bool(false));
        }
        Ok(Value::Instance(iterator))
    }

    pub(super) fn builtin_os_scandir_iter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("__iter__() expects no arguments"));
        }
        Ok(args[0].clone())
    }

    pub(super) fn builtin_os_scandir_next(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("__next__() expects no arguments"));
        }
        let Value::Instance(instance) = &args[0] else {
            return Err(RuntimeError::new("__next__() expects scandir iterator"));
        };
        let is_closed = matches!(
            Self::instance_attr_get(instance, "_closed"),
            Some(Value::Bool(true))
        );
        if is_closed {
            return Err(RuntimeError::new("StopIteration"));
        }
        let index = match Self::instance_attr_get(instance, "_index") {
            Some(Value::Int(value)) if value >= 0 => value as usize,
            _ => 0,
        };
        let entries = match Self::instance_attr_get(instance, "_entries") {
            Some(Value::List(entries_obj)) => entries_obj,
            _ => return Err(RuntimeError::new("__next__() expects scandir iterator")),
        };
        let Object::List(values) = &*entries.kind() else {
            return Err(RuntimeError::new("__next__() expects scandir iterator"));
        };
        if index >= values.len() {
            return Err(RuntimeError::new("StopIteration"));
        }
        Self::instance_attr_set(instance, "_index", Value::Int((index + 1) as i64))?;
        Ok(values[index].clone())
    }

    pub(super) fn builtin_os_scandir_enter(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("__enter__() expects no arguments"));
        }
        Ok(args[0].clone())
    }

    pub(super) fn builtin_os_scandir_exit(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 4 {
            return Err(RuntimeError::new("__exit__() expects three arguments"));
        }
        let Value::Instance(instance) = &args[0] else {
            return Err(RuntimeError::new("__exit__() expects scandir iterator"));
        };
        Self::instance_attr_set(instance, "_closed", Value::Bool(true))?;
        Ok(Value::Bool(false))
    }

    pub(super) fn builtin_os_scandir_close(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("close() expects no arguments"));
        }
        let Value::Instance(instance) = &args[0] else {
            return Err(RuntimeError::new("close() expects scandir iterator"));
        };
        Self::instance_attr_set(instance, "_closed", Value::Bool(true))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_os_direntry_is_dir(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("is_dir() expects at most one argument"));
        }
        let follow_symlinks = if args.len() == 2 {
            is_truthy(&args[1])
        } else if let Some(value) = kwargs.remove("follow_symlinks") {
            is_truthy(&value)
        } else {
            true
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "is_dir() got an unexpected keyword argument",
            ));
        }
        let Value::Instance(instance) = &args[0] else {
            return Err(RuntimeError::new("is_dir() expects DirEntry"));
        };
        let path = match Self::instance_attr_get(instance, "path") {
            Some(Value::Str(path)) => path,
            _ => return Ok(Value::Bool(false)),
        };
        let is_dir = if follow_symlinks {
            fs::metadata(path)
                .map(|meta| meta.is_dir())
                .unwrap_or(false)
        } else {
            fs::symlink_metadata(path)
                .map(|meta| meta.file_type().is_dir())
                .unwrap_or(false)
        };
        Ok(Value::Bool(is_dir))
    }

    pub(super) fn builtin_os_direntry_is_file(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("is_file() expects at most one argument"));
        }
        let follow_symlinks = if args.len() == 2 {
            is_truthy(&args[1])
        } else if let Some(value) = kwargs.remove("follow_symlinks") {
            is_truthy(&value)
        } else {
            true
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "is_file() got an unexpected keyword argument",
            ));
        }
        let Value::Instance(instance) = &args[0] else {
            return Err(RuntimeError::new("is_file() expects DirEntry"));
        };
        let path = match Self::instance_attr_get(instance, "path") {
            Some(Value::Str(path)) => path,
            _ => return Ok(Value::Bool(false)),
        };
        let is_file = if follow_symlinks {
            fs::metadata(path)
                .map(|meta| meta.is_file())
                .unwrap_or(false)
        } else {
            fs::symlink_metadata(path)
                .map(|meta| meta.file_type().is_file())
                .unwrap_or(false)
        };
        Ok(Value::Bool(is_file))
    }

    pub(super) fn builtin_os_direntry_is_symlink(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("is_symlink() expects no arguments"));
        }
        let Value::Instance(instance) = &args[0] else {
            return Err(RuntimeError::new("is_symlink() expects DirEntry"));
        };
        let path = match Self::instance_attr_get(instance, "path") {
            Some(Value::Str(path)) => path,
            _ => return Ok(Value::Bool(false)),
        };
        let is_symlink = fs::symlink_metadata(path)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false);
        Ok(Value::Bool(is_symlink))
    }

    pub(super) fn builtin_os_walk(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::new("walk() expects one positional argument"));
        }
        let root_str = self.path_arg_to_string(args.remove(0))?;
        let topdown = kwargs
            .remove("topdown")
            .map(|value| is_truthy(&value))
            .unwrap_or(true);
        let followlinks = kwargs
            .remove("followlinks")
            .map(|value| is_truthy(&value))
            .unwrap_or(false);
        let onerror = kwargs.remove("onerror");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "walk() got an unexpected keyword argument",
            ));
        }

        let root = PathBuf::from(&root_str);
        let mut rows = Vec::new();

        fn os_walk_error_value(path: &Path, err: &std::io::Error) -> Value {
            let name = match err.kind() {
                std::io::ErrorKind::NotFound => "FileNotFoundError",
                std::io::ErrorKind::PermissionDenied => "PermissionError",
                std::io::ErrorKind::AlreadyExists => "FileExistsError",
                _ => "OSError",
            };
            let exception = ExceptionObject::new(name, Some(err.to_string()));
            {
                let mut attrs = exception.attrs.borrow_mut();
                attrs.insert(
                    "filename".to_string(),
                    Value::Str(path.to_string_lossy().to_string()),
                );
                if let Some(errno) = err.raw_os_error() {
                    attrs.insert("errno".to_string(), Value::Int(errno as i64));
                }
                attrs.insert("strerror".to_string(), Value::Str(err.to_string()));
            }
            Value::Exception(Box::new(exception))
        }

        fn collect_walk(
            vm: &mut Vm,
            rows: &mut Vec<Value>,
            current: &Path,
            topdown: bool,
            followlinks: bool,
            onerror: Option<Value>,
        ) -> Result<(), RuntimeError> {
            let entries = match fs::read_dir(current) {
                Ok(entries) => entries,
                Err(err) => {
                    if let Some(callback) = onerror {
                        let exc = os_walk_error_value(current, &err);
                        match vm.call_internal(callback, vec![exc], HashMap::new())? {
                            InternalCallOutcome::Value(_) => {}
                            InternalCallOutcome::CallerExceptionHandled => {
                                return Err(vm.runtime_error_from_active_exception(
                                    "walk() onerror callback raised",
                                ));
                            }
                        }
                    }
                    return Ok(());
                }
            };
            let mut dir_entries: Vec<(String, PathBuf)> = Vec::new();
            let mut file_entries: Vec<String> = Vec::new();
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(err) => {
                        if let Some(callback) = onerror.clone() {
                            let exc = os_walk_error_value(current, &err);
                            match vm.call_internal(callback, vec![exc], HashMap::new())? {
                                InternalCallOutcome::Value(_) => {}
                                InternalCallOutcome::CallerExceptionHandled => {
                                    return Err(vm.runtime_error_from_active_exception(
                                        "walk() onerror callback raised",
                                    ));
                                }
                            }
                        }
                        continue;
                    }
                };
                let name = entry.file_name().to_string_lossy().to_string();
                let path = entry.path();
                let file_type = entry
                    .file_type()
                    .map_err(|err| RuntimeError::new(format!("walk failed: {err}")))?;
                if file_type.is_dir() {
                    dir_entries.push((name, path));
                } else if file_type.is_symlink() && followlinks {
                    dir_entries.push((name, path));
                } else {
                    file_entries.push(name);
                }
            }
            dir_entries.sort_by(|a, b| a.0.cmp(&b.0));
            file_entries.sort();

            let emit_row = |vm: &mut Vm, rows: &mut Vec<Value>| {
                let dirnames = vm.heap.alloc_list(
                    dir_entries
                        .iter()
                        .map(|(name, _)| Value::Str(name.clone()))
                        .collect(),
                );
                let filenames = vm.heap.alloc_list(
                    file_entries
                        .iter()
                        .map(|name| Value::Str(name.clone()))
                        .collect(),
                );
                rows.push(vm.heap.alloc_tuple(vec![
                    Value::Str(current.to_string_lossy().to_string()),
                    dirnames,
                    filenames,
                ]));
            };

            if topdown {
                emit_row(vm, rows);
            }
            for (_, child) in &dir_entries {
                collect_walk(vm, rows, child, topdown, followlinks, onerror.clone())?;
            }
            if !topdown {
                emit_row(vm, rows);
            }
            Ok(())
        }

        collect_walk(self, &mut rows, &root, topdown, followlinks, onerror)?;
        Ok(self.heap.alloc_list(rows))
    }

    pub(super) fn builtin_os_listdir(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() > 1 {
            return Err(RuntimeError::new("listdir() expects at most one argument"));
        }
        let path = if args.is_empty() {
            ".".to_string()
        } else {
            self.path_arg_to_string(args[0].clone())?
        };
        let mut names = Vec::new();
        let entries = fs::read_dir(&path)
            .map_err(|err| RuntimeError::new(format!("listdir failed: {err}")))?;
        for entry in entries {
            let entry = entry.map_err(|err| RuntimeError::new(format!("listdir failed: {err}")))?;
            let name = entry.file_name().to_string_lossy().to_string();
            names.push(Value::Str(name));
        }
        names.sort_by(|a, b| format_value(a).cmp(&format_value(b)));
        Ok(self.heap.alloc_list(names))
    }

    pub(super) fn builtin_os_access(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "access() expects path, mode, and optional keyword-only arguments",
            ));
        }
        let (path, _) = self.path_arg_to_string_and_type(args.remove(0))?;
        let mode = value_to_int(args.remove(0))?;
        if let Some(dir_fd) = kwargs.remove("dir_fd") {
            if !matches!(dir_fd, Value::None) {
                return Err(RuntimeError::new("access() dir_fd is unsupported"));
            }
        }
        if let Some(effective_ids) = kwargs.remove("effective_ids") {
            if is_truthy(&effective_ids) {
                return Err(RuntimeError::new("access() effective_ids is unsupported"));
            }
        }
        if let Some(follow_symlinks) = kwargs.remove("follow_symlinks") {
            if !is_truthy(&follow_symlinks) {
                return Err(RuntimeError::new("access() follow_symlinks is unsupported"));
            }
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "access() got an unexpected keyword argument",
            ));
        }

        let path_buf = PathBuf::from(path);
        if !path_buf.exists() {
            return Ok(Value::Bool(false));
        }

        let metadata = fs::metadata(&path_buf)
            .map_err(|err| RuntimeError::new(format!("access failed: {err}")))?;
        let mut allowed = true;
        if mode & 2 != 0 {
            allowed &= !metadata.permissions().readonly();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if mode & 1 != 0 {
                allowed &= metadata.permissions().mode() & 0o111 != 0;
            }
        }
        Ok(Value::Bool(allowed))
    }

    pub(super) fn builtin_os_fspath(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("fspath() expects one argument"));
        }
        let value = args[0].clone();
        match value {
            Value::Str(_) | Value::Bytes(_) => Ok(value),
            _ => {
                let Some(fspath) = self.lookup_bound_special_method(&value, "__fspath__")? else {
                    return Err(RuntimeError::new(
                        "TypeError: expected str, bytes or os.PathLike object",
                    ));
                };
                match self.call_internal(fspath, Vec::new(), HashMap::new())? {
                    InternalCallOutcome::Value(path) => match path {
                        Value::Str(_) | Value::Bytes(_) => Ok(path),
                        _ => Err(RuntimeError::new(
                            "TypeError: __fspath__() must return str or bytes",
                        )),
                    },
                    InternalCallOutcome::CallerExceptionHandled => {
                        Err(self.runtime_error_from_active_exception("__fspath__() failed"))
                    }
                }
            }
        }
    }

    pub(super) fn builtin_os_fsencode(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("fsencode() expects one argument"));
        }
        match &args[0] {
            Value::Str(value) => Ok(self.heap.alloc_bytes(value.as_bytes().to_vec())),
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(bytes) => Ok(self.heap.alloc_bytes(bytes.clone())),
                _ => Err(RuntimeError::new(
                    "fsencode() expects str, bytes or bytearray",
                )),
            },
            Value::ByteArray(obj) => match &*obj.kind() {
                Object::ByteArray(bytes) => Ok(self.heap.alloc_bytes(bytes.clone())),
                _ => Err(RuntimeError::new(
                    "fsencode() expects str, bytes or bytearray",
                )),
            },
            _ => Err(RuntimeError::new(
                "fsencode() expects str, bytes or bytearray",
            )),
        }
    }

    pub(super) fn builtin_os_fsdecode(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("fsdecode() expects one argument"));
        }
        match &args[0] {
            Value::Str(value) => Ok(Value::Str(value.clone())),
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(bytes) => Ok(Value::Str(String::from_utf8_lossy(bytes).to_string())),
                _ => Err(RuntimeError::new(
                    "fsdecode() expects str, bytes or bytearray",
                )),
            },
            Value::ByteArray(obj) => match &*obj.kind() {
                Object::ByteArray(bytes) => {
                    Ok(Value::Str(String::from_utf8_lossy(bytes).to_string()))
                }
                _ => Err(RuntimeError::new(
                    "fsdecode() expects str, bytes or bytearray",
                )),
            },
            _ => Err(RuntimeError::new(
                "fsdecode() expects str, bytes or bytearray",
            )),
        }
    }

    pub(super) fn builtin_os_remove(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("remove() expects one argument"));
        }
        let path = self.path_arg_to_string(args[0].clone())?;
        fs::remove_file(&path).map_err(|err| RuntimeError::new(format!("remove failed: {err}")))?;
        Ok(Value::None)
    }

    pub(super) fn builtin_os_waitstatus_to_exitcode(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "waitstatus_to_exitcode() expects one argument",
            ));
        }
        let status = value_to_int(args[0].clone())?;
        if status < 0 {
            return Err(RuntimeError::new("process status cannot be negative"));
        }
        let code = if (status & 0x7f) == 0 {
            (status >> 8) & 0xff
        } else {
            -(status & 0x7f)
        };
        Ok(Value::Int(code))
    }

    pub(super) fn builtin_os_waitpid(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("waitpid() expects pid and options"));
        }
        let pid = value_to_int(args[0].clone())?;
        let options = value_to_int(args[1].clone())?;
        if pid <= 0 {
            return Err(RuntimeError::new("waitpid() pid must be positive"));
        }
        if let Some(status) = self.child_exit_status.get(&pid) {
            return Ok(self
                .heap
                .alloc_tuple(vec![Value::Int(pid), Value::Int(*status)]));
        }
        if options & 1 != 0 {
            if let Some(child) = self.child_processes.get_mut(&pid) {
                match child
                    .try_wait()
                    .map_err(|err| RuntimeError::new(format!("waitpid failed: {err}")))?
                {
                    Some(status) => {
                        #[cfg(unix)]
                        let wait_status = Self::status_to_wait_status(status);
                        #[cfg(not(unix))]
                        let wait_status = 0;
                        self.child_processes.remove(&pid);
                        self.child_exit_status.insert(pid, wait_status);
                        return Ok(self
                            .heap
                            .alloc_tuple(vec![Value::Int(pid), Value::Int(wait_status)]));
                    }
                    None => {
                        return Ok(self.heap.alloc_tuple(vec![Value::Int(0), Value::Int(0)]));
                    }
                }
            }
            return Ok(self.heap.alloc_tuple(vec![Value::Int(0), Value::Int(0)]));
        }
        if let Some(child) = self.child_processes.get_mut(&pid) {
            let status = child
                .wait()
                .map_err(|err| RuntimeError::new(format!("waitpid failed: {err}")))?;
            #[cfg(unix)]
            let wait_status = Self::status_to_wait_status(status);
            #[cfg(not(unix))]
            let wait_status = 0;
            self.child_processes.remove(&pid);
            self.child_exit_status.insert(pid, wait_status);
            return Ok(self
                .heap
                .alloc_tuple(vec![Value::Int(pid), Value::Int(wait_status)]));
        }
        Ok(self.heap.alloc_tuple(vec![Value::Int(pid), Value::Int(0)]))
    }

    pub(super) fn builtin_posixsubprocess_fork_exec(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "_posixsubprocess.fork_exec() does not accept keyword arguments",
            ));
        }
        if args.len() < 22 {
            return Err(RuntimeError::new(
                "_posixsubprocess.fork_exec() received insufficient arguments",
            ));
        }
        #[cfg(not(unix))]
        {
            return Err(RuntimeError::new(
                "_posixsubprocess.fork_exec() is only supported on unix in pyrs",
            ));
        }
        #[cfg(unix)]
        {
            let argv = collect_process_argv(&args[0])?;
            if argv.is_empty() {
                return Err(RuntimeError::new("fork_exec() argv must be non-empty"));
            }

            let executable_list = collect_process_argv(&args[1])?;
            let executable = executable_list
                .first()
                .cloned()
                .unwrap_or_else(|| argv[0].clone());
            let cwd = match &args[4] {
                Value::None => None,
                value => Some(self.path_arg_to_string(value.clone())?),
            };
            let env = match &args[5] {
                Value::None => None,
                value => Some(collect_env_entries(value)?),
            };

            let p2cread = value_to_int(args[6].clone())?;
            let c2pwrite = value_to_int(args[9].clone())?;
            let errwrite = value_to_int(args[11].clone())?;

            let mut command = Command::new(executable);
            if argv.len() > 1 {
                command.args(&argv[1..]);
            }
            if let Some(path) = cwd {
                command.current_dir(path);
            }
            if let Some(entries) = env {
                command.env_clear();
                for (key, value) in entries {
                    command.env(key, value);
                }
            }
            command.stdin(self.stdio_from_vm_fd(p2cread, Stdio::null())?);
            command.stdout(self.stdio_from_vm_fd(c2pwrite, Stdio::inherit())?);
            command.stderr(self.stdio_from_vm_fd(errwrite, Stdio::inherit())?);

            let child = command
                .spawn()
                .map_err(|err| RuntimeError::new(format!("fork_exec spawn failed: {err}")))?;
            let pid = child.id() as i64;
            self.child_processes.insert(pid, child);
            Ok(Value::Int(pid))
        }
    }

    pub(super) fn subprocess_env_from_value(
        &self,
        value: Value,
    ) -> Result<Vec<(String, String)>, RuntimeError> {
        match value {
            Value::None => Ok(Vec::new()),
            Value::Dict(dict) => {
                let Object::Dict(entries) = &*dict.kind() else {
                    return Err(RuntimeError::new("env must be dict or None"));
                };
                let mut out = Vec::with_capacity(entries.len());
                for (key, value) in entries {
                    out.push((value_to_process_text(key)?, value_to_process_text(value)?));
                }
                Ok(out)
            }
            _ => Err(RuntimeError::new("env must be dict or None")),
        }
    }

    pub(super) fn subprocess_argv_from_value(
        &self,
        value: Value,
    ) -> Result<Vec<String>, RuntimeError> {
        match value {
            Value::Str(text) => Ok(vec![text]),
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(bytes) => Ok(vec![String::from_utf8_lossy(bytes).into_owned()]),
                _ => Err(RuntimeError::new("args must be str/bytes or sequence")),
            },
            other => collect_process_argv(&other),
        }
    }

    pub(super) fn rewrite_pyrs_subprocess_argv(
        &self,
        argv: Vec<String>,
    ) -> Result<Vec<String>, RuntimeError> {
        if argv.is_empty() {
            return Err(RuntimeError::new("empty command"));
        }
        let executable = argv[0].clone();
        if !is_pyrs_executable(&executable) {
            return Ok(argv);
        }
        let mut rewritten = vec![executable];
        let mut iter = argv.into_iter().skip(1);
        let mut inline_code: Option<String> = None;
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-E" | "-I" => {}
                "-X" => {
                    let _ = iter.next();
                }
                "-c" => {
                    inline_code = iter.next();
                    break;
                }
                _ => rewritten.push(arg),
            }
        }
        if let Some(code) = inline_code {
            let modules_to_block = parse_modules_to_block_literal(&code);
            let sanitized_code = if modules_to_block.is_empty() {
                code.clone()
            } else {
                let mut out = String::new();
                for line in code.lines() {
                    if line
                        .trim_start()
                        .starts_with("modules_to_block = frozenset(")
                    {
                        out.push_str("modules_to_block = frozenset()\n");
                    } else {
                        out.push_str(line);
                        out.push('\n');
                    }
                }
                out
            };
            let mut rewritten_code = String::new();
            if !modules_to_block.is_empty() {
                rewritten_code.push_str("import sys\n");
                for module_name in modules_to_block {
                    rewritten_code.push_str(&format!("sys.modules.pop({module_name:?}, None)\n"));
                }
            }
            rewritten_code.push_str(&sanitized_code);
            let mut path = std::env::temp_dir();
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            path.push(format!("pyrs_subprocess_{nonce}.py"));
            fs::write(&path, rewritten_code).map_err(|err| {
                RuntimeError::new(format!("failed to write inline script: {err}"))
            })?;
            rewritten.push(path.to_string_lossy().to_string());
        }
        for remaining in iter {
            rewritten.push(remaining);
        }
        Ok(rewritten)
    }

    pub(super) fn builtin_subprocess_popen_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "Popen.__init__")?;
        let cmd_value = if let Some(value) = args.first() {
            value.clone()
        } else if let Some(value) = kwargs.remove("args") {
            value
        } else {
            return Err(RuntimeError::new("Popen() missing args"));
        };
        let mut argv = self.subprocess_argv_from_value(cmd_value)?;
        argv = self.rewrite_pyrs_subprocess_argv(argv)?;
        if argv.is_empty() {
            return Err(RuntimeError::new("Popen() empty command"));
        }

        let cwd = match kwargs.remove("cwd") {
            Some(Value::None) | None => None,
            Some(value) => Some(self.path_arg_to_string(value)?),
        };
        let env = match kwargs.remove("env") {
            Some(value) => Some(self.subprocess_env_from_value(value)?),
            None => None,
        };
        let stdin_spec = kwargs.remove("stdin").unwrap_or(Value::None);
        let stdout_spec = kwargs.remove("stdout").unwrap_or(Value::None);
        let stderr_spec = kwargs.remove("stderr").unwrap_or(Value::None);

        let mut command = Command::new(&argv[0]);
        if argv.len() > 1 {
            command.args(&argv[1..]);
        }
        if let Some(path) = cwd {
            command.current_dir(path);
        }
        if let Some(env_entries) = env {
            command.env_clear();
            for (key, value) in env_entries {
                command.env(key, value);
            }
        }
        if matches!(stdin_spec, Value::Int(-1)) {
            command.stdin(Stdio::piped());
        } else {
            command.stdin(Stdio::null());
        }
        if matches!(stdout_spec, Value::Int(-1)) {
            command.stdout(Stdio::piped());
        } else {
            command.stdout(Stdio::inherit());
        }
        if matches!(stderr_spec, Value::Int(-1)) {
            command.stderr(Stdio::piped());
        } else {
            command.stderr(Stdio::inherit());
        }

        let child = command
            .spawn()
            .map_err(|err| RuntimeError::new(format!("subprocess spawn failed: {err}")))?;
        let pid = child.id() as i64;
        self.child_processes.insert(pid, child);
        Self::instance_attr_set(&instance, "pid", Value::Int(pid))?;
        Self::instance_attr_set(&instance, "returncode", Value::None)?;
        Self::instance_attr_set(&instance, "_pyrs_stdout", Value::None)?;
        Self::instance_attr_set(&instance, "_pyrs_stderr", Value::None)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_subprocess_popen_communicate(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance = self.take_bound_instance_arg(&mut args, "Popen.communicate")?;
        let input = if let Some(value) = kwargs.remove("input") {
            Some(value)
        } else if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let _timeout = if let Some(value) = kwargs.remove("timeout") {
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            Value::None
        };

        let pid = match Self::instance_attr_get(&instance, "pid") {
            Some(Value::Int(pid)) => pid,
            _ => return Err(RuntimeError::new("invalid subprocess handle")),
        };
        if let Some(mut child) = self.child_processes.remove(&pid) {
            if let Some(input) = input {
                if let Some(stdin) = child.stdin.as_mut() {
                    let payload = self.value_to_bytes_payload(input)?;
                    stdin
                        .write_all(&payload)
                        .map_err(|err| RuntimeError::new(format!("stdin write failed: {err}")))?;
                }
            }
            let output = child
                .wait_with_output()
                .map_err(|err| RuntimeError::new(format!("communicate failed: {err}")))?;
            #[cfg(unix)]
            let wait_status = Self::status_to_wait_status(output.status);
            #[cfg(not(unix))]
            let wait_status = 0;
            self.child_exit_status.insert(pid, wait_status);
            let returncode = if let Some(code) = output.status.code() {
                Value::Int(code as i64)
            } else {
                Value::Int(-1)
            };
            Self::instance_attr_set(&instance, "returncode", returncode)?;
            Self::instance_attr_set(
                &instance,
                "_pyrs_stdout",
                self.heap.alloc_bytes(output.stdout),
            )?;
            Self::instance_attr_set(
                &instance,
                "_pyrs_stderr",
                self.heap.alloc_bytes(output.stderr),
            )?;
        }

        let stdout = Self::instance_attr_get(&instance, "_pyrs_stdout")
            .unwrap_or_else(|| self.heap.alloc_bytes(Vec::new()));
        let stderr = Self::instance_attr_get(&instance, "_pyrs_stderr")
            .unwrap_or_else(|| self.heap.alloc_bytes(Vec::new()));
        Ok(self.heap.alloc_tuple(vec![stdout, stderr]))
    }

    pub(super) fn builtin_subprocess_popen_wait(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "Popen.wait() does not support keyword arguments",
            ));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Popen.wait")?;
        let pid = match Self::instance_attr_get(&instance, "pid") {
            Some(Value::Int(pid)) => pid,
            _ => return Err(RuntimeError::new("invalid subprocess handle")),
        };
        if let Some(mut child) = self.child_processes.remove(&pid) {
            let status = child
                .wait()
                .map_err(|err| RuntimeError::new(format!("wait failed: {err}")))?;
            #[cfg(unix)]
            let wait_status = Self::status_to_wait_status(status);
            #[cfg(not(unix))]
            let wait_status = 0;
            self.child_exit_status.insert(pid, wait_status);
            let returncode = status.code().unwrap_or(-1) as i64;
            Self::instance_attr_set(&instance, "returncode", Value::Int(returncode))?;
        }
        Ok(Self::instance_attr_get(&instance, "returncode").unwrap_or(Value::None))
    }

    pub(super) fn builtin_subprocess_popen_kill(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            let instance = self.take_bound_instance_arg(&mut args, "Popen.kill")?;
            let pid = match Self::instance_attr_get(&instance, "pid") {
                Some(Value::Int(pid)) => pid,
                _ => return Err(RuntimeError::new("invalid subprocess handle")),
            };
            if let Some(child) = self.child_processes.get_mut(&pid) {
                let _ = child.kill();
            }
            return Ok(Value::None);
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_subprocess_popen_poll(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("Popen.poll() expects no arguments"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Popen.poll")?;
        let pid = match Self::instance_attr_get(&instance, "pid") {
            Some(Value::Int(pid)) => pid,
            _ => return Err(RuntimeError::new("invalid subprocess handle")),
        };
        if let Some(child) = self.child_processes.get_mut(&pid) {
            if let Some(status) = child
                .try_wait()
                .map_err(|err| RuntimeError::new(format!("poll failed: {err}")))?
            {
                #[cfg(unix)]
                let wait_status = Self::status_to_wait_status(status);
                #[cfg(not(unix))]
                let wait_status = 0;
                self.child_exit_status.insert(pid, wait_status);
                self.child_processes.remove(&pid);
                let returncode = status.code().unwrap_or(-1) as i64;
                Self::instance_attr_set(&instance, "returncode", Value::Int(returncode))?;
            }
        }
        Ok(Self::instance_attr_get(&instance, "returncode").unwrap_or(Value::None))
    }

    pub(super) fn builtin_subprocess_popen_enter(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("Popen.__enter__() expects no keywords"));
        }
        let instance = self.take_bound_instance_arg(&mut args, "Popen.__enter__")?;
        Ok(Value::Instance(instance))
    }

    pub(super) fn builtin_subprocess_popen_exit(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("Popen.__exit__() expects no keywords"));
        }
        let _instance = self.take_bound_instance_arg(&mut args, "Popen.__exit__")?;
        Ok(Value::Bool(false))
    }

    pub(super) fn builtin_subprocess_cleanup(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("_cleanup() expects no arguments"));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_subprocess_check_call(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() && !kwargs.contains_key("args") {
            return Err(RuntimeError::new("check_call() missing args"));
        }
        let cmd_value = if !args.is_empty() {
            args.remove(0)
        } else {
            kwargs
                .remove("args")
                .ok_or_else(|| RuntimeError::new("check_call() missing args"))?
        };
        let mut argv = self.subprocess_argv_from_value(cmd_value)?;
        argv = self.rewrite_pyrs_subprocess_argv(argv)?;
        if argv.is_empty() {
            return Err(RuntimeError::new("check_call() empty command"));
        }
        let cwd = match kwargs.remove("cwd") {
            Some(Value::None) | None => None,
            Some(value) => Some(self.path_arg_to_string(value)?),
        };
        let env = match kwargs.remove("env") {
            Some(value) => Some(self.subprocess_env_from_value(value)?),
            None => None,
        };
        let mut command = Command::new(&argv[0]);
        if argv.len() > 1 {
            command.args(&argv[1..]);
        }
        if let Some(path) = cwd {
            command.current_dir(path);
        }
        if let Some(env_entries) = env {
            command.env_clear();
            for (key, value) in env_entries {
                command.env(key, value);
            }
        }
        let output = command
            .output()
            .map_err(|err| RuntimeError::new(format!("check_call failed: {err}")))?;
        let is_pyrs = is_pyrs_executable(&argv[0]);
        let stderr_text = String::from_utf8_lossy(&output.stderr);
        let success = output.status.success() || (is_pyrs && stderr_text.contains("SystemExit: 0"));
        if success {
            Ok(Value::Int(0))
        } else {
            Err(RuntimeError::new("CalledProcessError"))
        }
    }

    pub(super) fn builtin_subprocess_completed_process_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let instance =
            self.take_bound_instance_arg(&mut args, "CompletedProcess.__init__")?;
        if args.len() > 4 {
            return Err(RuntimeError::new(
                "CompletedProcess() expects args, returncode and optional stdout/stderr",
            ));
        }
        let has_args_kw = kwargs.contains_key("args");
        let has_returncode_kw = kwargs.contains_key("returncode");
        let has_stdout_kw = kwargs.contains_key("stdout");
        let has_stderr_kw = kwargs.contains_key("stderr");
        if !args.is_empty() && has_args_kw {
            return Err(RuntimeError::new(
                "CompletedProcess() got multiple values for argument 'args'",
            ));
        }
        if args.len() > 1 && has_returncode_kw {
            return Err(RuntimeError::new(
                "CompletedProcess() got multiple values for argument 'returncode'",
            ));
        }
        if args.len() > 2 && has_stdout_kw {
            return Err(RuntimeError::new(
                "CompletedProcess() got multiple values for argument 'stdout'",
            ));
        }
        if args.len() > 3 && has_stderr_kw {
            return Err(RuntimeError::new(
                "CompletedProcess() got multiple values for argument 'stderr'",
            ));
        }

        let mut cp_args = if let Some(value) = args.first() {
            Some(value.clone())
        } else {
            kwargs.remove("args")
        };
        let mut returncode = if args.len() > 1 {
            Some(args[1].clone())
        } else {
            kwargs.remove("returncode")
        };
        let mut stdout = if args.len() > 2 {
            Some(args[2].clone())
        } else {
            kwargs.remove("stdout")
        };
        let mut stderr = if args.len() > 3 {
            Some(args[3].clone())
        } else {
            kwargs.remove("stderr")
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "CompletedProcess() got an unexpected keyword argument",
            ));
        }

        let cp_args = cp_args
            .take()
            .ok_or_else(|| RuntimeError::new("CompletedProcess() missing args"))?;
        let returncode = returncode
            .take()
            .ok_or_else(|| RuntimeError::new("CompletedProcess() missing returncode"))?;
        let returncode = Value::Int(value_to_int(returncode)?);
        let stdout = stdout.take().unwrap_or(Value::None);
        let stderr = stderr.take().unwrap_or(Value::None);

        Self::instance_attr_set(&instance, "args", cp_args)?;
        Self::instance_attr_set(&instance, "returncode", returncode)?;
        Self::instance_attr_set(&instance, "stdout", stdout)?;
        Self::instance_attr_set(&instance, "stderr", stderr)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_os_wifstopped(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let status = self.parse_wait_status_arg(args, kwargs)?;
        Ok(Value::Bool((status & 0xff) == 0x7f))
    }

    pub(super) fn builtin_os_wstopsig(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let status = self.parse_wait_status_arg(args, kwargs)?;
        Ok(Value::Int((status >> 8) & 0xff))
    }

    pub(super) fn builtin_os_wifsignaled(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let status = self.parse_wait_status_arg(args, kwargs)?;
        let signal = status & 0x7f;
        Ok(Value::Bool(signal != 0 && signal != 0x7f))
    }

    pub(super) fn builtin_os_wtermsig(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let status = self.parse_wait_status_arg(args, kwargs)?;
        Ok(Value::Int(status & 0x7f))
    }

    pub(super) fn builtin_os_wifexited(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let status = self.parse_wait_status_arg(args, kwargs)?;
        Ok(Value::Bool((status & 0x7f) == 0))
    }

    pub(super) fn builtin_os_wexitstatus(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let status = self.parse_wait_status_arg(args, kwargs)?;
        Ok(Value::Int((status >> 8) & 0xff))
    }

    pub(super) fn parse_wait_status_arg(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<i64, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("status helper expects one argument"));
        }
        value_to_int(args[0].clone())
    }

    pub(super) fn build_stat_result(
        &self,
        metadata: fs::Metadata,
        use_symlink_mode: bool,
    ) -> Result<Value, RuntimeError> {
        let stat_result_class = self
            .modules
            .get("os")
            .and_then(|module| match &*module.kind() {
                Object::Module(module_data) => module_data.globals.get("stat_result").cloned(),
                _ => None,
            })
            .and_then(|value| match value {
                Value::Class(class) => Some(class),
                _ => None,
            })
            .ok_or_else(|| RuntimeError::new("os.stat_result missing"))?;

        if let Object::Class(class_data) = &mut *stat_result_class.kind_mut() {
            class_data
                .attrs
                .entry("__pyrs_tuple_backed_type__".to_string())
                .or_insert(Value::Bool(true));
        }

        let instance = match self
            .heap
            .alloc_instance(InstanceObject::new(stat_result_class))
        {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };

        let file_type = metadata.file_type();
        let mut st_mode = if file_type.is_dir() {
            0o040000
        } else if file_type.is_symlink() || use_symlink_mode {
            0o120000
        } else {
            0o100000
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            st_mode |= i64::from(metadata.permissions().mode() & 0o7777);
        }

        let st_size = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
        let st_atime = metadata
            .accessed()
            .ok()
            .and_then(system_time_to_secs_f64)
            .unwrap_or(0.0);
        let st_mtime = metadata
            .modified()
            .ok()
            .and_then(system_time_to_secs_f64)
            .unwrap_or(0.0);
        let st_ctime = metadata
            .created()
            .ok()
            .and_then(system_time_to_secs_f64)
            .unwrap_or(st_mtime);

        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data
                .attrs
                .insert("st_mode".to_string(), Value::Int(st_mode));
            instance_data
                .attrs
                .insert("st_size".to_string(), Value::Int(st_size));
            instance_data
                .attrs
                .insert("st_atime".to_string(), Value::Float(st_atime));
            instance_data
                .attrs
                .insert("st_mtime".to_string(), Value::Float(st_mtime));
            instance_data
                .attrs
                .insert("st_ctime".to_string(), Value::Float(st_ctime));

            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                instance_data
                    .attrs
                    .insert("st_dev".to_string(), Value::Int(metadata.dev() as i64));
                instance_data
                    .attrs
                    .insert("st_ino".to_string(), Value::Int(metadata.ino() as i64));
                instance_data
                    .attrs
                    .insert("st_nlink".to_string(), Value::Int(metadata.nlink() as i64));
                instance_data
                    .attrs
                    .insert("st_uid".to_string(), Value::Int(metadata.uid() as i64));
                instance_data
                    .attrs
                    .insert("st_gid".to_string(), Value::Int(metadata.gid() as i64));
            }
            #[cfg(not(unix))]
            {
                instance_data
                    .attrs
                    .insert("st_dev".to_string(), Value::Int(0));
                instance_data
                    .attrs
                    .insert("st_ino".to_string(), Value::Int(0));
                instance_data
                    .attrs
                    .insert("st_nlink".to_string(), Value::Int(0));
                instance_data
                    .attrs
                    .insert("st_uid".to_string(), Value::Int(0));
                instance_data
                    .attrs
                    .insert("st_gid".to_string(), Value::Int(0));
            }

            // Model os.stat_result as a tuple-backed struct-sequence so tuple
            // protocol behavior (len/iter/equality/pickle) matches CPython.
            let st_ino = instance_data
                .attrs
                .get("st_ino")
                .cloned()
                .unwrap_or(Value::Int(0));
            let st_dev = instance_data
                .attrs
                .get("st_dev")
                .cloned()
                .unwrap_or(Value::Int(0));
            let st_nlink = instance_data
                .attrs
                .get("st_nlink")
                .cloned()
                .unwrap_or(Value::Int(0));
            let st_uid = instance_data
                .attrs
                .get("st_uid")
                .cloned()
                .unwrap_or(Value::Int(0));
            let st_gid = instance_data
                .attrs
                .get("st_gid")
                .cloned()
                .unwrap_or(Value::Int(0));
            let tuple_payload = self.heap.alloc_tuple(vec![
                Value::Int(st_mode),
                st_ino,
                st_dev,
                st_nlink,
                st_uid,
                st_gid,
                Value::Int(st_size),
                Value::Float(st_atime),
                Value::Float(st_mtime),
                Value::Float(st_ctime),
            ]);
            instance_data
                .attrs
                .insert(TUPLE_BACKING_STORAGE_ATTR.to_string(), tuple_payload);
        }

        Ok(Value::Instance(instance))
    }

    pub(super) fn builtin_os_path_exists(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("path_exists() expects one argument"));
        }
        match &args[0] {
            Value::Int(fd) => {
                if *fd < 0 {
                    return Ok(Value::Bool(false));
                }
                if self.find_open_file(*fd).is_some() {
                    return Ok(Value::Bool(true));
                }
                let fd_path = format!("/proc/self/fd/{fd}");
                let fallback_fd_path = format!("/dev/fd/{fd}");
                Ok(Value::Bool(
                    fs::metadata(&fd_path).is_ok() || fs::metadata(&fallback_fd_path).is_ok(),
                ))
            }
            Value::Bool(flag) => {
                let fd = if *flag { 1 } else { 0 };
                if self.find_open_file(fd).is_some() {
                    return Ok(Value::Bool(true));
                }
                let fd_path = format!("/proc/self/fd/{fd}");
                let fallback_fd_path = format!("/dev/fd/{fd}");
                Ok(Value::Bool(
                    fs::metadata(&fd_path).is_ok() || fs::metadata(&fallback_fd_path).is_ok(),
                ))
            }
            _ => {
                let (path, _) = self.path_arg_to_string_and_type(args[0].clone())?;
                Ok(Value::Bool(PathBuf::from(path).exists()))
            }
        }
    }

    pub(super) fn path_arg_to_string_and_type(
        &mut self,
        value: Value,
    ) -> Result<(String, bool), RuntimeError> {
        let validate_path =
            |path: String, is_bytes: bool| -> Result<(String, bool), RuntimeError> {
                if path.contains('\0') {
                    return Err(RuntimeError::new(
                        "ValueError: embedded null character in path",
                    ));
                }
                Ok((path, is_bytes))
            };
        let normalized = match value {
            Value::Str(_) | Value::Bytes(_) => value,
            other => self.builtin_os_fspath(vec![other], HashMap::new())?,
        };
        match normalized {
            Value::Str(path) => validate_path(path, false),
            Value::Bytes(obj) => match &*obj.kind() {
                Object::Bytes(bytes) => {
                    validate_path(String::from_utf8_lossy(bytes).into_owned(), true)
                }
                _ => Err(RuntimeError::new("path must be string or bytes")),
            },
            _ => Err(RuntimeError::new("path must be string or bytes")),
        }
    }

    pub(super) fn path_arg_to_string(&mut self, value: Value) -> Result<String, RuntimeError> {
        let (path, _) = self.path_arg_to_string_and_type(value)?;
        Ok(path)
    }

    pub(super) fn builtin_os_path_join(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "path_join() does not accept keyword arguments",
            ));
        }
        if args.is_empty() {
            return Ok(Value::Str(".".to_string()));
        }
        let (first, first_is_bytes) = self.path_arg_to_string_and_type(args.remove(0))?;
        let mut out = PathBuf::from(first);
        for part in args {
            let (part, part_is_bytes) = self.path_arg_to_string_and_type(part)?;
            if part_is_bytes != first_is_bytes {
                return Err(RuntimeError::new(
                    "can't mix strings and bytes in path components",
                ));
            }
            out.push(part);
        }
        let joined = out.to_string_lossy().to_string();
        if first_is_bytes {
            Ok(self.heap.alloc_bytes(joined.into_bytes()))
        } else {
            Ok(Value::Str(joined))
        }
    }

    pub(super) fn builtin_os_path_normpath(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("_path_normpath() expects one argument"));
        }
        let (path, return_bytes) = self.path_arg_to_string_and_type(args[0].clone())?;
        if path.is_empty() {
            return if return_bytes {
                Ok(self.heap.alloc_bytes(b".".to_vec()))
            } else {
                Ok(Value::Str(".".to_string()))
            };
        }

        let (initial_slashes, remainder) = if let Some(stripped) = path.strip_prefix("//") {
            if stripped.starts_with('/') {
                ("/", path.trim_start_matches('/'))
            } else {
                ("//", stripped)
            }
        } else if let Some(stripped) = path.strip_prefix('/') {
            ("/", stripped)
        } else {
            ("", path.as_str())
        };

        let mut comps: Vec<&str> = Vec::new();
        for comp in remainder.split('/') {
            if comp.is_empty() || comp == "." {
                continue;
            }
            if comp != ".."
                || (initial_slashes.is_empty() && comps.is_empty())
                || comps.last().copied() == Some("..")
            {
                comps.push(comp);
            } else if !comps.is_empty() {
                comps.pop();
            }
        }

        let joined = comps.join("/");
        let out = format!("{initial_slashes}{joined}");
        if out.is_empty() {
            if return_bytes {
                Ok(self.heap.alloc_bytes(b".".to_vec()))
            } else {
                Ok(Value::Str(".".to_string()))
            }
        } else {
            if return_bytes {
                Ok(self.heap.alloc_bytes(out.into_bytes()))
            } else {
                Ok(Value::Str(out))
            }
        }
    }

    pub(super) fn builtin_os_path_normcase(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("normcase() expects one argument"));
        }
        let (path, return_bytes) = self.path_arg_to_string_and_type(args[0].clone())?;
        let out = if cfg!(windows) {
            path.replace('/', "\\").to_ascii_lowercase()
        } else {
            path
        };
        if return_bytes {
            Ok(self.heap.alloc_bytes(out.into_bytes()))
        } else {
            Ok(Value::Str(out))
        }
    }

    pub(super) fn builtin_os_path_splitroot_ex(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "_path_splitroot_ex() expects one argument",
            ));
        }
        let (path, _) = self.path_arg_to_string_and_type(args[0].clone())?;
        let bytes = path.as_bytes();
        let (drive, root, tail) = if bytes.first().copied() != Some(b'/') {
            ("".to_string(), "".to_string(), path)
        } else if bytes.get(1).copied() != Some(b'/') || bytes.get(2).copied() == Some(b'/') {
            ("".to_string(), "/".to_string(), path[1..].to_string())
        } else {
            ("".to_string(), path[..2].to_string(), path[2..].to_string())
        };
        Ok(self
            .heap
            .alloc_tuple(vec![Value::Str(drive), Value::Str(root), Value::Str(tail)]))
    }

    pub(super) fn builtin_os_path_dirname(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("dirname() expects one argument"));
        }
        let (path, return_bytes) = self.path_arg_to_string_and_type(args[0].clone())?;
        if path.is_empty() {
            return if return_bytes {
                Ok(self.heap.alloc_bytes(Vec::new()))
            } else {
                Ok(Value::Str(String::new()))
            };
        }
        let idx = path.rfind('/').map(|value| value + 1).unwrap_or(0);
        let mut head = path[..idx].to_string();
        if !head.is_empty() && head != "/".repeat(head.len()) {
            while head.ends_with('/') {
                head.pop();
            }
        }
        if return_bytes {
            Ok(self.heap.alloc_bytes(head.into_bytes()))
        } else {
            Ok(Value::Str(head))
        }
    }

    pub(super) fn builtin_os_path_split(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("split() expects one argument"));
        }
        let (path, return_bytes) = self.path_arg_to_string_and_type(args[0].clone())?;
        let idx = path.rfind('/').map(|value| value + 1).unwrap_or(0);
        let mut head = path[..idx].to_string();
        let tail = path[idx..].to_string();
        if !head.is_empty() && head != "/".repeat(head.len()) {
            while head.ends_with('/') {
                head.pop();
            }
        }
        if return_bytes {
            Ok(self.heap.alloc_tuple(vec![
                self.heap.alloc_bytes(head.into_bytes()),
                self.heap.alloc_bytes(tail.into_bytes()),
            ]))
        } else {
            Ok(self
                .heap
                .alloc_tuple(vec![Value::Str(head), Value::Str(tail)]))
        }
    }

    pub(super) fn builtin_os_path_basename(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("basename() expects one argument"));
        }
        let (path, return_bytes) = self.path_arg_to_string_and_type(args[0].clone())?;
        let idx = path.rfind('/').map(|value| value + 1).unwrap_or(0);
        let tail = path[idx..].to_string();
        if return_bytes {
            Ok(self.heap.alloc_bytes(tail.into_bytes()))
        } else {
            Ok(Value::Str(tail))
        }
    }

    pub(super) fn builtin_os_path_isabs(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("isabs() expects one argument"));
        }
        let (path, _) = self.path_arg_to_string_and_type(args[0].clone())?;
        Ok(Value::Bool(path.starts_with('/')))
    }

    pub(super) fn builtin_os_path_isdir(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("isdir() expects one argument"));
        }
        let (path, _) = self.path_arg_to_string_and_type(args[0].clone())?;
        Ok(Value::Bool(PathBuf::from(path).is_dir()))
    }

    pub(super) fn builtin_os_path_isfile(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("isfile() expects one argument"));
        }
        let (path, _) = self.path_arg_to_string_and_type(args[0].clone())?;
        Ok(Value::Bool(PathBuf::from(path).is_file()))
    }

    pub(super) fn builtin_os_path_islink(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("islink() expects one argument"));
        }
        let (path, _) = self.path_arg_to_string_and_type(args[0].clone())?;
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(_) => return Ok(Value::Bool(false)),
        };
        Ok(Value::Bool(metadata.file_type().is_symlink()))
    }

    pub(super) fn builtin_os_path_isjunction(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("isjunction() expects one argument"));
        }
        #[cfg(windows)]
        {
            let (path, _) = self.path_arg_to_string_and_type(args[0].clone())?;
            let metadata = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(_) => return Ok(Value::Bool(false)),
            };
            return Ok(Value::Bool(
                metadata.file_type().is_symlink() && metadata.is_dir(),
            ));
        }
        #[cfg(not(windows))]
        {
            Ok(Value::Bool(false))
        }
    }

    pub(super) fn builtin_os_path_splitext(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("splitext() expects one argument"));
        }
        let (path, return_bytes) = self.path_arg_to_string_and_type(args[0].clone())?;
        let slash = path.rfind('/').map(|idx| idx + 1).unwrap_or(0);
        let dot = path[slash..]
            .rfind('.')
            .map(|idx| slash + idx)
            .filter(|idx| *idx > slash);
        let (root, ext) = if let Some(idx) = dot {
            (path[..idx].to_string(), path[idx..].to_string())
        } else {
            (path, String::new())
        };
        if return_bytes {
            Ok(self.heap.alloc_tuple(vec![
                self.heap.alloc_bytes(root.into_bytes()),
                self.heap.alloc_bytes(ext.into_bytes()),
            ]))
        } else {
            Ok(self
                .heap
                .alloc_tuple(vec![Value::Str(root), Value::Str(ext)]))
        }
    }

    pub(super) fn builtin_os_path_abspath(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("abspath() expects one argument"));
        }
        let (path, return_bytes) = self.path_arg_to_string_and_type(args[0].clone())?;
        let joined = if path.starts_with('/') {
            path
        } else {
            let cwd = std::env::current_dir()
                .map_err(|err| RuntimeError::new(format!("abspath failed: {err}")))?;
            cwd.join(path).to_string_lossy().to_string()
        };
        let normalized = self.builtin_os_path_normpath(vec![Value::Str(joined)], HashMap::new())?;
        if return_bytes {
            match normalized {
                Value::Str(text) => Ok(self.heap.alloc_bytes(text.into_bytes())),
                other => Ok(other),
            }
        } else {
            Ok(normalized)
        }
    }

    pub(super) fn builtin_os_path_expanduser(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("expanduser() expects one argument"));
        }
        let (path, return_bytes) = self.path_arg_to_string_and_type(args[0].clone())?;
        if !path.starts_with('~') {
            return if return_bytes {
                Ok(self.heap.alloc_bytes(path.into_bytes()))
            } else {
                Ok(Value::Str(path))
            };
        }
        if path != "~" && !path.starts_with("~/") {
            return if return_bytes {
                Ok(self.heap.alloc_bytes(path.into_bytes()))
            } else {
                Ok(Value::Str(path))
            };
        }

        let home = std::env::var("HOME")
            .ok()
            .or_else(|| std::env::var("USERPROFILE").ok())
            .or_else(|| {
                let drive = std::env::var("HOMEDRIVE").ok()?;
                let home = std::env::var("HOMEPATH").ok()?;
                Some(format!("{drive}{home}"))
            })
            .unwrap_or_else(|| "~".to_string());
        if path == "~" {
            if return_bytes {
                Ok(self.heap.alloc_bytes(home.into_bytes()))
            } else {
                Ok(Value::Str(home))
            }
        } else {
            let expanded = format!("{home}{}", &path[1..]);
            if return_bytes {
                Ok(self.heap.alloc_bytes(expanded.into_bytes()))
            } else {
                Ok(Value::Str(expanded))
            }
        }
    }

    pub(super) fn builtin_os_path_realpath(
        &mut self,
        args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::new("realpath() expects one argument"));
        }
        if let Some(strict) = kwargs.remove("strict") {
            match strict {
                Value::Bool(_) | Value::Int(_) | Value::None => {}
                _ => return Err(RuntimeError::new("realpath() strict must be bool")),
            }
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "realpath() got an unexpected keyword argument",
            ));
        }
        self.builtin_os_path_abspath(args, HashMap::new())
    }

    pub(super) fn builtin_os_path_relpath(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "relpath() expects path and optional start",
            ));
        }

        let path = self.path_arg_to_string(args.remove(0))?;
        let start = if let Some(value) = args.pop() {
            self.path_arg_to_string(value)?
        } else {
            ".".to_string()
        };
        let path_abs = match self.builtin_os_path_abspath(vec![Value::Str(path)], HashMap::new())? {
            Value::Str(path) => path,
            _ => return Err(RuntimeError::new("relpath() internal error")),
        };
        let start_abs =
            match self.builtin_os_path_abspath(vec![Value::Str(start)], HashMap::new())? {
                Value::Str(path) => path,
                _ => return Err(RuntimeError::new("relpath() internal error")),
            };

        let mut path_parts: Vec<&str> = path_abs
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        let mut start_parts: Vec<&str> = start_abs
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();

        let mut common = 0usize;
        let max_common = path_parts.len().min(start_parts.len());
        while common < max_common && path_parts[common] == start_parts[common] {
            common += 1;
        }

        path_parts.drain(0..common);
        start_parts.drain(0..common);
        let mut rel_parts: Vec<String> = Vec::new();
        rel_parts.extend(std::iter::repeat("..".to_string()).take(start_parts.len()));
        rel_parts.extend(path_parts.into_iter().map(ToOwned::to_owned));

        if rel_parts.is_empty() {
            Ok(Value::Str(".".to_string()))
        } else {
            Ok(Value::Str(rel_parts.join("/")))
        }
    }

    pub(super) fn builtin_os_path_commonprefix(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "commonprefix() expects one iterable argument",
            ));
        }
        let values = self.collect_iterable_values(args.remove(0))?;
        if values.is_empty() {
            return Ok(Value::Str(String::new()));
        }
        let mut parts = Vec::with_capacity(values.len());
        for value in values {
            match value {
                Value::Str(text) => parts.push(text),
                _ => {
                    return Err(RuntimeError::new(
                        "commonprefix() expects iterable of strings",
                    ));
                }
            }
        }
        let mut prefix = parts[0].clone();
        for text in parts.iter().skip(1) {
            let mut idx = 0usize;
            let prefix_bytes = prefix.as_bytes();
            let text_bytes = text.as_bytes();
            let max = prefix_bytes.len().min(text_bytes.len());
            while idx < max && prefix_bytes[idx] == text_bytes[idx] {
                idx += 1;
            }
            prefix.truncate(idx);
            if prefix.is_empty() {
                break;
            }
        }
        Ok(Value::Str(prefix))
    }

    pub(super) fn builtin_pylong_int_to_decimal_string(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "int_to_decimal_string() expects one argument",
            ));
        }
        let value = value_to_bigint(args.remove(0))?;
        Ok(Value::Str(value.to_string()))
    }

    pub(super) fn builtin_pylong_int_divmod(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("int_divmod() expects two arguments"));
        }
        let left = value_to_bigint(args.remove(0))?;
        let right = value_to_bigint(args.remove(0))?;
        let (quotient, remainder) = left
            .div_mod_floor(&right)
            .ok_or_else(|| RuntimeError::new("int_divmod() division by zero"))?;
        Ok(self.heap.alloc_tuple(vec![
            value_from_bigint(quotient),
            value_from_bigint(remainder),
        ]))
    }

    pub(super) fn builtin_pylong_int_from_string(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("int_from_string() expects one argument"));
        }
        let text = match args.remove(0) {
            Value::Str(text) => text,
            _ => return Err(RuntimeError::new("int_from_string() expects a string")),
        };
        Ok(value_from_bigint(parse_decimal_bigint_literal(&text)?))
    }

    pub(super) fn builtin_pylong_compute_powers(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 3 {
            return Err(RuntimeError::new(
                "compute_powers() expects w, base, and more_than",
            ));
        }
        let _need_hi = kwargs.remove("need_hi");
        let _show = kwargs.remove("show");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "compute_powers() got an unexpected keyword argument",
            ));
        }
        let w = value_to_int(args.remove(0))?;
        let base = args.remove(0);
        let more_than = value_to_int(args.remove(0))?;
        if w < 0 || more_than < 0 {
            return Err(RuntimeError::new(
                "compute_powers() expects non-negative bounds",
            ));
        }
        if w <= more_than {
            return Ok(self.heap.alloc_dict(Vec::new()));
        }
        let max_entries = (w - more_than).min(2048) as usize;
        if (w - more_than) as usize > max_entries {
            return Err(RuntimeError::new("compute_powers() range too large"));
        }
        let mut entries = Vec::with_capacity(max_entries);
        match base {
            Value::Int(base) => {
                let base_big = BigInt::from_i64(base);
                for exponent in (more_than + 1)..=w {
                    let exponent_u32 = u32::try_from(exponent)
                        .map_err(|_| RuntimeError::new("compute_powers() exponent out of range"))?;
                    let value = base_big.pow_u64(exponent_u32 as u64);
                    entries.push((Value::Int(exponent as i64), value_from_bigint(value)));
                }
            }
            Value::Bool(base) => {
                let base_big = BigInt::from_i64(if base { 1 } else { 0 });
                for exponent in (more_than + 1)..=w {
                    let exponent_u32 = u32::try_from(exponent)
                        .map_err(|_| RuntimeError::new("compute_powers() exponent out of range"))?;
                    let value = base_big.pow_u64(exponent_u32 as u64);
                    entries.push((Value::Int(exponent as i64), value_from_bigint(value)));
                }
            }
            Value::BigInt(base_big) => {
                for exponent in (more_than + 1)..=w {
                    let exponent_u32 = u32::try_from(exponent)
                        .map_err(|_| RuntimeError::new("compute_powers() exponent out of range"))?;
                    let value = base_big.pow_u64(exponent_u32 as u64);
                    entries.push((Value::Int(exponent as i64), value_from_bigint(value)));
                }
            }
            Value::Float(base) => {
                for exponent in (more_than + 1)..=w {
                    let exponent = i32::try_from(exponent)
                        .map_err(|_| RuntimeError::new("compute_powers() exponent out of range"))?;
                    entries.push((
                        Value::Int(exponent as i64),
                        Value::Float(base.powi(exponent)),
                    ));
                }
            }
            _ => {
                return Err(RuntimeError::new(
                    "compute_powers() base must be int, bool, bigint, or float",
                ));
            }
        }
        Ok(self.heap.alloc_dict(entries))
    }

    pub(super) fn builtin_pylong_dec_str_to_int_inner(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::new(
                "_dec_str_to_int_inner() expects one string argument",
            ));
        }
        let _guard = kwargs.remove("GUARD");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "_dec_str_to_int_inner() got an unexpected keyword argument",
            ));
        }
        let text = match args.remove(0) {
            Value::Str(text) => text,
            _ => {
                return Err(RuntimeError::new(
                    "_dec_str_to_int_inner() expects a string",
                ));
            }
        };
        Ok(value_from_bigint(parse_decimal_bigint_literal(&text)?))
    }

    pub(super) fn builtin_string_formatter_parser(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("formatter_parser() expects one argument"));
        }
        let format_string = match &args[0] {
            Value::Str(text) => text.as_str(),
            _ => {
                return Err(RuntimeError::new(
                    "formatter_parser() expects string argument",
                ));
            }
        };
        let parsed = parse_string_formatter(format_string)?;
        let mut rows = Vec::with_capacity(parsed.len());
        for (literal, field_name, format_spec, conversion) in parsed {
            let tuple = self.heap.alloc_tuple(vec![
                Value::Str(literal),
                match field_name {
                    Some(name) => Value::Str(name),
                    None => Value::None,
                },
                match format_spec {
                    Some(spec) => Value::Str(spec),
                    None => Value::None,
                },
                match conversion {
                    Some(conv) => Value::Str(conv),
                    None => Value::None,
                },
            ]);
            rows.push(tuple);
        }
        Ok(self.heap.alloc_list(rows))
    }

    pub(super) fn builtin_string_formatter_field_name_split(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "formatter_field_name_split() expects one argument",
            ));
        }
        let field_name = match &args[0] {
            Value::Str(text) => text.as_str(),
            _ => {
                return Err(RuntimeError::new(
                    "formatter_field_name_split() expects string argument",
                ));
            }
        };
        let (first, rest) = split_formatter_field_name(field_name)?;
        let mut rest_values = Vec::with_capacity(rest.len());
        for (is_attr, key) in rest {
            let key_value = match key {
                FormatterFieldKey::Int(value) => Value::Int(value),
                FormatterFieldKey::Str(value) => Value::Str(value),
            };
            rest_values.push(self.heap.alloc_tuple(vec![Value::Bool(is_attr), key_value]));
        }
        let first_value = match first {
            FormatterFieldKey::Int(value) => Value::Int(value),
            FormatterFieldKey::Str(value) => Value::Str(value),
        };
        Ok(self
            .heap
            .alloc_tuple(vec![first_value, self.heap.alloc_list(rest_values)]))
    }

    pub(super) fn builtin_codecs_encode(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 3 {
            return Err(RuntimeError::new(
                "encode() expects object, optional encoding, optional errors",
            ));
        }
        let mut encoding = if args.len() >= 2 {
            Some(args.remove(1))
        } else {
            None
        };
        let mut errors = if args.len() >= 2 {
            Some(args.remove(1))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("encoding") {
            if encoding.is_some() {
                return Err(RuntimeError::new(
                    "encode() got multiple values for encoding",
                ));
            }
            encoding = Some(value);
        }
        if let Some(value) = kwargs.remove("errors") {
            if errors.is_some() {
                return Err(RuntimeError::new("encode() got multiple values for errors"));
            }
            errors = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "encode() got an unexpected keyword argument",
            ));
        }
        let source = args.remove(0);
        let text = match source {
            Value::Str(text) => text,
            _ => return Err(RuntimeError::new("encode() argument must be str")),
        };
        let encoding =
            normalize_codec_encoding(encoding.unwrap_or(Value::Str("utf-8".to_string())))?;
        let errors = normalize_codec_errors(errors.unwrap_or(Value::Str("strict".to_string())))?;
        let encoded = encode_text_bytes(&text, &encoding, &errors)?;
        Ok(self.heap.alloc_bytes(encoded))
    }

    pub(super) fn builtin_codecs_decode(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 3 {
            return Err(RuntimeError::new(
                "decode() expects object, optional encoding, optional errors",
            ));
        }
        let mut encoding = if args.len() >= 2 {
            Some(args.remove(1))
        } else {
            None
        };
        let mut errors = if args.len() >= 2 {
            Some(args.remove(1))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("encoding") {
            if encoding.is_some() {
                return Err(RuntimeError::new(
                    "decode() got multiple values for encoding",
                ));
            }
            encoding = Some(value);
        }
        if let Some(value) = kwargs.remove("errors") {
            if errors.is_some() {
                return Err(RuntimeError::new("decode() got multiple values for errors"));
            }
            errors = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "decode() got an unexpected keyword argument",
            ));
        }
        let source = args.remove(0);
        let bytes = bytes_like_from_value(source)?;
        let encoding =
            normalize_codec_encoding(encoding.unwrap_or(Value::Str("utf-8".to_string())))?;
        let errors = normalize_codec_errors(errors.unwrap_or(Value::Str("strict".to_string())))?;
        let decoded = decode_text_bytes(&bytes, &encoding, &errors)?;
        Ok(Value::Str(decoded))
    }

    pub(super) fn builtin_codecs_escape_decode(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "escape_decode() expects object, optional errors",
            ));
        }
        let mut errors = if args.len() == 2 {
            Some(args.remove(1))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("errors") {
            if errors.is_some() {
                return Err(RuntimeError::new(
                    "escape_decode() got multiple values for errors",
                ));
            }
            errors = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "escape_decode() got an unexpected keyword argument",
            ));
        }
        let source = args.remove(0);
        let bytes = match source {
            Value::Str(text) => text.into_bytes(),
            other => bytes_like_from_value(other)?,
        };
        let errors = normalize_codec_errors(errors.unwrap_or(Value::Str("strict".to_string())))?;
        let decoded = decode_escape_bytes(&bytes, &errors)?;
        Ok(self.heap.alloc_tuple(vec![
            self.heap.alloc_bytes(decoded),
            Value::Int(bytes.len() as i64),
        ]))
    }

    pub(super) fn builtin_codecs_lookup(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("lookup() expects one argument"));
        }
        let encoding = normalize_codec_encoding(args.remove(0))?;
        let codec_module = self
            .modules
            .get("codecs")
            .cloned()
            .ok_or_else(|| RuntimeError::new("codecs module unavailable"))?;
        let codec_info_class = match &*codec_module.kind() {
            Object::Module(module_data) => match module_data.globals.get("CodecInfo") {
                Some(Value::Class(class)) => class.clone(),
                _ => return Err(RuntimeError::new("codecs.CodecInfo missing")),
            },
            _ => return Err(RuntimeError::new("invalid codecs module")),
        };
        let incremental_encoder = self.builtin_codecs_getincrementalencoder(
            vec![Value::Str(encoding.clone())],
            HashMap::new(),
        )?;
        let incremental_decoder = self.builtin_codecs_getincrementaldecoder(
            vec![Value::Str(encoding.clone())],
            HashMap::new(),
        )?;
        let instance = match self
            .heap
            .alloc_instance(InstanceObject::new(codec_info_class))
        {
            Value::Instance(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Instance(instance_data) = &mut *instance.kind_mut() {
            instance_data
                .attrs
                .insert("name".to_string(), Value::Str(encoding));
            instance_data.attrs.insert(
                "encode".to_string(),
                Value::Builtin(BuiltinFunction::CodecsEncode),
            );
            instance_data.attrs.insert(
                "decode".to_string(),
                Value::Builtin(BuiltinFunction::CodecsDecode),
            );
            instance_data
                .attrs
                .insert("incrementalencoder".to_string(), incremental_encoder);
            instance_data
                .attrs
                .insert("incrementaldecoder".to_string(), incremental_decoder);
            instance_data
                .attrs
                .insert("streamwriter".to_string(), Value::None);
            instance_data
                .attrs
                .insert("streamreader".to_string(), Value::None);
            instance_data
                .attrs
                .insert("_is_text_encoding".to_string(), Value::Bool(true));
        }
        Ok(Value::Instance(instance))
    }

    pub(super) fn builtin_codecs_register(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("register() expects one argument"));
        }
        if !self.is_callable_value(&args[0]) {
            return Err(RuntimeError::new("argument must be callable"));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_codecs_getincrementalencoder(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "getincrementalencoder() expects one argument",
            ));
        }
        let encoding = normalize_codec_encoding(args.remove(0))?;
        let factory = match self.heap.alloc_module(ModuleObject::new(
            "__codecs_incremental_encoder_factory__".to_string(),
        )) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *factory.kind_mut() {
            module_data
                .globals
                .insert("encoding".to_string(), Value::Str(encoding));
        }
        Ok(self.alloc_native_bound_method(
            NativeMethodKind::CodecsIncrementalEncoderFactoryCall,
            factory,
        ))
    }

    pub(super) fn builtin_codecs_getincrementaldecoder(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "getincrementaldecoder() expects one argument",
            ));
        }
        let encoding = normalize_codec_encoding(args.remove(0))?;
        let factory = match self.heap.alloc_module(ModuleObject::new(
            "__codecs_incremental_decoder_factory__".to_string(),
        )) {
            Value::Module(obj) => obj,
            _ => unreachable!(),
        };
        if let Object::Module(module_data) = &mut *factory.kind_mut() {
            module_data
                .globals
                .insert("encoding".to_string(), Value::Str(encoding));
        }
        Ok(self.alloc_native_bound_method(
            NativeMethodKind::CodecsIncrementalDecoderFactoryCall,
            factory,
        ))
    }

    pub(super) fn builtin_codecs_incremental_encoder_encode(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "IncrementalEncoder.encode() expects input and optional final argument",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "IncrementalEncoder.encode() requires instance receiver",
                ));
            }
        };
        let mut final_arg = if args.len() == 2 {
            Some(args.remove(1))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("final") {
            if final_arg.is_some() {
                return Err(RuntimeError::new("encode() got multiple values for final"));
            }
            final_arg = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "encode() got an unexpected keyword argument",
            ));
        }
        if let Some(value) = final_arg {
            let _ = is_truthy(&value);
        }
        let input = args.remove(0);
        let text = match input {
            Value::Str(text) => text,
            _ => return Err(RuntimeError::new("encoder input must be str")),
        };
        let (encoding, errors) = match &*receiver.kind() {
            Object::Instance(instance_data) => {
                let encoding = match instance_data.attrs.get(CODECS_ATTR_ENCODING) {
                    Some(Value::Str(value)) => value.clone(),
                    _ => return Err(RuntimeError::new("incremental encoder is uninitialized")),
                };
                let errors = match instance_data.attrs.get(CODECS_ATTR_ERRORS) {
                    Some(Value::Str(value)) => value.clone(),
                    _ => "strict".to_string(),
                };
                (encoding, errors)
            }
            _ => return Err(RuntimeError::new("incremental encoder is uninitialized")),
        };
        let encoded = encode_text_bytes(&text, &encoding, &errors)?;
        Ok(self.heap.alloc_bytes(encoded))
    }

    pub(super) fn builtin_codecs_incremental_encoder_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "IncrementalEncoder.__init__() accepts optional errors",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "IncrementalEncoder.__init__() requires instance receiver",
                ));
            }
        };
        let mut errors = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("errors") {
            if errors.is_some() {
                return Err(RuntimeError::new(
                    "IncrementalEncoder.__init__() got multiple values for errors",
                ));
            }
            errors = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "IncrementalEncoder.__init__() got unexpected keyword argument",
            ));
        }
        let errors = normalize_codec_errors(errors.unwrap_or(Value::Str("strict".to_string())))?;
        if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
            instance_data
                .attrs
                .entry(CODECS_ATTR_ENCODING.to_string())
                .or_insert_with(|| Value::Str("utf-8".to_string()));
            instance_data
                .attrs
                .insert(CODECS_ATTR_ERRORS.to_string(), Value::Str(errors));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_codecs_incremental_encoder_reset(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "IncrementalEncoder.reset() expects no arguments",
            ));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_codecs_incremental_encoder_getstate(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "IncrementalEncoder.getstate() expects no arguments",
            ));
        }
        Ok(Value::Int(0))
    }

    pub(super) fn builtin_codecs_incremental_encoder_setstate(
        &self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "IncrementalEncoder.setstate() expects state argument",
            ));
        }
        let _receiver = args.remove(0);
        let mut state = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("state") {
            if state.is_some() {
                return Err(RuntimeError::new(
                    "setstate() got multiple values for state",
                ));
            }
            state = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "setstate() got an unexpected keyword argument",
            ));
        }
        let state = state
            .ok_or_else(|| RuntimeError::new("IncrementalEncoder.setstate() missing state"))?;
        let _ = value_to_int(state)?;
        Ok(Value::None)
    }

    pub(super) fn builtin_codecs_incremental_decoder_decode(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(RuntimeError::new(
                "IncrementalDecoder.decode() expects input and optional final argument",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "IncrementalDecoder.decode() requires instance receiver",
                ));
            }
        };
        let mut final_arg = if args.len() == 2 {
            Some(args.remove(1))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("final") {
            if final_arg.is_some() {
                return Err(RuntimeError::new("decode() got multiple values for final"));
            }
            final_arg = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "decode() got an unexpected keyword argument",
            ));
        }
        let final_decode = if let Some(value) = final_arg {
            is_truthy(&value)
        } else {
            false
        };
        let input = bytes_like_from_value(args.remove(0))?;
        let (encoding, errors, pending, state_flag) = match &*receiver.kind() {
            Object::Instance(instance_data) => {
                let encoding = match instance_data.attrs.get(CODECS_ATTR_ENCODING) {
                    Some(Value::Str(value)) => value.clone(),
                    _ => return Err(RuntimeError::new("incremental decoder is uninitialized")),
                };
                let errors = match instance_data.attrs.get(CODECS_ATTR_ERRORS) {
                    Some(Value::Str(value)) => value.clone(),
                    _ => "strict".to_string(),
                };
                let pending = match instance_data.attrs.get(CODECS_ATTR_PENDING) {
                    Some(value) => bytes_like_from_value(value.clone())?,
                    None => Vec::new(),
                };
                let state_flag = match instance_data.attrs.get(CODECS_ATTR_STATE_FLAG) {
                    Some(Value::Int(value)) => *value,
                    _ => 0,
                };
                (encoding, errors, pending, state_flag)
            }
            _ => return Err(RuntimeError::new("incremental decoder is uninitialized")),
        };

        let mut combined = pending;
        combined.extend_from_slice(&input);
        let (decoded, pending_tail) = if final_decode {
            (
                decode_text_bytes(&combined, &encoding, &errors)?,
                Vec::new(),
            )
        } else {
            let max_tail = match encoding.as_str() {
                "utf-8" => 3usize,
                "utf-16" | "utf-16-le" | "utf-16-be" => 1usize,
                "utf-32" | "utf-32-le" | "utf-32-be" => 3usize,
                _ => 0usize,
            };
            let max_try = max_tail.min(combined.len());
            let mut parsed = None;
            for tail_len in 0..=max_try {
                let split_at = combined.len() - tail_len;
                match decode_text_bytes(&combined[..split_at], &encoding, &errors) {
                    Ok(text) => {
                        parsed = Some((text, combined[split_at..].to_vec()));
                        break;
                    }
                    Err(_) => continue,
                }
            }
            match parsed {
                Some(value) => value,
                None => (
                    decode_text_bytes(&combined, &encoding, &errors)?,
                    Vec::new(),
                ),
            }
        };
        let pending_value = self.heap.alloc_bytes(pending_tail);
        if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
            instance_data
                .attrs
                .insert(CODECS_ATTR_PENDING.to_string(), pending_value);
            instance_data
                .attrs
                .insert(CODECS_ATTR_STATE_FLAG.to_string(), Value::Int(state_flag));
        }
        Ok(Value::Str(decoded))
    }

    pub(super) fn builtin_codecs_incremental_decoder_init(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "IncrementalDecoder.__init__() accepts optional errors",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "IncrementalDecoder.__init__() requires instance receiver",
                ));
            }
        };
        let mut errors = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("errors") {
            if errors.is_some() {
                return Err(RuntimeError::new(
                    "IncrementalDecoder.__init__() got multiple values for errors",
                ));
            }
            errors = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "IncrementalDecoder.__init__() got unexpected keyword argument",
            ));
        }
        let errors = normalize_codec_errors(errors.unwrap_or(Value::Str("strict".to_string())))?;
        let pending = self.heap.alloc_bytes(Vec::new());
        if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
            instance_data
                .attrs
                .entry(CODECS_ATTR_ENCODING.to_string())
                .or_insert_with(|| Value::Str("utf-8".to_string()));
            instance_data
                .attrs
                .insert(CODECS_ATTR_ERRORS.to_string(), Value::Str(errors));
            instance_data
                .attrs
                .insert(CODECS_ATTR_PENDING.to_string(), pending);
            instance_data
                .attrs
                .insert(CODECS_ATTR_STATE_FLAG.to_string(), Value::Int(0));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_codecs_incremental_decoder_reset(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "IncrementalDecoder.reset() expects no arguments",
            ));
        }
        let receiver = match &args[0] {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "IncrementalDecoder.reset() invalid receiver",
                ));
            }
        };
        let pending = self.heap.alloc_bytes(Vec::new());
        if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
            instance_data
                .attrs
                .insert(CODECS_ATTR_PENDING.to_string(), pending);
            instance_data
                .attrs
                .insert(CODECS_ATTR_STATE_FLAG.to_string(), Value::Int(0));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_codecs_incremental_decoder_getstate(
        &self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "IncrementalDecoder.getstate() expects no arguments",
            ));
        }
        let receiver = match &args[0] {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "IncrementalDecoder.getstate() invalid receiver",
                ));
            }
        };
        let (pending, state_flag) = match &*receiver.kind() {
            Object::Instance(instance_data) => {
                let pending = instance_data
                    .attrs
                    .get(CODECS_ATTR_PENDING)
                    .cloned()
                    .unwrap_or_else(|| self.heap.alloc_bytes(Vec::new()));
                let state_flag = match instance_data.attrs.get(CODECS_ATTR_STATE_FLAG) {
                    Some(Value::Int(value)) => *value,
                    _ => 0,
                };
                (pending, state_flag)
            }
            _ => {
                return Err(RuntimeError::new(
                    "IncrementalDecoder.getstate() invalid receiver",
                ));
            }
        };
        Ok(self.heap.alloc_tuple(vec![pending, Value::Int(state_flag)]))
    }

    pub(super) fn builtin_codecs_incremental_decoder_setstate(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "IncrementalDecoder.setstate() expects state argument",
            ));
        }
        let receiver = match args.remove(0) {
            Value::Instance(instance) => instance,
            _ => {
                return Err(RuntimeError::new(
                    "IncrementalDecoder.setstate() invalid receiver",
                ));
            }
        };
        let mut state = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("state") {
            if state.is_some() {
                return Err(RuntimeError::new(
                    "setstate() got multiple values for state",
                ));
            }
            state = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "setstate() got an unexpected keyword argument",
            ));
        }
        let state = state
            .ok_or_else(|| RuntimeError::new("IncrementalDecoder.setstate() missing state"))?;
        let values = match state {
            Value::Tuple(tuple_obj) => match &*tuple_obj.kind() {
                Object::Tuple(values) => values.clone(),
                _ => return Err(RuntimeError::new("state must be a (buffer, flag) tuple")),
            },
            _ => return Err(RuntimeError::new("state must be a (buffer, flag) tuple")),
        };
        if values.len() != 2 {
            return Err(RuntimeError::new("state must be a (buffer, flag) tuple"));
        }
        let pending_bytes = bytes_like_from_value(values[0].clone())?;
        let state_flag = value_to_int(values[1].clone())?;
        let pending = self.heap.alloc_bytes(pending_bytes);
        if let Object::Instance(instance_data) = &mut *receiver.kind_mut() {
            instance_data
                .attrs
                .insert(CODECS_ATTR_PENDING.to_string(), pending);
            instance_data
                .attrs
                .insert(CODECS_ATTR_STATE_FLAG.to_string(), Value::Int(state_flag));
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_unicodedata_normalize(
        &self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 2 {
            return Err(RuntimeError::new(
                "normalize() expects form and unistr arguments",
            ));
        }
        let mut form = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        let mut unistr = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("form") {
            if form.is_some() {
                return Err(RuntimeError::new(
                    "normalize() got multiple values for argument 'form'",
                ));
            }
            form = Some(value);
        }
        if let Some(value) = kwargs.remove("unistr") {
            if unistr.is_some() {
                return Err(RuntimeError::new(
                    "normalize() got multiple values for argument 'unistr'",
                ));
            }
            unistr = Some(value);
        }
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "normalize() got an unexpected keyword argument",
            ));
        }
        let form =
            form.ok_or_else(|| RuntimeError::new("normalize() missing required argument"))?;
        let unistr =
            unistr.ok_or_else(|| RuntimeError::new("normalize() missing required argument"))?;
        let _form = match form {
            Value::Str(value) => value,
            _ => return Err(RuntimeError::new("normalize() form must be str")),
        };
        match unistr {
            Value::Str(value) => Ok(Value::Str(value)),
            _ => Err(RuntimeError::new("normalize() unistr must be str")),
        }
    }

    pub(super) fn builtin_binascii_crc32(
        &self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("crc32() expects data and optional value"));
        }
        let data_kw = kwargs.remove("data");
        let value_kw = kwargs.remove("value").or_else(|| kwargs.remove("crc"));
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "crc32() got an unexpected keyword argument",
            ));
        }
        if !args.is_empty() && data_kw.is_some() {
            return Err(RuntimeError::new("crc32() got multiple values for data"));
        }
        if args.len() > 1 && value_kw.is_some() {
            return Err(RuntimeError::new("crc32() got multiple values for value"));
        }
        let data = if let Some(value) = data_kw {
            value
        } else {
            args.remove(0)
        };
        let seed = if let Some(value) = value_kw {
            value_to_int(value)? as u32
        } else if !args.is_empty() {
            value_to_int(args.remove(0))? as u32
        } else {
            0
        };
        let bytes = bytes_like_from_value(data)?;
        let mut crc = !seed;
        for byte in bytes {
            let mut value = (crc ^ u32::from(byte)) & 0xFF;
            for _ in 0..8 {
                if value & 1 != 0 {
                    value = 0xEDB8_8320 ^ (value >> 1);
                } else {
                    value >>= 1;
                }
            }
            crc = (crc >> 8) ^ value;
        }
        Ok(Value::Int((!crc) as i64))
    }

    pub(super) fn builtin_binascii_b2a_base64(
        &self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::new(
                "b2a_base64() expects one positional data argument",
            ));
        }
        let newline = match kwargs.remove("newline") {
            None => true,
            Some(Value::Bool(flag)) => flag,
            Some(Value::Int(number)) => number != 0,
            Some(Value::None) => false,
            Some(_) => return Err(RuntimeError::new("b2a_base64() newline must be bool")),
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "b2a_base64() got an unexpected keyword argument",
            ));
        }
        let data = bytes_like_from_value(args.remove(0))?;
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::with_capacity(((data.len() + 2) / 3) * 4 + if newline { 1 } else { 0 });
        let mut i = 0usize;
        while i < data.len() {
            let b0 = data[i];
            let b1 = if i + 1 < data.len() { data[i + 1] } else { 0 };
            let b2 = if i + 2 < data.len() { data[i + 2] } else { 0 };
            let triple = ((b0 as u32) << 16) | ((b1 as u32) << 8) | b2 as u32;
            out.push(TABLE[((triple >> 18) & 0x3f) as usize]);
            out.push(TABLE[((triple >> 12) & 0x3f) as usize]);
            if i + 1 < data.len() {
                out.push(TABLE[((triple >> 6) & 0x3f) as usize]);
            } else {
                out.push(b'=');
            }
            if i + 2 < data.len() {
                out.push(TABLE[(triple & 0x3f) as usize]);
            } else {
                out.push(b'=');
            }
            i += 3;
        }
        if newline {
            out.push(b'\n');
        }
        Ok(self.heap.alloc_bytes(out))
    }

    pub(super) fn builtin_binascii_a2b_base64(
        &self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError::new(
                "a2b_base64() expects one positional data argument",
            ));
        }
        let strict_mode = match kwargs.remove("strict_mode") {
            None => false,
            Some(Value::Bool(flag)) => flag,
            Some(Value::Int(number)) => number != 0,
            Some(Value::None) => false,
            Some(_) => return Err(RuntimeError::new("a2b_base64() strict_mode must be bool")),
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "a2b_base64() got an unexpected keyword argument",
            ));
        }
        let raw = args.remove(0);
        let mut input = match raw {
            Value::Str(text) => text.into_bytes(),
            other => bytes_like_from_value(other)?,
        };
        if !strict_mode {
            input.retain(|byte| !byte.is_ascii_whitespace());
        }
        if input.is_empty() {
            return Ok(self.heap.alloc_bytes(Vec::new()));
        }
        if input.len() % 4 != 0 {
            return Err(RuntimeError::new("Incorrect padding"));
        }

        let decode_char = |byte: u8| -> Option<u8> {
            match byte {
                b'A'..=b'Z' => Some(byte - b'A'),
                b'a'..=b'z' => Some(byte - b'a' + 26),
                b'0'..=b'9' => Some(byte - b'0' + 52),
                b'+' => Some(62),
                b'/' => Some(63),
                _ => None,
            }
        };

        let mut out = Vec::with_capacity((input.len() / 4) * 3);
        for chunk in input.chunks_exact(4) {
            let mut sextets = [0_u8; 4];
            let mut pad = 0_u8;
            for (idx, byte) in chunk.iter().copied().enumerate() {
                if byte == b'=' {
                    pad = pad.saturating_add(1);
                    sextets[idx] = 0;
                    continue;
                }
                let Some(value) = decode_char(byte) else {
                    return Err(RuntimeError::new(
                        "Non-base64 digit found in a2b_base64",
                    ));
                };
                sextets[idx] = value;
            }
            if pad > 2 || (pad > 0 && chunk[3] != b'=') || (pad == 2 && chunk[2] != b'=') {
                return Err(RuntimeError::new("Incorrect padding"));
            }
            let triple = ((sextets[0] as u32) << 18)
                | ((sextets[1] as u32) << 12)
                | ((sextets[2] as u32) << 6)
                | sextets[3] as u32;
            out.push(((triple >> 16) & 0xff) as u8);
            if pad < 2 {
                out.push(((triple >> 8) & 0xff) as u8);
            }
            if pad == 0 {
                out.push((triple & 0xff) as u8);
            }
        }
        Ok(self.heap.alloc_bytes(out))
    }

    pub(super) fn builtin_atexit_register(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            return Err(RuntimeError::new(
                "register() expects at least one argument",
            ));
        }
        let callable = args[0].clone();
        if !self.is_callable_value(&callable) {
            return Err(RuntimeError::new(
                "register() first argument must be callable",
            ));
        }
        self.atexit_handlers.push(AtexitHandler {
            callable: callable.clone(),
            args: args[1..].to_vec(),
            kwargs,
        });
        Ok(callable)
    }

    pub(super) fn builtin_atexit_unregister(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("unregister() expects one argument"));
        }
        let target = args[0].clone();
        self.atexit_handlers
            .retain(|handler| handler.callable != target);
        Ok(Value::None)
    }

    pub(super) fn builtin_atexit_run_exitfuncs(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("_run_exitfuncs() expects no arguments"));
        }
        while let Some(handler) = self.atexit_handlers.pop() {
            match self.call_internal(handler.callable, handler.args, handler.kwargs)? {
                InternalCallOutcome::Value(_) => {}
                InternalCallOutcome::CallerExceptionHandled => {
                    return Err(RuntimeError::new("atexit callback raised"));
                }
            }
        }
        Ok(Value::None)
    }

    pub(super) fn builtin_atexit_clear(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("_clear() expects no arguments"));
        }
        self.atexit_handlers.clear();
        Ok(Value::None)
    }

    pub(super) fn builtin_select_select(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let timeout = kwargs
            .remove("timeout")
            .or_else(|| if args.len() > 3 { args.pop() } else { None });
        if let Some(timeout) = timeout {
            if !matches!(timeout, Value::None) {
                let timeout_secs = value_to_f64(timeout)?;
                if timeout_secs > 0.0 {
                    std::thread::sleep(Duration::from_secs_f64(timeout_secs.min(0.01)));
                }
            }
        }
        let read_values = match args.first() {
            Some(value) => self
                .collect_iterable_values(value.clone())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let write_values = match args.get(1) {
            Some(value) => self
                .collect_iterable_values(value.clone())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let exc_values = match args.get(2) {
            Some(value) => self
                .collect_iterable_values(value.clone())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let read_ready = self.heap.alloc_list(read_values);
        let write_ready = self.heap.alloc_list(write_values);
        let exc_ready = self.heap.alloc_list(exc_values);
        Ok(self
            .heap
            .alloc_tuple(vec![read_ready, write_ready, exc_ready]))
    }
}
