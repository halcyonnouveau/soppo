use std::collections::HashMap;

use super::Infer;
use crate::error::{Result, SoppoError};
use crate::syntax::{BinOp, EnumVariant, Expr, ExprKind, ModuleId, Span, Symbol, UnaryOp};
use crate::types::Type;
use crate::types::ctx::TypeDefKind;
use crate::types::symbols::{SymbolInfo, SymbolKind};
use crate::types::ty::Nullability;

impl Infer {
    /// Infer the type of an expression (internal version that returns Result).
    ///
    /// **Prefer `infer_expr`** which collects errors and returns `Type::Error` on failure.
    /// This version should only be used when you need to explicitly check if inference failed.
    pub fn infer_expr_inner(&mut self, expr: &Expr) -> Result<Type> {
        match &expr.kind {
            ExprKind::Integer(_, _) => Ok(Type::simple("int")),

            ExprKind::Float(_) => Ok(Type::simple("float64")),

            ExprKind::String(_) => Ok(Type::simple("string")),

            ExprKind::RawString(_) => Ok(Type::simple("string")),

            ExprKind::Rune(_) => Ok(Type::simple("rune")),

            ExprKind::StringInterpolation(parts) => {
                // Type check each interpolated expression and validate format specifiers
                let mut had_error = false;
                for part in parts {
                    if let crate::syntax::StringPart::Expr { expr, format } = part {
                        let ty = self.infer_expr(expr);
                        if ty.is_error() {
                            had_error = true;
                            continue;
                        }

                        // Validate format specifier against the expression type
                        if let Some(fmt) = format
                            && let Err(msg) = validate_format_specifier(fmt, &ty)
                        {
                            self.emit_error(SoppoError::Type {
                                message: msg,
                                span: expr.span,
                            });
                            had_error = true;
                        }
                    }
                }
                if had_error {
                    return Ok(Type::error());
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
                    return Ok(ty);
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
                    return Ok(ty);
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
                    return Ok(const_def.ty);
                }

                Err(SoppoError::UndefinedVariable {
                    name: name.clone(),
                    span: expr.span,
                })
            }

            ExprKind::Binary { op, left, right } => {
                // For && operator, apply short-circuit narrowing:
                // In `x != nil && f(x)`, x is known non-nil when evaluating f(x)
                // Left was TRUE, so apply narrowing as-is
                if matches!(op, BinOp::And) {
                    let left_ty = self.infer_expr(left);
                    if left_ty.is_error() {
                        // Still infer right for more error collection
                        self.push_nil_scope();
                        self.infer_expr(right);
                        self.pop_nil_scope();
                        return Ok(Type::error());
                    }
                    self.unify(&left_ty, &Type::simple("bool"), &left.span);

                    // Extract nil checks from left side and apply narrowing for right side
                    let nil_checks = super::stmt::extract_nil_checks(left);
                    self.push_nil_scope();
                    for check in &nil_checks {
                        if check.is_not_nil {
                            self.set_nil_state(check.expr_key.clone(), Nullability::NonNull);
                        }
                    }
                    let right_ty = self.infer_expr(right);
                    self.pop_nil_scope();
                    if right_ty.is_error() {
                        return Ok(Type::error());
                    }
                    self.unify(&right_ty, &Type::simple("bool"), &right.span);

                    return Ok(Type::simple("bool"));
                }

                // For || operator, apply short-circuit narrowing with OPPOSITE logic:
                // In `x == nil || f(x)`, x is known non-nil when evaluating f(x)
                // Left was FALSE, so apply the opposite narrowing
                if matches!(op, BinOp::Or) {
                    let left_ty = self.infer_expr(left);
                    if left_ty.is_error() {
                        // Still infer right for more error collection
                        self.push_nil_scope();
                        self.infer_expr(right);
                        self.pop_nil_scope();
                        return Ok(Type::error());
                    }
                    self.unify(&left_ty, &Type::simple("bool"), &left.span);

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
                    let right_ty = self.infer_expr(right);
                    self.pop_nil_scope();
                    if right_ty.is_error() {
                        return Ok(Type::error());
                    }
                    self.unify(&right_ty, &Type::simple("bool"), &right.span);

                    return Ok(Type::simple("bool"));
                }

                let left_ty = self.infer_expr(left);
                let right_ty = self.infer_expr(right);

                // If either operand failed, return error
                if left_ty.is_error() || right_ty.is_error() {
                    return Ok(Type::error());
                }

                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                        // Arithmetic: try normal unification first
                        // Point error at right operand since left is typically the "expected" type
                        if self.unify_inner(&left_ty, &right_ty, &right.span).is_ok() {
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

                        // Neither worked - emit the unification error and return error type
                        self.unify(&left_ty, &right_ty, &right.span);
                        Ok(Type::error())
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        // Comparison: both must be same type, result is bool
                        self.unify(&left_ty, &right_ty, &expr.span);
                        Ok(Type::simple("bool"))
                    }
                    BinOp::And | BinOp::Or => {
                        // Logical: both must be bool, result is bool (handled above for narrowing)
                        self.unify(&left_ty, &Type::simple("bool"), &left.span);
                        self.unify(&right_ty, &Type::simple("bool"), &right.span);
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
                            self.unify(&left_ty, &right_ty, &right.span);
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
                let container_ty = self.infer_expr(expr);
                let index_ty = self.infer_expr(index);

                // If either failed, return error
                if container_ty.is_error() || index_ty.is_error() {
                    return Ok(Type::error());
                }

                let container_ty = self.substitute(container_ty);

                // Map indexing: map[K]V - index is K, result is V
                if let Some((key_ty, val_ty)) = Self::extract_map_elements(&container_ty) {
                    self.unify(&index_ty, &key_ty, &index.span);
                    return Ok(val_ty);
                }

                // Slice indexing: []T - index is int, result is T
                if let Some(elem_ty) = Self::extract_slice_element(&container_ty) {
                    self.unify(&index_ty, &Type::simple("int"), &index.span);
                    return Ok(elem_ty);
                }

                if let Type::Con { sym, args, .. } = &container_ty {
                    // Array indexing: array or [N]T - index is int
                    if sym.name == "array" && args.len() == 1 {
                        self.unify(&index_ty, &Type::simple("int"), &index.span);
                        return Ok(args[0].clone());
                    }

                    // String indexing - index is int, result is byte
                    if sym.name == "string" {
                        self.unify(&index_ty, &Type::simple("int"), &index.span);
                        return Ok(Type::simple("byte"));
                    }
                }

                // Default: assume int index
                self.unify(&index_ty, &Type::simple("int"), &index.span);
                Ok(self.fresh_ty_var())
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
                    let first_ty = self.infer_expr(&elements[0]);
                    if first_ty.is_error() {
                        // Still check remaining elements for more errors
                        for elem in elements.iter().skip(1) {
                            self.infer_expr(elem);
                        }
                        return Ok(Type::error());
                    }
                    (first_ty, None, None)
                } else {
                    (self.fresh_ty_var(), None, None)
                };

                let mut had_error = false;

                // All elements must have the same type
                for elem in elements {
                    // Check: assigning nil to a non-nilable element type is an error
                    if matches!(elem.kind, ExprKind::Nil)
                        && let Some(err) = Self::check_nil_to_non_nilable(&elem_ty, elem.span)
                    {
                        self.emit_error(err);
                        had_error = true;
                        continue;
                    }

                    // Special handling for implicit StructLit with positional fields
                    // when element type is an anonymous struct
                    if let (
                        Some(field_defs),
                        ExprKind::StructLit {
                            ty: None,
                            fields: struct_fields,
                        },
                    ) = (&anon_struct_fields, &elem.kind)
                    {
                        // Check positional fields against struct field definitions
                        for (i, (field_name, value)) in struct_fields.iter().enumerate() {
                            let value_ty = self.infer_expr(value);
                            if value_ty.is_error() {
                                had_error = true;
                                continue;
                            }
                            match field_name {
                                Some(name) => {
                                    // Named field - look up by name
                                    if let Some((_, expected_ty)) =
                                        field_defs.iter().find(|(n, _)| n == name)
                                    {
                                        self.unify(expected_ty, &value_ty, &value.span);
                                    }
                                }
                                None => {
                                    // Positional field - look up by index
                                    if let Some((_, expected_ty)) = field_defs.get(i) {
                                        self.unify(expected_ty, &value_ty, &value.span);
                                    }
                                }
                            }
                        }
                    } else {
                        let elem_ty_actual = self.infer_expr(elem);
                        if !elem_ty_actual.is_error() {
                            self.unify(&elem_ty, &elem_ty_actual, &elem.span);
                        } else {
                            had_error = true;
                        }
                    }
                }

                if had_error {
                    return Ok(Type::error());
                }

                // Return proper slice/array type
                if let Some(declared_ty) = declared_type {
                    Ok(declared_ty)
                } else if ty.is_some() {
                    // Explicit type without [] prefix - use array type
                    Ok(Type::array(elem_ty))
                } else {
                    // Implicit composite literal (no type specified) - use slice type
                    // This handles cases like [][]int{{1, 2}, {3, 4}} where {1, 2} is implicit
                    Ok(Type::slice(elem_ty))
                }
            }

            ExprKind::StructLit { ty, fields } => {
                // Handle implicit struct literal (ty is None) - type inferred from context
                let Some(ty) = ty else {
                    // Just type check field values, return a fresh type variable
                    // The type will be unified with the expected type from context
                    let mut had_error = false;
                    for (_field_name, value) in fields {
                        let value_ty = self.infer_expr(value);
                        if value_ty.is_error() {
                            had_error = true;
                        }
                    }
                    if had_error {
                        return Ok(Type::error());
                    }
                    return Ok(self.fresh_ty_var());
                };

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

                        let mut had_error = false;

                        // Type check each field
                        for (field_name, value) in fields {
                            if let Some(name) = field_name
                                && let Some(field_ty) = field_types.get(name.as_str())
                            {
                                // Check: assigning nil to a non-nilable field is an error
                                if matches!(value.kind, ExprKind::Nil)
                                    && let Some(err) =
                                        Self::check_nil_to_non_nilable(field_ty, value.span)
                                {
                                    self.emit_error(err);
                                    had_error = true;
                                    continue;
                                }
                            }
                            // TODO: Handle positional fields (field_name = None)
                            let value_ty = self.infer_expr(value);
                            if value_ty.is_error() {
                                had_error = true;
                            }
                        }

                        if had_error {
                            return Ok(Type::error());
                        }
                    }
                } else {
                    // Fallback: just type check values without nil check
                    let mut had_error = false;
                    for (_field_name, value) in fields {
                        let value_ty = self.infer_expr(value);
                        if value_ty.is_error() {
                            had_error = true;
                        }
                    }
                    if had_error {
                        return Ok(Type::error());
                    }
                }

                // Check if this is a qualified type (e.g., pkg.Type)
                if ty.name.contains('.') {
                    let parts: Vec<&str> = ty.name.split('.').collect();
                    if parts.len() == 2 {
                        let pkg_name = parts[0];
                        let type_name = parts[1];

                        // Check if it's an enum variant (e.g., Shape.Circle)
                        if self.global_state.is_local_enum(pkg_name) {
                            // Check if this is a generic enum with type args
                            if !ty.args.is_empty() {
                                // Generic enum struct literal: Option[int].Some{...}
                                let type_arg_types: Vec<Type> =
                                    ty.args.iter().map(|ta| self.resolve_type(ta)).collect();
                                return Ok(Type::generic(pkg_name, type_arg_types));
                            }

                            // Check if the enum is generic - if so, use fresh type variables
                            let generics_count = self
                                .global_state
                                .lookup_type(pkg_name)
                                .map(|td| td.generics.len())
                                .unwrap_or(0);
                            if generics_count > 0 {
                                let type_vars: Vec<Type> =
                                    (0..generics_count).map(|_| self.fresh_ty_var()).collect();
                                return Ok(Type::generic(pkg_name, type_vars));
                            }
                            return Ok(Type::simple(pkg_name));
                        }

                        // Check if it's a cross-package enum variant
                        if self.global_state.is_soppo_enum(pkg_name, type_name) {
                            return Ok(Type::Con {
                                sym: Symbol {
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
                            sym: Symbol {
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
                        let k_ty = self.infer_expr(k);
                        let v_ty = self.infer_expr(v);
                        if k_ty.is_error() || v_ty.is_error() {
                            // Still check remaining entries for more errors
                            for (key, value) in entries.iter().skip(1) {
                                self.infer_expr(key);
                                self.infer_expr(value);
                            }
                            return Ok(Type::error());
                        }
                        (k_ty, v_ty)
                    } else {
                        (self.fresh_ty_var(), self.fresh_ty_var())
                    }
                };

                let mut had_error = false;

                // Type check all entries
                for (key, value) in entries {
                    // Check: assigning nil to a non-nilable value type is an error
                    if matches!(value.kind, ExprKind::Nil)
                        && let Some(err) = Self::check_nil_to_non_nilable(&val_ty, value.span)
                    {
                        self.emit_error(err);
                        had_error = true;
                        // Still infer the key
                        self.infer_expr(key);
                        continue;
                    }
                    let k_ty = self.infer_expr(key);
                    let v_ty = self.infer_expr(value);
                    if !k_ty.is_error() {
                        self.unify(&key_ty, &k_ty, &key.span);
                    } else {
                        had_error = true;
                    }
                    if !v_ty.is_error() {
                        self.unify(&val_ty, &v_ty, &value.span);
                    } else {
                        had_error = true;
                    }
                }

                if had_error {
                    return Ok(Type::error());
                }

                // Return map[K]V type with proper Go format in name
                let map_name = format!("map[{}]{}", key_ty, val_ty);
                Ok(Type::generic(&map_name, vec![key_ty, val_ty]))
            }

            ExprKind::Unary { op, operand } => self.infer_unary(op, operand),

            ExprKind::FuncLit {
                params,
                returns,
                body,
            } => {
                // Save the current expected return types
                let prev_expected = self.expected_return_types.take();

                // Create a new scope for the function body
                self.push_scope();

                // Add parameters to scope - use resolve_type for proper qualified type handling
                for param in params {
                    let param_ty = self.resolve_type(&param.ty);
                    if let Err(e) =
                        self.insert_var(param.ident.name.clone(), param_ty, Some(param.ident.span))
                    {
                        self.emit_error(e);
                    }
                }

                // Set expected return types for this function
                let expected_ret_types: Vec<Type> =
                    returns.iter().map(|r| self.resolve_type(&r.ty)).collect();
                if !expected_ret_types.is_empty() {
                    self.expected_return_types = Some(expected_ret_types.clone());
                }

                // Infer body
                self.infer_block(body);

                self.pop_scope();

                // Restore previous expected return types
                self.expected_return_types = prev_expected;

                // Build function type - use resolve_type for proper qualified type handling
                let param_types: Vec<Type> =
                    params.iter().map(|p| self.resolve_type(&p.ty)).collect();
                let ret_ty = if returns.is_empty() {
                    Type::unit()
                } else if returns.len() == 1 {
                    self.resolve_type(&returns[0].ty)
                } else {
                    // Multiple return types - use a tuple type
                    let ret_types: Vec<Type> =
                        returns.iter().map(|r| self.resolve_type(&r.ty)).collect();
                    Type::generic("tuple", ret_types)
                };

                Ok(Type::fun(param_types, ret_ty))
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

                // Type check each field value against its declared type
                for (i, (field_name, value)) in fields.iter().enumerate() {
                    let value_ty = self.infer_expr(value);
                    if value_ty.is_error() {
                        had_error = true;
                        continue;
                    }
                    match field_name {
                        Some(name) => {
                            if let Some(expected_ty) = field_types.get(name) {
                                self.unify(expected_ty, &value_ty, &value.span);
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
                                self.unify(expected_ty, &value_ty, &value.span);
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
                }

                if had_error {
                    return Ok(Type::error());
                }

                // Build a unique type name for this anonymous struct
                // We'll use a hash or just generate inline in codegen
                // For type checking, we use a structural anonymous type
                let field_type_list: Vec<(String, Type)> = field_defs
                    .iter()
                    .map(|f| (f.ident.name.clone(), self.resolve_type(&f.ty)))
                    .collect();

                Ok(Type::anon_struct(field_type_list))
            }

            ExprKind::Block(block) => Ok(self.infer_block(block)),

            ExprKind::Slice {
                expr,
                low,
                high,
                cap,
            } => {
                // Slicing returns the same type as the sliced expression
                let expr_ty = self.infer_expr(expr);

                let mut had_error = expr_ty.is_error();

                // Check that indices are integers
                if let Some(l) = low {
                    let l_ty = self.infer_expr(l);
                    if !l_ty.is_error() {
                        self.unify(&l_ty, &Type::simple("int"), &l.span);
                    } else {
                        had_error = true;
                    }
                }
                if let Some(h) = high {
                    let h_ty = self.infer_expr(h);
                    if !h_ty.is_error() {
                        self.unify(&h_ty, &Type::simple("int"), &h.span);
                    } else {
                        had_error = true;
                    }
                }
                if let Some(c) = cap {
                    let c_ty = self.infer_expr(c);
                    if !c_ty.is_error() {
                        self.unify(&c_ty, &Type::simple("int"), &c.span);
                    } else {
                        had_error = true;
                    }
                }

                if had_error {
                    return Ok(Type::error());
                }

                Ok(expr_ty)
            }

            ExprKind::TypeAssert {
                expr,
                ty,
                known_match,
            } => {
                // Type assertions return the asserted type directly.
                // Like Rust pattern matching - variable is typed as the enum,
                // and assertions extract the variant data.
                //
                // For enum variants: panics if wrong variant (use comma-ok for safe check)
                // For Go interfaces: panics if wrong type (use comma-ok for safe check)
                let inner_ty = self.infer_expr(expr);
                if inner_ty.is_error() {
                    return Ok(Type::error());
                }

                let is_enum_variant = ty.name.contains('.');

                if is_enum_variant {
                    let inner_ty = Type::simple(&ty.name.replace('.', "_"));

                    // if we know the variant matches, mark it so codegen can skip the runtime assertion
                    if let ExprKind::Ident(name) = &expr.kind
                        && let Some(known) = self.get_variant_state(name)
                        && known == &ty.name
                    {
                        known_match.set(true);
                    }

                    Ok(inner_ty)
                } else {
                    let target_ty = self.resolve_type(ty);
                    Ok(target_ty)
                }
            }

            ExprKind::NilAssert { expr } => {
                // Nil assertion: x.(!nil) - assert the expression is non-nil
                let ty = self.infer_expr_inner(expr)?;
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

            ExprKind::Paren(inner) => {
                // Parenthesised expression has the type of its inner expression
                self.infer_expr_inner(inner)
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
            // Record symbol for the package name itself (e.g., "bufio" in bufio.NewScanner)
            // Go-to-definition on the package name goes to the import statement
            if let Some((import_path, import_span)) = self.get_import_info(name) {
                self.record_symbol(
                    field_expr.span,
                    SymbolInfo {
                        name: name.to_string(),
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
            if self.is_soppo_import(name) {
                // Mark the import as used
                self.mark_import_used(name);

                // Try to look up as a function first
                if let Some((func_ty, def_span, name_span, doc_comment)) =
                    self.lookup_soppo_function(name, field)
                {
                    // Record symbol for go-to-definition
                    self.record_symbol(
                        *field_span,
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
                    return Ok(func_ty);
                }
                // Try to look up as a type
                if let Some((ty, def_span, name_span, doc_comment)) =
                    self.lookup_soppo_type(name, field)
                {
                    // Record symbol for go-to-definition
                    self.record_symbol(
                        *field_span,
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
                    return Ok(ty);
                }
                // Try to look up as a constant
                if let Some((ty, def_span, name_span, doc_comment)) =
                    self.lookup_soppo_constant(name, field)
                {
                    // Record symbol for go-to-definition
                    self.record_symbol(
                        *field_span,
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
                    return Ok(ty);
                }
                // Not found
                return Err(SoppoError::Type {
                    message: format!("`{}` not found in Soppo module `{}`", field, name),
                    span: *field_span,
                });
            }

            // Go packages: try to look up as a function first
            if let Some((func_ty, go_location, doc_comment)) = self.lookup_go_function(name, field)
            {
                // Record symbol for go-to-definition (with Go source location)
                self.record_symbol(
                    *field_span,
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
                return Ok(func_ty);
            }
            // Try to look up as a type or constant
            if let Some((ty, go_location, doc_comment)) = self.lookup_go_type(name, field) {
                // Record symbol for go-to-definition (with Go source location)
                self.record_symbol(
                    *field_span,
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
                return Ok(ty);
            }
            // Couldn't find it - error
            return Err(SoppoError::Type {
                message: format!("`{}` not found in package `{}`", field, name),
                span: *field_span,
            });
        }

        // Check if this is a generic enum variant like Option[int].None or Option[int].Some
        // The parser generates Call { func: Ident(type_name), type_args, args: [] } for Option[int]
        if let ExprKind::Call {
            func,
            type_args,
            args,
        } = &field_expr.kind
            && let ExprKind::Ident(type_name) = &func.kind
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
                    span: field_expr.span,
                });
            }

            // Validate and collect type arg substitutions
            let mut generic_subst: HashMap<String, Type> = HashMap::new();
            for (generic_param, type_arg) in type_def.generics.iter().zip(type_args.iter()) {
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
                generic_subst.insert(generic_param.name.clone(), concrete_ty);
            }

            // Find the variant
            for variant in variants {
                let variant_name = match variant {
                    EnumVariant::Unit { ident, .. } => &ident.name,
                    EnumVariant::Single { ident, .. } => &ident.name,
                    EnumVariant::Struct { ident, .. } => &ident.name,
                };

                if variant_name == field {
                    // Build the return type with type arguments: Option[int]
                    let type_arg_types: Vec<Type> =
                        type_args.iter().map(|ta| self.resolve_type(ta)).collect();
                    let return_ty = Type::generic(type_name, type_arg_types);

                    return match variant {
                        EnumVariant::Unit { .. } => {
                            // Unit variant with type args: Option[int].None
                            Ok(return_ty)
                        }
                        EnumVariant::Single { ty, .. } => {
                            // Single variant: Option[int].Some -> fn(int) -> Option[int]
                            let ty_simple = Type::simple(&ty.name);
                            let param_ty =
                                Self::instantiate_generic_type(&ty_simple, &generic_subst);
                            Ok(Type::fun(vec![param_ty], return_ty))
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
                            Ok(Type::fun(param_tys, return_ty))
                        }
                    };
                }
            }

            // Variant not found
            return Err(SoppoError::Type {
                message: format!("Enum `{}` has no variant `{}`", type_name, field),
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
                        .map(|g| (g.name.clone(), self.fresh_ty_var()))
                        .collect();

                    // Build the return type - use generic type for generic enums
                    let return_ty = if type_def.generics.is_empty() {
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
                            // Found the variant
                            return match variant {
                                EnumVariant::Unit { .. } => {
                                    // Unit variant: return the enum type
                                    // For generic enums, the type vars will be inferred from context
                                    Ok(return_ty)
                                }
                                EnumVariant::Single { ty, .. } => {
                                    // Single variant: returns a constructor function
                                    // Ok(T) -> fn(T) -> Result[T, E]
                                    // Instantiate generic params with fresh type vars
                                    let ty_simple = Type::simple(&ty.name);
                                    let param_ty =
                                        Self::instantiate_generic_type(&ty_simple, &generic_subst);
                                    Ok(Type::fun(vec![param_ty], return_ty))
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
        let expr_ty = self.infer_expr(field_expr);
        if expr_ty.is_error() {
            return Ok(Type::error());
        }
        let expr_ty = self.substitute(expr_ty);

        // Check for nil dereference on field access
        // If the expression is a nilable type, verify it's not nullable
        // Skip check if expression is a NilAssert - that explicitly makes it non-null
        // Skip if type is non-nullable in Soppo (*T vs ?*T) - non-nullable types can't be nil
        if Self::is_nilable_type(&expr_ty)
            && expr_ty.is_nullable()
            && !matches!(field_expr.kind, ExprKind::NilAssert { .. })
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
                self.emit_error(SoppoError::NilPointer {
                    name: name_for_error,
                    span: field_expr.span,
                });
            }
        }

        // Handle built-in error type's Error() method
        if let Type::Con { sym, .. } = &expr_ty
            && sym.name == "error"
            && field == "Error"
        {
            // error.Error() returns string
            return Ok(Type::fun(vec![], Type::simple("string")));
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
            return Ok(field_ty);
        }

        // Check if this is a method call on a Go package type
        if let (Some(struct_name), Some(module_name)) = (&struct_name, &module_name)
            && let Some((method_ty, go_location, doc_comment)) =
                self.lookup_go_method(module_name, struct_name, field)
        {
            // Record symbol for go-to-definition (with Go source location)
            self.record_symbol(
                *field_span,
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
            return Ok(method_ty);
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
                    *field_span,
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
                return Ok(method_ty);
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
            if let Some(type_def) = self.global_state.lookup_type(struct_name).cloned()
                && let TypeDefKind::Struct { fields } = &type_def.kind
            {
                // Check if the field exists
                if let Some((_, field_ty)) = fields.iter().find(|(f, _)| f == field) {
                    // Record field access for LSP
                    self.record_symbol(
                        *field_span,
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
                    return Ok(field_ty.clone());
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
                            *field_span,
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
                        return Ok(method_ty);
                    }

                    // Not a method - check if it might be a UFCS function call
                    // If we can find a function with this name, return a type variable
                    // and let the Call handler deal with it
                    if self.global_state.lookup_function(field).is_some() {
                        return Ok(self.fresh_ty_var());
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
                let method_ty = Type::fun(param_tys, ret_ty);

                // Record symbol for go-to-definition
                self.record_symbol(
                    *field_span,
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
                return Ok(method_ty);
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
        type_args: &[crate::syntax::TypeAnnotation],
        args: &[crate::syntax::CallArg],
        expr_span: &Span,
    ) -> Result<Type> {
        // Handle generic unit variant calls: Option.None[int]
        // Must be handled BEFORE infer_expr(func) since bare generic unit variants are invalid
        if let ExprKind::Field {
            expr: type_expr,
            field: variant_name,
            ..
        } = &func.kind
            && let ExprKind::Ident(type_name) = &type_expr.kind
            && let Some(type_def) = self.global_state.lookup_type(type_name).cloned()
            && let TypeDefKind::Enum { variants } = &type_def.kind
        {
            // Check if this is a unit variant of a generic enum
            for variant in variants {
                if let EnumVariant::Unit { ident, .. } = variant
                    && ident.name == *variant_name
                    && !type_def.generics.is_empty()
                {
                    // This is a generic unit variant call
                    if type_args.is_empty() {
                        return Err(SoppoError::GenericUnitVariant {
                            enum_name: type_name.clone(),
                            variant_name: variant_name.clone(),
                            span: func.span,
                        });
                    }

                    // Validate type arg constraints
                    for (generic_param, type_arg) in type_def.generics.iter().zip(type_args.iter())
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

                    // Return the enum type - args are handled by codegen
                    return Ok(Type::simple(type_name));
                }
            }
        }

        // Handle Go built-in functions
        if let ExprKind::Ident(name) = &func.kind {
            // close(channel) - closes a channel, returns unit
            if name == "close" && args.len() == 1 {
                let channel_ty = self.infer_expr(&args[0].1);
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
                        span: args[0].1.span,
                    });
                    return Ok(Type::error());
                }
                return Ok(Type::unit());
            }

            if name == "make" && !type_args.is_empty() {
                // make(type, ...) - returns the type
                // Validate additional arguments are integers (size, capacity)
                let mut had_error = false;
                for (_, arg, _) in args {
                    let arg_ty = self.infer_expr(arg);
                    if arg_ty.is_error() {
                        had_error = true;
                        continue;
                    }
                    self.unify(&arg_ty, &Type::simple("int"), &arg.span);
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
                let arg_ty = self.infer_expr(&args[0].1);
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
                        span: args[0].1.span,
                    });
                    return Ok(Type::error());
                }
                return Ok(Type::simple("int"));
            }

            // cap(v) - returns capacity of slice, channel, or variadic
            if name == "cap" && args.len() == 1 {
                let arg_ty = self.infer_expr(&args[0].1);
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
                        span: args[0].1.span,
                    });
                    return Ok(Type::error());
                }
                return Ok(Type::simple("int"));
            }

            // append(slice, elems...) - returns the same slice type
            if name == "append" && !args.is_empty() {
                let slice_ty = self.infer_expr(&args[0].1);
                if slice_ty.is_error() {
                    // Still infer remaining args for error collection
                    for (_, arg, _) in args.iter().skip(1) {
                        self.infer_expr(arg);
                    }
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
                            span: args[0].1.span,
                        });
                        // Still infer remaining args for error collection
                        for (_, arg, _) in args.iter().skip(1) {
                            self.infer_expr(arg);
                        }
                        return Ok(Type::error());
                    }
                };
                // Type check remaining arguments against element type
                // Handle spread: append(a, b...) where b is a slice
                let mut had_error = false;
                for (_, arg, is_spread) in args.iter().skip(1) {
                    let arg_ty = self.infer_expr(arg);
                    if arg_ty.is_error() {
                        had_error = true;
                        continue;
                    }
                    if *is_spread {
                        // Spread arg: extract element type from slice and unify
                        let spread_elem = Self::extract_slice_element(&arg_ty).unwrap_or(arg_ty);
                        self.unify(&elem_ty, &spread_elem, &arg.span);
                    } else {
                        self.unify(&elem_ty, &arg_ty, &arg.span);
                    }
                }
                if had_error {
                    return Ok(Type::error());
                }
                return Ok(slice_ty);
            }

            // copy(dst, src) - returns int (number of elements copied)
            if name == "copy" && args.len() == 2 {
                let dst_ty = self.infer_expr(&args[0].1);
                let src_ty = self.infer_expr(&args[1].1);

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
                        span: args[0].1.span,
                    });
                    had_error = true;
                }
                if !src_is_slice && !src_is_string {
                    self.emit_error(SoppoError::Type {
                        message: format!(
                            "second argument to copy must be a slice or string; got {}",
                            src_ty
                        ),
                        span: args[1].1.span,
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
                        span: args[0].1.span,
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
                let map_ty = self.infer_expr(&args[0].1);
                let arg_key_ty = self.infer_expr(&args[1].1);

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
                            span: args[0].1.span,
                        });
                        return Ok(Type::error());
                    }
                };
                // Type check key argument
                self.unify(&key_ty, &arg_key_ty, &args[1].1.span);
                return Ok(Type::unit());
            }

            // panic(v) - panics with value, returns never (diverges)
            if name == "panic" && args.len() == 1 {
                // panic accepts any type
                self.infer_expr(&args[0].1);
                return Ok(Type::never());
            }

            // recover() - returns any (interface{})
            if name == "recover" && args.is_empty() {
                return Ok(Type::simple("any"));
            }

            // print and println - variadic, accept any types, return unit
            if name == "print" || name == "println" {
                for (_, arg, _) in args {
                    self.infer_expr(arg);
                }
                return Ok(Type::unit());
            }

            // complex(r, i) - creates complex number from two float64
            if name == "complex" && args.len() == 2 {
                let r_ty = self.infer_expr(&args[0].1);
                let i_ty = self.infer_expr(&args[1].1);
                if r_ty.is_error() || i_ty.is_error() {
                    return Ok(Type::error());
                }
                self.unify(&r_ty, &Type::simple("float64"), &args[0].1.span);
                self.unify(&i_ty, &Type::simple("float64"), &args[1].1.span);
                return Ok(Type::simple("complex128"));
            }

            // real(c) - extracts real part of complex number
            if name == "real" && args.len() == 1 {
                let c_ty = self.infer_expr(&args[0].1);
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
                            span: args[0].1.span,
                        });
                        return Ok(Type::error());
                    }
                }
            }

            // imag(c) - extracts imaginary part of complex number
            if name == "imag" && args.len() == 1 {
                let c_ty = self.infer_expr(&args[0].1);
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
                            span: args[0].1.span,
                        });
                        return Ok(Type::error());
                    }
                }
            }
        }

        // Check if this is a type conversion: TypeName(value) or pkg.TypeName(value)
        // Also handles slice type conversions like []byte(str)
        if let ExprKind::Ident(type_name) = &func.kind
            && (self.global_state.has_type(type_name)
                || Type::is_builtin_type(type_name)
                || Self::is_slice_type_conversion(type_name))
        {
            // This is a type conversion, not a function call
            // Type conversions take exactly one argument
            if args.len() != 1 {
                self.emit_error(SoppoError::Type {
                    message: format!(
                        "Type conversion requires exactly 1 argument, but got {}",
                        args.len()
                    ),
                    span: *expr_span,
                });
                return Ok(Type::error());
            }

            // Infer the argument type (we don't need to use it, just check it's valid)
            self.infer_expr(&args[0].1);

            // Return the target type
            return Ok(Type::simple(type_name));
        }

        // Check if this is a call on an imported package: pkg.Func(args) or pkg.Type(value)
        if let ExprKind::Field {
            expr: pkg_expr,
            field: name,
            field_span,
        } = &func.kind
            && let ExprKind::Ident(pkg_name) = &pkg_expr.kind
            && self.is_imported_package(pkg_name)
        {
            // Record symbol for the package name itself (e.g., "helpers" in helpers.Add)
            // Go-to-definition on the package name goes to the import statement
            if let Some((import_path, import_span)) = self.get_import_info(pkg_name) {
                self.record_symbol(
                    pkg_expr.span,
                    SymbolInfo {
                        name: pkg_name.to_string(),
                        ty: Type::simple(import_path),
                        definition_span: Some(import_span),
                        name_span: None,
                        kind: SymbolKind::Package,
                        doc_comment: Some(format!("import \"{}\"", import_path)),
                        go_location: None,
                    },
                );
            }

            // For Soppo imports, look up the function from GlobalCtxt
            if self.is_soppo_import(pkg_name) {
                // Mark the import as used
                self.mark_import_used(pkg_name);

                if let Some((func_ty, def_span, name_span, doc_comment)) =
                    self.lookup_soppo_function(pkg_name, name)
                {
                    // Record symbol for go-to-definition
                    self.record_symbol(
                        *field_span,
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

                    // Found the function - infer args and check against signature
                    let mut arg_tys: Vec<(Option<Type>, Span)> = Vec::new();
                    for (_, arg, _) in args {
                        let ty = self.infer_expr(arg);
                        if ty.is_error() {
                            arg_tys.push((None, arg.span));
                        } else {
                            arg_tys.push((Some(ty), arg.span));
                        }
                    }

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
                        *field_span,
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
                            span: *expr_span,
                        });
                        return Ok(Type::error());
                    }
                    self.infer_expr(&args[0].1);
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
                    *field_span,
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
                        span: *expr_span,
                    });
                    return Ok(Type::error());
                }

                // Infer the argument type (we don't need to use it, just check it's valid)
                self.infer_expr(&args[0].1);

                // Return the target type
                return Ok(ty);
            }
        }

        // Regular function call
        let func_ty = self.infer_expr(func);
        if func_ty.is_error() {
            // Still infer all arguments for error collection
            for (_, arg, _) in args {
                self.infer_expr(arg);
            }
            return Ok(Type::error());
        }
        let func_ty = self.substitute(func_ty);

        // If this is a generic function call, instantiate it
        let func_ty = if let ExprKind::Ident(func_name) = &func.kind {
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
                        // Still infer all arguments for error collection
                        for (_, arg, _) in args {
                            self.infer_expr(arg);
                        }
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
        let has_named = args.iter().any(|(name, _, _)| name.is_some());

        // Reorder arguments based on named arguments
        // Track spread flag along with args: (expr, span, is_spread)
        let ordered_args: Vec<(&Expr, Span, bool)> = if !has_named {
            // All positional - just use them in order
            args.iter()
                .map(|(_, e, spread)| (e, e.span, *spread))
                .collect()
        } else if let Some(param_names) = &param_names {
            // We have named args and know parameter names - reorder
            // Rules:
            // - Named args reserve their specific slots first
            // - Positional args fill remaining slots in order
            // - Positional args after named args only allowed for variadic functions
            // - Extra positional args go to variadic
            let mut result: Vec<Option<(&Expr, Span, bool)>> = vec![None; param_names.len()];
            let mut variadic_args: Vec<(&Expr, Span, bool)> = Vec::new();
            let mut positional_args: Vec<(&Expr, bool)> = Vec::new();
            let mut seen_named = false;

            // First pass: process named args to reserve their slots, collect positional args
            for (name, arg_expr, spread) in args {
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
                            result[idx] = Some((arg_expr, arg_expr.span, *spread));
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
                                span: arg_expr.span,
                            });
                        }
                        positional_args.push((arg_expr, *spread));
                    }
                }
            }

            // Second pass: fill remaining slots with positional args
            let mut positional_iter = positional_args.into_iter();
            for slot in result.iter_mut() {
                if slot.is_none()
                    && let Some((arg_expr, spread)) = positional_iter.next()
                {
                    *slot = Some((arg_expr, arg_expr.span, spread));
                }
            }

            // Any remaining positional args go to variadic
            for (arg_expr, spread) in positional_iter {
                variadic_args.push((arg_expr, arg_expr.span, spread));
            }

            // Check all required params are provided
            let mut ordered = Vec::new();
            for (i, slot) in result.iter().enumerate() {
                match slot {
                    Some((arg, span, spread)) => ordered.push((*arg, *span, *spread)),
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

        // Infer argument types with their spans, nil check, and spread flag
        // Use infer_expr_narrowed for nil-state narrowing with error-collecting
        let mut arg_tys: Vec<(Option<Type>, Span, bool, bool)> = Vec::new();
        for (arg, span, is_spread) in &ordered_args {
            let is_nil = matches!(arg.kind, ExprKind::Nil);
            let ty = self.infer_expr_narrowed(arg);
            if ty.is_error() {
                arg_tys.push((None, *span, is_nil, *is_spread));
            } else {
                arg_tys.push((Some(ty), *span, is_nil, *is_spread));
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
                self.unify(&func_ty, &expected_func_ty, expr_span);
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
    fn infer_unary(&mut self, op: &UnaryOp, operand: &Expr) -> Result<Type> {
        let operand_ty = self.infer_expr(operand);
        if operand_ty.is_error() {
            return Ok(Type::error());
        }

        match op {
            UnaryOp::Neg => {
                // -x: operand must be numeric, result is same type
                Ok(operand_ty)
            }
            UnaryOp::Not => {
                // !x: operand must be bool, result is bool
                self.unify(&operand_ty, &Type::simple("bool"), &operand.span);
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
                            ExprKind::Field { field_span, .. } => *field_span,
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
                    return Ok(pointee_ty);
                }
                // If we can't determine the pointer type, return a type variable
                Ok(self.fresh_ty_var())
            }
            UnaryOp::Recv => {
                // <-ch: operand must be chan T, result is T
                let operand_ty = self.substitute(operand_ty);
                // Extract the element type from chan T
                if let Some(elem_ty) = Self::extract_channel_element(&operand_ty) {
                    return Ok(elem_ty);
                }
                // If we can't determine the channel type, return a type variable
                Ok(self.fresh_ty_var())
            }
        }
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
        let ty = infer.infer_expr_inner(&expr)?;
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
        let ty = infer.infer_expr_inner(&expr).unwrap();

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
