use std::collections::HashMap;

use super::Infer;
use crate::error::{Result, SoppoError};
use crate::syntax::{BinOp, EnumVariant, Expr, ExprKind, UnaryOp};
use crate::types::Type;
use crate::types::ctx::TypeDefKind;

impl Infer {
    /// Infer the type of an expression
    pub fn infer_expr(&mut self, expr: &Expr) -> Result<Type> {
        match &expr.kind {
            ExprKind::Integer(_) => Ok(Type::simple("int")),

            ExprKind::Float(_) => Ok(Type::simple("float64")),

            ExprKind::String(_) => Ok(Type::simple("string")),

            ExprKind::Bool(_) => Ok(Type::simple("bool")),

            ExprKind::Ident(name) => {
                // Handle blank identifier specially - it accepts any type on assignment
                if name == "_" {
                    return Ok(Type::unit());
                }
                self.lookup_var(name)
                    .ok_or_else(|| SoppoError::UndefinedVariable {
                        name: name.clone(),
                        span: expr.span,
                    })
            }

            ExprKind::Binary { op, left, right } => {
                let left_ty = self.infer_expr(left)?;
                let right_ty = self.infer_expr(right)?;

                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                        // Arithmetic: try normal unification first
                        // Point error at right operand since left is typically the "expected" type
                        if self.unify(&left_ty, &right_ty, &right.span).is_ok() {
                            return Ok(self.substitute(left_ty));
                        }

                        // If unification failed, check if we have a defined type with numeric
                        // underlying type on one side and a compatible numeric on the other.
                        // In Go: `time.Duration * int` is allowed because Duration's underlying
                        // type is int64.
                        let left_ty_sub = self.substitute(left_ty.clone());
                        let right_ty_sub = self.substitute(right_ty.clone());

                        // Try to check if types are compatible via underlying type
                        if let Some(result_ty) =
                            self.check_numeric_underlying_compatibility(&left_ty_sub, &right_ty_sub)
                        {
                            return Ok(result_ty);
                        }

                        // Neither worked - return the original unification error
                        // Point to right operand as the "found" type
                        self.unify(&left_ty, &right_ty, &right.span)?;
                        Ok(self.substitute(left_ty))
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        // Comparison: both must be same type, result is bool
                        self.unify(&left_ty, &right_ty, &expr.span)?;
                        Ok(Type::simple("bool"))
                    }
                    BinOp::And | BinOp::Or => {
                        // Logical: both must be bool, result is bool
                        self.unify(&left_ty, &Type::simple("bool"), &left.span)?;
                        self.unify(&right_ty, &Type::simple("bool"), &right.span)?;
                        Ok(Type::simple("bool"))
                    }
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                        // Bitwise: both must be integer types, result is same type
                        // For shifts, right side must be integer but can differ from left
                        if matches!(op, BinOp::Shl | BinOp::Shr) {
                            // Shift: left type is preserved, right must be integer
                            // We don't strictly check right is integer here since Go allows it
                            Ok(self.substitute(left_ty))
                        } else {
                            // Bitwise AND/OR/XOR: operands must be same type
                            self.unify(&left_ty, &right_ty, &right.span)?;
                            Ok(self.substitute(left_ty))
                        }
                    }
                }
            }

            ExprKind::Call {
                func,
                type_args,
                args,
            } => {
                // Handle built-in make(type, ...), new(type), close(ch)
                if let ExprKind::Ident(name) = &func.kind {
                    // close(channel) - closes a channel, returns unit
                    if name == "close" && args.len() == 1 {
                        let channel_ty = self.infer_expr(&args[0])?;
                        let channel_ty = self.substitute(channel_ty);
                        // Verify it's a channel type
                        if let Type::Con { name, .. } = &channel_ty
                            && !name.name.starts_with("chan ")
                        {
                            return Err(SoppoError::Type {
                                message: format!(
                                    "close requires a channel argument, got {}",
                                    channel_ty
                                ),
                                span: args[0].span,
                            });
                        }
                        return Ok(Type::unit());
                    }

                    if name == "make" && !type_args.is_empty() {
                        // make(type, ...) - returns the type
                        // Validate additional arguments are integers (size, capacity)
                        for arg in args {
                            let arg_ty = self.infer_expr(arg)?;
                            self.unify(&arg_ty, &Type::simple("int"), &arg.span)?;
                        }
                        // Return the type being made (properly resolving type args)
                        let ty = &type_args[0];
                        return Ok(self.resolve_type(ty));
                    }

                    if name == "new" && !type_args.is_empty() {
                        // new(type) - returns *type
                        // Return a pointer to the type
                        let ty = &type_args[0];
                        let inner_ty = self.resolve_type(ty);
                        // Use *{type} naming pattern consistent with UnaryOp::Ref
                        let ptr_name = format!("*{}", inner_ty);
                        return Ok(Type::generic(&ptr_name, vec![inner_ty]));
                    }
                }

                // Check if this is a type conversion: TypeName(value) or pkg.TypeName(value)
                if let ExprKind::Ident(type_name) = &func.kind
                    && self.global_state.has_type(type_name)
                {
                    // This is a type conversion, not a function call
                    // Type conversions take exactly one argument
                    if args.len() != 1 {
                        return Err(SoppoError::Type {
                            message: format!(
                                "Type conversion requires exactly 1 argument, but got {}",
                                args.len()
                            ),
                            span: expr.span,
                        });
                    }

                    // Infer the argument type (we don't need to use it, just check it's valid)
                    self.infer_expr(&args[0])?;

                    // Return the target type
                    return Ok(Type::simple(type_name));
                }

                // Check if this is a call on an imported package: pkg.Func(args) or pkg.Type(value)
                if let ExprKind::Field {
                    expr: pkg_expr,
                    field: name,
                    ..
                } = &func.kind
                    && let ExprKind::Ident(pkg_name) = &pkg_expr.kind
                    && self.is_imported_package(pkg_name)
                {
                    // For Soppo imports, look up the function from GlobalCtxt
                    if self.is_soppo_import(pkg_name) {
                        if let Some(func_ty) = self.lookup_soppo_function(pkg_name, name) {
                            // Found the function - infer args and check against signature
                            let mut arg_tys = Vec::new();
                            for arg in args {
                                arg_tys.push((self.infer_expr(arg)?, arg.span));
                            }

                            // Extract param types and return type from func_ty
                            if let Type::Fun {
                                args: param_tys,
                                ret,
                            } = &func_ty
                            {
                                // Check argument count
                                if arg_tys.len() != param_tys.len() {
                                    return Err(SoppoError::Type {
                                        message: format!(
                                            "Function `{}` has {} arguments, but expected {}",
                                            name,
                                            arg_tys.len(),
                                            param_tys.len()
                                        ),
                                        span: func.span,
                                    });
                                }

                                // Check each argument type
                                for (param_ty, (arg_ty, arg_span)) in
                                    param_tys.iter().zip(arg_tys.iter())
                                {
                                    self.unify(param_ty, arg_ty, arg_span)?;
                                }

                                return Ok(self.substitute(ret.as_ref().clone()));
                            }
                        }

                        // Try type conversion: pkg.Type(value)
                        if let Some(ty) = self.lookup_soppo_type(pkg_name, name) {
                            if args.len() != 1 {
                                return Err(SoppoError::Type {
                                    message: format!(
                                        "Type conversion requires exactly 1 argument, but got {}",
                                        args.len()
                                    ),
                                    span: expr.span,
                                });
                            }
                            self.infer_expr(&args[0])?;
                            return Ok(ty);
                        }

                        // Not found in Soppo module
                        return Err(SoppoError::Type {
                            message: format!("`{}` not found in Soppo module `{}`", name, pkg_name),
                            span: func.span,
                        });
                    }

                    // Look up the type from a Go package
                    if let Some(ty) = self.lookup_go_type(pkg_name, name) {
                        // This is a type conversion
                        if args.len() != 1 {
                            return Err(SoppoError::Type {
                                message: format!(
                                    "Type conversion requires exactly 1 argument, but got {}",
                                    args.len()
                                ),
                                span: expr.span,
                            });
                        }

                        // Infer the argument type (we don't need to use it, just check it's valid)
                        self.infer_expr(&args[0])?;

                        // Return the target type
                        return Ok(ty);
                    }
                }

                // Regular function call
                let func_ty = self.infer_expr(func)?;
                let func_ty = self.substitute(func_ty);

                // Infer argument types with their spans
                let mut arg_tys = Vec::new();
                for arg in args {
                    arg_tys.push((self.infer_expr(arg)?, arg.span));
                }

                // Check function call with detailed error spans
                match &func_ty {
                    Type::Fun {
                        args: param_tys,
                        ret,
                    } => {
                        // Check if last param is variadic
                        let has_variadic = param_tys.last().is_some_and(|last| {
                            matches!(last, Type::Con { name, .. } if name.name == "variadic")
                        });

                        if has_variadic {
                            let fixed_params = &param_tys[..param_tys.len() - 1];
                            let variadic_param = param_tys.last().expect("checked above");
                            let variadic_elem = if let Type::Con { args, .. } = variadic_param {
                                args.first().cloned().unwrap_or(Type::simple("any"))
                            } else {
                                Type::simple("any")
                            };

                            // Check we have at least the fixed params
                            if arg_tys.len() < fixed_params.len() {
                                return Err(SoppoError::Type {
                                    message: format!(
                                        "Function has {} arguments, but expected at least {}",
                                        arg_tys.len(),
                                        fixed_params.len()
                                    ),
                                    span: func.span,
                                });
                            }

                            // Check fixed params
                            for (param_ty, (arg_ty, arg_span)) in
                                fixed_params.iter().zip(arg_tys.iter())
                            {
                                self.unify(param_ty, arg_ty, arg_span)?;
                            }

                            // Check variadic args
                            for (arg_ty, arg_span) in arg_tys.iter().skip(fixed_params.len()) {
                                if variadic_elem != Type::simple("any") {
                                    self.unify(&variadic_elem, arg_ty, arg_span)?;
                                }
                            }
                        } else {
                            // Non-variadic: exact arg count required
                            if arg_tys.len() != param_tys.len() {
                                return Err(SoppoError::Type {
                                    message: format!(
                                        "Function has {} arguments, but expected {}",
                                        arg_tys.len(),
                                        param_tys.len()
                                    ),
                                    span: func.span,
                                });
                            }

                            // Check each argument type
                            for (param_ty, (arg_ty, arg_span)) in
                                param_tys.iter().zip(arg_tys.iter())
                            {
                                self.unify(param_ty, arg_ty, arg_span)?;
                            }
                        }

                        Ok(self.substitute(ret.as_ref().clone()))
                    }
                    Type::Var(_) => {
                        // Function type is unknown, use standard unification
                        let result_ty = self.fresh_ty_var();
                        let arg_types: Vec<Type> = arg_tys.into_iter().map(|(ty, _)| ty).collect();
                        let expected_func_ty = Type::fun(arg_types, result_ty.clone());
                        self.unify(&func_ty, &expected_func_ty, &expr.span)?;
                        Ok(self.substitute(result_ty))
                    }
                    _ => Err(SoppoError::Type {
                        message: format!("Cannot call non-function type `{}`", func_ty),
                        span: func.span,
                    }),
                }
            }

            ExprKind::Field {
                expr: field_expr,
                field,
                field_span,
            } => {
                // Check if this is accessing something from an imported package
                // e.g., fmt.Println, strings.HasPrefix, or helpers.Add (sop: import)
                if let ExprKind::Ident(name) = &field_expr.kind
                    && self.is_imported_package(name)
                {
                    // For Soppo imports, look up from GlobalCtxt
                    if self.is_soppo_import(name) {
                        // Try to look up as a function first
                        if let Some(func_ty) = self.lookup_soppo_function(name, field) {
                            return Ok(func_ty);
                        }
                        // Try to look up as a type
                        if let Some(ty) = self.lookup_soppo_type(name, field) {
                            return Ok(ty);
                        }
                        // Try to look up as a constant
                        if let Some(ty) = self.lookup_soppo_constant(name, field) {
                            return Ok(ty);
                        }
                        // Not found
                        return Err(SoppoError::Type {
                            message: format!("`{}` not found in Soppo module `{}`", field, name),
                            span: *field_span,
                        });
                    }

                    // Go packages: try to look up as a function first
                    if let Some(func_ty) = self.lookup_go_function(name, field) {
                        return Ok(func_ty);
                    }
                    // Try to look up as a type or constant
                    if let Some(ty) = self.lookup_go_type(name, field) {
                        return Ok(ty);
                    }
                    // Couldn't find it - error
                    return Err(SoppoError::Type {
                        message: format!("`{}` not found in package `{}`", field, name),
                        span: *field_span,
                    });
                }

                // Check if this is an enum constructor like Colour.Red or Result.Ok
                if let ExprKind::Ident(type_name) = &field_expr.kind {
                    // Check if type_name is a registered type
                    if let Some(type_def) = self.global_state.lookup_type(type_name).cloned() {
                        // Check if this is an enum variant
                        if let TypeDefKind::Enum { variants } = &type_def.kind {
                            // Create fresh type variables for generic params
                            let generic_subst: HashMap<String, Type> = type_def
                                .generics
                                .iter()
                                .map(|g| (g.clone(), self.fresh_ty_var()))
                                .collect();

                            // Find the variant
                            for variant in variants {
                                let variant_name = match variant {
                                    EnumVariant::Unit { name, .. } => name,
                                    EnumVariant::Single { name, .. } => name,
                                    EnumVariant::Struct { name, .. } => name,
                                };

                                if variant_name == field {
                                    // Found the variant
                                    return match variant {
                                        EnumVariant::Unit { .. } => {
                                            // Unit variant: just returns the enum type
                                            Ok(Type::simple(type_name))
                                        }
                                        EnumVariant::Single { ty, .. } => {
                                            // Single variant: returns a constructor function
                                            // Ok(T) -> fn(T) -> Result[T, E]
                                            // Instantiate generic params with fresh type vars
                                            let param_ty =
                                                self.instantiate_type(&ty.name, &generic_subst);
                                            let return_ty = Type::simple(type_name);
                                            Ok(Type::fun(vec![param_ty], return_ty))
                                        }
                                        EnumVariant::Struct { fields, .. } => {
                                            // Struct variant: returns a constructor function
                                            // taking all fields as parameters
                                            let param_tys: Vec<Type> = fields
                                                .iter()
                                                .map(|f| {
                                                    self.instantiate_type(
                                                        &f.ty.name,
                                                        &generic_subst,
                                                    )
                                                })
                                                .collect();
                                            let return_ty = Type::simple(type_name);
                                            Ok(Type::fun(param_tys, return_ty))
                                        }
                                    };
                                }
                            }
                        }
                        // Not an enum, but still a type - might be for other purposes
                        return Ok(Type::simple(type_name));
                    }
                }

                // Otherwise it's a regular field access
                let expr_ty = self.infer_expr(field_expr)?;
                let expr_ty = self.substitute(expr_ty);

                // Handle built-in error type's Error() method
                if let Type::Con { name, .. } = &expr_ty
                    && name.name == "error"
                    && field == "Error"
                {
                    // error.Error() returns string
                    return Ok(Type::fun(vec![], Type::simple("string")));
                }

                // Look up the struct type to validate field access
                if let Type::Con { name, .. } = &expr_ty
                    && let Some(type_def) = self.global_state.lookup_type(&name.name)
                    && let TypeDefKind::Struct { fields } = &type_def.kind
                {
                    // Check if the field exists
                    if let Some((_, field_ty)) = fields.iter().find(|(f, _)| f == field) {
                        return Ok(field_ty.clone());
                    } else {
                        // Field not found - check if it might be a method
                        // If we can find a function with this name, return a type variable
                        // and let the Call handler deal with it
                        if self.global_state.lookup_function(field).is_some() {
                            return Ok(self.fresh_ty_var());
                        }

                        return Err(SoppoError::Type {
                            message: format!(
                                "Struct `{}` has no field named `{}`",
                                name.name, field
                            ),
                            span: field_expr.span,
                        });
                    }
                }

                // If we can't determine the struct type, return a type variable
                // (this allows field access on generic/unknown types)
                Ok(self.fresh_ty_var())
            }

            ExprKind::Index { expr, index } => {
                let container_ty = self.infer_expr(expr)?;
                let container_ty = self.substitute(container_ty);
                let index_ty = self.infer_expr(index)?;

                if let Type::Con { name, args } = &container_ty {
                    // Map indexing: map[K]V - index is K, result is V
                    if (name.name == "map" || name.name.starts_with("map[")) && args.len() == 2 {
                        self.unify(&index_ty, &args[0], &index.span)?;
                        return Ok(args[1].clone());
                    }

                    // Slice indexing: []T - index is int, result is T
                    if name.name.starts_with("[]") {
                        self.unify(&index_ty, &Type::simple("int"), &index.span)?;
                        if args.len() == 1 {
                            return Ok(args[0].clone());
                        }
                        let elem_name = &name.name[2..];
                        return Ok(Type::simple(elem_name));
                    }

                    // Array indexing: array or [N]T - index is int
                    if name.name == "array" && args.len() == 1 {
                        self.unify(&index_ty, &Type::simple("int"), &index.span)?;
                        return Ok(args[0].clone());
                    }

                    // String indexing - index is int, result is byte
                    if name.name == "string" {
                        self.unify(&index_ty, &Type::simple("int"), &index.span)?;
                        return Ok(Type::simple("byte"));
                    }
                }

                // Default: assume int index
                self.unify(&index_ty, &Type::simple("int"), &index.span)?;
                Ok(self.fresh_ty_var())
            }

            ExprKind::ArrayLit { ty, elements } => {
                // Infer element type from the declared type or first element
                let (elem_ty, declared_type) = if let Some(ty) = ty {
                    // Extract element type from []T or T
                    if ty.name.starts_with("[]") {
                        let elem_name = &ty.name[2..];
                        // Return the resolved type to match how return types are handled
                        (Type::simple(elem_name), Some(self.resolve_type(ty)))
                    } else {
                        (Type::simple(&ty.name), None)
                    }
                } else if !elements.is_empty() {
                    (self.infer_expr(&elements[0])?, None)
                } else {
                    (self.fresh_ty_var(), None)
                };

                // All elements must have the same type
                for elem in elements {
                    let elem_ty_actual = self.infer_expr(elem)?;
                    self.unify(&elem_ty, &elem_ty_actual, &elem.span)?;
                }

                // Return proper slice/array type
                if let Some(declared_ty) = declared_type {
                    Ok(declared_ty)
                } else {
                    Ok(Type::array(elem_ty))
                }
            }

            ExprKind::StructLit { ty, fields } => {
                // Type check each field
                for (_field_name, value) in fields {
                    self.infer_expr(value)?;
                }

                // Check if this is an enum variant (e.g., Shape.Circle)
                // If so, return the enum type, not the variant
                if ty.name.contains('.') {
                    let parts: Vec<&str> = ty.name.split('.').collect();
                    if parts.len() == 2 {
                        let enum_name = parts[0];
                        return Ok(Type::simple(enum_name));
                    }
                }

                // Return the struct type
                Ok(Type::simple(&ty.name))
            }

            ExprKind::MapLit { ty, entries } => {
                // Extract key and value types from map[K]V
                let (key_ty, val_ty) = if ty.args.len() == 2 {
                    (
                        Type::simple(&ty.args[0].name),
                        Type::simple(&ty.args[1].name),
                    )
                } else {
                    // Fallback: infer from first entry
                    if let Some((k, v)) = entries.first() {
                        (self.infer_expr(k)?, self.infer_expr(v)?)
                    } else {
                        (self.fresh_ty_var(), self.fresh_ty_var())
                    }
                };

                // Type check all entries
                for (key, value) in entries {
                    let k_ty = self.infer_expr(key)?;
                    let v_ty = self.infer_expr(value)?;
                    self.unify(&key_ty, &k_ty, &key.span)?;
                    self.unify(&val_ty, &v_ty, &value.span)?;
                }

                // Return map[K]V type
                Ok(Type::generic("map", vec![key_ty, val_ty]))
            }

            ExprKind::Unary { op, operand } => {
                let operand_ty = self.infer_expr(operand)?;

                match op {
                    UnaryOp::Neg => {
                        // -x: operand must be numeric, result is same type
                        // We allow any numeric type here
                        Ok(operand_ty)
                    }
                    UnaryOp::Not => {
                        // !x: operand must be bool, result is bool
                        self.unify(&operand_ty, &Type::simple("bool"), &operand.span)?;
                        Ok(Type::simple("bool"))
                    }
                    UnaryOp::Ref => {
                        // &x: result is *T where T is the operand type
                        let operand_ty = self.substitute(operand_ty);
                        let ptr_name = format!("*{}", operand_ty);
                        Ok(Type::generic(&ptr_name, vec![operand_ty]))
                    }
                    UnaryOp::Deref => {
                        // *p: operand must be *T, result is T
                        let operand_ty = self.substitute(operand_ty);
                        // Extract the pointee type from *T
                        if let Type::Con { name, args } = &operand_ty {
                            if name.name.starts_with('*') && args.len() == 1 {
                                return Ok(args[0].clone());
                            }
                            // Also handle case where type name encodes the pointee
                            if name.name.starts_with('*') {
                                let pointee_name = &name.name[1..];
                                return Ok(Type::simple(pointee_name));
                            }
                        }
                        // If we can't determine the pointer type, return a type variable
                        Ok(self.fresh_ty_var())
                    }
                    UnaryOp::Recv => {
                        // <-ch: operand must be chan T, result is T
                        let operand_ty = self.substitute(operand_ty);
                        // Extract the element type from chan T
                        if let Type::Con { name, args } = &operand_ty {
                            // Handle "chan T" type with args
                            if name.name.starts_with("chan ") && args.len() == 1 {
                                return Ok(args[0].clone());
                            }
                            // Also handle case where type name encodes the element type
                            if name.name.starts_with("chan ") {
                                let elem_name = &name.name[5..]; // skip "chan "
                                return Ok(Type::simple(elem_name));
                            }
                        }
                        // If we can't determine the channel type, return a type variable
                        Ok(self.fresh_ty_var())
                    }
                }
            }

            ExprKind::FuncLit {
                params,
                return_types,
                body,
            } => {
                // Save the current expected return types
                let prev_expected = self.expected_return_types.take();

                // Create a new scope for the function body
                self.push_scope();

                // Add parameters to scope
                for param in params {
                    let param_ty = Type::simple(&param.ty.name);
                    self.insert_var(param.name.clone(), param_ty);
                }

                // Set expected return types for this function
                let expected_ret_types: Vec<Type> =
                    return_types.iter().map(|t| Type::simple(&t.name)).collect();
                if !expected_ret_types.is_empty() {
                    self.expected_return_types = Some(expected_ret_types.clone());
                }

                // Infer body
                self.infer_block(body)?;

                self.pop_scope();

                // Restore previous expected return types
                self.expected_return_types = prev_expected;

                // Build function type
                let param_types: Vec<Type> =
                    params.iter().map(|p| Type::simple(&p.ty.name)).collect();
                let ret_ty = if return_types.is_empty() {
                    Type::unit()
                } else if return_types.len() == 1 {
                    Type::simple(&return_types[0].name)
                } else {
                    // Multiple return types - use a tuple type
                    let ret_types: Vec<Type> =
                        return_types.iter().map(|t| Type::simple(&t.name)).collect();
                    Type::generic("tuple", ret_types)
                };

                Ok(Type::fun(param_types, ret_ty))
            }

            ExprKind::Block(block) => self.infer_block(block),

            ExprKind::Slice {
                expr,
                low,
                high,
                cap,
            } => {
                // Slicing returns the same type as the sliced expression
                let expr_ty = self.infer_expr(expr)?;

                // Check that indices are integers
                if let Some(l) = low {
                    let l_ty = self.infer_expr(l)?;
                    self.unify(&l_ty, &Type::simple("int"), &l.span)?;
                }
                if let Some(h) = high {
                    let h_ty = self.infer_expr(h)?;
                    self.unify(&h_ty, &Type::simple("int"), &h.span)?;
                }
                if let Some(c) = cap {
                    let c_ty = self.infer_expr(c)?;
                    self.unify(&c_ty, &Type::simple("int"), &c.span)?;
                }

                Ok(expr_ty)
            }

            ExprKind::TypeAssert { expr, ty } => {
                // Type assertion: x.(Type) - infer expression and return the asserted type
                self.infer_expr(expr)?;
                Ok(Type::simple(&ty.name))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result;
    use crate::syntax::{FileId, Parser};

    fn infer_expr_helper(source: &str) -> Result<Type> {
        let mut parser = Parser::new(source, FileId(0));
        let expr = parser.parse_expr()?;
        let mut infer = Infer::new().expect("Failed to create Infer");
        let ty = infer.infer_expr(&expr)?;
        Ok(infer.substitute(ty))
    }

    #[test]
    fn test_infer_integer() {
        let ty = infer_expr_helper("42").unwrap();
        assert_eq!(ty, Type::simple("int"));
    }

    #[test]
    fn test_infer_string() {
        let ty = infer_expr_helper(r#""hello""#).unwrap();
        assert_eq!(ty, Type::simple("string"));
    }

    #[test]
    fn test_infer_bool() {
        let ty = infer_expr_helper("true").unwrap();
        assert_eq!(ty, Type::simple("bool"));
    }

    #[test]
    fn test_infer_arithmetic() {
        let ty = infer_expr_helper("1 + 2 * 3").unwrap();
        assert_eq!(ty, Type::simple("int"));
    }

    #[test]
    fn test_infer_comparison() {
        let ty = infer_expr_helper("1 < 2").unwrap();
        assert_eq!(ty, Type::simple("bool"));
    }

    #[test]
    fn test_infer_complex_expr() {
        let ty = infer_expr_helper("(1 + 2) * 3 - 4 / 2").unwrap();
        assert_eq!(ty, Type::simple("int"));
    }

    #[test]
    fn test_type_error_arithmetic() {
        let result = infer_expr_helper(r#"1 + "hello""#);
        assert!(result.is_err());
    }

    #[test]
    fn test_array_literal_type() {
        // Test that array literals have proper array type
        let source = "[5]int{1, 2, 3, 4, 5}";
        let mut parser = Parser::new(source, FileId(0));
        let expr = parser.parse_expr().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");
        let ty = infer.infer_expr(&expr).unwrap();

        // Should be array[int]
        if let Type::Con { name, args } = ty {
            assert_eq!(name.name, "array");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0], Type::simple("int"));
        } else {
            panic!("Expected array type, got: {:?}", ty);
        }
    }
}
