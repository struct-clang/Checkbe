use std::collections::HashMap;
use std::path::Path;

use inkwell::basic_block::BasicBlock;
use inkwell::builder::{Builder, BuilderError};
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate, OptimizationLevel};

use crate::ast::{
    AssignTarget, BinaryOp, CallTarget, Expr, Item, Program, Stmt, UnaryOp, ValueType, VarDecl,
};
use crate::module_system::ResolvedExternalCall;
use crate::parser::parse_expression_fragment;
use crate::sema::SemanticModel;
use crate::span::Span;
use crate::string_interp::{parse_segments, Segment};

#[derive(Clone)]
struct VarBinding<'ctx> {
    ptr: PointerValue<'ctx>,
    ty: ValueType,
}

struct TypedValue<'ctx> {
    value: BasicValueEnum<'ctx>,
    ty: ValueType,
}

pub fn generate_object(
    program: &Program,
    semantic_model: &SemanticModel,
    object_path: &Path,
) -> Result<(), String> {
    let context = Context::create();
    let mut generator = CodeGenerator::new(&context, "checkbe", semantic_model);
    generator.emit(program)?;
    generator.write_object_file(object_path)
}

struct CodeGenerator<'ctx, 'a> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    semantic_model: &'a SemanticModel,
    globals: HashMap<String, VarBinding<'ctx>>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    extern_functions: HashMap<String, FunctionValue<'ctx>>,
    runtime_init_fn: FunctionValue<'ctx>,
    gc_strdup_fn: FunctionValue<'ctx>,
    string_eq_fn: FunctionValue<'ctx>,
    string_concat_fn: FunctionValue<'ctx>,
    int_to_string_fn: FunctionValue<'ctx>,
    float_to_string_fn: FunctionValue<'ctx>,
    bool_to_string_fn: FunctionValue<'ctx>,
    to_int_fn: FunctionValue<'ctx>,
    globals_init_fn: Option<FunctionValue<'ctx>>,
}

impl<'ctx, 'a> CodeGenerator<'ctx, 'a> {
    fn new(context: &'ctx Context, module_name: &str, semantic_model: &'a SemanticModel) -> Self {
        let module = context.create_module(module_name);
        let builder = context.create_builder();
        let ptr = context.ptr_type(AddressSpace::default());

        let runtime_init_fn = module.add_function(
            "checkbe_runtime_init",
            context.void_type().fn_type(&[], false),
            None,
        );
        let gc_strdup_fn =
            module.add_function("cb_gc_strdup", ptr.fn_type(&[ptr.into()], false), None);
        let string_eq_fn = module.add_function(
            "cb_string_eq",
            context
                .bool_type()
                .fn_type(&[ptr.into(), ptr.into()], false),
            None,
        );
        let string_concat_fn = module.add_function(
            "cb_string_concat",
            ptr.fn_type(&[ptr.into(), ptr.into()], false),
            None,
        );
        let int_to_string_fn = module.add_function(
            "cb_int_to_string",
            ptr.fn_type(&[context.i64_type().into()], false),
            None,
        );
        let float_to_string_fn = module.add_function(
            "cb_float_to_string",
            ptr.fn_type(&[context.f64_type().into()], false),
            None,
        );
        let bool_to_string_fn = module.add_function(
            "cb_bool_to_string",
            ptr.fn_type(&[context.bool_type().into()], false),
            None,
        );
        let to_int_fn = module.add_function(
            "cb_to_int",
            context.i64_type().fn_type(&[ptr.into()], false),
            None,
        );

        Self {
            context,
            module,
            builder,
            semantic_model,
            globals: HashMap::new(),
            functions: HashMap::new(),
            extern_functions: HashMap::new(),
            runtime_init_fn,
            gc_strdup_fn,
            string_eq_fn,
            string_concat_fn,
            int_to_string_fn,
            float_to_string_fn,
            bool_to_string_fn,
            to_int_fn,
            globals_init_fn: None,
        }
    }

    fn emit(&mut self, program: &Program) -> Result<(), String> {
        self.declare_globals(program)?;
        self.declare_functions(program)?;
        self.emit_globals_init(program)?;
        self.emit_function_bodies(program)?;
        self.emit_c_main()?;
        Ok(())
    }

    fn write_object_file(&self, object_path: &Path) -> Result<(), String> {
        Target::initialize_native(&InitializationConfig::default())
            .map_err(|err| format!("LLVM target init failed: {err}"))?;

        let triple = TargetMachine::get_default_triple();
        self.module.set_triple(&triple);

        let target = Target::from_triple(&triple)
            .map_err(|err| format!("target from triple failed: {err}"))?;
        let machine = target
            .create_target_machine(
                &triple,
                "generic",
                "",
                OptimizationLevel::Default,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or_else(|| "target machine creation failed".to_string())?;

        self.module
            .set_data_layout(&machine.get_target_data().get_data_layout());
        self.module
            .verify()
            .map_err(|err| format!("LLVM verify failed: {err}"))?;

        machine
            .write_to_file(&self.module, FileType::Object, object_path)
            .map_err(|err| format!("write object failed: {err}"))
    }

    fn declare_globals(&mut self, program: &Program) -> Result<(), String> {
        for item in &program.body.items {
            let Item::Var(var_decl) = item else {
                continue;
            };

            let info = self
                .semantic_model
                .globals
                .get(&var_decl.name)
                .ok_or_else(|| format!("missing semantic info for global '{}'", var_decl.name))?;

            let ty = self.llvm_type(&info.ty)?;
            let global = self
                .module
                .add_global(ty, None, &format!("g_{}", var_decl.name));
            global.set_initializer(&self.zero_value(&info.ty)?);

            self.globals.insert(
                var_decl.name.clone(),
                VarBinding {
                    ptr: global.as_pointer_value(),
                    ty: info.ty.clone(),
                },
            );
        }

        Ok(())
    }

    fn declare_functions(&mut self, program: &Program) -> Result<(), String> {
        for item in &program.body.items {
            let Item::Func(func) = item else {
                continue;
            };

            let mut param_types = Vec::with_capacity(func.params.len());
            for param in &func.params {
                param_types.push(self.llvm_type(&param.ty)?.into());
            }

            let fn_type = if func.return_type == ValueType::Void {
                self.context.void_type().fn_type(&param_types, false)
            } else {
                self.llvm_type(&func.return_type)?
                    .fn_type(&param_types, false)
            };

            let llvm_name = if func.name == "main" {
                "__checkbe_user_main".to_string()
            } else {
                func.name.clone()
            };

            let function = self.module.add_function(&llvm_name, fn_type, None);
            self.functions.insert(func.name.clone(), function);
        }
        Ok(())
    }

    fn emit_globals_init(&mut self, program: &Program) -> Result<(), String> {
        let init_fn = self.module.add_function(
            "__checkbe_init_globals",
            self.context.void_type().fn_type(&[], false),
            None,
        );
        let entry = self.context.append_basic_block(init_fn, "entry");
        self.builder.position_at_end(entry);

        let locals = HashMap::new();
        for item in &program.body.items {
            let Item::Var(var_decl) = item else {
                continue;
            };
            let binding = self
                .globals
                .get(&var_decl.name)
                .cloned()
                .ok_or_else(|| format!("global '{}' missing in codegen", var_decl.name))?;

            let value = self.generate_value(&var_decl.initializer, &locals)?;
            let casted = self.cast_value_if_needed(value, &binding.ty)?;
            self.builder
                .build_store(binding.ptr, casted.value)
                .map_err(builder_error)?;
        }

        self.builder.build_return(None).map_err(builder_error)?;
        self.globals_init_fn = Some(init_fn);
        Ok(())
    }

    fn emit_function_bodies(&mut self, program: &Program) -> Result<(), String> {
        for item in &program.body.items {
            let Item::Func(func_decl) = item else {
                continue;
            };

            let function = *self
                .functions
                .get(&func_decl.name)
                .ok_or_else(|| format!("function '{}' not declared", func_decl.name))?;

            let entry = self.context.append_basic_block(function, "entry");
            self.builder.position_at_end(entry);

            let mut locals = HashMap::new();
            for (index, param) in func_decl.params.iter().enumerate() {
                let llvm_param = function
                    .get_nth_param(index as u32)
                    .ok_or_else(|| format!("missing parameter {} in '{}'", index, func_decl.name))?;
                llvm_param.set_name(&param.name);
                let alloca =
                    self.create_entry_alloca(function, &param.ty, &format!("arg_{}", param.name))?;
                self.builder
                    .build_store(alloca, llvm_param)
                    .map_err(builder_error)?;
                locals.insert(
                    param.name.clone(),
                    VarBinding {
                        ptr: alloca,
                        ty: param.ty.clone(),
                    },
                );
            }

            for stmt in &func_decl.body {
                if self.current_block_terminated() {
                    break;
                }
                self.generate_stmt(stmt, &mut locals, &func_decl.return_type)?;
            }

            if !self.current_block_terminated() {
                if func_decl.return_type == ValueType::Void {
                    self.builder.build_return(None).map_err(builder_error)?;
                } else {
                    return Err(format!(
                        "Function '{}' may exit without returning '{}'",
                        func_decl.name,
                        func_decl.return_type.as_str()
                    ));
                }
            }
        }

        Ok(())
    }

    fn emit_c_main(&mut self) -> Result<(), String> {
        let i32_type = self.context.i32_type();
        let c_main = self
            .module
            .add_function("main", i32_type.fn_type(&[], false), None);
        let entry = self.context.append_basic_block(c_main, "entry");
        self.builder.position_at_end(entry);

        self.builder
            .build_call(self.runtime_init_fn, &[], "runtime_init")
            .map_err(builder_error)?;

        if let Some(init_fn) = self.globals_init_fn {
            self.builder
                .build_call(init_fn, &[], "init_globals")
                .map_err(builder_error)?;
        }

        if let Some(user_main) = self.functions.get("main") {
            self.builder
                .build_call(*user_main, &[], "user_main")
                .map_err(builder_error)?;
        }

        self.builder
            .build_return(Some(&i32_type.const_int(0, false)))
            .map_err(builder_error)?;
        Ok(())
    }

    fn generate_stmt(
        &mut self,
        stmt: &Stmt,
        locals: &mut HashMap<String, VarBinding<'ctx>>,
        function_return_type: &ValueType,
    ) -> Result<(), String> {
        match stmt {
            Stmt::VarDecl(var_decl) => self.generate_local_var_decl(var_decl, locals),
            Stmt::Assign {
                target,
                value,
                span,
            } => self.generate_assignment(target, value, *span, locals),
            Stmt::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => self.generate_if(condition, then_branch, else_branch, locals, function_return_type),
            Stmt::While {
                condition,
                body,
                ..
            } => self.generate_while(condition, body, locals, function_return_type),
            Stmt::DoWhile {
                body,
                condition,
                ..
            } => self.generate_do_while(body, condition, locals, function_return_type),
            Stmt::Expr { expr, .. } => {
                self.generate_expr(expr, locals)?;
                Ok(())
            }
            Stmt::Block { statements, .. } => {
                let mut nested = locals.clone();
                for stmt in statements {
                    if self.current_block_terminated() {
                        break;
                    }
                    self.generate_stmt(stmt, &mut nested, function_return_type)?;
                }
                Ok(())
            }
            Stmt::Return { value, span } => {
                match (function_return_type, value) {
                    (ValueType::Void, None) => {
                        self.builder.build_return(None).map_err(builder_error)?;
                        Ok(())
                    }
                    (ValueType::Void, Some(_)) => {
                        Err(format!("void function cannot return a value ({})", span))
                    }
                    (expected, Some(expr)) => {
                        let produced = self.generate_value(expr, locals)?;
                        let casted = self.cast_value_if_needed(produced, expected)?;
                        self.builder
                            .build_return(Some(&casted.value))
                            .map_err(builder_error)?;
                        Ok(())
                    }
                    (expected, None) => Err(format!(
                        "function must return '{}' ({})",
                        expected.as_str(),
                        span
                    )),
                }
            }
        }
    }

    fn generate_assignment(
        &mut self,
        target: &AssignTarget,
        value: &Expr,
        span: Span,
        locals: &mut HashMap<String, VarBinding<'ctx>>,
    ) -> Result<(), String> {
        match target {
            AssignTarget::Identifier(name) => {
                let binding = self
                    .lookup_binding(name, locals)
                    .cloned()
                    .ok_or_else(|| format!("unknown variable '{}' ({})", name, span))?;
                let assigned = self.generate_value(value, locals)?;
                let assigned = self.cast_value_if_needed(assigned, &binding.ty)?;
                self.builder
                    .build_store(binding.ptr, assigned.value)
                    .map_err(builder_error)?;
                Ok(())
            }
            AssignTarget::Index { array, index } => {
                let array_value = self.generate_value(array, locals)?;
                let index_value = self.generate_value(index, locals)?;
                let assigned = self.generate_value(value, locals)?;
                self.store_array_element(array_value, index_value, assigned, span)
            }
        }
    }

    fn generate_local_var_decl(
        &mut self,
        var_decl: &VarDecl,
        locals: &mut HashMap<String, VarBinding<'ctx>>,
    ) -> Result<(), String> {
        if locals.contains_key(&var_decl.name) {
            return Err(format!("local '{}' already declared", var_decl.name));
        }

        let init_value = self.generate_value(&var_decl.initializer, locals)?;
        let resolved_ty = var_decl
            .explicit_type
            .clone()
            .unwrap_or_else(|| init_value.ty.clone());

        let ptr =
            self.create_entry_alloca(self.current_function()?, &resolved_ty, &var_decl.name)?;
        let casted = self.cast_value_if_needed(init_value, &resolved_ty)?;

        self.builder
            .build_store(ptr, casted.value)
            .map_err(builder_error)?;

        locals.insert(
            var_decl.name.clone(),
            VarBinding {
                ptr,
                ty: resolved_ty,
            },
        );
        Ok(())
    }

    fn generate_if(
        &mut self,
        condition: &Expr,
        then_branch: &[Stmt],
        else_branch: &[Stmt],
        locals: &mut HashMap<String, VarBinding<'ctx>>,
        function_return_type: &ValueType,
    ) -> Result<(), String> {
        let cond_value = self.generate_value(condition, locals)?;
        let cond = self.expect_bool(cond_value, condition.span())?;

        let function = self.current_function()?;
        let then_bb = self.context.append_basic_block(function, "if_then");
        let else_bb = self.context.append_basic_block(function, "if_else");
        let cont_bb = self.context.append_basic_block(function, "if_cont");

        self.builder
            .build_conditional_branch(cond, then_bb, else_bb)
            .map_err(builder_error)?;

        self.builder.position_at_end(then_bb);
        let mut then_locals = locals.clone();
        for stmt in then_branch {
            if self.current_block_terminated() {
                break;
            }
            self.generate_stmt(stmt, &mut then_locals, function_return_type)?;
        }
        if !self.current_block_terminated() {
            self.builder
                .build_unconditional_branch(cont_bb)
                .map_err(builder_error)?;
        }

        self.builder.position_at_end(else_bb);
        let mut else_locals = locals.clone();
        for stmt in else_branch {
            if self.current_block_terminated() {
                break;
            }
            self.generate_stmt(stmt, &mut else_locals, function_return_type)?;
        }
        if !self.current_block_terminated() {
            self.builder
                .build_unconditional_branch(cont_bb)
                .map_err(builder_error)?;
        }

        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    fn generate_while(
        &mut self,
        condition: &Expr,
        body: &[Stmt],
        locals: &mut HashMap<String, VarBinding<'ctx>>,
        function_return_type: &ValueType,
    ) -> Result<(), String> {
        let function = self.current_function()?;
        let cond_bb = self.context.append_basic_block(function, "while_cond");
        let body_bb = self.context.append_basic_block(function, "while_body");
        let cont_bb = self.context.append_basic_block(function, "while_cont");

        self.builder
            .build_unconditional_branch(cond_bb)
            .map_err(builder_error)?;

        self.builder.position_at_end(cond_bb);
        let cond_value = self.generate_value(condition, locals)?;
        let cond = self.expect_bool(cond_value, condition.span())?;
        self.builder
            .build_conditional_branch(cond, body_bb, cont_bb)
            .map_err(builder_error)?;

        self.builder.position_at_end(body_bb);
        let mut loop_locals = locals.clone();
        for stmt in body {
            if self.current_block_terminated() {
                break;
            }
            self.generate_stmt(stmt, &mut loop_locals, function_return_type)?;
        }
        if !self.current_block_terminated() {
            self.builder
                .build_unconditional_branch(cond_bb)
                .map_err(builder_error)?;
        }

        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    fn generate_do_while(
        &mut self,
        body: &[Stmt],
        condition: &Expr,
        locals: &mut HashMap<String, VarBinding<'ctx>>,
        function_return_type: &ValueType,
    ) -> Result<(), String> {
        let function = self.current_function()?;
        let body_bb = self.context.append_basic_block(function, "do_body");
        let cond_bb = self.context.append_basic_block(function, "do_cond");
        let cont_bb = self.context.append_basic_block(function, "do_cont");

        self.builder
            .build_unconditional_branch(body_bb)
            .map_err(builder_error)?;

        self.builder.position_at_end(body_bb);
        let mut loop_locals = locals.clone();
        for stmt in body {
            if self.current_block_terminated() {
                break;
            }
            self.generate_stmt(stmt, &mut loop_locals, function_return_type)?;
        }
        if !self.current_block_terminated() {
            self.builder
                .build_unconditional_branch(cond_bb)
                .map_err(builder_error)?;
        }

        self.builder.position_at_end(cond_bb);
        let cond_value = self.generate_value(condition, locals)?;
        let cond = self.expect_bool(cond_value, condition.span())?;
        self.builder
            .build_conditional_branch(cond, body_bb, cont_bb)
            .map_err(builder_error)?;

        self.builder.position_at_end(cont_bb);
        Ok(())
    }

    fn generate_expr(
        &mut self,
        expr: &Expr,
        locals: &HashMap<String, VarBinding<'ctx>>,
    ) -> Result<Option<TypedValue<'ctx>>, String> {
        match expr {
            Expr::IntLiteral(v, _) => Ok(Some(TypedValue {
                value: self.context.i64_type().const_int(*v as u64, true).into(),
                ty: ValueType::Int,
            })),
            Expr::FloatLiteral(v, _) => Ok(Some(TypedValue {
                value: self.context.f64_type().const_float(*v).into(),
                ty: ValueType::Float,
            })),
            Expr::BoolLiteral(v, _) => Ok(Some(TypedValue {
                value: self
                    .context
                    .bool_type()
                    .const_int(if *v { 1 } else { 0 }, false)
                    .into(),
                ty: ValueType::Bool,
            })),
            Expr::StringLiteral(text, span) => self
                .generate_interpolated_string(text, *span, locals)
                .map(Some),
            Expr::ArrayLiteral(elements, span) => self
                .generate_array_literal(elements, *span, locals)
                .map(Some),
            Expr::Identifier(name, span) => {
                let binding = self
                    .lookup_binding(name, locals)
                    .cloned()
                    .ok_or_else(|| format!("unknown variable '{}' ({})", name, span))?;
                let loaded = self
                    .builder
                    .build_load(
                        self.llvm_type(&binding.ty)?,
                        binding.ptr,
                        &format!("load_{}", name),
                    )
                    .map_err(builder_error)?;
                Ok(Some(TypedValue {
                    value: loaded,
                    ty: binding.ty,
                }))
            }
            Expr::Index { array, index, span } => {
                let array_value = self.generate_value(array, locals)?;
                let index_value = self.generate_value(index, locals)?;
                self.load_array_element(array_value, index_value, *span)
                    .map(Some)
            }
            Expr::Unary { op, expr, span } => {
                let inner = self.generate_value(expr, locals)?;
                match op {
                    UnaryOp::Neg => self.generate_neg(inner, *span).map(Some),
                    UnaryOp::Not => {
                        let b = self.expect_bool(inner, *span)?;
                        let value = self.builder.build_not(b, "not").map_err(builder_error)?;
                        Ok(Some(TypedValue {
                            value: value.into(),
                            ty: ValueType::Bool,
                        }))
                    }
                }
            }
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => {
                let left = self.generate_value(left, locals)?;
                let right = self.generate_value(right, locals)?;
                self.generate_binary(left, *op, right, *span).map(Some)
            }
            Expr::Ternary {
                condition,
                then_expr,
                else_expr,
                span,
            } => self
                .generate_ternary(condition, then_expr, else_expr, *span, locals)
                .map(Some),
            Expr::Call { callee, args, span } => self.generate_call(callee, args, *span, locals),
            Expr::NewObject { span, .. } => {
                Err(format!("new object is compile-time only ({})", span))
            }
        }
    }

    fn generate_interpolated_string(
        &mut self,
        text: &str,
        span: Span,
        locals: &HashMap<String, VarBinding<'ctx>>,
    ) -> Result<TypedValue<'ctx>, String> {
        let segments = parse_segments(text).map_err(|message| format!("{} ({})", message, span))?;
        if segments.len() == 1 {
            if let Segment::Text(single) = &segments[0] {
                return self.gc_string_literal(single);
            }
        }

        let mut acc = self.gc_string_literal("")?;
        for segment in segments {
            let segment_value = match segment {
                Segment::Text(part) => self.gc_string_literal(&part)?,
                Segment::Expression(code) => {
                    let expr = parse_expression_fragment(&code)
                        .map_err(|message| format!("Invalid interpolation '{}': {}", code, message))?;
                    let value = self.generate_value(&expr, locals)?;
                    self.value_to_string(value, span)?
                }
            };
            acc = self.concat_strings(acc, segment_value)?;
        }

        Ok(acc)
    }

    fn gc_string_literal(&mut self, text: &str) -> Result<TypedValue<'ctx>, String> {
        let global = self
            .builder
            .build_global_string_ptr(text, "str_lit")
            .map_err(builder_error)?;
        let call = self
            .builder
            .build_call(
                self.gc_strdup_fn,
                &[global.as_pointer_value().into()],
                "gc_strdup",
            )
            .map_err(builder_error)?;
        let value = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "cb_gc_strdup returned void".to_string())?;
        Ok(TypedValue {
            value,
            ty: ValueType::String,
        })
    }

    fn value_to_string(
        &mut self,
        value: TypedValue<'ctx>,
        span: Span,
    ) -> Result<TypedValue<'ctx>, String> {
        let ptr_value = match value.ty {
            ValueType::String => value.value.into_pointer_value(),
            ValueType::Int => self
                .builder
                .build_call(
                    self.int_to_string_fn,
                    &[value.value.into_int_value().into()],
                    "i_to_s",
                )
                .map_err(builder_error)?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| "cb_int_to_string returned void".to_string())?
                .into_pointer_value(),
            ValueType::Float => self
                .builder
                .build_call(
                    self.float_to_string_fn,
                    &[value.value.into_float_value().into()],
                    "f_to_s",
                )
                .map_err(builder_error)?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| "cb_float_to_string returned void".to_string())?
                .into_pointer_value(),
            ValueType::Bool => self
                .builder
                .build_call(
                    self.bool_to_string_fn,
                    &[value.value.into_int_value().into()],
                    "b_to_s",
                )
                .map_err(builder_error)?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| "cb_bool_to_string returned void".to_string())?
                .into_pointer_value(),
            other => {
                return Err(format!(
                    "Interpolation does not support type '{}' ({})",
                    other.as_str(),
                    span
                ))
            }
        };

        Ok(TypedValue {
            value: ptr_value.into(),
            ty: ValueType::String,
        })
    }

    fn concat_strings(
        &mut self,
        left: TypedValue<'ctx>,
        right: TypedValue<'ctx>,
    ) -> Result<TypedValue<'ctx>, String> {
        let call = self
            .builder
            .build_call(
                self.string_concat_fn,
                &[
                    left.value.into_pointer_value().into(),
                    right.value.into_pointer_value().into(),
                ],
                "str_concat",
            )
            .map_err(builder_error)?;
        let value = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "cb_string_concat returned void".to_string())?;
        Ok(TypedValue {
            value,
            ty: ValueType::String,
        })
    }

    fn generate_array_literal(
        &mut self,
        elements: &[Expr],
        span: Span,
        locals: &HashMap<String, VarBinding<'ctx>>,
    ) -> Result<TypedValue<'ctx>, String> {
        if elements.is_empty() {
            return Err(format!(
                "empty array literal is not supported without explicit type ({})",
                span
            ));
        }

        let mut values = Vec::with_capacity(elements.len());
        for element in elements {
            values.push(self.generate_value(element, locals)?);
        }

        let mut element_type = values[0].ty.clone();
        for value in &values[1..] {
            element_type = unify_array_element_type(&element_type, &value.ty)
                .ok_or_else(|| format!("incompatible array literal element types ({})", span))?;
        }

        let new_fn = self.array_new_fn(&element_type);
        let call = self
            .builder
            .build_call(
                new_fn,
                &[self
                    .context
                    .i64_type()
                    .const_int(values.len() as u64, false)
                    .into()],
                "arr_new",
            )
            .map_err(builder_error)?;
        let array_ptr = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "array constructor returned void".to_string())?
            .into_pointer_value();

        for (index, value) in values.into_iter().enumerate() {
            let idx = TypedValue {
                value: self
                    .context
                    .i64_type()
                    .const_int(index as u64, false)
                    .into(),
                ty: ValueType::Int,
            };
            self.store_array_element(
                TypedValue {
                    value: array_ptr.into(),
                    ty: ValueType::Array(Box::new(element_type.clone())),
                },
                idx,
                value,
                span,
            )?;
        }

        Ok(TypedValue {
            value: array_ptr.into(),
            ty: ValueType::Array(Box::new(element_type)),
        })
    }

    fn load_array_element(
        &mut self,
        array_value: TypedValue<'ctx>,
        index_value: TypedValue<'ctx>,
        span: Span,
    ) -> Result<TypedValue<'ctx>, String> {
        let ValueType::Array(inner_type) = array_value.ty else {
            return Err(format!("indexing requires array value ({})", span));
        };
        if index_value.ty != ValueType::Int {
            return Err(format!("array index must be int ({})", span));
        }

        let (get_fn, element_type) = self.array_get_fn(&inner_type);
        let call = self
            .builder
            .build_call(
                get_fn,
                &[
                    array_value.value.into_pointer_value().into(),
                    index_value.value.into_int_value().into(),
                ],
                "arr_get",
            )
            .map_err(builder_error)?;
        let value = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "array get returned void".to_string())?;

        if element_type == ValueType::Bool {
            let byte = value.into_int_value();
            let boolean = self
                .builder
                .build_int_compare(
                    IntPredicate::NE,
                    byte,
                    self.context.i8_type().const_zero(),
                    "bool_from_i8",
                )
                .map_err(builder_error)?;
            Ok(TypedValue {
                value: boolean.into(),
                ty: ValueType::Bool,
            })
        } else {
            Ok(TypedValue {
                value,
                ty: element_type,
            })
        }
    }

    fn store_array_element(
        &mut self,
        array_value: TypedValue<'ctx>,
        index_value: TypedValue<'ctx>,
        assigned_value: TypedValue<'ctx>,
        span: Span,
    ) -> Result<(), String> {
        let ValueType::Array(inner_type) = array_value.ty else {
            return Err(format!("index assignment requires array target ({})", span));
        };
        if index_value.ty != ValueType::Int {
            return Err(format!("array index must be int ({})", span));
        }

        let target_type = *inner_type;
        let casted = self.cast_value_if_needed(assigned_value, &target_type)?;
        let set_fn = self.array_set_fn(&target_type);

        let value_arg: BasicValueEnum<'ctx> = if target_type == ValueType::Bool {
            let bool_value = casted.value.into_int_value();
            let as_byte = self
                .builder
                .build_int_cast(bool_value, self.context.i8_type(), "bool_to_i8")
                .map_err(builder_error)?;
            as_byte.into()
        } else {
            casted.value
        };

        self.builder
            .build_call(
                set_fn,
                &[
                    array_value.value.into_pointer_value().into(),
                    index_value.value.into_int_value().into(),
                    value_arg.into(),
                ],
                "arr_set",
            )
            .map_err(builder_error)?;

        Ok(())
    }

    fn generate_neg(
        &mut self,
        value: TypedValue<'ctx>,
        span: Span,
    ) -> Result<TypedValue<'ctx>, String> {
        match value.ty {
            ValueType::Int => {
                let out = self
                    .builder
                    .build_int_neg(value.value.into_int_value(), "ineg")
                    .map_err(builder_error)?;
                Ok(TypedValue {
                    value: out.into(),
                    ty: ValueType::Int,
                })
            }
            ValueType::Float => {
                let out = self
                    .builder
                    .build_float_neg(value.value.into_float_value(), "fneg")
                    .map_err(builder_error)?;
                Ok(TypedValue {
                    value: out.into(),
                    ty: ValueType::Float,
                })
            }
            _ => Err(format!(
                "invalid unary '-' for type '{}' ({})",
                value.ty.as_str(),
                span
            )),
        }
    }

    fn generate_binary(
        &mut self,
        left: TypedValue<'ctx>,
        op: BinaryOp,
        right: TypedValue<'ctx>,
        span: Span,
    ) -> Result<TypedValue<'ctx>, String> {
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                self.generate_arithmetic(left, op, right, span)
            }
            BinaryOp::Eq
            | BinaryOp::NotEq
            | BinaryOp::Greater
            | BinaryOp::GreaterEq
            | BinaryOp::Less
            | BinaryOp::LessEq => self.generate_compare(left, op, right, span),
            BinaryOp::And | BinaryOp::Or => {
                let l = self.expect_bool(left, span)?;
                let r = self.expect_bool(right, span)?;
                let out = match op {
                    BinaryOp::And => self.builder.build_and(l, r, "and").map_err(builder_error)?,
                    BinaryOp::Or => self.builder.build_or(l, r, "or").map_err(builder_error)?,
                    _ => unreachable!(),
                };
                Ok(TypedValue {
                    value: out.into(),
                    ty: ValueType::Bool,
                })
            }
        }
    }

    fn generate_arithmetic(
        &mut self,
        left: TypedValue<'ctx>,
        op: BinaryOp,
        right: TypedValue<'ctx>,
        span: Span,
    ) -> Result<TypedValue<'ctx>, String> {
        if !is_numeric(&left.ty) || !is_numeric(&right.ty) {
            return Err(format!(
                "arithmetic requires int/float, got '{}' and '{}' ({})",
                left.ty.as_str(),
                right.ty.as_str(),
                span
            ));
        }

        if left.ty == ValueType::Float || right.ty == ValueType::Float {
            let l = self.cast_to_float(left)?.value.into_float_value();
            let r = self.cast_to_float(right)?.value.into_float_value();
            let out = match op {
                BinaryOp::Add => self
                    .builder
                    .build_float_add(l, r, "fadd")
                    .map_err(builder_error)?,
                BinaryOp::Sub => self
                    .builder
                    .build_float_sub(l, r, "fsub")
                    .map_err(builder_error)?,
                BinaryOp::Mul => self
                    .builder
                    .build_float_mul(l, r, "fmul")
                    .map_err(builder_error)?,
                BinaryOp::Div => self
                    .builder
                    .build_float_div(l, r, "fdiv")
                    .map_err(builder_error)?,
                BinaryOp::Mod => self
                    .builder
                    .build_float_rem(l, r, "frem")
                    .map_err(builder_error)?,
                _ => unreachable!(),
            };
            return Ok(TypedValue {
                value: out.into(),
                ty: ValueType::Float,
            });
        }

        let l = left.value.into_int_value();
        let r = right.value.into_int_value();
        let out = match op {
            BinaryOp::Add => self
                .builder
                .build_int_add(l, r, "iadd")
                .map_err(builder_error)?,
            BinaryOp::Sub => self
                .builder
                .build_int_sub(l, r, "isub")
                .map_err(builder_error)?,
            BinaryOp::Mul => self
                .builder
                .build_int_mul(l, r, "imul")
                .map_err(builder_error)?,
            BinaryOp::Div => self
                .builder
                .build_int_signed_div(l, r, "idiv")
                .map_err(builder_error)?,
            BinaryOp::Mod => self
                .builder
                .build_int_signed_rem(l, r, "irem")
                .map_err(builder_error)?,
            _ => unreachable!(),
        };

        Ok(TypedValue {
            value: out.into(),
            ty: ValueType::Int,
        })
    }

    fn generate_compare(
        &mut self,
        left: TypedValue<'ctx>,
        op: BinaryOp,
        right: TypedValue<'ctx>,
        span: Span,
    ) -> Result<TypedValue<'ctx>, String> {
        if is_numeric(&left.ty) && is_numeric(&right.ty) {
            if left.ty == ValueType::Float || right.ty == ValueType::Float {
                let l = self.cast_to_float(left)?.value.into_float_value();
                let r = self.cast_to_float(right)?.value.into_float_value();
                let pred = match op {
                    BinaryOp::Eq => FloatPredicate::OEQ,
                    BinaryOp::NotEq => FloatPredicate::ONE,
                    BinaryOp::Greater => FloatPredicate::OGT,
                    BinaryOp::GreaterEq => FloatPredicate::OGE,
                    BinaryOp::Less => FloatPredicate::OLT,
                    BinaryOp::LessEq => FloatPredicate::OLE,
                    _ => unreachable!(),
                };
                let out = self
                    .builder
                    .build_float_compare(pred, l, r, "fcmp")
                    .map_err(builder_error)?;
                return Ok(TypedValue {
                    value: out.into(),
                    ty: ValueType::Bool,
                });
            }

            let l = left.value.into_int_value();
            let r = right.value.into_int_value();
            let pred = match op {
                BinaryOp::Eq => IntPredicate::EQ,
                BinaryOp::NotEq => IntPredicate::NE,
                BinaryOp::Greater => IntPredicate::SGT,
                BinaryOp::GreaterEq => IntPredicate::SGE,
                BinaryOp::Less => IntPredicate::SLT,
                BinaryOp::LessEq => IntPredicate::SLE,
                _ => unreachable!(),
            };
            let out = self
                .builder
                .build_int_compare(pred, l, r, "icmp")
                .map_err(builder_error)?;
            return Ok(TypedValue {
                value: out.into(),
                ty: ValueType::Bool,
            });
        }

        if left.ty == ValueType::Bool && right.ty == ValueType::Bool {
            let l = left.value.into_int_value();
            let r = right.value.into_int_value();
            let pred = match op {
                BinaryOp::Eq => IntPredicate::EQ,
                BinaryOp::NotEq => IntPredicate::NE,
                _ => return Err(format!("invalid bool compare op {:?} ({})", op, span)),
            };
            let out = self
                .builder
                .build_int_compare(pred, l, r, "bcmp")
                .map_err(builder_error)?;
            return Ok(TypedValue {
                value: out.into(),
                ty: ValueType::Bool,
            });
        }

        if left.ty == ValueType::String && right.ty == ValueType::String {
            let call = self
                .builder
                .build_call(
                    self.string_eq_fn,
                    &[
                        left.value.into_pointer_value().into(),
                        right.value.into_pointer_value().into(),
                    ],
                    "str_eq",
                )
                .map_err(builder_error)?;
            let eq = call
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| "cb_string_eq returned void".to_string())?
                .into_int_value();

            let out = match op {
                BinaryOp::Eq => eq,
                BinaryOp::NotEq => self
                    .builder
                    .build_not(eq, "str_ne")
                    .map_err(builder_error)?,
                _ => return Err(format!("invalid string compare op {:?} ({})", op, span)),
            };
            return Ok(TypedValue {
                value: out.into(),
                ty: ValueType::Bool,
            });
        }

        Err(format!(
            "incompatible compare types '{}' and '{}' ({})",
            left.ty.as_str(),
            right.ty.as_str(),
            span
        ))
    }

    fn generate_ternary(
        &mut self,
        condition: &Expr,
        then_expr: &Expr,
        else_expr: &Expr,
        span: Span,
        locals: &HashMap<String, VarBinding<'ctx>>,
    ) -> Result<TypedValue<'ctx>, String> {
        let cond_val = self.generate_value(condition, locals)?;
        let cond = self.expect_bool(cond_val, condition.span())?;

        let function = self.current_function()?;
        let then_bb = self.context.append_basic_block(function, "tern_then");
        let else_bb = self.context.append_basic_block(function, "tern_else");
        let merge_bb = self.context.append_basic_block(function, "tern_merge");

        self.builder
            .build_conditional_branch(cond, then_bb, else_bb)
            .map_err(builder_error)?;

        self.builder.position_at_end(then_bb);
        let then_val = self.generate_value(then_expr, locals)?;
        let then_end = self.current_block()?;
        if !self.current_block_terminated() {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(builder_error)?;
        }

        self.builder.position_at_end(else_bb);
        let else_val = self.generate_value(else_expr, locals)?;
        let else_end = self.current_block()?;
        if !self.current_block_terminated() {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(builder_error)?;
        }

        let out_ty = unify_ternary_type(&then_val.ty, &else_val.ty)
            .ok_or_else(|| format!("ternary branches incompatible ({})", span))?;

        self.builder.position_at_end(merge_bb);
        let phi = self
            .builder
            .build_phi(self.llvm_type(&out_ty)?, "tern_phi")
            .map_err(builder_error)?;
        let then_cast = self.cast_value_if_needed(then_val, &out_ty)?;
        let else_cast = self.cast_value_if_needed(else_val, &out_ty)?;
        phi.add_incoming(&[(&then_cast.value, then_end), (&else_cast.value, else_end)]);

        Ok(TypedValue {
            value: phi.as_basic_value(),
            ty: out_ty,
        })
    }

    fn generate_call(
        &mut self,
        callee: &CallTarget,
        args: &[Expr],
        span: Span,
        locals: &HashMap<String, VarBinding<'ctx>>,
    ) -> Result<Option<TypedValue<'ctx>>, String> {
        match callee {
            CallTarget::Name(name) if name == "Int" => {
                if args.len() != 1 {
                    return Err(format!("Int(...) expects one argument ({})", span));
                }
                let arg = self.generate_value(&args[0], locals)?;
                let converted = match arg.ty {
                    ValueType::Int => arg,
                    ValueType::Float => {
                        let v = self
                            .builder
                            .build_float_to_signed_int(
                                arg.value.into_float_value(),
                                self.context.i64_type(),
                                "f2i",
                            )
                            .map_err(builder_error)?;
                        TypedValue {
                            value: v.into(),
                            ty: ValueType::Int,
                        }
                    }
                    ValueType::Bool => {
                        let v = self
                            .builder
                            .build_int_z_extend(
                                arg.value.into_int_value(),
                                self.context.i64_type(),
                                "b2i",
                            )
                            .map_err(builder_error)?;
                        TypedValue {
                            value: v.into(),
                            ty: ValueType::Int,
                        }
                    }
                    ValueType::String => {
                        let call = self
                            .builder
                            .build_call(
                                self.to_int_fn,
                                &[arg.value.into_pointer_value().into()],
                                "s2i",
                            )
                            .map_err(builder_error)?;
                        let v = call
                            .try_as_basic_value()
                            .basic()
                            .ok_or_else(|| "cb_to_int returned void".to_string())?;
                        TypedValue {
                            value: v,
                            ty: ValueType::Int,
                        }
                    }
                    _ => {
                        return Err(format!(
                            "Int(...) cannot convert from '{}' ({})",
                            arg.ty.as_str(),
                            span
                        ))
                    }
                };
                Ok(Some(converted))
            }
            CallTarget::Name(name) => {
                let signature = self
                    .semantic_model
                    .functions
                    .get(name)
                    .ok_or_else(|| format!("unknown function '{}' ({})", name, span))?;
                if signature.params.len() != args.len() {
                    return Err(format!(
                        "function '{}' expects {} argument(s), got {} ({})",
                        name,
                        signature.params.len(),
                        args.len(),
                        span
                    ));
                }

                let function = self
                    .functions
                    .get(name)
                    .copied()
                    .ok_or_else(|| format!("unknown function '{}' ({})", name, span))?;

                let mut call_args = Vec::with_capacity(args.len());
                for (index, arg_expr) in args.iter().enumerate() {
                    let value = self.generate_value(arg_expr, locals)?;
                    let expected = &signature.params[index];
                    let casted = self.cast_value_if_needed(value, expected)?;
                    call_args.push(casted.value.into());
                }

                let call = self
                    .builder
                    .build_call(function, &call_args, "call_user")
                    .map_err(builder_error)?;

                if signature.return_type == ValueType::Void {
                    Ok(None)
                } else {
                    let value = call
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| format!("function '{}' returned void", name))?;
                    Ok(Some(TypedValue {
                        value,
                        ty: signature.return_type.clone(),
                    }))
                }
            }
            CallTarget::Qualified { module, name } => {
                if module == "Bridge" {
                    if name == "println" || name == "print" {
                        for arg in args {
                            let value = self.generate_value(arg, locals)?;
                            let print_fn = self.bridge_print_function(&value.ty)?;
                            self.builder
                                .build_call(print_fn, &[value.value.into()], "bridge_print")
                                .map_err(builder_error)?;
                        }
                        if name == "println" {
                            let newline_fn = self.bridge_newline_function();
                            self.builder
                                .build_call(newline_fn, &[], "bridge_newline")
                                .map_err(builder_error)?;
                        }
                        return Ok(None);
                    }

                    if name == "sleep" || name == "usleep" {
                        if args.len() != 1 {
                            return Err(format!(
                                "Bridge.{} expects exactly one int argument ({})",
                                name, span
                            ));
                        }
                        let arg = self.generate_value(&args[0], locals)?;
                        if arg.ty != ValueType::Int {
                            return Err(format!(
                                "Bridge.{} expects int, got '{}' ({})",
                                name,
                                arg.ty.as_str(),
                                span
                            ));
                        }
                        let sleep_fn = if name == "sleep" {
                            self.bridge_sleep_function()
                        } else {
                            self.bridge_usleep_function()
                        };
                        self.builder
                            .build_call(sleep_fn, &[arg.value.into()], "bridge_sleep")
                            .map_err(builder_error)?;
                        return Ok(None);
                    }

                    if name == "system" {
                        let command = self.build_system_command(args, locals, span)?;
                        let system_fn = self.bridge_system_function();
                        self.builder
                            .build_call(system_fn, &[command.value.into()], "bridge_system")
                            .map_err(builder_error)?;
                        return Ok(None);
                    }
                }

                let mut typed_args = Vec::with_capacity(args.len());
                let mut types = Vec::with_capacity(args.len());
                for arg in args {
                    let value = self.generate_value(arg, locals)?;
                    types.push(value.ty.clone());
                    typed_args.push(value);
                }

                let resolved = self
                    .semantic_model
                    .modules
                    .resolve_call(module, name, &types)
                    .ok_or_else(|| {
                        format!(
                            "no overload for {}.{}({}) ({})",
                            module,
                            name,
                            types
                                .iter()
                                .map(ValueType::as_str)
                                .collect::<Vec<_>>()
                                .join(", "),
                            span
                        )
                    })?;

                let function = self.get_or_declare_extern(&resolved)?;
                let mut call_args = Vec::with_capacity(typed_args.len());
                for (index, value) in typed_args.into_iter().enumerate() {
                    let expected = resolved
                        .argument_types
                        .get(index)
                        .ok_or_else(|| "resolved call argument mismatch".to_string())?;
                    let casted = self.cast_value_if_needed(value, expected)?;
                    call_args.push(casted.value.into());
                }

                let call = self
                    .builder
                    .build_call(function, &call_args, "call_ext")
                    .map_err(builder_error)?;

                if resolved.return_type == ValueType::Void {
                    Ok(None)
                } else {
                    let value = call
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| format!("extern '{}' returned void", resolved.symbol))?;
                    Ok(Some(TypedValue {
                        value,
                        ty: resolved.return_type,
                    }))
                }
            }
        }
    }

    fn get_or_declare_extern(
        &mut self,
        resolved: &ResolvedExternalCall,
    ) -> Result<FunctionValue<'ctx>, String> {
        if let Some(function) = self.extern_functions.get(&resolved.symbol) {
            return Ok(*function);
        }

        let arg_types: Vec<BasicMetadataTypeEnum<'ctx>> = resolved
            .argument_types
            .iter()
            .map(|ty| self.llvm_type(ty).map(Into::into))
            .collect::<Result<Vec<_>, _>>()?;

        let fn_type = if resolved.return_type == ValueType::Void {
            self.context.void_type().fn_type(&arg_types, false)
        } else {
            self.llvm_type(&resolved.return_type)?
                .fn_type(&arg_types, false)
        };

        let function = self.module.add_function(&resolved.symbol, fn_type, None);
        self.extern_functions
            .insert(resolved.symbol.clone(), function);
        Ok(function)
    }

    fn bridge_print_function(&self, ty: &ValueType) -> Result<FunctionValue<'ctx>, String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let void = self.context.void_type();
        let i64_type = self.context.i64_type();
        let f64_type = self.context.f64_type();
        let bool_type = self.context.bool_type();

        let (name, fn_type) = match ty {
            ValueType::Int => ("bridge_print_int", void.fn_type(&[i64_type.into()], false)),
            ValueType::Float => (
                "bridge_print_float",
                void.fn_type(&[f64_type.into()], false),
            ),
            ValueType::String => ("bridge_print_string", void.fn_type(&[ptr.into()], false)),
            ValueType::Bool => (
                "bridge_print_bool",
                void.fn_type(&[bool_type.into()], false),
            ),
            other => {
                return Err(format!(
                    "Bridge.println cannot print type '{}'",
                    other.as_str()
                ))
            }
        };

        Ok(self
            .module
            .get_function(name)
            .unwrap_or_else(|| self.module.add_function(name, fn_type, None)))
    }

    fn bridge_newline_function(&self) -> FunctionValue<'ctx> {
        self.module
            .get_function("bridge_print_newline")
            .unwrap_or_else(|| {
                self.module.add_function(
                    "bridge_print_newline",
                    self.context.void_type().fn_type(&[], false),
                    None,
                )
            })
    }

    fn bridge_sleep_function(&self) -> FunctionValue<'ctx> {
        self.module
            .get_function("bridge_sleep")
            .unwrap_or_else(|| {
                self.module.add_function(
                    "bridge_sleep",
                    self.context
                        .void_type()
                        .fn_type(&[self.context.i64_type().into()], false),
                    None,
                )
            })
    }

    fn bridge_usleep_function(&self) -> FunctionValue<'ctx> {
        self.module
            .get_function("bridge_usleep")
            .unwrap_or_else(|| {
                self.module.add_function(
                    "bridge_usleep",
                    self.context
                        .void_type()
                        .fn_type(&[self.context.i64_type().into()], false),
                    None,
                )
            })
    }

    fn bridge_system_function(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.module
            .get_function("bridge_system")
            .unwrap_or_else(|| {
                self.module
                    .add_function("bridge_system", self.context.void_type().fn_type(&[ptr.into()], false), None)
            })
    }

    fn build_system_command(
        &mut self,
        args: &[Expr],
        locals: &HashMap<String, VarBinding<'ctx>>,
        span: Span,
    ) -> Result<TypedValue<'ctx>, String> {
        if args.is_empty() {
            return self.gc_string_literal("");
        }

        let mut command = self.generate_value(&args[0], locals)?;
        if command.ty != ValueType::String {
            return Err(format!(
                "Bridge.system expects only string arguments, got '{}' ({})",
                command.ty.as_str(),
                span
            ));
        }

        for argument in args.iter().skip(1) {
            let next = self.generate_value(argument, locals)?;
            if next.ty != ValueType::String {
                return Err(format!(
                    "Bridge.system expects only string arguments, got '{}' ({})",
                    next.ty.as_str(),
                    span
                ));
            }
            let separator = self.gc_string_literal(" ")?;
            command = self.concat_strings(command, separator)?;
            command = self.concat_strings(command, next)?;
        }

        Ok(command)
    }

    fn cast_value_if_needed(
        &mut self,
        value: TypedValue<'ctx>,
        target: &ValueType,
    ) -> Result<TypedValue<'ctx>, String> {
        if &value.ty == target {
            return Ok(value);
        }

        match (&value.ty, target) {
            (ValueType::Int, ValueType::Float) => {
                let casted = self
                    .builder
                    .build_signed_int_to_float(
                        value.value.into_int_value(),
                        self.context.f64_type(),
                        "i2f",
                    )
                    .map_err(builder_error)?;
                Ok(TypedValue {
                    value: casted.into(),
                    ty: ValueType::Float,
                })
            }
            _ => Err(format!(
                "unsupported cast '{}' -> '{}'",
                value.ty.as_str(),
                target.as_str()
            )),
        }
    }

    fn cast_to_float(&mut self, value: TypedValue<'ctx>) -> Result<TypedValue<'ctx>, String> {
        self.cast_value_if_needed(value, &ValueType::Float)
    }

    fn generate_value(
        &mut self,
        expr: &Expr,
        locals: &HashMap<String, VarBinding<'ctx>>,
    ) -> Result<TypedValue<'ctx>, String> {
        let value = self.generate_expr(expr, locals)?;
        self.require_value(value, expr.span())
    }

    fn require_value(
        &self,
        value: Option<TypedValue<'ctx>>,
        span: Span,
    ) -> Result<TypedValue<'ctx>, String> {
        value.ok_or_else(|| format!("expected value but got void ({})", span))
    }

    fn expect_bool(&self, value: TypedValue<'ctx>, span: Span) -> Result<IntValue<'ctx>, String> {
        if value.ty != ValueType::Bool {
            return Err(format!(
                "expected bool but got '{}' ({})",
                value.ty.as_str(),
                span
            ));
        }
        Ok(value.value.into_int_value())
    }

    fn lookup_binding<'b>(
        &'b self,
        name: &str,
        locals: &'b HashMap<String, VarBinding<'ctx>>,
    ) -> Option<&'b VarBinding<'ctx>> {
        locals.get(name).or_else(|| self.globals.get(name))
    }

    fn llvm_type(&self, ty: &ValueType) -> Result<BasicTypeEnum<'ctx>, String> {
        match ty {
            ValueType::Int => Ok(self.context.i64_type().into()),
            ValueType::Float => Ok(self.context.f64_type().into()),
            ValueType::String => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            ValueType::Bool => Ok(self.context.bool_type().into()),
            ValueType::Array(_) => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            ValueType::Void => Err("void is not a value type".to_string()),
        }
    }

    fn zero_value(&self, ty: &ValueType) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            ValueType::Int => Ok(self.context.i64_type().const_zero().into()),
            ValueType::Float => Ok(self.context.f64_type().const_float(0.0).into()),
            ValueType::String | ValueType::Array(_) => Ok(self
                .context
                .ptr_type(AddressSpace::default())
                .const_null()
                .into()),
            ValueType::Bool => Ok(self.context.bool_type().const_zero().into()),
            ValueType::Void => Err("void is not a value type".to_string()),
        }
    }

    fn create_entry_alloca(
        &self,
        function: FunctionValue<'ctx>,
        ty: &ValueType,
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let entry = function
            .get_first_basic_block()
            .ok_or_else(|| "function has no entry block".to_string())?;

        let builder = self.context.create_builder();
        if let Some(first) = entry.get_first_instruction() {
            builder.position_before(&first);
        } else {
            builder.position_at_end(entry);
        }

        builder
            .build_alloca(self.llvm_type(ty)?, name)
            .map_err(builder_error)
    }

    fn array_new_fn(&self, element_type: &ValueType) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let name = match element_type {
            ValueType::Int => "cb_array_new_i64",
            ValueType::Float => "cb_array_new_f64",
            ValueType::String => "cb_array_new_str",
            ValueType::Bool => "cb_array_new_bool",
            _ => "cb_array_new_ptr",
        };
        self.module.get_function(name).unwrap_or_else(|| {
            self.module
                .add_function(name, ptr.fn_type(&[i64_type.into()], false), None)
        })
    }

    fn array_get_fn(&self, element_type: &ValueType) -> (FunctionValue<'ctx>, ValueType) {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let f64_type = self.context.f64_type();
        let i8_type = self.context.i8_type();

        let (name, fn_type, output) = match element_type {
            ValueType::Int => (
                "cb_array_get_i64",
                i64_type.fn_type(&[ptr.into(), i64_type.into()], false),
                ValueType::Int,
            ),
            ValueType::Float => (
                "cb_array_get_f64",
                f64_type.fn_type(&[ptr.into(), i64_type.into()], false),
                ValueType::Float,
            ),
            ValueType::String => (
                "cb_array_get_str",
                ptr.fn_type(&[ptr.into(), i64_type.into()], false),
                ValueType::String,
            ),
            ValueType::Bool => (
                "cb_array_get_bool",
                i8_type.fn_type(&[ptr.into(), i64_type.into()], false),
                ValueType::Bool,
            ),
            _ => (
                "cb_array_get_ptr",
                ptr.fn_type(&[ptr.into(), i64_type.into()], false),
                ValueType::Array(Box::new(ValueType::Int)),
            ),
        };

        let function = self
            .module
            .get_function(name)
            .unwrap_or_else(|| self.module.add_function(name, fn_type, None));
        (function, output)
    }

    fn array_set_fn(&self, element_type: &ValueType) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let f64_type = self.context.f64_type();
        let i8_type = self.context.i8_type();
        let void = self.context.void_type();

        let (name, fn_type) = match element_type {
            ValueType::Int => (
                "cb_array_set_i64",
                void.fn_type(&[ptr.into(), i64_type.into(), i64_type.into()], false),
            ),
            ValueType::Float => (
                "cb_array_set_f64",
                void.fn_type(&[ptr.into(), i64_type.into(), f64_type.into()], false),
            ),
            ValueType::String => (
                "cb_array_set_str",
                void.fn_type(&[ptr.into(), i64_type.into(), ptr.into()], false),
            ),
            ValueType::Bool => (
                "cb_array_set_bool",
                void.fn_type(&[ptr.into(), i64_type.into(), i8_type.into()], false),
            ),
            _ => (
                "cb_array_set_ptr",
                void.fn_type(&[ptr.into(), i64_type.into(), ptr.into()], false),
            ),
        };

        self.module
            .get_function(name)
            .unwrap_or_else(|| self.module.add_function(name, fn_type, None))
    }

    fn current_block(&self) -> Result<BasicBlock<'ctx>, String> {
        self.builder
            .get_insert_block()
            .ok_or_else(|| "builder has no insertion block".to_string())
    }

    fn current_function(&self) -> Result<FunctionValue<'ctx>, String> {
        self.current_block()?
            .get_parent()
            .ok_or_else(|| "cannot determine current function".to_string())
    }

    fn current_block_terminated(&self) -> bool {
        self.builder
            .get_insert_block()
            .and_then(|block| block.get_terminator())
            .is_some()
    }
}

fn is_numeric(ty: &ValueType) -> bool {
    matches!(ty, ValueType::Int | ValueType::Float)
}

fn unify_ternary_type(left: &ValueType, right: &ValueType) -> Option<ValueType> {
    if left == right {
        return Some(left.clone());
    }
    if is_numeric(left) && is_numeric(right) {
        return Some(ValueType::Float);
    }
    None
}

fn unify_array_element_type(left: &ValueType, right: &ValueType) -> Option<ValueType> {
    if left == right {
        return Some(left.clone());
    }
    if is_numeric(left) && is_numeric(right) {
        return Some(ValueType::Float);
    }
    None
}

fn builder_error(error: BuilderError) -> String {
    format!("LLVM builder error: {error:?}")
}
