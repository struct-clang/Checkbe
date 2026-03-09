impl<'a> SemanticAnalyzer<'a> {
    fn analyze_binary_expr(
        &mut self,
        op: &BinaryOp,
        left_type: &ValueType,
        right_type: &ValueType,
        span: Span,
    ) -> Option<ValueType> {
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                if is_numeric(left_type) && is_numeric(right_type) {
                    if left_type == &ValueType::Float || right_type == &ValueType::Float {
                        Some(ValueType::Float)
                    } else {
                        Some(ValueType::Int)
                    }
                } else {
                    self.diagnostics.error(
                        Some(span),
                        format!(
                            "Arithmetic operator requires int/float, got '{}' and '{}'",
                            left_type.as_str(),
                            right_type.as_str()
                        ),
                    );
                    None
                }
            }
            BinaryOp::Eq | BinaryOp::NotEq => {
                if left_type == right_type || (is_numeric(left_type) && is_numeric(right_type)) {
                    Some(ValueType::Bool)
                } else {
                    self.diagnostics.error(
                        Some(span),
                        format!(
                            "Equality comparison requires compatible types, got '{}' and '{}'",
                            left_type.as_str(),
                            right_type.as_str()
                        ),
                    );
                    None
                }
            }
            BinaryOp::Greater | BinaryOp::GreaterEq | BinaryOp::Less | BinaryOp::LessEq => {
                if is_numeric(left_type) && is_numeric(right_type) {
                    Some(ValueType::Bool)
                } else {
                    self.diagnostics.error(
                        Some(span),
                        format!(
                            "Comparison operator requires int/float, got '{}' and '{}'",
                            left_type.as_str(),
                            right_type.as_str()
                        ),
                    );
                    None
                }
            }
            BinaryOp::And | BinaryOp::Or => {
                if left_type == &ValueType::Bool && right_type == &ValueType::Bool {
                    Some(ValueType::Bool)
                } else {
                    self.diagnostics.error(
                        Some(span),
                        format!(
                            "Logical operator requires bool, got '{}' and '{}'",
                            left_type.as_str(),
                            right_type.as_str()
                        ),
                    );
                    None
                }
            }
        }
    }

    fn lookup_variable(
        &mut self,
        name: &str,
        locals: &HashMap<String, VariableInfo>,
    ) -> Option<VariableInfo> {
        if let Some(local) = locals.get(name) {
            return Some(local.clone());
        }

        if let Some(global) = self.globals.get(name) {
            if global.ty == ValueType::Void {
                self.diagnostics.error(
                    Some(global.span),
                    format!(
                        "Global variable '{}' is used before type is resolved (check declaration order)",
                        name
                    ),
                );
                return None;
            }
            return Some(global.clone());
        }

        None
    }

    fn enforce_read_access(
        &mut self,
        variable: &VariableInfo,
        right: Option<&PermissionSet>,
        span: Span,
    ) {
        self.enforce_right(right, Operation::Read, span);
        self.enforce_capability(variable, Operation::Read, span);
    }

    fn enforce_write_access(
        &mut self,
        variable: &VariableInfo,
        right: Option<&PermissionSet>,
        span: Span,
    ) {
        self.enforce_right(right, Operation::Write, span);
        self.enforce_capability(variable, Operation::Write, span);
    }

    fn enforce_right(&mut self, right: Option<&PermissionSet>, operation: Operation, span: Span) {
        if let Some(right) = right {
            if !right.allows(operation) {
                self.diagnostics.error(
                    Some(span),
                    format!(
                        "Right violation: operation '{}' is not allowed (rule: {})",
                        op_name(operation),
                        right.raw.join(", ")
                    ),
                );
            }
        }
    }

    fn enforce_capability(&mut self, variable: &VariableInfo, operation: Operation, span: Span) {
        let Some(capability_name) = &variable.entitlement else {
            return;
        };

        if let Some(set) = self.capabilities.get(capability_name) {
            if !set.allows(operation) {
                self.diagnostics.error(
                    Some(span),
                    format!(
                        "Capability violation '{}': operation '{}' is not allowed (rule: {})",
                        capability_name,
                        op_name(operation),
                        set.raw.join(", ")
                    ),
                );
            }
        }
    }

    fn assignment_target_info(
        &mut self,
        target: &AssignTarget,
        locals: &HashMap<String, VariableInfo>,
        right: Option<&PermissionSet>,
        span: Span,
    ) -> Option<(VariableInfo, ValueType)> {
        match target {
            AssignTarget::Identifier(name) => {
                let variable = self.lookup_variable(name, locals)?;
                Some((variable.clone(), variable.ty))
            }
            AssignTarget::Index { array, index } => {
                let array_type = self.analyze_expr(array, locals, right)?;
                let index_type = self.analyze_expr(index, locals, right)?;
                if index_type != ValueType::Int {
                    self.diagnostics.error(
                        Some(span),
                        format!("Array index must be int, got '{}'", index_type.as_str()),
                    );
                    return None;
                }

                let owning_variable = self.find_root_variable(array, locals).ok_or_else(|| {
                    self.diagnostics.error(
                        Some(span),
                        "Array assignment target must originate from a variable",
                    );
                });
                let Ok(owning_variable) = owning_variable else {
                    return None;
                };

                if let ValueType::Array(inner) = array_type {
                    Some((owning_variable, (*inner).clone()))
                } else {
                    self.diagnostics.error(
                        Some(span),
                        format!(
                            "Index assignment requires array target, got '{}'",
                            array_type.as_str()
                        ),
                    );
                    None
                }
            }
        }
    }

    fn find_root_variable(
        &mut self,
        expr: &Expr,
        locals: &HashMap<String, VariableInfo>,
    ) -> Option<VariableInfo> {
        match expr {
            Expr::Identifier(name, _) => self.lookup_variable(name, locals),
            Expr::Index { array, .. } => self.find_root_variable(array, locals),
            _ => None,
        }
    }
}

fn is_numeric(ty: &ValueType) -> bool {
    matches!(ty, ValueType::Int | ValueType::Float)
}

fn is_assignable(target: &ValueType, source: &ValueType) -> bool {
    if target == source {
        return true;
    }

    if target == &ValueType::Float && source == &ValueType::Int {
        return true;
    }

    matches!(
        (target, source),
        (ValueType::Array(target_inner), ValueType::Array(source_inner))
            if is_assignable(target_inner, source_inner)
    )
}

fn op_name(operation: Operation) -> &'static str {
    match operation {
        Operation::Read => "read",
        Operation::Write => "write",
    }
}
