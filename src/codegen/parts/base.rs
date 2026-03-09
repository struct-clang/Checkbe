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
    argv_array_fn: FunctionValue<'ctx>,
    array_str_to_string_fn: FunctionValue<'ctx>,
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
        let argv_array_fn = module.add_function(
            "cb_build_argv_array",
            ptr.fn_type(&[context.i64_type().into(), ptr.into()], false),
            None,
        );
        let array_str_to_string_fn = module.add_function(
            "cb_array_str_to_string",
            ptr.fn_type(&[ptr.into()], false),
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
            argv_array_fn,
            array_str_to_string_fn,
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
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let c_main = self
            .module
            .add_function(
                "main",
                i32_type.fn_type(&[i32_type.into(), ptr_type.into()], false),
                None,
            );
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
            let main_signature = self
                .semantic_model
                .functions
                .get("main")
                .ok_or_else(|| "Missing semantic signature for main".to_string())?;

            match main_signature.params.as_slice() {
                [] => {
                    self.builder
                        .build_call(*user_main, &[], "user_main")
                        .map_err(builder_error)?;
                }
                [first, second]
                    if *first == ValueType::Int
                        && *second == ValueType::Array(Box::new(ValueType::String)) =>
                {
                    let argc_i32 = c_main
                        .get_nth_param(0)
                        .ok_or_else(|| "c main argc parameter missing".to_string())?
                        .into_int_value();
                    let argv_ptr = c_main
                        .get_nth_param(1)
                        .ok_or_else(|| "c main argv parameter missing".to_string())?
                        .into_pointer_value();
                    let argc_i64 = self
                        .builder
                        .build_int_s_extend(argc_i32, self.context.i64_type(), "argc_i64")
                        .map_err(builder_error)?;
                    let argv_array = self
                        .builder
                        .build_call(
                            self.argv_array_fn,
                            &[argc_i64.into(), argv_ptr.into()],
                            "build_argv_array",
                        )
                        .map_err(builder_error)?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| "cb_build_argv_array returned void".to_string())?;
                    self.builder
                        .build_call(
                            *user_main,
                            &[argc_i64.into(), argv_array.into()],
                            "user_main",
                        )
                        .map_err(builder_error)?;
                }
                _ => return Err("main() has unsupported signature".to_string()),
            }
        }

        self.builder
            .build_return(Some(&i32_type.const_int(0, false)))
            .map_err(builder_error)?;
        Ok(())
    }
}
