impl<'ctx, 'a> CodeGenerator<'ctx, 'a> {
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
