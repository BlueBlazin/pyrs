//! Production-grade Python interpreter in Rust (CPython 3.14 compatible).

pub mod ast;
pub mod bytecode;
pub mod cli;
pub mod compiler;
pub mod extensions;
pub mod parser;
pub mod runtime;
pub mod vm;

/// Public version for CLI and diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
