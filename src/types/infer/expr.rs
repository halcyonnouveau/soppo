use std::collections::HashMap;

use super::Infer;
use crate::error::{Result, SoppoError};
use crate::syntax::{
    BinOp, EnumVariant, Expr, ExprKind, ModuleId, Span, Symbol, TypeAnnotation, UnaryOp,
};
use crate::types::Type;
use crate::types::ast::{TypedCallArg, TypedExpr, TypedExprKind, TypedParam, TypedStringPart};
use crate::types::ctx::TypeDefKind;
use crate::types::sym::{SymbolInfo, SymbolKind};
use crate::types::ty::Nullability;

/// Result of inferring a field access expression.
/// Used internally to distinguish package member, enum variant, and regular field access.
enum FieldAccessResult {
    /// Package member access: `fmt.Println`, `helpers.Point`
    PackageMember {
        pkg: String,
        member: String,
        ty: Type,
    },
    /// Enum variant access: `Option.Some`, `Colour.Red`
    EnumVariant {
        enum_ty: Type,
        variant: String,
        ty: Type,
    },
    /// Regular field/method access on a struct: `point.x`, `user.Name()`
    Field { ty: Type },
}

impl Infer {
    /// Infer the type of an expression and return a TypedExpr.
    ///
    /// **Prefer `infer_expr`** which collects errors and returns error TypedExpr on failure.
    /// This version should only be used when you need to explicitly check if inference failed.
    pub fn infer_expr_inner(&mut self, expr: &Expr) -> Result<TypedExpr> {
        let span = expr.span;
        match &expr.kind {
            ExprKind::Integer(val, fmt) => Ok(TypedExpr::new(
                TypedExprKind::Integer(*val, *fmt),
                Type::simple("int"),
                span,
            )),

            ExprKind::Float(val) => Ok(TypedExpr::new(
                TypedExprKind::Float(*val),
                Type::simple("float64"),
                span,
            )),

            ExprKind::String(s) => Ok(TypedExpr::new(
                TypedExprKind::String(s.clone()),
                Type::simple("string"),
                span,
            )),

            ExprKind::RawString(s) => Ok(TypedExpr::new(
                TypedExprKind::RawString(s.clone()),
                Type::simple("string"),
                span,
            )),

            ExprKind::Rune(s) => Ok(TypedExpr::new(
                TypedExprKind::Rune(s.clone()),
                Type::simple("rune"),
                span,
            )),

            ExprKind::StringInterpolation(parts) => {
                // Type check each interpolated expression and validate format specifiers
                let mut had_error = false;
                let mut typed_parts = Vec::new();
                for part in parts {
                    match part {
                        crate::syntax::StringPart::Literal(s) => {
                            typed_parts.push(TypedStringPart::Literal(s.clone()));
                        }
                        crate::syntax::StringPart::Expr {
                            expr: inner,
                            format,
                        } => {
                            let typed_inner = self.infer_expr(inner);
                            if typed_inner.ty.is_error() {
                                had_error = true;
                            } else if let Some(fmt) = format
                                && let Err(msg) = validate_format_specifier(fmt, &typed_inner.ty)
                            {
                                self.emit_error(SoppoError::Type {
                                    message: msg,
                                    span: inner.span,
                                });
                                had_error = true;
                            }
                            typed_parts.push(TypedStringPart::Expr {
                                expr: Box::new(typed_inner),
                                format: format.clone(),
                            });
                        }
                    }
                }
                if had_error {
                    return Ok(TypedExpr::error(span));
                }
                Ok(TypedExpr::new(
                    TypedExprKind::StringInterpolation(typed_parts),
                    Type::simple("string"),
                    span,
                ))
            }

            ExprKind::Bool(b) => Ok(TypedExpr::new(
                TypedExprKind::Bool(*b),
                Type::simple("bool"),
                span,
            )),

            // nil is a special value that can be any pointer, interface, slice, map, channel, or function type
            // We use a fresh type variable so it unifies with the expected type
            ExprKind::Nil => Ok(TypedExpr::new(
                TypedExprKind::Nil,
                self.fresh_ty_var(),
                span,
            )),

            ExprKind::Ident(name) => {
                // Handle blank identifier specially - it accepts any type on assignment
                if name == "_" {
                    return Ok(TypedExpr::new(
                        TypedExprKind::Ident(name.clone()),
                        Type::unit(),
                        span,
                    ));
                }
                // Handle iota - a Go builtin constant generator for const blocks
                if name == "iota" {
                    return Ok(TypedExpr::new(
                        TypedExprKind::Ident(name.clone()),
                        Type::simple("int"),
                        span,
                    ));
                }

                // First, check local scopes
                if let Some((ty, def_span)) = self.lookup_var(name) {
                    // Check if this is a function type - functions are stored in local scope
                    // but should be recorded as Function symbols with their doc comments
                    let (kind, doc_comment, name_span) = if matches!(ty, Type::Func { .. }) {
                        // Look up the function definition to get doc comment and name span
                        if let Some(func_def) = self.global_state.lookup_function(name) {
                            (
                                SymbolKind::Function,
                                func_def.doc_comment.clone(),
                                func_def.name_span,
                            )
                        } else {
                            (SymbolKind::Variable, None, def_span)
                        }
                    } else {
                        (SymbolKind::Variable, None, def_span)
                    };

                    self.record_symbol(
                        expr.span,
                        SymbolInfo {
                            name: name.clone(),
                            ty: ty.clone(),
                            definition_span: def_span,
                            name_span,
                            kind,
                            doc_comment,
                            go_location: None,
                        },
                    );
                    return Ok(TypedExpr::new(TypedExprKind::Ident(name.clone()), ty, span));
                }

                // Then, check GlobalCtxt for same-module functions
                if let Some(func_def) = self.global_state.lookup_function(name).cloned() {
                    // Build function type from the function definition
                    let params: Vec<(Option<String>, Type)> = func_def
                        .params
                        .iter()
                        .map(|(n, t)| (Some(n.clone()), t.clone()))
                        .collect();
                    let ret = if func_def.return_types.is_empty() {
                        Type::unit()
                    } else if func_def.return_types.len() == 1 {
                        func_def.return_types[0].clone()
                    } else {
                        Type::generic("tuple", func_def.return_types.clone())
                    };
                    let ty = Type::fun_named(params, ret);

                    self.record_symbol(
                        expr.span,
                        SymbolInfo {
                            name: name.clone(),
                            ty: ty.clone(),
                            definition_span: func_def.span,
                            name_span: func_def.name_span,
                            kind: SymbolKind::Function,
                            doc_comment: func_def.doc_comment.clone(),
                            go_location: None,
                        },
                    );
                    return Ok(TypedExpr::new(TypedExprKind::Ident(name.clone()), ty, span));
                }

                // Check GlobalCtxt for same-module constants
                if let Some(const_def) = self.global_state.lookup_constant(name).cloned() {
                    self.record_symbol(
                        expr.span,
                        SymbolInfo {
                            name: name.clone(),
                            ty: const_def.ty.clone(),
                            definition_span: const_def.span,
                            name_span: const_def.name_span,
                            kind: SymbolKind::Constant,
                            doc_comment: const_def.doc_comment.clone(),
                            go_location: None,
                        },
                    );
                    return Ok(TypedExpr::new(
                        TypedExprKind::Ident(name.clone()),
                        const_def.ty,
                        span,
                    ));
                }

                // Check if it's a builtin type/function (don't error on those)
                let is_builtin_or_type = self.global_state.has_type(name)
                    || Type::is_builtin(name)
                    || Self::is_slice_type_conversion(name);

                if !is_builtin_or_type {
                    // Variable not found - emit error but return TypedExpr with Ident kind
                    self.emit_error(SoppoError::UndefinedVariable {
                        name: name.clone(),
                        span: expr.span,
                    });
                }

                Ok(TypedExpr::new(
                    TypedExprKind::Ident(name.clone()),
                    Type::error(),
                    span,
                ))
            }

            ExprKind::Binary { op, left, right } => {
                // For && operator, apply short-circuit narrowing:
                // In `x != nil && f(x)`, x is known non-nil when evaluating f(x)
                // Left was TRUE, so apply narrowing as-is
                if matches!(op, BinOp::And) {
                    let typed_left = self.infer_expr(left);
                    if typed_left.is_error() {
                        // Still infer right for more error collection
                        self.push_nil_scope();
                        let typed_right = self.infer_expr(right);
                        self.pop_nil_scope();
                        return Ok(TypedExpr::new(
                            TypedExprKind::Binary {
                                op: *op,
                                left: Box::new(typed_left),
                                right: Box::new(typed_right),
                            },
                            Type::error(),
                            span,
                        ));
                    }
                    self.unify(&typed_left.ty, &Type::simple("bool"), &left.span);

                    // Extract nil checks from left side and apply narrowing for right side
                    let nil_checks = super::stmt::extract_nil_checks(left);
                    self.push_nil_scope();
                    for check in &nil_checks {
                        if check.is_not_nil {
                            self.set_nil_state(check.expr_key.clone(), Nullability::NonNull);
                        }
                    }
                    let typed_right = self.infer_expr(right);
                    self.pop_nil_scope();
                    if typed_right.is_error() {
                        return Ok(TypedExpr::new(
                            TypedExprKind::Binary {
                                op: *op,
                                left: Box::new(typed_left),
                                right: Box::new(typed_right),
                            },
                            Type::error(),
                            span,
                        ));
                    }
                    self.unify(&typed_right.ty, &Type::simple("bool"), &right.span);

                    return Ok(TypedExpr::new(
                        TypedExprKind::Binary {
                            op: *op,
                            left: Box::new(typed_left),
                            right: Box::new(typed_right),
                        },
                        Type::simple("bool"),
                        span,
                    ));
                }

                // For || operator, apply short-circuit narrowing with OPPOSITE logic:
                // In `x == nil || f(x)`, x is known non-nil when evaluating f(x)
                // Left was FALSE, so apply the opposite narrowing
                if matches!(op, BinOp::Or) {
                    let typed_left = self.infer_expr(left);
                    if typed_left.is_error() {
                        // Still infer right for more error collection
                        self.push_nil_scope();
                        let typed_right = self.infer_expr(right);
                        self.pop_nil_scope();
                        return Ok(TypedExpr::new(
                            TypedExprKind::Binary {
                                op: *op,
                                left: Box::new(typed_left),
                                right: Box::new(typed_right),
                            },
                            Type::error(),
                            span,
                        ));
                    }
                    self.unify(&typed_left.ty, &Type::simple("bool"), &left.span);

                    // Extract nil checks from left side and apply OPPOSITE narrowing
                    let nil_checks = super::stmt::extract_nil_checks(left);
                    self.push_nil_scope();
                    for check in &nil_checks {
                        // Opposite: if left checked `x == nil` (is_not_nil=false),
                        // and left is false, then x != nil
                        if !check.is_not_nil {
                            self.set_nil_state(check.expr_key.clone(), Nullability::NonNull);
                        }
                    }
                    let typed_right = self.infer_expr(right);
                    self.pop_nil_scope();
                    if typed_right.is_error() {
                        return Ok(TypedExpr::new(
                            TypedExprKind::Binary {
                                op: *op,
                                left: Box::new(typed_left),
                                right: Box::new(typed_right),
                            },
                            Type::error(),
                            span,
                        ));
                    }
                    self.unify(&typed_right.ty, &Type::simple("bool"), &right.span);

                    return Ok(TypedExpr::new(
                        TypedExprKind::Binary {
                            op: *op,
                            left: Box::new(typed_left),
                            right: Box::new(typed_right),
                        },
                        Type::simple("bool"),
                        span,
                    ));
                }

                let typed_left = self.infer_expr(left);
                let typed_right = self.infer_expr(right);

                // If either operand failed, return error
                if typed_left.is_error() || typed_right.is_error() {
                    return Ok(TypedExpr::new(
                        TypedExprKind::Binary {
                            op: *op,
                            left: Box::new(typed_left),
                            right: Box::new(typed_right),
                        },
                        Type::error(),
                        span,
                    ));
                }

                let result_ty = match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                        // Arithmetic: try normal unification first
                        // Point error at right operand since left is typically the "expected" type
                        if self
                            .unify_inner(&typed_left.ty, &typed_right.ty, &right.span)
                            .is_ok()
                        {
                            self.substitute(typed_left.ty.clone())
                        } else {
                            // If unification failed, check if we have a defined type with numeric
                            // underlying type on one side and a compatible numeric on the other.
                            let left_ty_sub = self.substitute(typed_left.ty.clone());
                            let right_ty_sub = self.substitute(typed_right.ty.clone());

                            // Try to check if types are compatible via underlying type
                            if let Some(result_ty) = self
                                .check_numeric_underlying_compatibility(&left_ty_sub, &right_ty_sub)
                            {
                                result_ty
                            } else {
                                // Neither worked - emit the unification error and return error type
                                self.unify(&typed_left.ty, &typed_right.ty, &right.span);
                                Type::error()
                            }
                        }
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        // Comparison: both must be same type, result is bool
                        self.unify(&typed_left.ty, &typed_right.ty, &expr.span);
                        Type::simple("bool")
                    }
                    BinOp::And | BinOp::Or => {
                        // Logical: both must be bool, result is bool (handled above for narrowing)
                        self.unify(&typed_left.ty, &Type::simple("bool"), &left.span);
                        self.unify(&typed_right.ty, &Type::simple("bool"), &right.span);
                        Type::simple("bool")
                    }
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                        // Bitwise: both must be integer types, result is same type
                        // For shifts, right side must be integer but can differ from left
                        if matches!(op, BinOp::Shl | BinOp::Shr) {
                            // Shift: left type is preserved, right must be integer
                            self.substitute(typed_left.ty.clone())
                        } else {
                            // Bitwise AND/OR/XOR: operands must be same type
                            self.unify(&typed_left.ty, &typed_right.ty, &right.span);
                            self.substitute(typed_left.ty.clone())
                        }
                    }
                };

                Ok(TypedExpr::new(
                    TypedExprKind::Binary {
                        op: *op,
                        left: Box::new(typed_left),
                        right: Box::new(typed_right),
                    },
                    result_ty,
                    span,
                ))
            }

            ExprKind::Call {
                func,
                type_args,
                args,
            } => {
                // Infer the function and arguments first
                let typed_func = self.infer_expr(func);
                let typed_args: Vec<TypedCallArg> = args
                    .iter()
                    .map(|(name, arg_expr, spread)| {
                        let typed_arg = self.infer_expr(arg_expr);
                        (name.clone(), typed_arg, *spread)
                    })
                    .collect();

                let type_arg_types: Vec<Type> =
                    type_args.iter().map(|ta| self.resolve_type(ta)).collect();

                // Check if this is a type conversion: TypeName(value)
                // Type conversions have exactly 1 argument and func is a type name
                let is_type_conversion = args.len() == 1 && type_args.is_empty() && {
                    match &typed_func.kind {
                        TypedExprKind::Ident(name) => {
                            self.global_state.has_type(name)
                                || Type::is_builtin_type(name)
                                || Self::is_slice_type_conversion(name)
                        }
                        TypedExprKind::PackageMember { pkg, member } => {
                            // pkg.Type(value) case
                            self.lookup_soppo_type(pkg, member).is_some()
                                || self.lookup_go_type(pkg, member).is_some()
                        }
                        _ => false,
                    }
                };

                if is_type_conversion {
                    // Get the target type and value
                    let (_, typed_value, _) = typed_args.into_iter().next().unwrap();
                    let target_ty = match &typed_func.kind {
                        TypedExprKind::Ident(name) => Type::simple(name),
                        TypedExprKind::PackageMember { pkg, member } => {
                            // Look up the actual type from the package
                            if let Some((ty, ..)) = self.lookup_soppo_type(pkg, member) {
                                ty
                            } else if let Some((ty, ..)) = self.lookup_go_type(pkg, member) {
                                ty
                            } else {
                                Type::simple(&format!("{}.{}", pkg, member))
                            }
                        }
                        _ => Type::error(),
                    };

                    return Ok(TypedExpr::new(
                        TypedExprKind::TypeConversion {
                            target_ty: target_ty.clone(),
                            value: Box::new(typed_value),
                        },
                        target_ty,
                        span,
                    ));
                }

                // Not a type conversion - call the regular type inference logic
                let result_ty = self.infer_call_type(&typed_func, type_args, &typed_args, span)?;

                Ok(TypedExpr::new(
                    TypedExprKind::Call {
                        func: Box::new(typed_func),
                        type_args: type_arg_types,
                        args: typed_args,
                    },
                    result_ty,
                    span,
                ))
            }

            ExprKind::TypeInst { ty, type_args } => {
                // Type instantiation: Option[int] for accessing generic type members
                // Resolve the base type name from the expression (handles nested paths)
                fn extract_type_path(expr: &Expr) -> Option<String> {
                    match &expr.kind {
                        ExprKind::Ident(name) => Some(name.clone()),
                        ExprKind::Field { expr, field, .. } => {
                            extract_type_path(expr).map(|base| format!("{}.{}", base, field))
                        }
                        _ => None,
                    }
                }

                let type_name = extract_type_path(ty).ok_or_else(|| SoppoError::Type {
                    message: format!(
                        "Expected type name in type instantiation, got {:?}",
                        ty.kind
                    ),
                    span,
                })?;

                // Validate type arguments exist
                for ta in type_args {
                    if let Err(e) = self.validate_type_arg(ta) {
                        self.emit_error(e);
                    }
                }

                let type_arg_types: Vec<Type> =
                    type_args.iter().map(|ta| self.resolve_type(ta)).collect();
                let instantiated_ty = Type::generic(&type_name, type_arg_types);

                Ok(TypedExpr::new(
                    TypedExprKind::TypeInst {
                        ty: instantiated_ty.clone(),
                    },
                    instantiated_ty,
                    span,
                ))
            }

            ExprKind::Field {
                expr: field_expr,
                field,
                span: field_span,
            } => {
                // Infer field access - returns a FieldAccessResult indicating the kind
                let typed_inner = self.infer_expr(field_expr);
                let result = self.infer_field_access(&typed_inner, field, field_span)?;

                let (kind, result_ty) = match result {
                    FieldAccessResult::PackageMember { pkg, member, ty } => {
                        (TypedExprKind::PackageMember { pkg, member }, ty)
                    }
                    FieldAccessResult::EnumVariant {
                        enum_ty,
                        variant,
                        ty,
                    } => (TypedExprKind::EnumVariant { enum_ty, variant }, ty),
                    FieldAccessResult::Field { ty } => (
                        TypedExprKind::Field {
                            expr: Box::new(typed_inner),
                            field: field.clone(),
                            span: *field_span,
                        },
                        ty,
                    ),
                };

                Ok(TypedExpr::new(kind, result_ty, span))
            }

            ExprKind::Index {
                expr: container_expr,
                index,
            } => {
                let typed_container = self.infer_expr(container_expr);
                let typed_index = self.infer_expr(index);

                // If either failed, return error
                if typed_container.is_error() || typed_index.is_error() {
                    return Ok(TypedExpr::new(
                        TypedExprKind::Index {
                            expr: Box::new(typed_container),
                            index: Box::new(typed_index),
                        },
                        Type::error(),
                        span,
                    ));
                }

                let container_ty = self.substitute(typed_container.ty.clone());

                // Map indexing: map[K]V - index is K, result is V
                if let Some((key_ty, val_ty)) = Self::extract_map_elements(&container_ty) {
                    self.unify(&typed_index.ty, &key_ty, &index.span);
                    return Ok(TypedExpr::new(
                        TypedExprKind::Index {
                            expr: Box::new(typed_container),
                            index: Box::new(typed_index),
                        },
                        val_ty,
                        span,
                    ));
                }

                // Slice indexing: []T - index is int, result is T
                if let Some(elem_ty) = Self::extract_slice_element(&container_ty) {
                    self.unify(&typed_index.ty, &Type::simple("int"), &index.span);
                    return Ok(TypedExpr::new(
                        TypedExprKind::Index {
                            expr: Box::new(typed_container),
                            index: Box::new(typed_index),
                        },
                        elem_ty,
                        span,
                    ));
                }

                if let Type::Con { sym, args, .. } = &container_ty {
                    // Array indexing: array or [N]T - index is int
                    if sym.name == "array" && args.len() == 1 {
                        self.unify(&typed_index.ty, &Type::simple("int"), &index.span);
                        return Ok(TypedExpr::new(
                            TypedExprKind::Index {
                                expr: Box::new(typed_container),
                                index: Box::new(typed_index),
                            },
                            args[0].clone(),
                            span,
                        ));
                    }

                    // String indexing - index is int, result is byte
                    if sym.name == "string" {
                        self.unify(&typed_index.ty, &Type::simple("int"), &index.span);
                        return Ok(TypedExpr::new(
                            TypedExprKind::Index {
                                expr: Box::new(typed_container),
                                index: Box::new(typed_index),
                            },
                            Type::simple("byte"),
                            span,
                        ));
                    }
                }

                // Default: assume int index
                self.unify(&typed_index.ty, &Type::simple("int"), &index.span);
                Ok(TypedExpr::new(
                    TypedExprKind::Index {
                        expr: Box::new(typed_container),
                        index: Box::new(typed_index),
                    },
                    self.fresh_ty_var(),
                    span,
                ))
            }

            ExprKind::ArrayLit { ty, elements } => {
                // Infer element type from the declared type or first element
                let (elem_ty, declared_type, anon_struct_fields) = if let Some(ty) = ty {
                    // Extract element type from []T or T
                    if ty.name.starts_with("[]") {
                        let elem_name = &ty.name[2..];
                        // Check if element type is an anonymous struct
                        let anon_fields = if elem_name.starts_with("struct{") {
                            self.parse_anon_struct_fields(elem_name)
                        } else {
                            None
                        };
                        // Return the resolved type to match how return types are handled
                        (
                            Type::simple(elem_name),
                            Some(self.resolve_type(ty)),
                            anon_fields,
                        )
                    } else {
                        (Type::simple(&ty.name), None, None)
                    }
                } else if !elements.is_empty() {
                    let first_typed = self.infer_expr(&elements[0]);
                    if first_typed.is_error() {
                        // Still check remaining elements for more errors
                        for elem in elements.iter().skip(1) {
                            self.infer_expr(elem);
                        }
                        return Ok(TypedExpr::error(span));
                    }
                    (first_typed.ty.clone(), None, None)
                } else {
                    (self.fresh_ty_var(), None, None)
                };

                let mut had_error = false;
                let mut typed_elements = Vec::new();

                // All elements must have the same type
                for elem in elements {
                    // Check: assigning nil to a non-nilable element type is an error
                    if matches!(elem.kind, ExprKind::Nil)
                        && let Some(err) = Self::check_nil_to_non_nilable(&elem_ty, elem.span)
                    {
                        self.emit_error(err);
                        had_error = true;
                        typed_elements.push(TypedExpr::error(elem.span));
                        continue;
                    }

                    // Special handling for implicit StructLit with positional fields
                    // when element type is an anonymous struct
                    if let (
                        Some(field_defs),
                        ExprKind::StructLit {
                            ty: None,
                            fields: struct_fields,
                            multiline,
                        },
                    ) = (&anon_struct_fields, &elem.kind)
                    {
                        // Check positional fields against struct field definitions
                        let mut typed_fields = Vec::new();
                        for (i, (field_name, value)) in struct_fields.iter().enumerate() {
                            let typed_value = self.infer_expr(value);
                            if typed_value.is_error() {
                                had_error = true;
                                typed_fields.push((field_name.clone(), typed_value));
                                continue;
                            }
                            match field_name {
                                Some(name) => {
                                    // Named field - look up by name
                                    if let Some((_, expected_ty)) =
                                        field_defs.iter().find(|(n, _)| n == name)
                                    {
                                        self.unify(expected_ty, &typed_value.ty, &value.span);
                                    }
                                }
                                None => {
                                    // Positional field - look up by index
                                    if let Some((_, expected_ty)) = field_defs.get(i) {
                                        self.unify(expected_ty, &typed_value.ty, &value.span);
                                    }
                                }
                            }
                            typed_fields.push((field_name.clone(), typed_value));
                        }
                        // Build implicit struct literal
                        typed_elements.push(TypedExpr::new(
                            TypedExprKind::StructLit {
                                struct_ty: elem_ty.clone(),
                                fields: typed_fields,
                                implicit: true,
                                multiline: *multiline,
                            },
                            elem_ty.clone(),
                            elem.span,
                        ));
                    } else {
                        let typed_elem = self.infer_expr(elem);
                        if !typed_elem.is_error() {
                            self.unify(&elem_ty, &typed_elem.ty, &elem.span);
                        } else {
                            had_error = true;
                        }
                        typed_elements.push(typed_elem);
                    }
                }

                if had_error {
                    return Ok(TypedExpr::error(span));
                }

                // Return proper slice/array type
                let result_ty = if let Some(declared_ty) = declared_type {
                    declared_ty
                } else if ty.is_some() {
                    // Explicit type without [] prefix - use array type
                    Type::array(elem_ty.clone())
                } else {
                    // Implicit composite literal (no type specified) - use slice type
                    // This handles cases like [][]int{{1, 2}, {3, 4}} where {1, 2} is implicit
                    Type::slice(elem_ty.clone())
                };

                Ok(TypedExpr::new(
                    TypedExprKind::ArrayLit {
                        elem_ty,
                        elements: typed_elements,
                    },
                    result_ty,
                    span,
                ))
            }

            ExprKind::StructLit {
                ty,
                fields,
                multiline,
            } => {
                // Handle implicit struct literal (ty is None) - type inferred from context
                let Some(ty) = ty else {
                    // Just type check field values, return a fresh type variable
                    // The type will be unified with the expected type from context
                    let mut had_error = false;
                    let mut typed_fields = Vec::new();
                    for (field_name, value) in fields {
                        let typed_value = self.infer_expr(value);
                        if typed_value.is_error() {
                            had_error = true;
                        }
                        typed_fields.push((field_name.clone(), typed_value));
                    }
                    if had_error {
                        return Ok(TypedExpr::error(span));
                    }
                    let fresh_ty = self.fresh_ty_var();
                    return Ok(TypedExpr::new(
                        TypedExprKind::StructLit {
                            struct_ty: fresh_ty.clone(),
                            fields: typed_fields,
                            implicit: true,
                            multiline: *multiline,
                        },
                        fresh_ty,
                        span,
                    ));
                };

                let mut typed_fields = Vec::new();
                let mut had_error = false;

                // Look up struct definition to check field types
                if let Some(type_def) = self.global_state.lookup_type(&ty.name).cloned() {
                    // Record the type name as a symbol for hover/go-to-definition
                    self.record_symbol(
                        ty.span,
                        SymbolInfo {
                            name: ty.name.clone(),
                            ty: Type::simple(&ty.name),
                            definition_span: type_def.span,
                            name_span: type_def.name_span,
                            kind: SymbolKind::Type,
                            doc_comment: type_def.doc_comment.clone(),
                            go_location: None,
                        },
                    );

                    if let TypeDefKind::Struct { fields: field_defs } = &type_def.kind {
                        // Build map of field name -> type
                        let field_types: std::collections::HashMap<_, _> = field_defs
                            .iter()
                            .map(|(name, ty)| (name.as_str(), ty.clone()))
                            .collect();

                        // Type check each field
                        for (idx, (field_name, value)) in fields.iter().enumerate() {
                            // Resolve field name - use explicit name or get from position
                            let resolved_name = if let Some(name) = field_name {
                                Some(name.clone())
                            } else {
                                // Positional field - get name from struct definition
                                field_defs.get(idx).map(|(name, _)| name.clone())
                            };

                            // Get field type - either by name or by position
                            let field_ty = if let Some(ref name) = resolved_name {
                                field_types.get(name.as_str()).cloned()
                            } else {
                                None
                            };

                            if let Some(ref fty) = field_ty {
                                // Check: assigning nil to a non-nilable field is an error
                                if matches!(value.kind, ExprKind::Nil)
                                    && let Some(err) =
                                        Self::check_nil_to_non_nilable(fty, value.span)
                                {
                                    self.emit_error(err);
                                    had_error = true;
                                    typed_fields
                                        .push((resolved_name, TypedExpr::error(value.span)));
                                    continue;
                                }
                            }
                            let typed_value = self.infer_expr(value);
                            if typed_value.is_error() {
                                had_error = true;
                            }
                            typed_fields.push((resolved_name, typed_value));
                        }
                    } else {
                        // Not a struct - type check fields anyway
                        for (field_name, value) in fields {
                            let typed_value = self.infer_expr(value);
                            if typed_value.is_error() {
                                had_error = true;
                            }
                            typed_fields.push((field_name.clone(), typed_value));
                        }
                    }
                } else {
                    // Fallback: just type check values without nil check
                    for (field_name, value) in fields {
                        let typed_value = self.infer_expr(value);
                        if typed_value.is_error() {
                            had_error = true;
                        }
                        typed_fields.push((field_name.clone(), typed_value));
                    }
                }

                if had_error {
                    return Ok(TypedExpr::error(span));
                }

                // Check if this is a local enum variant (e.g., Shape.Circle)
                // For enum variants, struct_ty and expr_ty differ:
                // - struct_ty = "Shape.Circle" (for codegen to generate Shape_Circle{})
                // - expr_ty = "Shape" (the enum interface type for type checking)
                if let Some((pkg_name, _variant_name)) = ty.name.split_once('.')
                    && self.global_state.is_local_enum(pkg_name)
                {
                    let struct_ty = Type::simple(&ty.name); // "Shape.Circle"

                    let expr_ty = if !ty.args.is_empty() {
                        // Generic enum struct literal: Option[int].Some{...}
                        let type_arg_types: Vec<Type> =
                            ty.args.iter().map(|ta| self.resolve_type(ta)).collect();
                        Type::generic(pkg_name, type_arg_types)
                    } else {
                        // Check if the enum is generic - if so, use fresh type variables
                        let generics_count = self
                            .global_state
                            .lookup_type(pkg_name)
                            .map(|td| td.generics.len())
                            .unwrap_or(0);
                        if generics_count > 0 {
                            let type_vars: Vec<Type> =
                                (0..generics_count).map(|_| self.fresh_ty_var()).collect();
                            Type::generic(pkg_name, type_vars)
                        } else {
                            Type::simple(pkg_name)
                        }
                    };

                    return Ok(TypedExpr::new(
                        TypedExprKind::StructLit {
                            struct_ty,
                            fields: typed_fields,
                            implicit: false,
                            multiline: *multiline,
                        },
                        expr_ty,
                        span,
                    ));
                }

                // For all other cases (regular structs, Go types, etc.),
                // struct_ty and expr_ty are the same
                let result_ty = if let Some((pkg_name, type_name)) = ty.name.split_once('.') {
                    // Qualified type: pkg.Type
                    self.mark_import_used(pkg_name);

                    Type::Con {
                        sym: Symbol {
                            module: ModuleId::new(pkg_name),
                            name: type_name.to_string(),
                            span: Span::dummy(),
                        },
                        args: vec![],
                        nullable: false,
                    }
                } else {
                    Type::simple(&ty.name)
                };

                Ok(TypedExpr::new(
                    TypedExprKind::StructLit {
                        struct_ty: result_ty.clone(),
                        fields: typed_fields,
                        implicit: false,
                        multiline: *multiline,
                    },
                    result_ty,
                    span,
                ))
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
                        let k_typed = self.infer_expr(k);
                        let v_typed = self.infer_expr(v);
                        if k_typed.is_error() || v_typed.is_error() {
                            // Still check remaining entries for more errors
                            for (key, value) in entries.iter().skip(1) {
                                self.infer_expr(key);
                                self.infer_expr(value);
                            }
                            return Ok(TypedExpr::error(span));
                        }
                        (k_typed.ty.clone(), v_typed.ty.clone())
                    } else {
                        (self.fresh_ty_var(), self.fresh_ty_var())
                    }
                };

                let mut had_error = false;
                let mut typed_entries = Vec::new();

                // Type check all entries
                for (key, value) in entries {
                    // Check: assigning nil to a non-nilable value type is an error
                    if matches!(value.kind, ExprKind::Nil)
                        && let Some(err) = Self::check_nil_to_non_nilable(&val_ty, value.span)
                    {
                        self.emit_error(err);
                        had_error = true;
                        // Still infer the key
                        let typed_key = self.infer_expr(key);
                        typed_entries.push((typed_key, TypedExpr::error(value.span)));
                        continue;
                    }
                    let typed_key = self.infer_expr(key);
                    let typed_val = self.infer_expr(value);
                    if !typed_key.is_error() {
                        self.unify(&key_ty, &typed_key.ty, &key.span);
                    } else {
                        had_error = true;
                    }
                    if !typed_val.is_error() {
                        self.unify(&val_ty, &typed_val.ty, &value.span);
                    } else {
                        had_error = true;
                    }
                    typed_entries.push((typed_key, typed_val));
                }

                if had_error {
                    return Ok(TypedExpr::error(span));
                }

                // Return map[K]V type with proper Go format in name
                let map_name = format!("map[{}]{}", key_ty, val_ty);
                let map_ty = Type::generic(&map_name, vec![key_ty, val_ty]);

                Ok(TypedExpr::new(
                    TypedExprKind::MapLit {
                        map_ty: map_ty.clone(),
                        entries: typed_entries,
                    },
                    map_ty,
                    span,
                ))
            }

            ExprKind::Unary { op, operand } => self.infer_unary(op, operand, span),

            ExprKind::FuncLit {
                params,
                returns,
                body,
            } => {
                // Save the current expected return types
                let prev_expected = self.expected_return_types.take();

                // Create a new scope for the function body
                self.push_scope();

                // Build typed parameters
                let mut typed_params = Vec::new();
                for param in params {
                    let param_ty = self.resolve_type(&param.ty);
                    if let Err(e) = self.insert_var(
                        param.ident.name.clone(),
                        param_ty.clone(),
                        Some(param.ident.span),
                    ) {
                        self.emit_error(e);
                    }
                    typed_params.push(TypedParam {
                        ident: param.ident.clone(),
                        ty: param_ty,
                        nullable: false,
                    });
                }

                // Set expected return types for this function
                let expected_ret_types: Vec<Type> =
                    returns.iter().map(|r| self.resolve_type(&r.ty)).collect();
                if !expected_ret_types.is_empty() {
                    self.expected_return_types = Some(expected_ret_types.clone());
                }

                // Infer body and get the TypedBlock
                let typed_body = self.infer_block(body);

                self.pop_scope();

                // Restore previous expected return types
                self.expected_return_types = prev_expected;

                // Build typed returns
                let typed_returns: Vec<TypedParam> = returns
                    .iter()
                    .map(|r| TypedParam {
                        ident: r.ident.clone(),
                        ty: self.resolve_type(&r.ty),
                        nullable: false,
                    })
                    .collect();

                // Build function type
                let param_types: Vec<Type> = typed_params.iter().map(|p| p.ty.clone()).collect();
                let ret_ty = if returns.is_empty() {
                    Type::unit()
                } else if returns.len() == 1 {
                    typed_returns[0].ty.clone()
                } else {
                    // Multiple return types - use a tuple type
                    let ret_types: Vec<Type> = typed_returns.iter().map(|r| r.ty.clone()).collect();
                    Type::generic("tuple", ret_types)
                };
                let func_ty = Type::fun(param_types, ret_ty);

                Ok(TypedExpr::new(
                    TypedExprKind::FuncLit {
                        params: typed_params,
                        returns: typed_returns,
                        body: typed_body,
                    },
                    func_ty,
                    span,
                ))
            }

            ExprKind::AnonStructLit { field_defs, fields } => {
                // Build a map of field names to types from the definition
                let field_types: std::collections::HashMap<String, Type> = field_defs
                    .iter()
                    .map(|f| (f.ident.name.clone(), self.resolve_type(&f.ty)))
                    .collect();

                // Build a list of field types in order for positional matching
                let field_types_ordered: Vec<Type> = field_defs
                    .iter()
                    .map(|f| self.resolve_type(&f.ty))
                    .collect();

                let mut had_error = false;
                let mut typed_fields = Vec::new();

                // Type check each field value against its declared type
                for (i, (field_name, value)) in fields.iter().enumerate() {
                    let typed_value = self.infer_expr(value);
                    if typed_value.is_error() {
                        had_error = true;
                        typed_fields.push((field_name.clone(), typed_value));
                        continue;
                    }
                    match field_name {
                        Some(name) => {
                            if let Some(expected_ty) = field_types.get(name) {
                                self.unify(expected_ty, &typed_value.ty, &value.span);
                            } else {
                                self.emit_error(SoppoError::Type {
                                    message: format!(
                                        "Anonymous struct has no field named `{}`",
                                        name
                                    ),
                                    span: value.span,
                                });
                                had_error = true;
                            }
                        }
                        None => {
                            // Positional field - match by index
                            if let Some(expected_ty) = field_types_ordered.get(i) {
                                self.unify(expected_ty, &typed_value.ty, &value.span);
                            } else {
                                self.emit_error(SoppoError::Type {
                                    message: format!(
                                        "Too many fields in anonymous struct literal (expected {})",
                                        field_defs.len()
                                    ),
                                    span: value.span,
                                });
                                had_error = true;
                            }
                        }
                    }
                    typed_fields.push((field_name.clone(), typed_value));
                }

                if had_error {
                    return Ok(TypedExpr::error(span));
                }

                // Build a unique type name for this anonymous struct
                // We'll use a hash or just generate inline in codegen
                // For type checking, we use a structural anonymous type
                let field_type_list: Vec<(String, Type)> = field_defs
                    .iter()
                    .map(|f| (f.ident.name.clone(), self.resolve_type(&f.ty)))
                    .collect();

                let struct_ty = Type::anon_struct(field_type_list);
                Ok(TypedExpr::new(
                    TypedExprKind::AnonStructLit {
                        struct_ty: struct_ty.clone(),
                        fields: typed_fields,
                    },
                    struct_ty,
                    span,
                ))
            }

            ExprKind::Block(block) => {
                // Infer block and return it as an expression
                // Block expressions have unit type (they don't have implicit returns)
                let typed_block = self.infer_block(block);
                Ok(TypedExpr::new(
                    TypedExprKind::Block(typed_block),
                    Type::unit(),
                    span,
                ))
            }

            ExprKind::Slice {
                expr: slice_expr,
                low,
                high,
                cap,
            } => {
                // Slicing returns the same type as the sliced expression
                let typed_expr = self.infer_expr(slice_expr);

                let mut had_error = typed_expr.is_error();
                let mut typed_low = None;
                let mut typed_high = None;
                let mut typed_cap = None;

                // Check that indices are integers
                if let Some(l) = low {
                    let typed_l = self.infer_expr(l);
                    if !typed_l.is_error() {
                        self.unify(&typed_l.ty, &Type::simple("int"), &l.span);
                    } else {
                        had_error = true;
                    }
                    typed_low = Some(Box::new(typed_l));
                }
                if let Some(h) = high {
                    let typed_h = self.infer_expr(h);
                    if !typed_h.is_error() {
                        self.unify(&typed_h.ty, &Type::simple("int"), &h.span);
                    } else {
                        had_error = true;
                    }
                    typed_high = Some(Box::new(typed_h));
                }
                if let Some(c) = cap {
                    let typed_c = self.infer_expr(c);
                    if !typed_c.is_error() {
                        self.unify(&typed_c.ty, &Type::simple("int"), &c.span);
                    } else {
                        had_error = true;
                    }
                    typed_cap = Some(Box::new(typed_c));
                }

                if had_error {
                    return Ok(TypedExpr::error(span));
                }

                let result_ty = typed_expr.ty.clone();
                Ok(TypedExpr::new(
                    TypedExprKind::Slice {
                        expr: Box::new(typed_expr),
                        low: typed_low,
                        high: typed_high,
                        cap: typed_cap,
                    },
                    result_ty,
                    span,
                ))
            }

            ExprKind::TypeAssert {
                expr: inner_expr,
                ty,
                ..
            } => {
                // Type assertions return the asserted type directly.
                // Like Rust pattern matching - variable is typed as the enum,
                // and assertions extract the variant data.
                //
                // For enum variants: panics if wrong variant (use comma-ok for safe check)
                // For Go interfaces: panics if wrong type (use comma-ok for safe check)
                let typed_inner = self.infer_expr(inner_expr);
                if typed_inner.is_error() {
                    return Ok(TypedExpr::error(span));
                }

                let is_enum_variant = ty.name.contains('.');

                let (target_ty, known_safe) = if is_enum_variant {
                    let variant_ty = Type::simple(&ty.name.replace('.', "_"));

                    // If we know the variant matches, record it so codegen can skip the runtime assertion
                    let known_safe = if let ExprKind::Ident(name) = &inner_expr.kind
                        && let Some(known) = self.get_variant_state(name)
                        && known == &ty.name
                    {
                        self.known_safe_asserts
                            .insert((expr.span.byte_start, expr.span.byte_end));
                        true
                    } else {
                        false
                    };

                    (variant_ty, known_safe)
                } else {
                    (self.resolve_type(ty), false)
                };

                Ok(TypedExpr::new(
                    TypedExprKind::TypeAssert {
                        expr: Box::new(typed_inner),
                        target_ty: target_ty.clone(),
                        known_safe,
                    },
                    target_ty,
                    span,
                ))
            }

            ExprKind::NilAssert { expr: inner_expr } => {
                // Nil assertion: x.(!nil) - assert the expression is non-nil
                let typed_inner = self.infer_expr_inner(inner_expr)?;
                let inner_ty = self.substitute(typed_inner.ty.clone());

                // If this is a nilable type with an identifier, mark it as non-nil
                if Self::is_nilable_type(&inner_ty)
                    && let ExprKind::Ident(name) = &inner_expr.kind
                {
                    self.set_nil_state(name.clone(), Nullability::NonNull);
                }

                // Return the non-nullable version of the type
                // x.(!nil) converts ?*T -> *T, ?[]T -> []T, etc.
                let result_ty = inner_ty.as_non_nullable();
                Ok(TypedExpr::new(
                    TypedExprKind::NilAssert {
                        expr: Box::new(typed_inner),
                    },
                    result_ty,
                    span,
                ))
            }

            ExprKind::Paren(inner) => {
                // Parenthesised expression has the type of its inner expression
                let typed_inner = self.infer_expr_inner(inner)?;
                let result_ty = typed_inner.ty.clone();
                Ok(TypedExpr::new(
                    TypedExprKind::Paren(Box::new(typed_inner)),
                    result_ty,
                    span,
                ))
            }
        }
    }

    /// Infer the type of a field access expression (returns just the Type)
    fn infer_field_access(
        &mut self,
        expr: &TypedExpr,
        field: &str,
        span: &Span,
    ) -> Result<FieldAccessResult> {
        // Check if this is accessing something from an imported package
        // e.g., fmt.Println, strings.HasPrefix, or helpers.Add (sop: import)
        if let TypedExprKind::Ident(pkg_name) = &expr.kind
            && self.is_imported_package(pkg_name)
        {
            // Record symbol for the package name itself (e.g., "bufio" in bufio.NewScanner)
            // Go-to-definition on the package name goes to the import statement
            if let Some((import_path, import_span)) = self.get_import_info(pkg_name) {
                self.record_symbol(
                    expr.span,
                    SymbolInfo {
                        name: pkg_name.to_string(),
                        ty: Type::simple(import_path), // Type shows the import path
                        definition_span: Some(import_span), // Definition is the import statement
                        name_span: None,
                        kind: SymbolKind::Package,
                        doc_comment: Some(format!("import \"{}\"", import_path)),
                        go_location: None,
                    },
                );
            }

            // For Soppo imports, look up from GlobalCtxt
            if self.is_soppo_import(pkg_name) {
                self.mark_import_used(pkg_name);

                // Try to look up as a function first
                if let Some((func_ty, def_span, name_span, doc_comment)) =
                    self.lookup_soppo_function(pkg_name, field)
                {
                    // Record symbol for go-to-definition
                    self.record_symbol(
                        *span,
                        SymbolInfo {
                            name: field.to_string(),
                            ty: func_ty.clone(),
                            definition_span: def_span,
                            name_span,
                            kind: SymbolKind::Function,
                            doc_comment,
                            go_location: None,
                        },
                    );
                    return Ok(FieldAccessResult::PackageMember {
                        pkg: pkg_name.clone(),
                        member: field.to_string(),
                        ty: func_ty,
                    });
                }

                // Try to look up as a type
                if let Some((ty, def_span, name_span, doc_comment)) =
                    self.lookup_soppo_type(pkg_name, field)
                {
                    // Record symbol for go-to-definition
                    self.record_symbol(
                        *span,
                        SymbolInfo {
                            name: field.to_string(),
                            ty: ty.clone(),
                            definition_span: def_span,
                            name_span,
                            kind: SymbolKind::Type,
                            doc_comment,
                            go_location: None,
                        },
                    );
                    return Ok(FieldAccessResult::PackageMember {
                        pkg: pkg_name.clone(),
                        member: field.to_string(),
                        ty,
                    });
                }

                // Try to look up as a constant
                if let Some((ty, def_span, name_span, doc_comment)) =
                    self.lookup_soppo_constant(pkg_name, field)
                {
                    // Record symbol for go-to-definition
                    self.record_symbol(
                        *span,
                        SymbolInfo {
                            name: field.to_string(),
                            ty: ty.clone(),
                            definition_span: def_span,
                            name_span,
                            kind: SymbolKind::Constant,
                            doc_comment,
                            go_location: None,
                        },
                    );
                    return Ok(FieldAccessResult::PackageMember {
                        pkg: pkg_name.clone(),
                        member: field.to_string(),
                        ty,
                    });
                }

                // Not found
                return Err(SoppoError::Type {
                    message: format!("`{}` not found in Soppo module `{}`", field, pkg_name),
                    span: *span,
                });
            }

            // Go packages: try to look up as a function first
            if let Some((func_ty, go_location, doc_comment)) =
                self.lookup_go_function(pkg_name, field)
            {
                // Record symbol for go-to-definition (with Go source location)
                self.record_symbol(
                    *span,
                    SymbolInfo {
                        name: field.to_string(),
                        ty: func_ty.clone(),
                        definition_span: None, // no Soppo definition span
                        name_span: None,       // no Soppo name span
                        kind: SymbolKind::Function,
                        doc_comment,
                        go_location,
                    },
                );
                return Ok(FieldAccessResult::PackageMember {
                    pkg: pkg_name.clone(),
                    member: field.to_string(),
                    ty: func_ty,
                });
            }
            // Try to look up as a type or constant
            if let Some((ty, go_location, doc_comment)) = self.lookup_go_type(pkg_name, field) {
                // Record symbol for go-to-definition (with Go source location)
                self.record_symbol(
                    *span,
                    SymbolInfo {
                        name: field.to_string(),
                        ty: ty.clone(),
                        definition_span: None, // no Soppo definition span
                        name_span: None,       // no Soppo name span
                        kind: SymbolKind::Type,
                        doc_comment,
                        go_location,
                    },
                );
                return Ok(FieldAccessResult::PackageMember {
                    pkg: pkg_name.clone(),
                    member: field.to_string(),
                    ty,
                });
            }
            // Couldn't find it - error
            return Err(SoppoError::Type {
                message: format!("`{}` not found in package `{}`", field, pkg_name),
                span: *span,
            });
        }

        // Check if this is a generic enum variant like Option[int].None or Option[int].Some
        // The parser generates Call { func: Ident(type_name), type_args, args: [] } for Option[int]
        if let TypedExprKind::Call {
            func,
            type_args,
            args,
        } = &expr.kind
            && let TypedExprKind::Ident(type_name) = &func.kind
            && !type_args.is_empty()
            && args.is_empty()
            && let Some(type_def) = self.global_state.lookup_type(type_name).cloned()
            && let TypeDefKind::Enum { variants } = &type_def.kind
        {
            // Validate type args match generic params
            if type_args.len() != type_def.generics.len() {
                return Err(SoppoError::Type {
                    message: format!(
                        "Expected {} type argument(s) for `{}`, got {}",
                        type_def.generics.len(),
                        type_name,
                        type_args.len()
                    ),
                    span: expr.span,
                });
            }

            // Build generic substitution from already-resolved type args
            let generic_subst: HashMap<String, Type> = type_def
                .generics
                .iter()
                .zip(type_args.iter())
                .map(|(g, ty)| (g.name.clone(), ty.clone()))
                .collect();

            // Find the variant
            for variant in variants {
                let variant_name = match variant {
                    EnumVariant::Unit { ident, .. } => &ident.name,
                    EnumVariant::Single { ident, .. } => &ident.name,
                    EnumVariant::Struct { ident, .. } => &ident.name,
                };

                if variant_name == field {
                    // Build the return type with type arguments: Option[int]
                    let enum_ty = Type::generic(type_name, type_args.clone());

                    let result_ty = match variant {
                        EnumVariant::Unit { .. } => {
                            // Unit variant with type args: Option[int].None
                            enum_ty.clone()
                        }
                        EnumVariant::Single { ty, .. } => {
                            // Single variant: Option[int].Some -> fn(int) -> Option[int]
                            let ty_simple = Type::simple(&ty.name);
                            let param_ty =
                                Self::instantiate_generic_type(&ty_simple, &generic_subst);
                            Type::fun(vec![param_ty], enum_ty.clone())
                        }
                        EnumVariant::Struct { fields, .. } => {
                            // Struct variant with type args
                            let param_tys: Vec<Type> = fields
                                .iter()
                                .map(|f| {
                                    let ty_simple = Type::simple(&f.ty.name);
                                    Self::instantiate_generic_type(&ty_simple, &generic_subst)
                                })
                                .collect();
                            Type::fun(param_tys, enum_ty.clone())
                        }
                    };

                    return Ok(FieldAccessResult::EnumVariant {
                        enum_ty,
                        variant: field.to_string(),
                        ty: result_ty,
                    });
                }
            }

            // Variant not found
            return Err(SoppoError::Type {
                message: format!("Enum `{}` has no variant `{}`", type_name, field),
                span: *span,
            });
        }

        // Check if this is an enum constructor like Colour.Red or Result.Ok
        if let TypedExprKind::Ident(type_name) = &expr.kind {
            // Check if type_name is a registered type
            if let Some(type_def) = self.global_state.lookup_type(type_name).cloned() {
                // Check if this is an enum variant
                if let TypeDefKind::Enum { variants } = &type_def.kind {
                    // Create fresh type variables for generic params
                    let generic_subst: HashMap<String, Type> = type_def
                        .generics
                        .iter()
                        .map(|g| (g.name.clone(), self.fresh_ty_var()))
                        .collect();

                    // Build the enum type - use generic type for generic enums
                    let enum_ty = if type_def.generics.is_empty() {
                        Type::simple(type_name)
                    } else {
                        // Generic enum: return Type::generic with fresh type vars
                        // These will be unified with the expected type from context
                        let type_vars: Vec<Type> = type_def
                            .generics
                            .iter()
                            .map(|g| generic_subst.get(&g.name).unwrap().clone())
                            .collect();
                        Type::generic(type_name, type_vars)
                    };

                    // Find the variant
                    for variant in variants {
                        let variant_name = match variant {
                            EnumVariant::Unit { ident, .. } => &ident.name,
                            EnumVariant::Single { ident, .. } => &ident.name,
                            EnumVariant::Struct { ident, .. } => &ident.name,
                        };

                        if variant_name == field {
                            // Found the variant - determine result type
                            let result_ty = match variant {
                                EnumVariant::Unit { .. } => {
                                    // Unit variant: return the enum type
                                    // For generic enums, the type vars will be inferred from context
                                    enum_ty.clone()
                                }
                                EnumVariant::Single { ty, .. } => {
                                    // Single variant: returns a constructor function
                                    // Ok(T) -> fn(T) -> Result[T, E]
                                    // Instantiate generic params with fresh type vars
                                    let ty_simple = Type::simple(&ty.name);
                                    let param_ty =
                                        Self::instantiate_generic_type(&ty_simple, &generic_subst);
                                    Type::fun(vec![param_ty], enum_ty.clone())
                                }
                                EnumVariant::Struct { fields, .. } => {
                                    // Struct variant: returns a constructor function
                                    // taking all fields as parameters
                                    let param_tys: Vec<Type> = fields
                                        .iter()
                                        .map(|f| {
                                            let ty_simple = Type::simple(&f.ty.name);
                                            Self::instantiate_generic_type(
                                                &ty_simple,
                                                &generic_subst,
                                            )
                                        })
                                        .collect();
                                    Type::fun(param_tys, enum_ty.clone())
                                }
                            };

                            return Ok(FieldAccessResult::EnumVariant {
                                enum_ty,
                                variant: field.to_string(),
                                ty: result_ty,
                            });
                        }
                    }
                }
                // Not an enum, but still a type - might be for other purposes
                return Ok(FieldAccessResult::Field {
                    ty: Type::simple(type_name),
                });
            }
        }

        // Otherwise it's a regular field access
        let expr_ty = expr.ty.clone();
        if expr_ty.is_error() {
            return Ok(FieldAccessResult::Field { ty: Type::error() });
        }
        let expr_ty = self.substitute(expr_ty);

        // Check for nil dereference on field access
        // If the expression is a nilable type, verify it's not nullable
        // Skip check if expression is a NilAssert - that explicitly makes it non-null
        // Skip if type is non-nullable in Soppo (*T vs ?*T) - non-nullable types can't be nil
        if Self::is_nilable_type(&expr_ty)
            && expr_ty.is_nullable()
            && !matches!(expr.kind, TypedExprKind::NilAssert { .. })
        {
            // Convert expression to a trackable key (supports identifiers and field chains)
            let expr_key = super::stmt::typed_expr_to_key(expr);

            // Check nil state for the expression, or assume nullable for complex expressions
            let is_nullable = match &expr_key {
                Some(key) => self.get_nil_state(key) == Nullability::Nullable,
                None => true, // Complex expressions are conservatively nullable
            };

            if is_nullable {
                let name_for_error = expr_key.unwrap_or_else(|| "expression".to_string());
                self.emit_error(SoppoError::NilPointer {
                    name: name_for_error,
                    span: expr.span,
                });
            }
        }

        // Handle built-in error type's Error() method
        if let Type::Con { sym, .. } = &expr_ty
            && sym.name == "error"
            && field == "Error"
        {
            // error.Error() returns string
            return Ok(FieldAccessResult::Field {
                ty: Type::fun(vec![], Type::simple("string")),
            });
        }

        // Look up the struct type to validate field access
        // For pointer types like *User, extract the inner type name (User)
        // Also extract the module name if present (for Go package types)
        let (struct_name, module_name): (Option<String>, Option<String>) =
            if let Type::Con { sym, args, .. } = &expr_ty {
                if sym.name.starts_with('*') && args.len() == 1 {
                    // Pointer type: extract inner type name from args or strip prefix
                    if let Type::Con { sym: inner, .. } = &args[0] {
                        let mod_name = if inner.module.0.is_empty() {
                            None
                        } else {
                            Some(inner.module.0.clone())
                        };
                        (Some(inner.name.clone()), mod_name)
                    } else {
                        (Some(sym.name[1..].to_string()), None)
                    }
                } else {
                    let mod_name = if sym.module.0.is_empty() {
                        None
                    } else {
                        Some(sym.module.0.clone())
                    };
                    (Some(sym.name.clone()), mod_name)
                }
            } else {
                (None, None)
            };

        // Check if this is a field access on a Go package type
        if let (Some(struct_name), Some(module_name)) = (&struct_name, &module_name)
            && let Some(field_ty) = self.lookup_go_struct_field(module_name, struct_name, field)
        {
            return Ok(FieldAccessResult::Field { ty: field_ty });
        }

        // Check if this is a method call on a Go package type
        if let (Some(struct_name), Some(module_name)) = (&struct_name, &module_name)
            && let Some((method_ty, go_location, doc_comment)) =
                self.lookup_go_method(module_name, struct_name, field)
        {
            // Record symbol for go-to-definition (with Go source location)
            self.record_symbol(
                *span,
                SymbolInfo {
                    name: field.to_string(),
                    ty: method_ty.clone(),
                    definition_span: None,
                    name_span: None,
                    kind: SymbolKind::Method,
                    doc_comment,
                    go_location,
                },
            );
            return Ok(FieldAccessResult::Field { ty: method_ty });
        }

        // Check if this is a method call on a type from another Soppo module
        // The module_name in the type IS the module ID (e.g., "internal/config")
        if let (Some(struct_name), Some(module_name)) = (&struct_name, &module_name) {
            let module_id = crate::syntax::ModuleId::new(module_name);
            if let Some(method) = self
                .global_state
                .lookup_method_in(&module_id, struct_name, field)
            {
                // Build function type from method signature
                let param_tys: Vec<Type> = method.params.iter().map(|(_, ty)| ty.clone()).collect();
                let ret_ty = match method.return_types.len() {
                    0 => Type::unit(),
                    1 => method.return_types[0].clone(),
                    _ => Type::generic("tuple", method.return_types.clone()),
                };
                let method_ty = Type::fun(param_tys, ret_ty);

                // Record symbol for go-to-definition
                self.record_symbol(
                    *span,
                    SymbolInfo {
                        name: field.to_string(),
                        ty: method_ty.clone(),
                        definition_span: method.span,
                        name_span: method.name_span,
                        kind: SymbolKind::Method,
                        doc_comment: method.doc_comment.clone(),
                        go_location: None,
                    },
                );
                return Ok(FieldAccessResult::Field { ty: method_ty });
            }
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
                            EnumVariant::Unit { ident, .. } => (ident.name.as_str(), None),
                            EnumVariant::Single { ident, ty, .. } => {
                                // Single variants have a "Value" field
                                (
                                    ident.name.as_str(),
                                    Some(vec![("Value".to_string(), Type::from_ast(ty))]),
                                )
                            }
                            EnumVariant::Struct { ident, fields, .. } => {
                                let fs: Vec<_> = fields
                                    .iter()
                                    .map(|f| (f.ident.name.clone(), Type::from_ast(&f.ty)))
                                    .collect();
                                (ident.name.as_str(), Some(fs))
                            }
                        };

                        if v_name == variant_name {
                            if let Some(fields) = v_fields
                                && let Some((_, field_ty)) = fields.iter().find(|(f, _)| f == field)
                            {
                                return Ok(FieldAccessResult::Field {
                                    ty: field_ty.clone(),
                                });
                            }
                            return Err(SoppoError::Type {
                                message: format!(
                                    "Enum variant `{}` has no field named `{}`",
                                    struct_name, field
                                ),
                                span: expr.span,
                            });
                        }
                    }
                }
            }

            // Regular struct lookup
            if let Some(type_def) = self.global_state.lookup_type(struct_name).cloned()
                && let TypeDefKind::Struct {
                    fields: struct_fields,
                } = &type_def.kind
            {
                // Check if the field exists
                if let Some((_, field_ty)) = struct_fields.iter().find(|(f, _)| f == field) {
                    // Record field access for LSP
                    self.record_symbol(
                        *span,
                        SymbolInfo {
                            name: field.to_string(),
                            ty: field_ty.clone(),
                            definition_span: type_def.span, // Point to struct definition
                            name_span: type_def.span, // No specific field span in TypeDef currently
                            kind: SymbolKind::Field,
                            doc_comment: None,
                            go_location: None,
                        },
                    );
                    return Ok(FieldAccessResult::Field {
                        ty: field_ty.clone(),
                    });
                } else {
                    // Field not found in struct - check if it's a method first
                    if let Some(method) = self.global_state.lookup_method(struct_name, field) {
                        // Build function type from method signature
                        let param_tys: Vec<Type> =
                            method.params.iter().map(|(_, ty)| ty.clone()).collect();
                        let ret_ty = match method.return_types.len() {
                            0 => Type::unit(),
                            1 => method.return_types[0].clone(),
                            _ => Type::generic("tuple", method.return_types.clone()),
                        };
                        let method_ty = Type::fun(param_tys, ret_ty);

                        // Record symbol for go-to-definition
                        self.record_symbol(
                            *span,
                            SymbolInfo {
                                name: field.to_string(),
                                ty: method_ty.clone(),
                                definition_span: method.span,
                                name_span: method.name_span,
                                kind: SymbolKind::Method,
                                doc_comment: method.doc_comment.clone(),
                                go_location: None,
                            },
                        );
                        return Ok(FieldAccessResult::Field { ty: method_ty });
                    }

                    // Not a method - check if it might be a UFCS function call
                    // If we can find a function with this name, return a type variable
                    // and let the Call handler deal with it
                    if self.global_state.lookup_function(field).is_some() {
                        return Ok(FieldAccessResult::Field {
                            ty: self.fresh_ty_var(),
                        });
                    }

                    return Err(SoppoError::Type {
                        message: format!("Struct `{}` has no field named `{}`", struct_name, field),
                        span: expr.span,
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
                let method_ty = Type::fun(param_tys, ret_ty);

                // Record symbol for go-to-definition
                self.record_symbol(
                    *span,
                    SymbolInfo {
                        name: field.to_string(),
                        ty: method_ty.clone(),
                        definition_span: method.span,
                        name_span: method.name_span,
                        kind: SymbolKind::Method,
                        doc_comment: method.doc_comment.clone(),
                        go_location: None,
                    },
                );
                return Ok(FieldAccessResult::Field { ty: method_ty });
            }
        }

        // If we can't determine the struct type, return a type variable
        // (this allows field access on generic/unknown types)
        Ok(FieldAccessResult::Field {
            ty: self.fresh_ty_var(),
        })
    }

    /// Infer the type of a function call expression (returns just the Type)
    fn infer_call_type(
        &mut self,
        func: &TypedExpr,
        type_args: &[TypeAnnotation],
        args: &[TypedCallArg],
        expr_span: Span,
    ) -> Result<Type> {
        // Helper closures for accessing typed argument types and spans
        let arg_ty = |i: usize| args[i].1.ty.clone();
        let arg_span = |i: usize| args[i].1.span;

        // Handle generic enum variant calls: Option.None[int](), Result.Ok[int, string](1)
        if let TypedExprKind::EnumVariant {
            enum_ty,
            variant: variant_name,
        } = &func.kind
        {
            // Extract the enum type name
            if let Type::Con { sym, .. } = enum_ty {
                let type_name = &sym.name;
                if let Some(type_def) = self.global_state.lookup_type(type_name).cloned()
                    && let TypeDefKind::Enum { variants } = &type_def.kind
                    && !type_def.generics.is_empty()
                {
                    // Validate type arg constraints if provided
                    if !type_args.is_empty() {
                        for (generic_param, type_arg) in
                            type_def.generics.iter().zip(type_args.iter())
                        {
                            // Validate the type argument is a real type
                            self.validate_type_arg(type_arg)?;

                            let concrete_ty = self.resolve_type(type_arg);
                            if !generic_param.satisfies(&concrete_ty) {
                                return Err(SoppoError::ConstraintNotSatisfied {
                                    ty: concrete_ty.to_string(),
                                    constraint: generic_param.constraint.clone(),
                                    hint: Self::constraint_hint(&generic_param.constraint),
                                    span: type_arg.span,
                                });
                            }
                        }
                    }

                    // Check which variant this is
                    for variant in variants {
                        let v_name = match variant {
                            EnumVariant::Unit { ident, .. } => &ident.name,
                            EnumVariant::Single { ident, .. } => &ident.name,
                            EnumVariant::Struct { ident, .. } => &ident.name,
                        };

                        if v_name == variant_name {
                            // Build the return type with explicit type args if provided
                            let return_ty = if !type_args.is_empty() {
                                let resolved_args: Vec<Type> =
                                    type_args.iter().map(|ta| self.resolve_type(ta)).collect();
                                Type::generic(type_name, resolved_args)
                            } else {
                                enum_ty.clone()
                            };

                            return match variant {
                                EnumVariant::Unit { .. } => {
                                    // Unit variant call: Option.None[int]()
                                    if !args.is_empty() {
                                        self.emit_error(SoppoError::Type {
                                            message: format!(
                                                "Unit variant `{}.{}` takes no arguments",
                                                type_name, variant_name
                                            ),
                                            span: expr_span,
                                        });
                                    }
                                    Ok(return_ty)
                                }
                                EnumVariant::Single { .. } => {
                                    // Single variant call: Result.Ok[int, string](1)
                                    if args.len() != 1 {
                                        self.emit_error(SoppoError::Type {
                                            message: format!(
                                                "Variant `{}.{}` requires exactly 1 argument",
                                                type_name, variant_name
                                            ),
                                            span: expr_span,
                                        });
                                    }
                                    Ok(return_ty)
                                }
                                EnumVariant::Struct { .. } => {
                                    // Struct variant should use struct literal syntax
                                    // This path shouldn't normally be hit
                                    Ok(return_ty)
                                }
                            };
                        }
                    }
                }
            }
        }

        // Handle Go built-in functions
        if let TypedExprKind::Ident(name) = &func.kind {
            // close(channel) - closes a channel, returns unit
            if name == "close" && args.len() == 1 {
                let channel_ty = arg_ty(0);
                if channel_ty.is_error() {
                    return Ok(Type::error());
                }
                let channel_ty = self.substitute(channel_ty);
                // Verify it's a channel type
                if let Type::Con { sym, .. } = &channel_ty
                    && !sym.name.starts_with("chan ")
                {
                    self.emit_error(SoppoError::Type {
                        message: format!("close requires a channel argument, got {}", channel_ty),
                        span: arg_span(0),
                    });
                    return Ok(Type::error());
                }
                return Ok(Type::unit());
            }

            if name == "make" && !type_args.is_empty() {
                // make(type, ...) - returns the type
                // Validate additional arguments are integers (size, capacity)
                let mut had_error = false;
                for (_, typed_arg, _) in args {
                    if typed_arg.ty.is_error() {
                        had_error = true;
                        continue;
                    }
                    self.unify(&typed_arg.ty, &Type::simple("int"), &typed_arg.span);
                }
                // Return the type being made (properly resolving type args)
                let ty = &type_args[0];
                if let Err(e) = self.validate_type_arg(ty) {
                    self.emit_error(e);
                    return Ok(Type::error());
                }
                if had_error {
                    return Ok(Type::error());
                }
                return Ok(self.resolve_type(ty));
            }

            if name == "new" && !type_args.is_empty() {
                // new(type) - returns *type
                // Return a pointer to the type
                let ty = &type_args[0];
                if let Err(e) = self.validate_type_arg(ty) {
                    self.emit_error(e);
                    return Ok(Type::error());
                }
                let inner_ty = self.resolve_type(ty);
                // Use *{type} naming pattern consistent with UnaryOp::Ref
                let ptr_name = format!("*{}", inner_ty);
                return Ok(Type::generic(&ptr_name, vec![inner_ty]));
            }

            // len(v) - returns length of array, slice, string, map, channel, or variadic
            if name == "len" && args.len() == 1 {
                let arg_ty = arg_ty(0);
                if arg_ty.is_error() {
                    return Ok(Type::error());
                }
                let arg_ty = self.substitute(arg_ty);
                // Verify it's a valid type for len
                // Variadic parameters (variadic[T] or ...T) are slices at runtime
                let valid = Self::is_slice_type(&arg_ty)
                    || Self::is_map_type(&arg_ty)
                    || Self::is_channel_type(&arg_ty)
                    || matches!(&arg_ty, Type::Con { sym, .. } if
                        sym.name == "string"
                        || sym.name == "array"
                        || sym.name.starts_with("[")
                        || sym.name == "variadic"
                        || sym.name.starts_with("..."));
                if !valid {
                    self.emit_error(SoppoError::Type {
                        message: format!(
                            "len requires array, slice, string, map, or channel; got {}",
                            arg_ty
                        ),
                        span: arg_span(0),
                    });
                    return Ok(Type::error());
                }
                return Ok(Type::simple("int"));
            }

            // cap(v) - returns capacity of slice, channel, or variadic
            if name == "cap" && args.len() == 1 {
                let arg_ty = arg_ty(0);
                if arg_ty.is_error() {
                    return Ok(Type::error());
                }
                let arg_ty = self.substitute(arg_ty);
                // Verify it's a valid type for cap
                // Variadic parameters (variadic[T] or ...T) are slices at runtime
                let valid = Self::is_slice_type(&arg_ty)
                    || Self::is_channel_type(&arg_ty)
                    || matches!(&arg_ty, Type::Con { sym, .. } if
                        sym.name == "array"
                        || sym.name.starts_with("[")
                        || sym.name == "variadic"
                        || sym.name.starts_with("..."));
                if !valid {
                    self.emit_error(SoppoError::Type {
                        message: format!("cap requires array, slice, or channel; got {}", arg_ty),
                        span: arg_span(0),
                    });
                    return Ok(Type::error());
                }
                return Ok(Type::simple("int"));
            }

            // append(slice, elems...) - returns the same slice type
            if name == "append" && !args.is_empty() {
                let slice_ty = arg_ty(0);
                if slice_ty.is_error() {
                    return Ok(Type::error());
                }
                let slice_ty = self.substitute(slice_ty);
                // Verify first arg is a slice and extract element type
                let elem_ty = match Self::extract_slice_element(&slice_ty) {
                    Some(elem) => elem,
                    None => {
                        self.emit_error(SoppoError::Type {
                            message: format!(
                                "first argument to append must be a slice; got {}",
                                slice_ty
                            ),
                            span: arg_span(0),
                        });
                        return Ok(Type::error());
                    }
                };
                // Type check remaining arguments against element type
                // Handle spread: append(a, b...) where b is a slice
                let mut had_error = false;
                for (_, typed_arg, is_spread) in args.iter().skip(1) {
                    if typed_arg.ty.is_error() {
                        had_error = true;
                        continue;
                    }
                    if *is_spread {
                        // Spread arg: extract element type from slice and unify
                        let spread_elem = Self::extract_slice_element(&typed_arg.ty)
                            .unwrap_or(typed_arg.ty.clone());
                        self.unify(&elem_ty, &spread_elem, &typed_arg.span);
                    } else {
                        self.unify(&elem_ty, &typed_arg.ty, &typed_arg.span);
                    }
                }
                if had_error {
                    return Ok(Type::error());
                }
                return Ok(slice_ty);
            }

            // copy(dst, src) - returns int (number of elements copied)
            if name == "copy" && args.len() == 2 {
                let dst_ty = arg_ty(0);
                let src_ty = arg_ty(1);

                // If either arg has error, return error
                if dst_ty.is_error() || src_ty.is_error() {
                    return Ok(Type::error());
                }

                let dst_ty = self.substitute(dst_ty);
                let src_ty = self.substitute(src_ty);
                // Both must be slices (or src can be string for []byte)
                let dst_is_slice = Self::is_slice_type(&dst_ty);
                let src_is_slice = Self::is_slice_type(&src_ty);
                let src_is_string =
                    matches!(&src_ty, Type::Con { sym, .. } if sym.name == "string");

                let mut had_error = false;
                if !dst_is_slice {
                    self.emit_error(SoppoError::Type {
                        message: format!("first argument to copy must be a slice; got {}", dst_ty),
                        span: arg_span(0),
                    });
                    had_error = true;
                }
                if !src_is_slice && !src_is_string {
                    self.emit_error(SoppoError::Type {
                        message: format!(
                            "second argument to copy must be a slice or string; got {}",
                            src_ty
                        ),
                        span: arg_span(1),
                    });
                    had_error = true;
                }
                // For string source, dst must be []byte
                if !had_error
                    && src_is_string
                    && let Type::Con { sym, .. } = &dst_ty
                    && sym.name != "[]byte"
                    && sym.name != "[]uint8"
                {
                    self.emit_error(SoppoError::Type {
                        message: format!("cannot copy string to {}; need []byte", dst_ty),
                        span: arg_span(0),
                    });
                    had_error = true;
                }
                if had_error {
                    return Ok(Type::error());
                }
                return Ok(Type::simple("int"));
            }

            // delete(map, key) - deletes key from map, returns unit
            if name == "delete" && args.len() == 2 {
                let map_ty = arg_ty(0);
                let arg_key_ty = arg_ty(1);

                // If either arg has error, return error
                if map_ty.is_error() || arg_key_ty.is_error() {
                    return Ok(Type::error());
                }

                let map_ty = self.substitute(map_ty);
                // Verify first arg is a map and extract key type
                let key_ty = match Self::extract_map_elements(&map_ty) {
                    Some((k, _)) => k,
                    None => {
                        self.emit_error(SoppoError::Type {
                            message: format!(
                                "first argument to delete must be a map; got {}",
                                map_ty
                            ),
                            span: arg_span(0),
                        });
                        return Ok(Type::error());
                    }
                };
                // Type check key argument
                self.unify(&key_ty, &arg_key_ty, &arg_span(1));
                return Ok(Type::unit());
            }

            // panic(v) - panics with value, returns never (diverges)
            if name == "panic" && args.len() == 1 {
                // panic accepts any type
                arg_ty(0);
                return Ok(Type::never());
            }

            // recover() - returns any (interface{})
            if name == "recover" && args.is_empty() {
                return Ok(Type::simple("any"));
            }

            // print and println - variadic, accept any types, return unit
            if name == "print" || name == "println" {
                return Ok(Type::unit());
            }

            // complex(r, i) - creates complex number from two float64
            if name == "complex" && args.len() == 2 {
                let r_ty = arg_ty(0);
                let i_ty = arg_ty(1);
                if r_ty.is_error() || i_ty.is_error() {
                    return Ok(Type::error());
                }
                self.unify(&r_ty, &Type::simple("float64"), &arg_span(0));
                self.unify(&i_ty, &Type::simple("float64"), &arg_span(1));
                return Ok(Type::simple("complex128"));
            }

            // real(c) - extracts real part of complex number
            if name == "real" && args.len() == 1 {
                let c_ty = arg_ty(0);
                if c_ty.is_error() {
                    return Ok(Type::error());
                }
                let c_ty = self.substitute(c_ty);
                match &c_ty {
                    Type::Con { sym, .. }
                        if sym.name == "complex128" || sym.name == "complex64" =>
                    {
                        let result = if sym.name == "complex128" {
                            "float64"
                        } else {
                            "float32"
                        };
                        return Ok(Type::simple(result));
                    }
                    _ => {
                        self.emit_error(SoppoError::Type {
                            message: format!("real requires complex argument; got {}", c_ty),
                            span: arg_span(0),
                        });
                        return Ok(Type::error());
                    }
                }
            }

            // imag(c) - extracts imaginary part of complex number
            if name == "imag" && args.len() == 1 {
                let c_ty = arg_ty(0);
                if c_ty.is_error() {
                    return Ok(Type::error());
                }
                let c_ty = self.substitute(c_ty);
                match &c_ty {
                    Type::Con { sym, .. }
                        if sym.name == "complex128" || sym.name == "complex64" =>
                    {
                        let result = if sym.name == "complex128" {
                            "float64"
                        } else {
                            "float32"
                        };
                        return Ok(Type::simple(result));
                    }
                    _ => {
                        self.emit_error(SoppoError::Type {
                            message: format!("imag requires complex argument; got {}", c_ty),
                            span: arg_span(0),
                        });
                        return Ok(Type::error());
                    }
                }
            }
        }

        // Check if this is a type conversion: TypeName(value) or pkg.TypeName(value)
        // Also handles slice type conversions like []byte(str)
        if let TypedExprKind::Ident(type_name) = &func.kind
            && (self.global_state.has_type(type_name)
                || Type::is_builtin_type(type_name)
                || Self::is_slice_type_conversion(type_name))
        {
            // Special case: generic type with type_args and no value args is a type reference
            // e.g., Option[int] used as a prefix for Option[int].None
            // Return the instantiated type - this is NOT an error
            if args.is_empty() && !type_args.is_empty() {
                let type_arg_types: Vec<Type> =
                    type_args.iter().map(|ta| self.resolve_type(ta)).collect();
                return Ok(Type::generic(type_name, type_arg_types));
            }

            // This is a type conversion, not a function call
            // Type conversions take exactly one argument
            if args.len() != 1 {
                self.emit_error(SoppoError::Type {
                    message: format!(
                        "Type conversion requires exactly 1 argument, but got {}",
                        args.len()
                    ),
                    span: expr_span,
                });
                return Ok(Type::error());
            }

            // Infer the argument type (we don't need to use it, just check it's valid)
            arg_ty(0);

            // Return the target type
            return Ok(Type::simple(type_name));
        }

        // Check if this is a call on an imported package: pkg.Func(args) or pkg.Type(value)
        if let TypedExprKind::PackageMember {
            pkg: pkg_name,
            member: name,
        } = &func.kind
        {
            // For Soppo imports, look up the function from GlobalCtxt
            if self.is_soppo_import(pkg_name) {
                // Mark the import as used
                self.mark_import_used(pkg_name);

                if let Some((func_ty, def_span, name_span, doc_comment)) =
                    self.lookup_soppo_function(pkg_name, name)
                {
                    // Record symbol for go-to-definition
                    self.record_symbol(
                        func.span,
                        SymbolInfo {
                            name: name.clone(),
                            ty: func_ty.clone(),
                            definition_span: def_span,
                            name_span,
                            kind: SymbolKind::Function,
                            doc_comment,
                            go_location: None,
                        },
                    );

                    // Found the function - collect arg types from typed_args
                    let arg_tys: Vec<(Option<Type>, Span)> = args
                        .iter()
                        .map(|(_, typed_arg, _)| {
                            if typed_arg.ty.is_error() {
                                (None, typed_arg.span)
                            } else {
                                (Some(typed_arg.ty.clone()), typed_arg.span)
                            }
                        })
                        .collect();

                    // Extract param types and return type from func_ty
                    if let Type::Func {
                        args: param_tys,
                        ret,
                        ..
                    } = &func_ty
                    {
                        // Check argument count
                        if arg_tys.len() != param_tys.len() {
                            self.emit_error(SoppoError::Type {
                                message: format!(
                                    "Function `{}` has {} arguments, but expected {}",
                                    name,
                                    arg_tys.len(),
                                    param_tys.len()
                                ),
                                span: func.span,
                            });
                            return Ok(Type::error());
                        }

                        // Check each argument type (skip those that had errors)
                        for ((_, param_ty), (arg_ty, arg_span)) in
                            param_tys.iter().zip(arg_tys.iter())
                        {
                            if let Some(arg_ty) = arg_ty {
                                self.unify(param_ty, arg_ty, arg_span);
                            }
                        }

                        return Ok(self.substitute(ret.as_ref().clone()));
                    }
                }

                // Try type conversion: pkg.Type(value)
                if let Some((ty, def_span, name_span, doc_comment)) =
                    self.lookup_soppo_type(pkg_name, name)
                {
                    // Record symbol for go-to-definition
                    self.record_symbol(
                        func.span,
                        SymbolInfo {
                            name: name.clone(),
                            ty: ty.clone(),
                            definition_span: def_span,
                            name_span,
                            kind: SymbolKind::Type,
                            doc_comment,
                            go_location: None,
                        },
                    );

                    if args.len() != 1 {
                        self.emit_error(SoppoError::Type {
                            message: format!(
                                "Type conversion requires exactly 1 argument, but got {}",
                                args.len()
                            ),
                            span: expr_span,
                        });
                        return Ok(Type::error());
                    }
                    arg_ty(0);
                    return Ok(ty);
                }

                // Not found in Soppo module
                self.emit_error(SoppoError::Type {
                    message: format!("`{}` not found in Soppo module `{}`", name, pkg_name),
                    span: func.span,
                });
                return Ok(Type::error());
            }

            // Look up the type from a Go package
            if let Some((ty, go_location, doc_comment)) = self.lookup_go_type(pkg_name, name) {
                // Record symbol for go-to-definition (with Go source location)
                self.record_symbol(
                    func.span,
                    SymbolInfo {
                        name: name.clone(),
                        ty: ty.clone(),
                        definition_span: None, // no Soppo definition span
                        name_span: None,       // no Soppo name span
                        kind: SymbolKind::Type,
                        doc_comment,
                        go_location,
                    },
                );

                // This is a type conversion
                if args.len() != 1 {
                    self.emit_error(SoppoError::Type {
                        message: format!(
                            "Type conversion requires exactly 1 argument, but got {}",
                            args.len()
                        ),
                        span: expr_span,
                    });
                    return Ok(Type::error());
                }

                // Infer the argument type (we don't need to use it, just check it's valid)
                arg_ty(0);

                // Return the target type
                return Ok(ty);
            }
        }

        // Regular function call - use the pre-inferred type from typed_func
        let func_ty = func.ty.clone();
        if func_ty.is_error() {
            return Ok(Type::error());
        }
        let func_ty = self.substitute(func_ty);

        // If this is a generic function call, instantiate it
        let func_ty = if let TypedExprKind::Ident(func_name) = &func.kind {
            // Clone generics to avoid borrow conflict
            let generics = self
                .global_state
                .lookup_function(func_name)
                .map(|f| f.generics.clone());

            if let Some(generics) = generics {
                if !generics.is_empty() {
                    // Build substitution map: generic param name -> type
                    let mut subst = std::collections::HashMap::new();
                    let mut had_error = false;
                    if !type_args.is_empty() {
                        // Explicit type args provided: validate constraints and use them
                        for (generic_param, type_arg) in generics.iter().zip(type_args.iter()) {
                            // Validate the type argument is a real type
                            if let Err(e) = self.validate_type_arg(type_arg) {
                                self.emit_error(e);
                                had_error = true;
                                continue;
                            }

                            let concrete_ty = self.resolve_type(type_arg);

                            // Validate constraint
                            if !generic_param.satisfies(&concrete_ty) {
                                self.emit_error(SoppoError::ConstraintNotSatisfied {
                                    ty: concrete_ty.to_string(),
                                    constraint: generic_param.constraint.clone(),
                                    hint: Self::constraint_hint(&generic_param.constraint),
                                    span: type_arg.span,
                                });
                                had_error = true;
                                continue;
                            }

                            subst.insert(generic_param.name.clone(), concrete_ty);
                        }
                    } else {
                        // No explicit type args: create fresh type variables for inference
                        for generic_param in &generics {
                            let ty_var = self.fresh_ty_var();
                            subst.insert(generic_param.name.clone(), ty_var);
                        }
                    }
                    if had_error {
                        return Ok(Type::error());
                    }
                    // Instantiate the function type
                    Self::instantiate_generic_type(&func_ty, &subst)
                } else {
                    func_ty
                }
            } else {
                func_ty
            }
        } else {
            func_ty
        };

        // Look up parameter info if this is a known function
        // Exclude variadic params (type name starts with "variadic" or "...")
        let (param_names, is_variadic): (Option<Vec<String>>, bool) =
            if let TypedExprKind::Ident(func_name) = &func.kind {
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
        let has_named = args.iter().any(|(name, _, _)| name.is_some());

        // Reorder arguments based on named arguments
        // Track spread flag along with typed args: (typed_expr, is_spread)
        let ordered_args: Vec<(&TypedExpr, bool)> = if !has_named {
            // All positional - just use them in order
            args.iter()
                .map(|(_, typed_arg, spread)| (typed_arg, *spread))
                .collect()
        } else if let Some(param_names) = &param_names {
            // We have named args and know parameter names - reorder
            // Rules:
            // - Named args reserve their specific slots first
            // - Positional args fill remaining slots in order
            // - Positional args after named args only allowed for variadic functions
            // - Extra positional args go to variadic
            let mut result: Vec<Option<(&TypedExpr, bool)>> = vec![None; param_names.len()];
            let mut variadic_args: Vec<(&TypedExpr, bool)> = Vec::new();
            let mut positional_args: Vec<(&TypedExpr, bool)> = Vec::new();
            let mut seen_named = false;

            // First pass: process named args to reserve their slots, collect positional args
            for (name, typed_arg, spread) in args {
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
                            result[idx] = Some((typed_arg, *spread));
                        } else {
                            return Err(SoppoError::Type {
                                message: format!("Unknown parameter name: `{}`", n),
                                span: *name_span,
                            });
                        }
                    }
                    None => {
                        // Positional after named - only allowed for variadic functions
                        if seen_named && !is_variadic {
                            return Err(SoppoError::Type {
                                message: "Positional argument cannot follow named argument (non-variadic function)".to_string(),
                                span: typed_arg.span,
                            });
                        }
                        positional_args.push((typed_arg, *spread));
                    }
                }
            }

            // Second pass: fill remaining slots with positional args
            let mut positional_iter = positional_args.into_iter();
            for slot in result.iter_mut() {
                if slot.is_none()
                    && let Some((typed_arg, spread)) = positional_iter.next()
                {
                    *slot = Some((typed_arg, spread));
                }
            }

            // Any remaining positional args go to variadic
            for (typed_arg, spread) in positional_iter {
                variadic_args.push((typed_arg, spread));
            }

            // Check all required params are provided
            let mut ordered = Vec::new();
            for (i, slot) in result.iter().enumerate() {
                match slot {
                    Some((typed_arg, spread)) => ordered.push((*typed_arg, *spread)),
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

        // Build argument type info from typed expressions
        // arg_tys: (Option<Type>, Span, is_nil, is_spread)
        let mut arg_tys: Vec<(Option<Type>, Span, bool, bool)> = Vec::new();
        for (typed_arg, is_spread) in &ordered_args {
            let is_nil = matches!(typed_arg.kind, TypedExprKind::Nil);
            if typed_arg.ty.is_error() {
                arg_tys.push((None, typed_arg.span, is_nil, *is_spread));
            } else {
                arg_tys.push((
                    Some(typed_arg.ty.clone()),
                    typed_arg.span,
                    is_nil,
                    *is_spread,
                ));
            }
        }

        // Check function call with detailed error spans
        match &func_ty {
            Type::Func {
                args: param_tys,
                ret,
                ..
            } => {
                // Check if last param is variadic
                let has_variadic = param_tys.last().is_some_and(|(_, last_ty)| {
                    matches!(last_ty, Type::Con { sym, .. } if sym.name == "variadic" || sym.name.starts_with("..."))
                });

                if has_variadic {
                    let fixed_params = &param_tys[..param_tys.len() - 1];
                    let (_, variadic_param_ty) = param_tys.last().expect("checked above");
                    let variadic_elem = if let Type::Con { args, .. } = variadic_param_ty {
                        args.first().cloned().unwrap_or(Type::simple("any"))
                    } else {
                        Type::simple("any")
                    };

                    // Check we have at least the fixed params
                    if arg_tys.len() < fixed_params.len() {
                        self.emit_error(SoppoError::Type {
                            message: format!(
                                "Function has {} arguments, but expected at least {}",
                                arg_tys.len(),
                                fixed_params.len()
                            ),
                            span: func.span,
                        });
                        return Ok(Type::error());
                    }

                    // Check fixed params (skip those with inference errors)
                    for ((_, param_ty), (arg_ty, arg_span, is_nil, _is_spread)) in
                        fixed_params.iter().zip(arg_tys.iter())
                    {
                        if let Some(arg_ty) = arg_ty {
                            // Check nil assignment
                            if *is_nil
                                && let Some(err) =
                                    Self::check_nil_to_non_nilable(param_ty, *arg_span)
                            {
                                self.emit_error(err);
                            }
                            self.unify(param_ty, arg_ty, arg_span);
                        }
                    }

                    // Check variadic args (skip those with inference errors)
                    for (arg_ty, arg_span, is_nil, is_spread) in
                        arg_tys.iter().skip(fixed_params.len())
                    {
                        if let Some(arg_ty) = arg_ty {
                            // For "any" type (or nullable any), any argument is valid
                            let is_any = match &variadic_elem {
                                Type::Con { sym, .. } => sym.name == "any",
                                _ => false,
                            };
                            if !is_any {
                                if *is_spread {
                                    // Spread arg: extract element type from slice
                                    // arg_ty should be []T or ?[]T, extract T
                                    let elem_ty = Self::extract_slice_element(arg_ty)
                                        .unwrap_or_else(|| {
                                            // If not a slice, unify directly (will fail with good error)
                                            arg_ty.clone()
                                        });
                                    self.unify(&variadic_elem, &elem_ty, arg_span);
                                } else {
                                    // Check nil assignment
                                    if *is_nil
                                        && let Some(err) = Self::check_nil_to_non_nilable(
                                            &variadic_elem,
                                            *arg_span,
                                        )
                                    {
                                        self.emit_error(err);
                                    }
                                    self.unify(&variadic_elem, arg_ty, arg_span);
                                }
                            }
                        }
                    }
                } else {
                    // Non-variadic: exact arg count required
                    if arg_tys.len() != param_tys.len() {
                        self.emit_error(SoppoError::Type {
                            message: format!(
                                "Function has {} arguments, but expected {}",
                                arg_tys.len(),
                                param_tys.len()
                            ),
                            span: func.span,
                        });
                        return Ok(Type::error());
                    }

                    // Check each argument type (skip those with inference errors)
                    for ((_, param_ty), (arg_ty, arg_span, is_nil, _is_spread)) in
                        param_tys.iter().zip(arg_tys.iter())
                    {
                        if let Some(arg_ty) = arg_ty {
                            // Check nil assignment
                            if *is_nil
                                && let Some(err) =
                                    Self::check_nil_to_non_nilable(param_ty, *arg_span)
                            {
                                self.emit_error(err);
                            }
                            self.unify(param_ty, arg_ty, arg_span);
                        }
                    }
                }

                Ok(self.substitute(ret.as_ref().clone()))
            }
            Type::Var(_) => {
                // Function type is unknown, use standard unification
                // Filter out arguments that had inference errors
                let result_ty = self.fresh_ty_var();
                let arg_types: Vec<Type> =
                    arg_tys.into_iter().filter_map(|(ty, _, _, _)| ty).collect();
                let expected_func_ty = Type::fun(arg_types, result_ty.clone());
                self.unify(&func_ty, &expected_func_ty, &expr_span);
                Ok(self.substitute(result_ty))
            }
            _ => {
                self.emit_error(SoppoError::Type {
                    message: format!("Cannot call non-function type `{}`", func_ty),
                    span: func.span,
                });
                Ok(Type::error())
            }
        }
    }

    /// Infer the type of a unary expression
    fn infer_unary(&mut self, op: &UnaryOp, operand: &Expr, span: Span) -> Result<TypedExpr> {
        let typed_operand = self.infer_expr(operand);
        if typed_operand.is_error() {
            return Ok(TypedExpr::new(
                TypedExprKind::Unary {
                    op: *op,
                    operand: Box::new(typed_operand),
                },
                Type::error(),
                span,
            ));
        }

        let result_ty = match op {
            UnaryOp::Neg => {
                // -x: operand must be numeric, result is same type
                typed_operand.ty.clone()
            }
            UnaryOp::Not => {
                // !x: operand must be bool, result is bool
                self.unify(&typed_operand.ty, &Type::simple("bool"), &operand.span);
                Type::simple("bool")
            }
            UnaryOp::Ref => {
                // &x: result is *T where T is the operand type
                let operand_ty = self.substitute(typed_operand.ty.clone());
                let ptr_name = format!("*{}", operand_ty);
                Type::generic(&ptr_name, vec![operand_ty])
            }
            UnaryOp::Deref => {
                // *p: operand must be *T, result is T
                let operand_ty = self.substitute(typed_operand.ty.clone());

                // Check for nil pointer dereference (only pointers can be dereferenced)
                // Skip if operand is a NilAssert - that explicitly asserts non-nil
                // Skip if type is non-nullable in Soppo (*T vs ?*T) - non-nullable types can't be nil
                if Self::is_pointer_type(&operand_ty)
                    && operand_ty.is_nullable()
                    && !Self::is_nil_asserted(operand)
                {
                    // Get a key for the expression (works for identifiers and field chains)
                    let expr_key = super::stmt::expr_to_key(operand);

                    // Check nil state for the expression, or assume nullable for complex expressions
                    let is_nullable = match &expr_key {
                        Some(key) => self.get_nil_state(key) == Nullability::Nullable,
                        None => true, // Complex expressions are conservatively nullable
                    };

                    if is_nullable {
                        let name_for_error = expr_key.unwrap_or_else(|| "expression".to_string());
                        // Use a more specific span: for field access, point to just the field name
                        let error_span = match &operand.kind {
                            ExprKind::Field {
                                span: field_span, ..
                            } => *field_span,
                            _ => operand.span,
                        };
                        self.emit_error(SoppoError::NilPointer {
                            name: name_for_error,
                            span: error_span,
                        });
                    }
                }

                // Extract the pointee type from *T
                if let Some(pointee_ty) = Self::extract_pointer_element(&operand_ty) {
                    pointee_ty
                } else {
                    // If we can't determine the pointer type, return a type variable
                    self.fresh_ty_var()
                }
            }
            UnaryOp::Recv => {
                // <-ch: operand must be chan T, result is T
                let operand_ty = self.substitute(typed_operand.ty.clone());
                // Extract the element type from chan T
                if let Some(elem_ty) = Self::extract_channel_element(&operand_ty) {
                    elem_ty
                } else {
                    // If we can't determine the channel type, return a type variable
                    self.fresh_ty_var()
                }
            }
        };

        Ok(TypedExpr::new(
            TypedExprKind::Unary {
                op: *op,
                operand: Box::new(typed_operand),
            },
            result_ty,
            span,
        ))
    }

    /// Parse anonymous struct field definitions from a string like "struct{a int; b string}"
    /// Returns None if parsing fails
    fn parse_anon_struct_fields(&self, s: &str) -> Option<Vec<(String, Type)>> {
        // Remove "struct{" prefix and "}" suffix
        let inner = s.strip_prefix("struct{")?.strip_suffix('}')?;
        if inner.is_empty() {
            return Some(Vec::new());
        }

        let mut fields = Vec::new();
        // Split by semicolons (field separator in Go struct types)
        for field_def in inner.split(';') {
            let field_def = field_def.trim();
            if field_def.is_empty() {
                continue;
            }
            // Each field is "name type" separated by whitespace
            let parts: Vec<&str> = field_def.splitn(2, ' ').collect();
            if parts.len() == 2 {
                let name = parts[0].trim().to_string();
                let ty_str = parts[1].trim();
                fields.push((name, Type::simple(ty_str)));
            }
        }

        Some(fields)
    }
}

/// Validate a format specifier against an expression type.
/// Returns Ok(()) if valid, Err(message) if invalid.
fn validate_format_specifier(format: &str, ty: &Type) -> std::result::Result<(), String> {
    // Extract the base verb from the format (e.g., "d" from "010d", "f" from ".2f")
    let base_verb = extract_base_verb(format);

    // Check if the type is compatible with the format verb
    match base_verb {
        // Integer verbs
        "d" | "b" | "o" | "x" | "X" | "c" => {
            if !is_integer_type(ty) && !is_rune_type(ty) {
                return Err(format!(
                    "format `%{}` requires integer type, found `{}`",
                    format, ty
                ));
            }
        }
        // Float verbs
        "f" | "F" | "e" | "E" | "g" | "G" => {
            if !is_float_type(ty) {
                return Err(format!(
                    "format `%{}` requires float type, found `{}`",
                    format, ty
                ));
            }
        }
        // String verbs
        "s" => {
            // %s works with string, []byte, and any type that implements Stringer
            // For simplicity, we allow string, []byte, and any interface
            if !is_string_type(ty) && !is_byte_slice_type(ty) && !is_interface_type(ty) {
                return Err(format!(
                    "format `%{}` requires string or []byte, found `{}`",
                    format, ty
                ));
            }
        }
        "q" => {
            // %q works with string, []byte, and rune
            if !is_string_type(ty) && !is_byte_slice_type(ty) && !is_rune_type(ty) {
                return Err(format!(
                    "format `%{}` requires string, []byte, or rune, found `{}`",
                    format, ty
                ));
            }
        }
        // Bool verb
        "t" => {
            if !is_bool_type(ty) {
                return Err(format!(
                    "format `%{}` requires bool, found `{}`",
                    format, ty
                ));
            }
        }
        // Pointer verb
        "p" => {
            if !is_pointer_type(ty) && !is_slice_type(ty) && !is_map_type(ty) {
                return Err(format!(
                    "format `%{}` requires pointer, slice, or map, found `{}`",
                    format, ty
                ));
            }
        }
        // Universal verbs - always valid
        "v" | "+v" | "#v" => {}
        // Unknown verb
        _ => {
            return Err(format!("unknown format verb `%{}`", format));
        }
    }

    Ok(())
}

/// Extract the base verb from a format specifier.
/// E.g., "d" from "010d", "f" from ".2f", "x" from "#x"
fn extract_base_verb(format: &str) -> &str {
    // Handle special cases for +v and #v
    if format == "+v" || format == "#v" {
        return format;
    }

    // Find the last character that is a letter - that's the verb
    let verb_start = format.rfind(|c: char| c.is_ascii_alphabetic()).unwrap_or(0);
    &format[verb_start..]
}

fn get_type_name(ty: &Type) -> Option<&str> {
    match ty {
        Type::Con { sym, .. } => Some(&sym.name),
        _ => None,
    }
}

fn is_integer_type(ty: &Type) -> bool {
    matches!(
        get_type_name(ty),
        Some(
            "int"
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
        )
    )
}

fn is_rune_type(ty: &Type) -> bool {
    matches!(get_type_name(ty), Some("rune"))
}

fn is_float_type(ty: &Type) -> bool {
    matches!(get_type_name(ty), Some("float32" | "float64"))
}

fn is_string_type(ty: &Type) -> bool {
    matches!(get_type_name(ty), Some("string"))
}

fn is_bool_type(ty: &Type) -> bool {
    matches!(get_type_name(ty), Some("bool"))
}

fn is_byte_slice_type(ty: &Type) -> bool {
    if let Type::Con { sym, args, .. } = ty
        && sym.name == "slice"
        && args.len() == 1
    {
        return matches!(get_type_name(&args[0]), Some("byte" | "uint8"));
    }
    false
}

fn is_slice_type(ty: &Type) -> bool {
    matches!(ty, Type::Con { sym, .. } if sym.name == "slice")
}

fn is_map_type(ty: &Type) -> bool {
    matches!(ty, Type::Con { sym, .. } if sym.name == "map")
}

fn is_pointer_type(ty: &Type) -> bool {
    matches!(ty, Type::Con { sym, .. } if sym.name == "ptr")
}

fn is_interface_type(ty: &Type) -> bool {
    matches!(ty, Type::Con { sym, .. } if sym.name == "interface")
        || matches!(get_type_name(ty), Some("error" | "any"))
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
        let ty = infer.infer_expr_inner(&expr)?.ty;
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
        let mut parser = Parser::new(r#"1 + "hello""#, FileId(0));
        let expr = parser.parse_expr().unwrap();
        let mut infer = Infer::new().expect("Failed to create Infer");
        let ty = infer.infer_expr_inner(&expr).unwrap();
        // With error-collecting, we get Type::error() and errors are collected
        assert!(ty.is_error() || infer.has_errors());
    }

    #[test]
    fn test_array_literal_type() {
        // Test that array literals have proper array type
        let source = "[5]int{1, 2, 3, 4, 5}";
        let mut parser = Parser::new(source, FileId(0));
        let expr = parser.parse_expr().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");
        let ty = infer.infer_expr_inner(&expr).unwrap().ty;

        // Should be array[int]
        if let Type::Con { sym, args, .. } = ty {
            assert_eq!(sym.name, "array");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0], Type::simple("int"));
        } else {
            panic!("Expected array type, got: {:?}", ty);
        }
    }
}
