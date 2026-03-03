#![cfg(not(target_arch = "wasm32"))]

use pyrs::{
    compiler,
    host::{HostCapability, NativeHost, VmHost},
    parser,
    vm::Vm,
};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct CountingHost {
    native: NativeHost,
    env_overrides: Mutex<HashMap<String, String>>,
    debug_depth_enabled_queries: AtomicUsize,
    debug_depth_limit_queries: AtomicUsize,
}

impl CountingHost {
    fn new_with_overrides(overrides: &[(&str, &str)]) -> Self {
        let mut env_overrides = HashMap::new();
        for (name, value) in overrides {
            env_overrides.insert((*name).to_string(), (*value).to_string());
        }
        Self {
            native: NativeHost,
            env_overrides: Mutex::new(env_overrides),
            debug_depth_enabled_queries: AtomicUsize::new(0),
            debug_depth_limit_queries: AtomicUsize::new(0),
        }
    }

    fn debug_depth_enabled_queries(&self) -> usize {
        self.debug_depth_enabled_queries.load(Ordering::Relaxed)
    }

    fn debug_depth_limit_queries(&self) -> usize {
        self.debug_depth_limit_queries.load(Ordering::Relaxed)
    }

    fn env_override(&self, name: &str) -> Option<String> {
        let guard = self
            .env_overrides
            .lock()
            .expect("counting host env overrides lock");
        guard.get(name).cloned()
    }
}

impl Default for CountingHost {
    fn default() -> Self {
        Self::new_with_overrides(&[])
    }
}

impl VmHost for CountingHost {
    fn current_dir(&self) -> Result<PathBuf, String> {
        self.native.current_dir()
    }

    fn env_var(&self, name: &str) -> Option<String> {
        if name == "PYRS_DEBUG_EXCEPTION_UNWIND_DEPTH_LIMIT" {
            self.debug_depth_limit_queries
                .fetch_add(1, Ordering::Relaxed);
        }
        self.env_override(name).or_else(|| self.native.env_var(name))
    }

    fn env_var_os(&self, name: &str) -> Option<OsString> {
        if name == "PYRS_DEBUG_EXCEPTION_UNWIND_DEPTH" {
            self.debug_depth_enabled_queries
                .fetch_add(1, Ordering::Relaxed);
        }
        self.env_override(name)
            .map(OsString::from)
            .or_else(|| self.native.env_var_os(name))
    }

    fn env_vars(&self) -> Vec<(String, String)> {
        self.native.env_vars()
    }

    fn path_is_dir(&self, path: &Path) -> bool {
        self.native.path_is_dir(path)
    }

    fn process_args(&self) -> Vec<String> {
        self.native.process_args()
    }

    fn current_exe(&self) -> Option<PathBuf> {
        self.native.current_exe()
    }

    fn os_name(&self) -> &'static str {
        self.native.os_name()
    }

    fn supports(&self, capability: HostCapability) -> bool {
        self.native.supports(capability)
    }
}

fn execute_fib_probe(vm: &mut Vm) {
    let source = r#"
fib = lambda n: n if n < 2 else fib(n - 1) + fib(n - 2)
result = fib(24)
"#;
    let ast = parser::parse_module(source).expect("parse fib probe");
    let code = compiler::compile_module(&ast).expect("compile fib probe");
    vm.execute(&code).expect("execute fib probe");
}

#[test]
fn debug_depth_env_is_read_once_when_disabled() {
    let host = Arc::new(CountingHost::default());
    let mut vm = Vm::new_with_host(host.clone());
    assert_eq!(host.debug_depth_enabled_queries(), 1);
    assert_eq!(host.debug_depth_limit_queries(), 0);

    execute_fib_probe(&mut vm);

    assert_eq!(
        host.debug_depth_enabled_queries(),
        1,
        "debug-depth env flag should not be re-read during execute"
    );
    assert_eq!(
        host.debug_depth_limit_queries(),
        0,
        "debug-depth limit should not be read when debug flag is disabled"
    );
}

#[test]
fn debug_depth_env_is_not_requeried_when_enabled() {
    let host = Arc::new(CountingHost::new_with_overrides(&[
        ("PYRS_DEBUG_EXCEPTION_UNWIND_DEPTH", "1"),
        ("PYRS_DEBUG_EXCEPTION_UNWIND_DEPTH_LIMIT", "1024"),
    ]));
    let mut vm = Vm::new_with_host(host.clone());
    assert_eq!(host.debug_depth_enabled_queries(), 1);
    assert_eq!(host.debug_depth_limit_queries(), 1);

    execute_fib_probe(&mut vm);

    assert_eq!(
        host.debug_depth_enabled_queries(),
        1,
        "debug-depth env flag should be cached at VM init"
    );
    assert_eq!(
        host.debug_depth_limit_queries(),
        1,
        "debug-depth limit should be cached at VM init"
    );
}
