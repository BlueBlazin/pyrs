//! Production-grade Python interpreter in Rust (CPython 3.14 compatible).

pub mod ast;
pub mod bytecode;
#[cfg(not(target_arch = "wasm32"))]
pub mod cli;
pub mod compiler;
pub mod extensions;
pub mod host;
pub mod parser;
pub mod runtime;
#[cfg(not(target_arch = "wasm32"))]
pub mod vm;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

/// Public version for CLI and diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
