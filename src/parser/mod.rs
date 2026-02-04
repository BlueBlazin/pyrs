//! Packrat parser entry points.

pub mod lexer;
pub mod token;

mod parser;

pub use parser::{parse_module, ParseError};
