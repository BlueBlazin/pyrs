//! AST to bytecode compiler (stubbed).

use crate::ast::Module;
use crate::bytecode::CodeObject;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub message: String,
}

impl CompileError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub fn compile_module(_module: &Module) -> Result<CodeObject, CompileError> {
    Ok(CodeObject::new("<module>"))
}
