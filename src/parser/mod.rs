//! Packrat parser entry points.

pub mod lexer;
pub mod token;

mod parser;

pub use parser::{ParseError, parse_module};
