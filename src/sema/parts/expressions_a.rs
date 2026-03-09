impl<'a> SemanticAnalyzer<'a> {
    fn analyze_expr(
        &mut self,
        expr: &Expr,
        locals: &HashMap<String, VariableInfo>,
        right: Option<&PermissionSet>,
    ) -> Option<ValueType> {
        match expr {
            Expr::IntLiteral(_, _) => Some(ValueType::Int),
            Expr::FloatLiteral(_, _) => Some(ValueType::Float),
            Expr::StringLiteral(text, span) => {
                let segments = match parse_segments(text) {
                    Ok(parts) => parts,
                    Err(message) => {
                        self.diagnostics.error(Some(*span), message);
                        return None;
                    }
                };

                for segment in segments {
                    if let Segment::Expression(code) = segment {
                        let parsed = match parse_expression_fragment(&code) {
                            Ok(expr) => expr,
                            Err(message) => {
                                self.diagnostics.error(
                                    Some(*span),
                                    format!("Invalid interpolation expression '{}': {}", code, message),
                                );
                                return None;
                            }
                        };

                        let Some(result_type) = self.analyze_expr(&parsed, locals, right) else {
                            self.diagnostics.error(
                                Some(*span),
                                format!("Interpolation expression '{}' is invalid", code),
                            );
                            return None;
                        };

                        match result_type {
                            ValueType::Int
                            | ValueType::Float
                            | ValueType::String
                            | ValueType::Bool => {}
                            ValueType::Array(inner) if *inner == ValueType::String => {}
                            _ => {
                                self.diagnostics.error(
                                    Some(*span),
                                    format!(
                                        "Interpolation does not support type '{}'",
                                        result_type.as_str()
                                    ),
                                );
                                return None;
                            }
                        }
                    }
                }

                Some(ValueType::String)
            }
            Expr::BoolLiteral(_, _) => Some(ValueType::Bool),
            Expr::ArrayLiteral(elements, span) => {
                if elements.is_empty() {
                    self.diagnostics.error(
                        Some(*span),
                        "Empty array literal requires explicit type annotation",
                    );
                    return None;
                }

                let mut element_type: Option<ValueType> = None;
                for element in elements {
                    let current = self.analyze_expr(element, locals, right)?;
                    element_type = match element_type {
                        None => Some(current),
                        Some(existing) if existing == current => Some(existing),
                        Some(existing) if is_numeric(&existing) && is_numeric(&current) => {
                            Some(ValueType::Float)
                        }
                        Some(existing) => {
                            self.diagnostics.error(
                                Some(*span),
                                format!(
                                    "Array literal has incompatible element types '{}' and '{}'",
                                    existing.as_str(),
                                    current.as_str()
                                ),
                            );
                            return None;
                        }
                    };
                }

                Some(ValueType::Array(Box::new(element_type?)))
            }
            Expr::Identifier(name, span) => {
                let variable = self.lookup_variable(name, locals)?;
                self.enforce_read_access(&variable, right, *span);
                Some(variable.ty)
            }
            Expr::Index { array, index, span } => {
                let array_type = self.analyze_expr(array, locals, right)?;
                let index_type = self.analyze_expr(index, locals, right)?;
                if index_type != ValueType::Int {
                    self.diagnostics.error(
                        Some(*span),
                        format!("Array index must be int, got '{}'", index_type.as_str()),
                    );
                    return None;
                }
                if let ValueType::Array(inner) = array_type {
                    Some((*inner).clone())
                } else {
                    self.diagnostics.error(
                        Some(*span),
                        format!(
                            "Indexing is only valid for arrays, got '{}'",
                            array_type.as_str()
                        ),
                    );
                    None
                }
            }
            Expr::NewObject { span, .. } => {
                self.diagnostics.error(
                    Some(*span),
                    "new Capability/new Right constructors are only allowed in capability/right declarations",
                );
                None
            }
            Expr::Unary { op, expr, span } => {
                let inner_type = self.analyze_expr(expr, locals, right)?;
                match op {
                    UnaryOp::Neg => {
                        if is_numeric(&inner_type) {
                            Some(inner_type)
                        } else {
                            self.diagnostics.error(
                                Some(*span),
                                format!(
                                    "Operator '-' is only valid for int/float, got '{}'",
                                    inner_type.as_str()
                                ),
                            );
                            return None;
                        }
                    }
                    UnaryOp::Not => {
                        if inner_type == ValueType::Bool {
                            Some(ValueType::Bool)
                        } else {
                            self.diagnostics.error(
                                Some(*span),
                                format!(
                                    "Operator '!' is only valid for bool, got '{}'",
                                    inner_type.as_str()
                                ),
                            );
                            None
                        }
                    }
                }
            }
            Expr::Binary {
                left,
                op,
                right: right_expr,
                span,
            } => {
                let left_type = self.analyze_expr(left, locals, right)?;
                let right_type = self.analyze_expr(right_expr, locals, right)?;
                self.analyze_binary_expr(op, &left_type, &right_type, *span)
            }
            Expr::Ternary {
                condition,
                then_expr,
                else_expr,
                span,
            } => {
                let condition_type = self.analyze_expr(condition, locals, right)?;
                if condition_type != ValueType::Bool {
                    self.diagnostics.error(
                        Some(condition.span()),
                        format!(
                            "Ternary condition must be bool, got '{}'",
                            condition_type.as_str()
                        ),
                    );
                    return None;
                }

                let then_type = self.analyze_expr(then_expr, locals, right)?;
                let else_type = self.analyze_expr(else_expr, locals, right)?;
                if then_type == else_type {
                    Some(then_type)
                } else if is_numeric(&then_type) && is_numeric(&else_type) {
                    Some(ValueType::Float)
                } else {
                    self.diagnostics.error(
                        Some(*span),
                        format!(
                            "Ternary branches have incompatible types '{}' and '{}'",
                            then_type.as_str(),
                            else_type.as_str()
                        ),
                    );
                    None
                }
            }
            Expr::Call { callee, args, span } => match callee {
                CallTarget::Name(function_name) => {
                    if function_name == "Int" {
                        if args.len() != 1 {
                            self.diagnostics.error(
                                Some(*span),
                                format!(
                                    "Int(...) expects exactly one argument, got {}",
                                    args.len()
                                ),
                            );
                            return None;
                        }
                        let arg_type = self.analyze_expr(&args[0], locals, right)?;
                        match arg_type {
                            ValueType::Int
                            | ValueType::Float
                            | ValueType::Bool
                            | ValueType::String => Some(ValueType::Int),
                            _ => {
                                self.diagnostics.error(
                                    Some(*span),
                                    format!(
                                        "Int(...) does not support conversion from '{}'",
                                        arg_type.as_str()
                                    ),
                                );
                                None
                            }
                        }
                    } else {
                        let Some(signature) = self.functions.get(function_name).cloned() else {
                            self.diagnostics.error(
                                Some(*span),
                                format!("Function '{}' not found", function_name),
                            );
                            return None;
                        };
                        if signature.params.len() != args.len() {
                            self.diagnostics.error(
                                Some(*span),
                                format!(
                                    "Function '{}' expects {} argument(s), got {}",
                                    function_name,
                                    signature.params.len(),
                                    args.len()
                                ),
                            );
                            return None;
                        }

                        for (index, argument) in args.iter().enumerate() {
                            let Some(argument_type) = self.analyze_expr(argument, locals, right)
                            else {
                                return None;
                            };
                            let expected = &signature.params[index];
                            if !is_assignable(expected, &argument_type) {
                                self.diagnostics.error(
                                    Some(argument.span()),
                                    format!(
                                        "Argument {} for '{}' has type '{}', expected '{}'",
                                        index + 1,
                                        function_name,
                                        argument_type.as_str(),
                                        expected.as_str()
                                    ),
                                );
                                return None;
                            }
                        }

                        Some(signature.return_type)
                    }
                }
                CallTarget::Qualified { module, name } => {
                    if !self.modules.contains(module) {
                        self.diagnostics
                            .error(Some(*span), format!("Module '{}' is not imported", module));
                        return None;
                    }

                    if module == "Bridge" {
                        if name == "println" || name == "print" {
                            for argument in args {
                                let argument_type = self.analyze_expr(argument, locals, right)?;
                                match argument_type {
                                    ValueType::Int
                                    | ValueType::Float
                                    | ValueType::String
                                    | ValueType::Bool => {}
                                    other => {
                                        self.diagnostics.error(
                                            Some(argument.span()),
                                            format!(
                                                "Bridge.{} does not support argument type '{}'",
                                                name,
                                                other.as_str()
                                            ),
                                        );
                                        return None;
                                    }
                                }
                            }
                            return Some(ValueType::Void);
                        }

                        if name == "sleep" || name == "usleep" {
                            if args.len() != 1 {
                                self.diagnostics.error(
                                    Some(*span),
                                    format!("Bridge.{} expects exactly one int argument", name),
                                );
                                return None;
                            }
                            let arg_ty = self.analyze_expr(&args[0], locals, right)?;
                            if arg_ty != ValueType::Int {
                                self.diagnostics.error(
                                    Some(args[0].span()),
                                    format!(
                                        "Bridge.{} expects int, got '{}'",
                                        name,
                                        arg_ty.as_str()
                                    ),
                                );
                                return None;
                            }
                            return Some(ValueType::Void);
                        }

                        if name == "system" {
                            for argument in args {
                                let arg_ty = self.analyze_expr(argument, locals, right)?;
                                if arg_ty != ValueType::String {
                                    self.diagnostics.error(
                                        Some(argument.span()),
                                        format!(
                                            "Bridge.system expects only string arguments, got '{}'",
                                            arg_ty.as_str()
                                        ),
                                    );
                                    return None;
                                }
                            }
                            return Some(ValueType::Void);
                        }
                    }

                    let mut argument_types = Vec::with_capacity(args.len());
                    for argument in args {
                        let argument_type = self.analyze_expr(argument, locals, right)?;
                        argument_types.push(argument_type);
                    }

                    if let Some(resolved) = self.modules.resolve_call(module, name, &argument_types)
                    {
                        Some(resolved.return_type)
                    } else {
                        let mut message = format!(
                            "No matching signature found for call {}.{}({})",
                            module,
                            name,
                            argument_types
                                .iter()
                                .map(ValueType::as_str)
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        let expected = self.modules.expected_signatures(module, name);
                        if !expected.is_empty() {
                            message.push_str(&format!(". Available: {}", expected.join("; ")));
                        }
                        self.diagnostics.error(Some(*span), message);
                        None
                    }
                }
            },
        }
    }
}
