//! AST to bytecode compiler (minimal subset).

use crate::ast::{Constant, Expr, Module, Stmt};
use crate::bytecode::{CodeObject, Instruction, Opcode};
use crate::runtime::Value;

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

pub fn compile_module(module: &Module) -> Result<CodeObject, CompileError> {
    let mut compiler = Compiler::new();
    compiler.compile_module(module)?;
    Ok(compiler.finish())
}

struct Compiler {
    code: CodeObject,
}

impl Compiler {
    fn new() -> Self {
        Self {
            code: CodeObject::new("<module>"),
        }
    }

    fn finish(mut self) -> CodeObject {
        self.emit(Opcode::LoadConst, Some(0));
        self.emit(Opcode::ReturnValue, None);
        self.code
    }

    fn compile_module(&mut self, module: &Module) -> Result<(), CompileError> {
        for stmt in &module.body {
            self.compile_stmt(stmt)?;
        }
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), CompileError> {
        match stmt {
            Stmt::Pass => {
                self.emit(Opcode::Nop, None);
                Ok(())
            }
            Stmt::Expr(expr) => {
                self.compile_expr(expr)?;
                self.emit(Opcode::PopTop, None);
                Ok(())
            }
            Stmt::If { .. } => Err(CompileError::new("if statements not supported yet")),
        }
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), CompileError> {
        match expr {
            Expr::Name(name) => {
                let idx = self.code.add_name(name.clone());
                self.emit(Opcode::LoadName, Some(idx));
                Ok(())
            }
            Expr::Constant(constant) => {
                let idx = self.code.add_const(constant_to_value(constant));
                self.emit(Opcode::LoadConst, Some(idx));
                Ok(())
            }
        }
    }

    fn emit(&mut self, opcode: Opcode, arg: Option<u32>) {
        self.code.instructions.push(Instruction::new(opcode, arg));
    }
}

fn constant_to_value(constant: &Constant) -> Value {
    match constant {
        Constant::None => Value::None,
        Constant::Bool(value) => Value::Bool(*value),
        Constant::Int(value) => Value::Int(*value),
        Constant::Str(value) => Value::Str(value.clone()),
    }
}
