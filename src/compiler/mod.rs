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
            Stmt::Assign { target, value } => {
                self.compile_expr(value)?;
                let idx = self.code.add_name(target.clone());
                self.emit(Opcode::StoreName, Some(idx));
                Ok(())
            }
            Stmt::If { test, body, orelse } => self.compile_if(test, body, orelse),
            Stmt::While { test, body } => self.compile_while(test, body),
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
            Expr::Binary { left, op, right } => {
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                let opcode = match op {
                    crate::ast::BinaryOp::Add => Opcode::BinaryAdd,
                    crate::ast::BinaryOp::Sub => Opcode::BinarySub,
                    crate::ast::BinaryOp::Mul => Opcode::BinaryMul,
                    crate::ast::BinaryOp::Eq => Opcode::CompareEq,
                    crate::ast::BinaryOp::Lt => Opcode::CompareLt,
                };
                self.emit(opcode, None);
                Ok(())
            }
        }
    }

    fn emit(&mut self, opcode: Opcode, arg: Option<u32>) {
        self.code.instructions.push(Instruction::new(opcode, arg));
    }

    fn emit_jump(&mut self, opcode: Opcode) -> usize {
        let index = self.code.instructions.len();
        self.code
            .instructions
            .push(Instruction::new(opcode, Some(0)));
        index
    }

    fn patch_jump(&mut self, index: usize, target: usize) -> Result<(), CompileError> {
        let instr = self
            .code
            .instructions
            .get_mut(index)
            .ok_or_else(|| CompileError::new("invalid jump patch"))?;
        instr.arg = Some(target as u32);
        Ok(())
    }

    fn current_ip(&self) -> usize {
        self.code.instructions.len()
    }

    fn compile_if(
        &mut self,
        test: &Expr,
        body: &[Stmt],
        orelse: &[Stmt],
    ) -> Result<(), CompileError> {
        self.compile_expr(test)?;
        let jump_if_false = self.emit_jump(Opcode::JumpIfFalse);

        for stmt in body {
            self.compile_stmt(stmt)?;
        }

        let jump_to_end = if !orelse.is_empty() {
            Some(self.emit_jump(Opcode::Jump))
        } else {
            None
        };

        let else_target = self.current_ip();
        self.patch_jump(jump_if_false, else_target)?;

        if !orelse.is_empty() {
            for stmt in orelse {
                self.compile_stmt(stmt)?;
            }
            let end_target = self.current_ip();
            if let Some(jump_to_end) = jump_to_end {
                self.patch_jump(jump_to_end, end_target)?;
            }
        }

        Ok(())
    }

    fn compile_while(&mut self, test: &Expr, body: &[Stmt]) -> Result<(), CompileError> {
        let loop_start = self.current_ip();
        self.compile_expr(test)?;
        let jump_if_false = self.emit_jump(Opcode::JumpIfFalse);

        for stmt in body {
            self.compile_stmt(stmt)?;
        }

        self.emit(Opcode::Jump, Some(loop_start as u32));
        let loop_end = self.current_ip();
        self.patch_jump(jump_if_false, loop_end)?;
        Ok(())
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
