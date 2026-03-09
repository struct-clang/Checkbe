impl<'ctx, 'a> CodeGenerator<'ctx, 'a> {
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
            ValueType::Array(inner) if *inner == ValueType::String => self
                .builder
                .build_call(
                    self.array_str_to_string_fn,
                    &[value.value.into_pointer_value().into()],
                    "arr_to_s",
                )
                .map_err(builder_error)?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| "cb_array_str_to_string returned void".to_string())?
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
}
