use std::collections::HashMap;

use super::Infer;
use crate::error::{Result, SoppoError};
use crate::syntax::{BinOp, EnumVariant, Expr, ExprKind, ModuleId, Span, Symbol, UnaryOp};
use crate::types::Type;
use crate::types::ctx::TypeDefKind;
use crate::types::ty::Nullability;

impl Infer {
    /// Infer the type of an expression
    pub fn infer_expr(&mut self, expr: &Expr) -> Result<Type> {
        match &expr.kind {
            ExprKind::Integer(_) => Ok(Type::simple("int")),

            ExprKind::Float(_) => Ok(Type::simple("float64")),

            ExprKind::String(_) => Ok(Type::simple("string")),

            ExprKind::Rune(_) => Ok(Type::simple("rune")),

            ExprKind::StringInterpolation(parts) => {
                // Type check each interpolated expression
                for part in parts {
                    if let crate::syntax::StringPart::Expr(expr) = part {
                        // Any type can be interpolated - it will be converted to string
                        self.infer_expr(expr)?;
                    }
                }
                Ok(Type::simple("string"))
            }

            ExprKind::Bool(_) => Ok(Type::simple("bool")),

            // nil is a special value that can be any pointer, interface, slice, map, channel, or function type
            // We use a fresh type variable so it unifies with the expected type
            ExprKind::Nil => Ok(self.fresh_ty_var()),

            ExprKind::Ident(name) => {
                // Handle blank identifier specially - it accepts any type on assignment
                if name == "_" {
                    return Ok(Type::unit());
                }
                // Handle iota - a Go builtin constant generator for const blocks
                if name == "iota" {
                    return Ok(Type::simple("int"));
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
            } => self.infer_call(func, type_args, args, &expr.span),

            ExprKind::Field {
                expr: field_expr,
                field,
                field_span,
            } => self.infer_field(field_expr, field, field_span),

            ExprKind::Index { expr, index } => {
                let container_ty = self.infer_expr(expr)?;
                let container_ty = self.substitute(container_ty);
                let index_ty = self.infer_expr(index)?;

                if let Type::Con { name, args, .. } = &container_ty {
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

                // Check if this is a qualified type (e.g., pkg.Type)
                if ty.name.contains('.') {
                    let parts: Vec<&str> = ty.name.split('.').collect();
                    if parts.len() == 2 {
                        let pkg_name = parts[0];
                        let type_name = parts[1];

                        // Check if it's an enum variant (e.g., Shape.Circle)
                        if self.global_state.is_local_enum(pkg_name) {
                            return Ok(Type::simple(pkg_name));
                        }

                        // Check if it's a cross-package enum variant
                        if self.global_state.is_soppo_enum(pkg_name, type_name) {
                            return Ok(Type::Con {
                                name: Symbol {
                                    module: ModuleId::new(pkg_name),
                                    name: type_name.to_string(),
                                    span: Span::dummy(),
                                },
                                args: vec![],
                                nullable: false,
                            });
                        }

                        // Return qualified type with module info (for Go packages)
                        return Ok(Type::Con {
                            name: Symbol {
                                module: ModuleId::new(pkg_name),
                                name: type_name.to_string(),
                                span: Span::dummy(),
                            },
                            args: vec![],
                            nullable: false,
                        });
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

                // Return map[K]V type with proper Go format in name
                let map_name = format!("map[{}]{}", key_ty, val_ty);
                Ok(Type::generic(&map_name, vec![key_ty, val_ty]))
            }

            ExprKind::Unary { op, operand } => self.infer_unary(op, operand),

            ExprKind::FuncLit {
                params,
                return_types,
                body,
            } => {
                // Save the current expected return types
                let prev_expected = self.expected_return_types.take();

                // Create a new scope for the function body
                self.push_scope();

                // Add parameters to scope - use resolve_type for proper qualified type handling
                for param in params {
                    let param_ty = self.resolve_type(&param.ty);
                    self.insert_var(param.name.clone(), param_ty);
                }

                // Set expected return types for this function
                let expected_ret_types: Vec<Type> =
                    return_types.iter().map(|t| self.resolve_type(t)).collect();
                if !expected_ret_types.is_empty() {
                    self.expected_return_types = Some(expected_ret_types.clone());
                }

                // Infer body
                self.infer_block(body)?;

                self.pop_scope();

                // Restore previous expected return types
                self.expected_return_types = prev_expected;

                // Build function type - use resolve_type for proper qualified type handling
                let param_types: Vec<Type> =
                    params.iter().map(|p| self.resolve_type(&p.ty)).collect();
                let ret_ty = if return_types.is_empty() {
                    Type::unit()
                } else if return_types.len() == 1 {
                    self.resolve_type(&return_types[0])
                } else {
                    // Multiple return types - use a tuple type
                    let ret_types: Vec<Type> =
                        return_types.iter().map(|t| self.resolve_type(t)).collect();
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
                // Type assertion: x.(Type) - returns a pointer to the asserted type
                // The pointer is nil if the assertion fails, non-nil if it succeeds
                // This enables: if x := val.(Type); x != nil { use(x) }
                self.infer_expr(expr)?;
                let inner_ty = Type::simple(&ty.name.replace('.', "_"));
                Ok(Type::ptr(inner_ty))
            }

            ExprKind::NilAssert { expr } => {
                // Nil assertion: x.(!nil) - assert the expression is non-nil
                let ty = self.infer_expr(expr)?;
                let ty = self.substitute(ty);

                // If this is a nilable type with an identifier, mark it as non-nil
                if Self::is_nilable_type(&ty)
                    && let ExprKind::Ident(name) = &expr.kind
                {
                    self.set_nil_state(name.clone(), Nullability::NonNull);
                }

                // Return the non-nullable version of the type
                // x.(!nil) converts ?*T -> *T, ?[]T -> []T, etc.
                Ok(ty.as_non_nullable())
            }
        }
    }

    /// Infer the type of a field access expression
    fn infer_field(&mut self, field_expr: &Expr, field: &str, field_span: &Span) -> Result<Type> {
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
                                    let param_ty = self.instantiate_type(&ty.name, &generic_subst);
                                    let return_ty = Type::simple(type_name);
                                    Ok(Type::fun(vec![param_ty], return_ty))
                                }
                                EnumVariant::Struct { fields, .. } => {
                                    // Struct variant: returns a constructor function
                                    // taking all fields as parameters
                                    let param_tys: Vec<Type> = fields
                                        .iter()
                                        .map(|f| self.instantiate_type(&f.ty.name, &generic_subst))
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

        // Check for nil dereference on field access
        // If the expression is a nilable type, verify it's not nullable
        // Skip check if expression is a NilAssert - that explicitly makes it non-null
        if Self::is_nilable_type(&expr_ty) && !matches!(field_expr.kind, ExprKind::NilAssert { .. })
        {
            // Convert expression to a trackable key (supports identifiers and field chains)
            let expr_key = super::stmt::expr_to_key(field_expr);

            // Check nil state for the expression, or assume nullable for complex expressions
            let is_nullable = match &expr_key {
                Some(key) => self.get_nil_state(key) == Nullability::Nullable,
                None => true, // Complex expressions are conservatively nullable
            };

            if is_nullable {
                let name_for_error = expr_key.unwrap_or_else(|| "expression".to_string());
                return Err(SoppoError::NilPointer {
                    name: name_for_error,
                    span: field_expr.span,
                });
            }
        }

        // Handle built-in error type's Error() method
        if let Type::Con { name, .. } = &expr_ty
            && name.name == "error"
            && field == "Error"
        {
            // error.Error() returns string
            return Ok(Type::fun(vec![], Type::simple("string")));
        }

        // Look up the struct type to validate field access
        // For pointer types like *User, extract the inner type name (User)
        // Also extract the module name if present (for Go package types)
        let (struct_name, module_name): (Option<String>, Option<String>) =
            if let Type::Con { name, args, .. } = &expr_ty {
                if name.name.starts_with('*') && args.len() == 1 {
                    // Pointer type: extract inner type name from args or strip prefix
                    if let Type::Con { name: inner, .. } = &args[0] {
                        let mod_name = if inner.module.0.is_empty() {
                            None
                        } else {
                            Some(inner.module.0.clone())
                        };
                        (Some(inner.name.clone()), mod_name)
                    } else {
                        (Some(name.name[1..].to_string()), None)
                    }
                } else {
                    let mod_name = if name.module.0.is_empty() {
                        None
                    } else {
                        Some(name.module.0.clone())
                    };
                    (Some(name.name.clone()), mod_name)
                }
            } else {
                (None, None)
            };

        // Check if this is a field access on a Go package type
        if let (Some(struct_name), Some(module_name)) = (&struct_name, &module_name)
            && let Some(field_ty) = self.lookup_go_struct_field(module_name, struct_name, field)
        {
            return Ok(field_ty);
        }

        // Check if this is a method call on a Go package type
        if let (Some(struct_name), Some(module_name)) = (&struct_name, &module_name)
            && let Some(method_ty) = self.lookup_go_method(module_name, struct_name, field)
        {
            return Ok(method_ty);
        }

        if let Some(struct_name) = &struct_name {
            // Check if this is an enum variant type (EnumName.VariantName)
            if let Some(dot_idx) = struct_name.find('.') {
                let enum_name = &struct_name[..dot_idx];
                let variant_name = &struct_name[dot_idx + 1..];

                if let Some(type_def) = self.global_state.lookup_type(enum_name)
                    && let TypeDefKind::Enum { variants } = &type_def.kind
                {
                    // Find the variant
                    for variant in variants {
                        let (v_name, v_fields) = match variant {
                            EnumVariant::Unit { name, .. } => (name.as_str(), None),
                            EnumVariant::Single { name, ty, .. } => {
                                // Single variants have a "Value" field
                                (
                                    name.as_str(),
                                    Some(vec![("Value".to_string(), Type::from_ast(ty))]),
                                )
                            }
                            EnumVariant::Struct { name, fields, .. } => {
                                let fs: Vec<_> = fields
                                    .iter()
                                    .map(|f| (f.name.clone(), Type::from_ast(&f.ty)))
                                    .collect();
                                (name.as_str(), Some(fs))
                            }
                        };

                        if v_name == variant_name {
                            if let Some(fields) = v_fields
                                && let Some((_, field_ty)) = fields.iter().find(|(f, _)| f == field)
                            {
                                return Ok(field_ty.clone());
                            }
                            return Err(SoppoError::Type {
                                message: format!(
                                    "Enum variant `{}` has no field named `{}`",
                                    struct_name, field
                                ),
                                span: field_expr.span,
                            });
                        }
                    }
                }
            }

            // Regular struct lookup
            if let Some(type_def) = self.global_state.lookup_type(struct_name)
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

                    // Field not found in struct - check if it's a method
                    if let Some(method) = self.global_state.lookup_method(struct_name, field) {
                        // Build function type from method signature
                        let param_tys: Vec<Type> =
                            method.params.iter().map(|(_, ty)| ty.clone()).collect();
                        let ret_ty = match method.return_types.len() {
                            0 => Type::unit(),
                            1 => method.return_types[0].clone(),
                            _ => Type::generic("tuple", method.return_types.clone()),
                        };
                        return Ok(Type::fun(param_tys, ret_ty));
                    }

                    return Err(SoppoError::Type {
                        message: format!("Struct `{}` has no field named `{}`", struct_name, field),
                        span: field_expr.span,
                    });
                }
            }

            // If no TypeDef found, still check for methods on this type
            if let Some(method) = self.global_state.lookup_method(struct_name, field) {
                let param_tys: Vec<Type> = method.params.iter().map(|(_, ty)| ty.clone()).collect();
                let ret_ty = match method.return_types.len() {
                    0 => Type::unit(),
                    1 => method.return_types[0].clone(),
                    _ => Type::generic("tuple", method.return_types.clone()),
                };
                return Ok(Type::fun(param_tys, ret_ty));
            }
        }

        // If we can't determine the struct type, return a type variable
        // (this allows field access on generic/unknown types)
        Ok(self.fresh_ty_var())
    }

    /// Infer the type of a function call expression
    fn infer_call(
        &mut self,
        func: &Expr,
        type_args: &[crate::syntax::Type],
        args: &[(Option<(String, Span)>, Expr)],
        expr_span: &Span,
    ) -> Result<Type> {
        // Handle Go built-in functions
        if let ExprKind::Ident(name) = &func.kind {
            // close(channel) - closes a channel, returns unit
            if name == "close" && args.len() == 1 {
                let channel_ty = self.infer_expr(&args[0].1)?;
                let channel_ty = self.substitute(channel_ty);
                // Verify it's a channel type
                if let Type::Con { name, .. } = &channel_ty
                    && !name.name.starts_with("chan ")
                {
                    return Err(SoppoError::Type {
                        message: format!("close requires a channel argument, got {}", channel_ty),
                        span: args[0].1.span,
                    });
                }
                return Ok(Type::unit());
            }

            if name == "make" && !type_args.is_empty() {
                // make(type, ...) - returns the type
                // Validate additional arguments are integers (size, capacity)
                for (_, arg) in args {
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

            // len(v) - returns length of array, slice, string, map, or channel
            if name == "len" && args.len() == 1 {
                let arg_ty = self.infer_expr(&args[0].1)?;
                let arg_ty = self.substitute(arg_ty);
                // Verify it's a valid type for len
                let valid = match &arg_ty {
                    Type::Con { name, .. } => {
                        name.name == "string"
                            || name.name == "array" // array[T] representation
                            || name.name.starts_with("[]")
                            || name.name.starts_with("[")
                            || name.name.starts_with("map") // map[K]V or map[K, V]
                            || name.name.starts_with("chan ")
                    }
                    _ => false,
                };
                if !valid {
                    return Err(SoppoError::Type {
                        message: format!(
                            "len requires array, slice, string, map, or channel; got {}",
                            arg_ty
                        ),
                        span: args[0].1.span,
                    });
                }
                return Ok(Type::simple("int"));
            }

            // cap(v) - returns capacity of slice or channel
            if name == "cap" && args.len() == 1 {
                let arg_ty = self.infer_expr(&args[0].1)?;
                let arg_ty = self.substitute(arg_ty);
                // Verify it's a valid type for cap
                let valid = match &arg_ty {
                    Type::Con { name, .. } => {
                        name.name == "array" // array[T] representation
                            || name.name.starts_with("[]")
                            || name.name.starts_with("[")
                            || name.name.starts_with("chan ")
                    }
                    _ => false,
                };
                if !valid {
                    return Err(SoppoError::Type {
                        message: format!("cap requires array, slice, or channel; got {}", arg_ty),
                        span: args[0].1.span,
                    });
                }
                return Ok(Type::simple("int"));
            }

            // append(slice, elems...) - returns the same slice type
            if name == "append" && !args.is_empty() {
                let slice_ty = self.infer_expr(&args[0].1)?;
                let slice_ty = self.substitute(slice_ty);
                // Verify first arg is a slice
                let elem_ty = match &slice_ty {
                    Type::Con { name, args, .. } if name.name.starts_with("[]") => {
                        if args.is_empty() {
                            // Extract element type from name like "[]int"
                            Type::simple(&name.name[2..])
                        } else {
                            args[0].clone()
                        }
                    }
                    _ => {
                        return Err(SoppoError::Type {
                            message: format!(
                                "first argument to append must be a slice; got {}",
                                slice_ty
                            ),
                            span: args[0].1.span,
                        });
                    }
                };
                // Type check remaining arguments against element type
                for (_, arg) in args.iter().skip(1) {
                    let arg_ty = self.infer_expr(arg)?;
                    self.unify(&elem_ty, &arg_ty, &arg.span)?;
                }
                return Ok(slice_ty);
            }

            // copy(dst, src) - returns int (number of elements copied)
            if name == "copy" && args.len() == 2 {
                let dst_ty = self.infer_expr(&args[0].1)?;
                let dst_ty = self.substitute(dst_ty);
                let src_ty = self.infer_expr(&args[1].1)?;
                let src_ty = self.substitute(src_ty);
                // Both must be slices (or src can be string for []byte)
                let dst_is_slice =
                    matches!(&dst_ty, Type::Con { name, .. } if name.name.starts_with("[]"));
                let src_is_slice =
                    matches!(&src_ty, Type::Con { name, .. } if name.name.starts_with("[]"));
                let src_is_string =
                    matches!(&src_ty, Type::Con { name, .. } if name.name == "string");

                if !dst_is_slice {
                    return Err(SoppoError::Type {
                        message: format!("first argument to copy must be a slice; got {}", dst_ty),
                        span: args[0].1.span,
                    });
                }
                if !src_is_slice && !src_is_string {
                    return Err(SoppoError::Type {
                        message: format!(
                            "second argument to copy must be a slice or string; got {}",
                            src_ty
                        ),
                        span: args[1].1.span,
                    });
                }
                // For string source, dst must be []byte
                if src_is_string
                    && let Type::Con { name, .. } = &dst_ty
                    && name.name != "[]byte"
                    && name.name != "[]uint8"
                {
                    return Err(SoppoError::Type {
                        message: format!("cannot copy string to {}; need []byte", dst_ty),
                        span: args[0].1.span,
                    });
                }
                return Ok(Type::simple("int"));
            }

            // delete(map, key) - deletes key from map, returns unit
            if name == "delete" && args.len() == 2 {
                let map_ty = self.infer_expr(&args[0].1)?;
                let map_ty = self.substitute(map_ty);
                // Verify first arg is a map
                let key_ty = match &map_ty {
                    Type::Con { name, args, .. } if name.name.starts_with("map") => {
                        if !args.is_empty() {
                            args[0].clone()
                        } else {
                            // Extract key type from name like "map[string]int"
                            let inner = &name.name[4..]; // skip "map["
                            if let Some(bracket_end) = inner.find(']') {
                                Type::simple(&inner[..bracket_end])
                            } else {
                                Type::simple("any")
                            }
                        }
                    }
                    _ => {
                        return Err(SoppoError::Type {
                            message: format!(
                                "first argument to delete must be a map; got {}",
                                map_ty
                            ),
                            span: args[0].1.span,
                        });
                    }
                };
                // Type check key argument
                let arg_key_ty = self.infer_expr(&args[1].1)?;
                self.unify(&key_ty, &arg_key_ty, &args[1].1.span)?;
                return Ok(Type::unit());
            }

            // panic(v) - panics with value, returns never (diverges)
            if name == "panic" && args.len() == 1 {
                // panic accepts any type
                self.infer_expr(&args[0].1)?;
                return Ok(Type::never());
            }

            // recover() - returns any (interface{})
            if name == "recover" && args.is_empty() {
                return Ok(Type::simple("any"));
            }

            // print and println - variadic, accept any types, return unit
            if name == "print" || name == "println" {
                for (_, arg) in args {
                    self.infer_expr(arg)?;
                }
                return Ok(Type::unit());
            }

            // complex(r, i) - creates complex number from two float64
            if name == "complex" && args.len() == 2 {
                let r_ty = self.infer_expr(&args[0].1)?;
                let i_ty = self.infer_expr(&args[1].1)?;
                self.unify(&r_ty, &Type::simple("float64"), &args[0].1.span)?;
                self.unify(&i_ty, &Type::simple("float64"), &args[1].1.span)?;
                return Ok(Type::simple("complex128"));
            }

            // real(c) - extracts real part of complex number
            if name == "real" && args.len() == 1 {
                let c_ty = self.infer_expr(&args[0].1)?;
                let c_ty = self.substitute(c_ty);
                match &c_ty {
                    Type::Con { name, .. }
                        if name.name == "complex128" || name.name == "complex64" =>
                    {
                        let result = if name.name == "complex128" {
                            "float64"
                        } else {
                            "float32"
                        };
                        return Ok(Type::simple(result));
                    }
                    _ => {
                        return Err(SoppoError::Type {
                            message: format!("real requires complex argument; got {}", c_ty),
                            span: args[0].1.span,
                        });
                    }
                }
            }

            // imag(c) - extracts imaginary part of complex number
            if name == "imag" && args.len() == 1 {
                let c_ty = self.infer_expr(&args[0].1)?;
                let c_ty = self.substitute(c_ty);
                match &c_ty {
                    Type::Con { name, .. }
                        if name.name == "complex128" || name.name == "complex64" =>
                    {
                        let result = if name.name == "complex128" {
                            "float64"
                        } else {
                            "float32"
                        };
                        return Ok(Type::simple(result));
                    }
                    _ => {
                        return Err(SoppoError::Type {
                            message: format!("imag requires complex argument; got {}", c_ty),
                            span: args[0].1.span,
                        });
                    }
                }
            }
        }

        // Check if this is a type conversion: TypeName(value) or pkg.TypeName(value)
        // Built-in types that can be used for type conversion
        let is_builtin_type = |name: &str| -> bool {
            matches!(
                name,
                "string"
                    | "int"
                    | "int8"
                    | "int16"
                    | "int32"
                    | "int64"
                    | "uint"
                    | "uint8"
                    | "uint16"
                    | "uint32"
                    | "uint64"
                    | "uintptr"
                    | "byte"
                    | "rune"
                    | "float32"
                    | "float64"
                    | "bool"
                    | "complex64"
                    | "complex128"
            )
        };

        if let ExprKind::Ident(type_name) = &func.kind
            && (self.global_state.has_type(type_name) || is_builtin_type(type_name))
        {
            // This is a type conversion, not a function call
            // Type conversions take exactly one argument
            if args.len() != 1 {
                return Err(SoppoError::Type {
                    message: format!(
                        "Type conversion requires exactly 1 argument, but got {}",
                        args.len()
                    ),
                    span: *expr_span,
                });
            }

            // Infer the argument type (we don't need to use it, just check it's valid)
            self.infer_expr(&args[0].1)?;

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
                    for (_, arg) in args {
                        arg_tys.push((self.infer_expr_narrowed(arg)?, arg.span));
                    }

                    // Extract param types and return type from func_ty
                    if let Type::Fun {
                        args: param_tys,
                        ret,
                        ..
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
                        for (param_ty, (arg_ty, arg_span)) in param_tys.iter().zip(arg_tys.iter()) {
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
                            span: *expr_span,
                        });
                    }
                    self.infer_expr(&args[0].1)?;
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
                        span: *expr_span,
                    });
                }

                // Infer the argument type (we don't need to use it, just check it's valid)
                self.infer_expr(&args[0].1)?;

                // Return the target type
                return Ok(ty);
            }
        }

        // Regular function call
        let func_ty = self.infer_expr(func)?;
        let func_ty = self.substitute(func_ty);

        // Look up parameter info if this is a known function
        // Exclude variadic params (type name starts with "variadic" or "...")
        let (param_names, is_variadic): (Option<Vec<String>>, bool) =
            if let ExprKind::Ident(func_name) = &func.kind {
                if let Some(f) = self.global_state.lookup_function(func_name) {
                    let is_variadic_type = |ty: &Type| {
                        let s = ty.to_string();
                        s.starts_with("variadic") || s.starts_with("...")
                    };
                    let has_variadic = f.params.last().is_some_and(|(_, ty)| is_variadic_type(ty));
                    let names = f
                        .params
                        .iter()
                        .filter(|(_, ty)| !is_variadic_type(ty))
                        .map(|(name, _)| name.clone())
                        .collect();
                    (Some(names), has_variadic)
                } else {
                    (None, false)
                }
            } else {
                (None, false)
            };

        // Check if any args are named
        let has_named = args.iter().any(|(name, _)| name.is_some());

        // Reorder arguments based on named arguments
        let ordered_args: Vec<(&Expr, Span)> = if !has_named {
            // All positional - just use them in order
            args.iter().map(|(_, e)| (e, e.span)).collect()
        } else if let Some(param_names) = &param_names {
            // We have named args and know parameter names - reorder
            // Rules:
            // - Positional args before any named arg fill fixed params in order
            // - Named args fill their named slots
            // - Positional args after a named arg go to variadic
            let mut result: Vec<Option<(&Expr, Span)>> = vec![None; param_names.len()];
            let mut variadic_args: Vec<(&Expr, Span)> = Vec::new();
            let mut seen_named = false;
            let mut next_positional_idx = 0;

            for (name, arg_expr) in args {
                match name {
                    Some((n, name_span)) => {
                        seen_named = true;
                        if let Some(idx) = param_names.iter().position(|p| p == n) {
                            if result[idx].is_some() {
                                return Err(SoppoError::Type {
                                    message: format!("Argument `{}` provided multiple times", n),
                                    span: *name_span,
                                });
                            }
                            result[idx] = Some((arg_expr, arg_expr.span));
                        } else {
                            return Err(SoppoError::Type {
                                message: format!("Unknown parameter name: `{}`", n),
                                span: *name_span,
                            });
                        }
                    }
                    None => {
                        if seen_named {
                            // Positional after named - only allowed for variadic functions
                            if !is_variadic {
                                return Err(SoppoError::Type {
                                    message: "Positional argument cannot follow named argument (non-variadic function)".to_string(),
                                    span: arg_expr.span,
                                });
                            }
                            variadic_args.push((arg_expr, arg_expr.span));
                        } else {
                            // Positional before any named fills fixed params
                            if next_positional_idx < param_names.len() {
                                result[next_positional_idx] = Some((arg_expr, arg_expr.span));
                                next_positional_idx += 1;
                            } else {
                                // Extra positional goes to variadic
                                variadic_args.push((arg_expr, arg_expr.span));
                            }
                        }
                    }
                }
            }

            // Check all required params are provided
            let mut ordered = Vec::new();
            for (i, slot) in result.iter().enumerate() {
                match slot {
                    Some((arg, span)) => ordered.push((*arg, *span)),
                    None => {
                        return Err(SoppoError::Type {
                            message: format!("Missing required argument: `{}`", param_names[i]),
                            span: func.span,
                        });
                    }
                }
            }

            // Add variadic args at the end
            ordered.extend(variadic_args);

            ordered
        } else {
            // Named args but unknown function - error
            return Err(SoppoError::Type {
                message: "Named arguments require a known function".to_string(),
                span: func.span,
            });
        };

        // Infer argument types with their spans
        // Use infer_expr_narrowed to apply nil-state narrowing
        let mut arg_tys = Vec::new();
        for (arg, span) in &ordered_args {
            arg_tys.push((self.infer_expr_narrowed(arg)?, *span));
        }

        // Check function call with detailed error spans
        match &func_ty {
            Type::Fun {
                args: param_tys,
                ret,
                ..
            } => {
                // Check if last param is variadic
                let has_variadic = param_tys.last().is_some_and(|last| {
                    matches!(last, Type::Con { name, .. } if name.name == "variadic" || name.name.starts_with("..."))
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
                    for (param_ty, (arg_ty, arg_span)) in fixed_params.iter().zip(arg_tys.iter()) {
                        self.unify(param_ty, arg_ty, arg_span)?;
                    }

                    // Check variadic args
                    for (arg_ty, arg_span) in arg_tys.iter().skip(fixed_params.len()) {
                        // For "any" type (or nullable any), any argument is valid
                        let is_any = match &variadic_elem {
                            Type::Con { name, .. } => name.name == "any",
                            _ => false,
                        };
                        if !is_any {
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
                    for (param_ty, (arg_ty, arg_span)) in param_tys.iter().zip(arg_tys.iter()) {
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
                self.unify(&func_ty, &expected_func_ty, expr_span)?;
                Ok(self.substitute(result_ty))
            }
            _ => Err(SoppoError::Type {
                message: format!("Cannot call non-function type `{}`", func_ty),
                span: func.span,
            }),
        }
    }

    /// Infer the type of a unary expression
    fn infer_unary(&mut self, op: &UnaryOp, operand: &Expr) -> Result<Type> {
        let operand_ty = self.infer_expr(operand)?;

        match op {
            UnaryOp::Neg => {
                // -x: operand must be numeric, result is same type
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

                // Check for nil pointer dereference (only pointers can be dereferenced)
                let is_ptr = matches!(&operand_ty, Type::Con { name, .. } if name.name.starts_with('*') || name.name == "ptr");
                if is_ptr {
                    // Get a key for the expression (works for identifiers and field chains)
                    let expr_key = super::stmt::expr_to_key(operand);

                    // Check nil state for the expression, or assume nullable for complex expressions
                    let is_nullable = match &expr_key {
                        Some(key) => self.get_nil_state(key) == Nullability::Nullable,
                        None => true, // Complex expressions are conservatively nullable
                    };

                    if is_nullable {
                        let name_for_error = expr_key.unwrap_or_else(|| "expression".to_string());
                        return Err(SoppoError::NilPointer {
                            name: name_for_error,
                            span: operand.span,
                        });
                    }
                }

                // Extract the pointee type from *T
                if let Type::Con { name, args, .. } = &operand_ty {
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
                if let Type::Con { name, args, .. } = &operand_ty {
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
        if let Type::Con { name, args, .. } = ty {
            assert_eq!(name.name, "array");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0], Type::simple("int"));
        } else {
            panic!("Expected array type, got: {:?}", ty);
        }
    }
}
