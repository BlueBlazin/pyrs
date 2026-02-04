//! AST to bytecode compiler (minimal subset).

use std::rc::Rc;

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
    temp_counter: usize,
    loop_stack: Vec<LoopContext>,
}

struct LoopContext {
    start: usize,
    continue_target: Option<usize>,
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

impl Compiler {
    fn new() -> Self {
        Self {
            code: CodeObject::new("<module>"),
            temp_counter: 0,
            loop_stack: Vec::new(),
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
            Stmt::AssignSubscript { target, value } => {
                if let Expr::Subscript { value: container, index } = target {
                    if let Expr::Name(name) = &**container {
                        let name = name.clone();
                        self.emit_load_name(&name);
                        self.compile_expr(index)?;
                        self.compile_expr(value)?;
                        self.emit(Opcode::StoreSubscript, None);
                        self.emit_store_name(&name);
                        Ok(())
                    } else {
                        Err(CompileError::new(
                            "only name-based subscript assignments supported",
                        ))
                    }
                } else {
                    Err(CompileError::new("invalid assignment target"))
                }
            }
            Stmt::If { test, body, orelse } => self.compile_if(test, body, orelse),
            Stmt::While { test, body } => self.compile_while(test, body),
            Stmt::FunctionDef { name, params, body } => {
                let func_code = self.compile_function(name, params, body)?;
                let const_idx = self.code.add_const(Value::Code(Rc::new(func_code)));
                self.emit(Opcode::MakeFunction, Some(const_idx));
                let name_idx = self.code.add_name(name.clone());
                self.emit(Opcode::StoreName, Some(name_idx));
                Ok(())
            }
            Stmt::Return { value } => {
                if let Some(expr) = value {
                    self.compile_expr(expr)?;
                } else {
                    self.emit(Opcode::LoadConst, Some(0));
                }
                self.emit(Opcode::ReturnValue, None);
                Ok(())
            }
            Stmt::For { target, iter, body } => self.compile_for(target, iter, body),
            Stmt::Break => self.compile_break(),
            Stmt::Continue => self.compile_continue(),
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
            Expr::Unary { op, operand } => {
                self.compile_expr(operand)?;
                let opcode = match op {
                    crate::ast::UnaryOp::Neg => Opcode::UnaryNeg,
                    crate::ast::UnaryOp::Not => Opcode::UnaryNot,
                };
                self.emit(opcode, None);
                Ok(())
            }
            Expr::BoolOp { op, left, right } => self.compile_bool_op(op, left, right),
            Expr::Call { func, args } => {
                self.compile_expr(func)?;
                for arg in args {
                    self.compile_expr(arg)?;
                }
                self.emit(Opcode::CallFunction, Some(args.len() as u32));
                Ok(())
            }
            Expr::List(elements) => {
                for elem in elements {
                    self.compile_expr(elem)?;
                }
                self.emit(Opcode::BuildList, Some(elements.len() as u32));
                Ok(())
            }
            Expr::Tuple(elements) => {
                for elem in elements {
                    self.compile_expr(elem)?;
                }
                self.emit(Opcode::BuildTuple, Some(elements.len() as u32));
                Ok(())
            }
            Expr::Dict(entries) => {
                for (key, value) in entries {
                    self.compile_expr(key)?;
                    self.compile_expr(value)?;
                }
                self.emit(Opcode::BuildDict, Some(entries.len() as u32));
                Ok(())
            }
            Expr::Subscript { value, index } => {
                self.compile_expr(value)?;
                self.compile_expr(index)?;
                self.emit(Opcode::Subscript, None);
                Ok(())
            }
        }
    }

    fn emit(&mut self, opcode: Opcode, arg: Option<u32>) {
        self.code.instructions.push(Instruction::new(opcode, arg));
    }

    fn emit_const(&mut self, value: Value) {
        let idx = self.code.add_const(value);
        self.emit(Opcode::LoadConst, Some(idx));
    }

    fn emit_load_name(&mut self, name: &str) {
        let idx = self.code.add_name(name.to_string());
        self.emit(Opcode::LoadName, Some(idx));
    }

    fn emit_store_name(&mut self, name: &str) {
        let idx = self.code.add_name(name.to_string());
        self.emit(Opcode::StoreName, Some(idx));
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

        self.loop_stack.push(LoopContext {
            start: loop_start,
            continue_target: Some(loop_start),
            breaks: Vec::new(),
            continues: Vec::new(),
        });

        for stmt in body {
            self.compile_stmt(stmt)?;
        }

        self.emit(Opcode::Jump, Some(loop_start as u32));
        let loop_end = self.current_ip();
        self.patch_jump(jump_if_false, loop_end)?;
        self.resolve_loop(loop_end)?;
        Ok(())
    }

    fn compile_function(
        &mut self,
        name: &str,
        params: &[String],
        body: &[Stmt],
    ) -> Result<CodeObject, CompileError> {
        let mut compiler = Compiler {
            code: CodeObject::new(name),
            temp_counter: 0,
            loop_stack: Vec::new(),
        };
        compiler.code.params = params.to_vec();
        for stmt in body {
            compiler.compile_stmt(stmt)?;
        }
        Ok(compiler.finish())
    }

    fn compile_for(
        &mut self,
        target: &str,
        iter: &Expr,
        body: &[Stmt],
    ) -> Result<(), CompileError> {
        let iter_temp = self.fresh_temp("iter");
        let index_temp = self.fresh_temp("idx");

        self.compile_expr(iter)?;
        self.emit_store_name(&iter_temp);

        self.emit_const(Value::Int(0));
        self.emit_store_name(&index_temp);

        let loop_start = self.current_ip();

        self.emit_load_name(&index_temp);
        self.emit_load_name("len");
        self.emit_load_name(&iter_temp);
        self.emit(Opcode::CallFunction, Some(1));
        self.emit(Opcode::CompareLt, None);
        let jump_if_false = self.emit_jump(Opcode::JumpIfFalse);

        self.emit_load_name(&iter_temp);
        self.emit_load_name(&index_temp);
        self.emit(Opcode::Subscript, None);
        self.emit_store_name(target);

        self.loop_stack.push(LoopContext {
            start: loop_start,
            continue_target: None,
            breaks: Vec::new(),
            continues: Vec::new(),
        });

        for stmt in body {
            self.compile_stmt(stmt)?;
        }

        let continue_target = self.current_ip();
        if let Some(ctx) = self.loop_stack.last_mut() {
            ctx.continue_target = Some(continue_target);
        }

        self.emit_load_name(&index_temp);
        self.emit_const(Value::Int(1));
        self.emit(Opcode::BinaryAdd, None);
        self.emit_store_name(&index_temp);

        self.emit(Opcode::Jump, Some(loop_start as u32));
        let loop_end = self.current_ip();
        self.patch_jump(jump_if_false, loop_end)?;
        self.resolve_loop(loop_end)?;

        Ok(())
    }

    fn compile_bool_op(
        &mut self,
        op: &crate::ast::BoolOp,
        left: &Expr,
        right: &Expr,
    ) -> Result<(), CompileError> {
        self.compile_expr(left)?;
        self.emit(Opcode::DupTop, None);

        match op {
            crate::ast::BoolOp::And => {
                let jump_if_false = self.emit_jump(Opcode::JumpIfFalse);
                self.emit(Opcode::PopTop, None);
                self.compile_expr(right)?;
                let end = self.current_ip();
                self.patch_jump(jump_if_false, end)?;
            }
            crate::ast::BoolOp::Or => {
                let jump_if_true = self.emit_jump(Opcode::JumpIfTrue);
                self.emit(Opcode::PopTop, None);
                self.compile_expr(right)?;
                let end = self.current_ip();
                self.patch_jump(jump_if_true, end)?;
            }
        }

        Ok(())
    }

    fn fresh_temp(&mut self, prefix: &str) -> String {
        let name = format!("__pyrs_{prefix}_{}", self.temp_counter);
        self.temp_counter += 1;
        name
    }

    fn compile_break(&mut self) -> Result<(), CompileError> {
        let jump = self.emit_jump(Opcode::Jump);
        let ctx = self
            .loop_stack
            .last_mut()
            .ok_or_else(|| CompileError::new("break outside loop"))?;
        ctx.breaks.push(jump);
        Ok(())
    }

    fn compile_continue(&mut self) -> Result<(), CompileError> {
        let jump = self.emit_jump(Opcode::Jump);
        let ctx = self
            .loop_stack
            .last_mut()
            .ok_or_else(|| CompileError::new("continue outside loop"))?;
        ctx.continues.push(jump);
        Ok(())
    }

    fn resolve_loop(&mut self, loop_end: usize) -> Result<(), CompileError> {
        let ctx = self
            .loop_stack
            .pop()
            .ok_or_else(|| CompileError::new("loop stack underflow"))?;
        for jump in ctx.breaks {
            self.patch_jump(jump, loop_end)?;
        }
        let continue_target = ctx.continue_target.unwrap_or(ctx.start);
        for jump in ctx.continues {
            self.patch_jump(jump, continue_target)?;
        }
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
