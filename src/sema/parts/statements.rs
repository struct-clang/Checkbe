impl<'a> SemanticAnalyzer<'a> {
    fn analyze_statement(
        &mut self,
        statement: &Stmt,
        locals: &mut HashMap<String, VariableInfo>,
        right: Option<&PermissionSet>,
        function_return_type: &ValueType,
        has_return: &mut bool,
    ) {
        match statement {
            Stmt::VarDecl(decl) => self.analyze_local_var_decl(decl, locals, right),
            Stmt::Assign {
                target,
                value,
                span,
            } => {
                let target_info = self.assignment_target_info(target, locals, right, *span);
                if let Some((owning_variable, target_type)) = target_info {
                    if let Some(value_type) = self.analyze_expr(value, locals, right) {
                        if !is_assignable(&target_type, &value_type) {
                            self.diagnostics.error(
                                Some(*span),
                                format!(
                                    "Cannot assign '{}' to target of type '{}'",
                                    value_type.as_str(),
                                    target_type.as_str()
                                ),
                            );
                        }
                    }
                    self.enforce_write_access(&owning_variable, right, *span);
                }
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
                span,
            } => {
                if let Some(condition_type) = self.analyze_expr(condition, locals, right) {
                    if condition_type != ValueType::Bool {
                        self.diagnostics.error(
                            Some(*span),
                            format!(
                                "If condition must be bool, got '{}'",
                                condition_type.as_str()
                            ),
                        );
                    }
                }

                let mut then_locals = locals.clone();
                for nested in then_branch {
                    self.analyze_statement(
                        nested,
                        &mut then_locals,
                        right,
                        function_return_type,
                        has_return,
                    );
                }

                let mut else_locals = locals.clone();
                for nested in else_branch {
                    self.analyze_statement(
                        nested,
                        &mut else_locals,
                        right,
                        function_return_type,
                        has_return,
                    );
                }
            }
            Stmt::While {
                condition,
                body,
                span,
            } => {
                if let Some(condition_type) = self.analyze_expr(condition, locals, right) {
                    if condition_type != ValueType::Bool {
                        self.diagnostics.error(
                            Some(*span),
                            format!(
                                "While condition must be bool, got '{}'",
                                condition_type.as_str()
                            ),
                        );
                    }
                }

                let mut loop_locals = locals.clone();
                for nested in body {
                    self.analyze_statement(
                        nested,
                        &mut loop_locals,
                        right,
                        function_return_type,
                        has_return,
                    );
                }
            }
            Stmt::DoWhile {
                body,
                condition,
                span,
            } => {
                let mut loop_locals = locals.clone();
                for nested in body {
                    self.analyze_statement(
                        nested,
                        &mut loop_locals,
                        right,
                        function_return_type,
                        has_return,
                    );
                }

                if let Some(condition_type) = self.analyze_expr(condition, locals, right) {
                    if condition_type != ValueType::Bool {
                        self.diagnostics.error(
                            Some(*span),
                            format!(
                                "Do-while condition must be bool, got '{}'",
                                condition_type.as_str()
                            ),
                        );
                    }
                }
            }
            Stmt::Expr { expr, .. } => {
                self.analyze_expr(expr, locals, right);
            }
            Stmt::Block { statements, .. } => {
                let mut nested = locals.clone();
                for nested_stmt in statements {
                    self.analyze_statement(
                        nested_stmt,
                        &mut nested,
                        right,
                        function_return_type,
                        has_return,
                    );
                }
            }
            Stmt::Return { value, span } => {
                *has_return = true;
                match (function_return_type, value) {
                    (ValueType::Void, None) => {}
                    (ValueType::Void, Some(_)) => self
                        .diagnostics
                        .error(Some(*span), "Void function cannot return a value"),
                    (expected, None) => self.diagnostics.error(
                        Some(*span),
                        format!("Function must return '{}'", expected.as_str()),
                    ),
                    (expected, Some(expr)) => {
                        if let Some(actual) = self.analyze_expr(expr, locals, right) {
                            if !is_assignable(expected, &actual) {
                                self.diagnostics.error(
                                    Some(*span),
                                    format!(
                                        "Return type '{}' is incompatible with '{}'",
                                        actual.as_str(),
                                        expected.as_str()
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    fn analyze_local_var_decl(
        &mut self,
        decl: &VarDecl,
        locals: &mut HashMap<String, VariableInfo>,
        right: Option<&PermissionSet>,
    ) {
        if locals.contains_key(&decl.name) {
            self.diagnostics.error(
                Some(decl.span),
                format!("Local variable '{}' is already declared", decl.name),
            );
            return;
        }

        if let Some(capability) = &decl.entitlement {
            if !self.capabilities.contains_key(capability) {
                self.diagnostics.error(
                    Some(decl.span),
                    format!("Capability '{}' not found", capability),
                );
            }
        }

        let Some(initializer_type) = self.analyze_expr(&decl.initializer, locals, right) else {
            return;
        };

        let resolved_type = if let Some(explicit_type) = &decl.explicit_type {
            if !is_assignable(explicit_type, &initializer_type) {
                self.diagnostics.error(
                    Some(decl.span),
                    format!(
                        "Cannot initialize '{}' with value of type '{}'",
                        explicit_type.as_str(),
                        initializer_type.as_str()
                    ),
                );
                explicit_type.clone()
            } else {
                explicit_type.clone()
            }
        } else {
            initializer_type
        };

        locals.insert(
            decl.name.clone(),
            VariableInfo {
                ty: resolved_type,
                entitlement: decl.entitlement.clone(),
                span: decl.span,
            },
        );
    }
}
