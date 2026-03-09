impl<'ctx, 'a> CodeGenerator<'ctx, 'a> {
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
}
