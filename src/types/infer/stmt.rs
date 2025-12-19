use std::collections::HashSet;

use super::Infer;
use crate::error::{Result, SoppoError};
use crate::syntax::{
    BinOp, EnumVariant, Expr, ExprKind, Literal, PatternKind, SelectCaseKind, Stmt, StmtKind,
};
use crate::types::ctx::TypeDefKind;
use crate::types::ty::Nullability;
use crate::types::{SymbolInfo, SymbolKind, Type};

/// Result of analysing a nil check condition
#[derive(Debug)]
pub(super) struct NilCheck {
    /// The expression key being checked (e.g., "user" or "user.profile")
    pub expr_key: String,
    /// True if the check is `expr != nil`, false if `expr == nil`
    pub is_not_nil: bool,
}

/// Convert an expression to a trackable key string
/// Returns Some(key) for identifiers and field access chains (e.g., "user.profile.address")
/// Returns None for complex expressions that can't be tracked
pub(super) fn expr_to_key(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Ident(name) => Some(name.clone()),
        ExprKind::Field {
            expr: base, field, ..
        } => {
            let base_key = expr_to_key(base)?;
            Some(format!("{}.{}", base_key, field))
        }
        _ => None,
    }
}

/// Extract nil check from a binary comparison expression
/// Returns Some(NilCheck) if the expression is of the form:
/// Works with simple identifiers and field access chains (e.g., user.profile != nil)
/// - x != nil or nil != x (is_not_nil = true)
/// - x == nil or nil == x (is_not_nil = false)
fn extract_nil_check(expr: &Expr) -> Option<NilCheck> {
    if let ExprKind::Binary { op, left, right } = &expr.kind {
        let is_not_nil = match op {
            BinOp::Ne => true,
            BinOp::Eq => false,
            _ => return None,
        };

        // Check for: expr != nil or expr == nil
        if matches!(right.kind, ExprKind::Nil)
            && let Some(key) = expr_to_key(left)
        {
            return Some(NilCheck {
                expr_key: key,
                is_not_nil,
            });
        }

        // Check for: nil != expr or nil == expr
        if matches!(left.kind, ExprKind::Nil)
            && let Some(key) = expr_to_key(right)
        {
            return Some(NilCheck {
                expr_key: key,
                is_not_nil,
            });
        }
    }

    None
}

/// Extract all nil checks from a condition expression
/// Handles compound conditions with && and || (e.g., `x != nil && y != nil`)
pub(super) fn extract_nil_checks(expr: &Expr) -> Vec<NilCheck> {
    let mut checks = Vec::new();

    if let ExprKind::Binary { op, left, right } = &expr.kind {
        // Handle && and || - collect checks from both sides
        // The caller handles the difference: && applies checks as-is, || applies opposite
        if matches!(op, BinOp::And | BinOp::Or) {
            checks.extend(extract_nil_checks(left));
            checks.extend(extract_nil_checks(right));
            return checks;
        }
    }

    // Try to extract a single nil check from this expression
    if let Some(check) = extract_nil_check(expr) {
        checks.push(check);
    }

    checks
}

impl Infer {
    /// Infer the type of a statement (internal version that returns Result).
    ///
    /// **Prefer `infer_stmt`** which collects errors and returns `Type::Error` on failure.
    /// This version should only be used when you need to explicitly check if inference failed.
    ///
    /// Returns the type of the statement (unit for most, or the type of the expression).
    pub fn infer_stmt_inner(&mut self, stmt: &Stmt) -> Result<Type> {
        match &stmt.kind {
            StmtKind::Decl { ident, value } => {
                // Use infer_expr_or_error to collect errors and continue with Type::Error
                let value_ty = self.infer_expr(value);
                let value_ty_sub = self.substitute(value_ty.clone());

                // Check for generic enum variants without type args (skip if already error)
                if !value_ty.is_error()
                    && let Err(e) = self.check_generic_enum_needs_type_args(value)
                {
                    self.emit_error(e);
                }

                // Insert variable even if value_ty is Error to prevent cascading errors
                if let Err(e) =
                    self.insert_var(ident.name.clone(), value_ty.clone(), Some(ident.span))
                {
                    self.emit_error(e);
                }

                // Record variable definition for LSP
                self.record_symbol(
                    ident.span,
                    SymbolInfo {
                        name: ident.name.clone(),
                        ty: value_ty,
                        definition_span: Some(stmt.span),
                        name_span: Some(ident.span),
                        kind: SymbolKind::Variable,
                        doc_comment: None,
                        go_location: None,
                    },
                );

                // Track nil state for pointer types (only if not error type)
                if !value_ty_sub.is_error() {
                    self.update_nil_state_for_assignment(&ident.name, value, &value_ty_sub);
                    // Track variant state for enum types
                    self.update_variant_state_for_assignment(&ident.name, value);
                }
                Ok(Type::unit())
            }

            StmtKind::MultiDecl {
                ident: names,
                values,
            } => {
                if values.len() == 1 && names.len() > 1 {
                    // a, b := f() (multi-return unpacking)
                    let value = &values[0];

                    // Check for comma-ok idiom: v, ok := expr
                    // Applies to: type assertions, map access, channel receive
                    if names.len() == 2
                        && let Some((value_ty, ok_ty)) = self.infer_comma_ok_expr(value)
                    {
                        // Check if sub-expression inference failed
                        if value_ty.is_error() {
                            for ident in names {
                                if let Err(e) = self.insert_var(
                                    ident.name.clone(),
                                    Type::error(),
                                    Some(ident.span),
                                ) {
                                    self.emit_error(e);
                                }
                            }
                            return Ok(Type::unit());
                        }
                        let vars = vec![
                            (names[0].name.clone(), value_ty, Some(names[0].span)),
                            (names[1].name.clone(), ok_ty, Some(names[1].span)),
                        ];
                        if let Err(e) = self.insert_short_decl_vars(&vars) {
                            self.emit_error(e);
                        }
                        return Ok(Type::unit());
                    }
                    // None = not a comma-ok expression, continue to tuple handling

                    let value_ty = self.infer_expr(value);

                    // If expression failed, insert vars with Error type to prevent cascading
                    if value_ty.is_error() {
                        for ident in names {
                            if let Err(e) =
                                self.insert_var(ident.name.clone(), Type::error(), Some(ident.span))
                            {
                                self.emit_error(e);
                            }
                        }
                        return Ok(Type::unit());
                    }

                    let value_ty = self.substitute(value_ty);

                    // The value should be a tuple type with matching arity
                    if let Type::Con {
                        sym: type_name,
                        args,
                        ..
                    } = &value_ty
                        && type_name.name == "tuple"
                        && args.len() == names.len()
                    {
                        // Use Go's short decl semantics - at least one var must be new
                        let vars: Vec<_> = names
                            .iter()
                            .zip(args.iter())
                            .map(|(ident, ty)| (ident.name.clone(), ty.clone(), Some(ident.span)))
                            .collect();
                        if let Err(e) = self.insert_short_decl_vars(&vars) {
                            self.emit_error(e);
                        }

                        // Track error companions: if last return is error, other returns are companions
                        // This enables narrowing like: `resp, err := f(); if err != nil { return }`
                        // After the early return, resp is known non-nil.
                        if let Some(last_ty) = args.last()
                            && matches!(last_ty, Type::Con { sym, .. } if sym.name == "error" || sym.name == "?error")
                            && names.len() >= 2
                        {
                            let err_name = names.last().unwrap().name.clone();
                            let companions: Vec<String> = names[..names.len() - 1]
                                .iter()
                                .map(|n| n.name.clone())
                                .collect();
                            self.error_companions.insert(err_name, companions);
                        }

                        return Ok(Type::unit());
                    }

                    // Not a tuple type or wrong arity - emit error but still define vars
                    self.emit_error(SoppoError::Type {
                        message: format!(
                            "Cannot unpack {} values from type `{}`",
                            names.len(),
                            value_ty
                        ),
                        span: value.span,
                    });
                    for ident in names {
                        if let Err(e) =
                            self.insert_var(ident.name.clone(), Type::error(), Some(ident.span))
                        {
                            self.emit_error(e);
                        }
                    }
                    Ok(Type::unit())
                } else {
                    // a, b := expr1, expr2 (one value per name)
                    let mut vars = Vec::with_capacity(names.len());
                    for (ident, value) in names.iter().zip(values.iter()) {
                        let value_ty = self.infer_expr(value);
                        vars.push((ident.name.clone(), value_ty, Some(ident.span)));
                    }
                    if let Err(e) = self.insert_short_decl_vars(&vars) {
                        self.emit_error(e);
                    }
                    Ok(Type::unit())
                }
            }

            StmtKind::VarDecl { ident, ty, value } => {
                // Record type annotation for LSP if present
                if let Some(t) = ty {
                    self.record_type_annotation(t);
                }

                let (var_ty, init_expr) = match (ty, value) {
                    (Some(t), Some(expr)) => {
                        // var x type = value: unify declared with inferred
                        let declared_ty = Type::from_ast(t);

                        // Check: assigning nil to a non-nilable type is an error
                        if matches!(expr.kind, ExprKind::Nil)
                            && let Some(err) = Self::check_nil_to_non_nilable(&declared_ty, t.span)
                        {
                            self.emit_error(err);
                            (Type::error(), Some(expr))
                        } else {
                            let value_ty = self.infer_expr(expr);
                            if !value_ty.is_error() {
                                self.unify(&declared_ty, &value_ty, &expr.span);
                            }
                            (declared_ty, Some(expr))
                        }
                    }
                    (Some(t), None) => {
                        // var x type: use declared type (zero value)
                        let declared_ty = Type::from_ast(t);

                        // Check: non-nilable types require initialisation
                        // Zero value for pointer/slice/map/etc is nil, which violates non-nilable
                        if declared_ty.is_nilable_kind() && !declared_ty.is_nullable() {
                            self.emit_error(SoppoError::NonNilableNoInit {
                                ty: declared_ty.to_string(),
                                span: stmt.span,
                            });
                            (Type::error(), None)
                        } else {
                            (declared_ty, None)
                        }
                    }
                    (None, Some(expr)) => {
                        // var x = value: infer from value
                        let ty = self.infer_expr(expr);
                        (ty, Some(expr))
                    }
                    (None, None) => {
                        // var x: error (should be caught by parser)
                        self.emit_error(SoppoError::Type {
                            message:
                                "Variable declaration requires either a type or an initialiser"
                                    .to_string(),
                            span: stmt.span,
                        });
                        (Type::error(), None)
                    }
                };
                let var_ty_sub = self.substitute(var_ty.clone());

                // Insert variable even if var_ty is Error to prevent cascading errors
                if let Err(e) =
                    self.insert_var(ident.name.clone(), var_ty.clone(), Some(ident.span))
                {
                    self.emit_error(e);
                }

                // Record variable definition for LSP
                self.record_symbol(
                    ident.span,
                    SymbolInfo {
                        name: ident.name.clone(),
                        ty: var_ty,
                        definition_span: Some(stmt.span),
                        name_span: Some(ident.span),
                        kind: SymbolKind::Variable,
                        doc_comment: None,
                        go_location: None,
                    },
                );

                // Track nil state for nilable types (only if not error type)
                if !var_ty_sub.is_error() {
                    if let Some(expr) = init_expr {
                        self.update_nil_state_for_assignment(&ident.name, expr, &var_ty_sub);
                    } else if Self::is_nilable_type(&var_ty_sub) {
                        // Zero-initialised nilable types are nil
                        self.set_nil_state(
                            ident.name.clone(),
                            crate::types::ty::Nullability::Nullable,
                        );
                    }
                }
                Ok(Type::unit())
            }

            StmtKind::MultiVarDecl {
                ident: names,
                ty,
                values,
            } => {
                // Record type annotation for LSP if present
                if let Some(t) = ty {
                    self.record_type_annotation(t);
                }

                if values.is_empty() {
                    // var a, b, c type (zero values)
                    let declared_ty = match ty.as_ref().map(Type::from_ast) {
                        Some(t) => t,
                        None => {
                            self.emit_error(SoppoError::Type {
                                message:
                                    "Multi-variable declaration without values requires a type"
                                        .to_string(),
                                span: stmt.span,
                            });
                            Type::error()
                        }
                    };
                    for ident in names {
                        if let Err(e) = self.insert_var(
                            ident.name.clone(),
                            declared_ty.clone(),
                            Some(ident.span),
                        ) {
                            self.emit_error(e);
                        }
                    }
                } else if values.len() == 1 && names.len() > 1 {
                    // var a, b = f() (multi-return unpacking)
                    let value = &values[0];

                    // Check for comma-ok idiom: v, ok := expr
                    // Applies to: type assertions, map access, channel receive
                    if names.len() == 2
                        && let Some((value_ty, ok_ty)) = self.infer_comma_ok_expr(value)
                    {
                        // Check if sub-expression inference failed
                        if value_ty.is_error() {
                            for ident in names {
                                if let Err(e) = self.insert_var(
                                    ident.name.clone(),
                                    Type::error(),
                                    Some(ident.span),
                                ) {
                                    self.emit_error(e);
                                }
                            }
                            return Ok(Type::unit());
                        }
                        // First variable gets the value type
                        let var_ty = if let Some(t) = ty {
                            let declared_ty = Type::from_ast(t);
                            self.unify(&declared_ty, &value_ty, &value.span);
                            declared_ty
                        } else {
                            value_ty
                        };
                        if let Err(e) =
                            self.insert_var(names[0].name.clone(), var_ty, Some(names[0].span))
                        {
                            self.emit_error(e);
                        }
                        // Second variable gets the ok type (bool)
                        if let Err(e) =
                            self.insert_var(names[1].name.clone(), ok_ty, Some(names[1].span))
                        {
                            self.emit_error(e);
                        }
                        return Ok(Type::unit());
                    }
                    // None = not a comma-ok expression, continue to tuple handling

                    let value_ty = self.infer_expr(value);

                    // If expression failed, insert vars with Error type to prevent cascading
                    if value_ty.is_error() {
                        for ident in names {
                            if let Err(e) =
                                self.insert_var(ident.name.clone(), Type::error(), Some(ident.span))
                            {
                                self.emit_error(e);
                            }
                        }
                        return Ok(Type::unit());
                    }

                    let value_ty = self.substitute(value_ty);

                    // The value should be a tuple type with matching arity
                    if let Type::Con {
                        sym: type_name,
                        args,
                        ..
                    } = &value_ty
                        && type_name.name == "tuple"
                        && args.len() == names.len()
                    {
                        for (ident, arg_ty) in names.iter().zip(args.iter()) {
                            let var_ty = if let Some(t) = ty {
                                let declared_ty = Type::from_ast(t);
                                self.unify(&declared_ty, arg_ty, &value.span);
                                declared_ty
                            } else {
                                arg_ty.clone()
                            };
                            if let Err(e) =
                                self.insert_var(ident.name.clone(), var_ty, Some(ident.span))
                            {
                                self.emit_error(e);
                            }
                        }
                        return Ok(Type::unit());
                    }

                    // Not a tuple type or wrong arity - emit error but still define vars
                    self.emit_error(SoppoError::Type {
                        message: format!(
                            "Cannot unpack {} values from type `{}`",
                            names.len(),
                            value_ty
                        ),
                        span: value.span,
                    });
                    for ident in names {
                        if let Err(e) =
                            self.insert_var(ident.name.clone(), Type::error(), Some(ident.span))
                        {
                            self.emit_error(e);
                        }
                    }
                } else {
                    // var a, b = expr1, expr2 or var a, b type = expr1, expr2
                    for (ident, value) in names.iter().zip(values.iter()) {
                        let value_ty = self.infer_expr(value);
                        let var_ty = if value_ty.is_error() {
                            Type::error()
                        } else if let Some(t) = ty {
                            let declared_ty = Type::from_ast(t);
                            self.unify(&declared_ty, &value_ty, &value.span);
                            declared_ty
                        } else {
                            value_ty
                        };
                        if let Err(e) =
                            self.insert_var(ident.name.clone(), var_ty, Some(ident.span))
                        {
                            self.emit_error(e);
                        }
                    }
                }
                Ok(Type::unit())
            }

            StmtKind::ConstDecl { ident, ty, value } => {
                // Infer the type of the value
                let value_ty = self.infer_expr(value);

                // Determine the constant's type
                let const_ty = if value_ty.is_error() {
                    Type::error()
                } else if let Some(t) = ty {
                    // const x type = value: unify declared with inferred
                    let declared_ty = Type::from_ast(t);
                    self.unify(&declared_ty, &value_ty, &value.span);
                    declared_ty
                } else {
                    // const x = value: infer from value
                    value_ty
                };

                if let Err(e) = self.insert_var(ident.name.clone(), const_ty, Some(ident.span)) {
                    self.emit_error(e);
                }
                Ok(Type::unit())
            }

            StmtKind::MultiConstDecl { idents, ty, values } => {
                // const a, b = expr1, expr2 or const a, b type = expr1, expr2
                for (ident, value) in idents.iter().zip(values.iter()) {
                    let value_ty = self.infer_expr(value);
                    let const_ty = if value_ty.is_error() {
                        Type::error()
                    } else if let Some(t) = ty {
                        let declared_ty = Type::from_ast(t);
                        self.unify(&declared_ty, &value_ty, &value.span);
                        declared_ty
                    } else {
                        value_ty
                    };
                    if let Err(e) = self.insert_var(ident.name.clone(), const_ty, Some(ident.span))
                    {
                        self.emit_error(e);
                    }
                }
                Ok(Type::unit())
            }

            StmtKind::Assign { target, value } => {
                // Special case: blank identifier accepts any type
                if let ExprKind::Ident(name) = &target.kind
                    && name == "_"
                {
                    // Just infer the value type, don't unify
                    self.infer_expr(value);
                    return Ok(Type::unit());
                }
                let target_ty = self.infer_expr(target);
                if target_ty.is_error() {
                    self.infer_expr(value);
                    return Ok(Type::unit());
                }
                let target_ty_sub = self.substitute(target_ty.clone());

                // Check: assigning nil to a non-nilable type is an error
                if matches!(value.kind, ExprKind::Nil)
                    && let Some(err) = Self::check_nil_to_non_nilable(&target_ty_sub, value.span)
                {
                    self.emit_error(err);
                    return Ok(Type::unit());
                }

                let value_ty = self.infer_expr(value);
                if !value_ty.is_error() {
                    let value_ty_sub = self.substitute(value_ty.clone());
                    self.unify(&target_ty, &value_ty, &stmt.span);
                    // Update nil state and variant state for reassignment
                    if let ExprKind::Ident(name) = &target.kind {
                        self.update_nil_state_for_assignment(name, value, &value_ty_sub);
                        self.update_variant_state_for_assignment(name, value);
                    }
                }
                Ok(Type::unit())
            }

            StmtKind::MultiAssign { targets, values } => {
                if values.len() == 1 && targets.len() > 1 {
                    // a, b = f() (multi-return unpacking)
                    let value = &values[0];
                    let value_ty = self.infer_expr(value);

                    if value_ty.is_error() {
                        // Still infer targets for LSP support
                        for target in targets {
                            self.infer_expr(target);
                        }
                        return Ok(Type::unit());
                    }

                    let value_ty = self.substitute(value_ty);

                    // The value should be a tuple type with matching arity
                    if let Type::Con {
                        sym: type_name,
                        args,
                        ..
                    } = &value_ty
                        && type_name.name == "tuple"
                        && args.len() == targets.len()
                    {
                        for (target, expected_ty) in targets.iter().zip(args.iter()) {
                            // Special case: blank identifier accepts any type
                            if let ExprKind::Ident(name) = &target.kind
                                && name == "_"
                            {
                                continue;
                            }
                            let target_ty = self.infer_expr(target);
                            if !target_ty.is_error() {
                                self.unify(&target_ty, expected_ty, &target.span);
                            }
                        }
                        return Ok(Type::unit());
                    }

                    // Not a tuple type or wrong arity
                    self.emit_error(SoppoError::Type {
                        message: format!(
                            "Cannot unpack {} values from type `{}`",
                            targets.len(),
                            value_ty
                        ),
                        span: value.span,
                    });
                    Ok(Type::unit())
                } else {
                    // a, b = expr1, expr2 (one value per target)
                    for (target, value) in targets.iter().zip(values.iter()) {
                        // Special case: blank identifier accepts any type
                        if let ExprKind::Ident(name) = &target.kind
                            && name == "_"
                        {
                            self.infer_expr(value);
                            continue;
                        }
                        let target_ty = self.infer_expr(target);
                        let value_ty = self.infer_expr(value);
                        if !target_ty.is_error() && !value_ty.is_error() {
                            self.unify(&target_ty, &value_ty, &target.span);
                        }
                    }
                    Ok(Type::unit())
                }
            }

            StmtKind::For { condition, body } => {
                // Check condition is bool
                let cond_ty = self.infer_expr(condition);
                if !cond_ty.is_error() {
                    self.unify(&Type::simple("bool"), &cond_ty, &condition.span);
                }

                // Type check body
                self.infer_block(body);

                Ok(Type::unit())
            }

            StmtKind::ForCStyle {
                init,
                condition,
                post,
                body,
            } => {
                // Create a new scope for the loop (init vars are scoped to the loop)
                self.push_scope();

                // Type check init statement if present
                if let Some(init_stmt) = init {
                    self.infer_stmt(init_stmt);
                }

                // Check condition is bool if present
                if let Some(cond) = condition {
                    let cond_ty = self.infer_expr(cond);
                    if !cond_ty.is_error() {
                        self.unify(&Type::simple("bool"), &cond_ty, &cond.span);
                    }
                }

                // Type check body
                self.infer_block(body);

                // Type check post statement if present
                if let Some(post_stmt) = post {
                    self.infer_stmt(post_stmt);
                }

                self.pop_scope();

                Ok(Type::unit())
            }

            StmtKind::ForRange {
                key,
                value,
                collection,
                body,
            } => {
                // Infer collection type
                let coll_ty = self.infer_expr(collection);
                let coll_ty = self.substitute(coll_ty);

                // Determine key and value types based on collection type
                let (key_ty, value_ty) = if coll_ty.is_error() {
                    // Collection failed - use error types for loop vars
                    (Type::error(), Type::error())
                } else if let Some(elem_ty) = Self::extract_slice_element(&coll_ty) {
                    // Slice: key is int, value is element type
                    (Type::simple("int"), elem_ty)
                } else if let Some((k, v)) = Self::extract_map_elements(&coll_ty) {
                    // Map: key is key type, value is value type
                    (k, v)
                } else if let Some(elem_ty) = Self::extract_channel_element(&coll_ty) {
                    // Channel: only one variable (value type)
                    (elem_ty.clone(), elem_ty)
                } else if matches!(&coll_ty, Type::Con { sym, .. } if sym.name == "string") {
                    // String: key is int (index), value is rune
                    (Type::simple("int"), Type::simple("rune"))
                } else {
                    (self.fresh_ty_var(), self.fresh_ty_var())
                };

                // Create a scope for the loop variables (scoped to the for statement)
                self.push_scope();

                // Bind the key variable
                if let Err(e) = self.insert_var(key.name.clone(), key_ty, Some(key.span)) {
                    self.emit_error(e);
                }

                // Bind the value variable if present
                if let Some(val_ident) = value
                    && let Err(e) =
                        self.insert_var(val_ident.name.clone(), value_ty, Some(val_ident.span))
                {
                    self.emit_error(e);
                }

                // Type check body
                self.infer_block(body);

                self.pop_scope();

                Ok(Type::unit())
            }

            StmtKind::If {
                init,
                condition,
                then_block,
                else_block,
            } => {
                // If there's an init, create a new scope for it that covers the entire if/else
                // This matches Go's scoping: `if x := 1; cond { }` - x is only in scope for the if
                let has_init = init.is_some();
                if has_init {
                    self.push_scope();
                }

                // Process init statement if present (Go-style: if x := expr; cond { })
                if let Some(init_stmt) = init {
                    self.infer_stmt(init_stmt);
                }

                // Check condition is bool
                let cond_ty = self.infer_expr(condition);
                if !cond_ty.is_error() {
                    self.unify(&Type::simple("bool"), &cond_ty, &condition.span);
                }

                // Extract nil checks from condition for flow-sensitive narrowing
                // Handles compound conditions like `x != nil && y != nil`
                let nil_checks = extract_nil_checks(condition);

                // Type check then block with narrowed nil state
                self.push_nil_scope();
                for check in &nil_checks {
                    if check.is_not_nil {
                        // `if x != nil { ... }` - x is non-nil in then block
                        self.set_nil_state(check.expr_key.clone(), Nullability::NonNull);
                    } else {
                        // `if x == nil { ... }` - x is nil in then block (not useful for access)
                        self.set_nil_state(check.expr_key.clone(), Nullability::Nullable);
                    }
                }
                let then_ty = self.infer_block(then_block);
                self.pop_nil_scope();

                // Type check else block with opposite narrowing
                let else_ty = if let Some(else_block) = else_block {
                    self.push_nil_scope();
                    for check in &nil_checks {
                        if check.is_not_nil {
                            // `if x != nil { ... } else { ... }` - x is nil in else block
                            self.set_nil_state(check.expr_key.clone(), Nullability::Nullable);
                        } else {
                            // `if x == nil { ... } else { ... }` - x is non-nil in else block
                            self.set_nil_state(check.expr_key.clone(), Nullability::NonNull);
                        }
                    }
                    let ty = self.infer_block(else_block);
                    self.pop_nil_scope();
                    ty
                } else {
                    Type::unit()
                };

                // Handle early return narrowing:
                // If then block diverges (returns/breaks) and condition was `x == nil`,
                // then after the if statement, x is known to be non-nil
                for check in &nil_checks {
                    if matches!(then_ty, Type::Never) && !check.is_not_nil {
                        // `if x == nil { return }` - x is non-nil after this point
                        self.set_nil_state(check.expr_key.clone(), Nullability::NonNull);
                    }
                    // Similarly, if else block diverges and condition was `x != nil`
                    if matches!(else_ty, Type::Never) && check.is_not_nil && else_block.is_some() {
                        // `if x != nil { ... } else { return }` - x is non-nil after
                        self.set_nil_state(check.expr_key.clone(), Nullability::NonNull);
                    }

                    // Handle error companion narrowing:
                    // `if err != nil { return }` - after this, err's companions are non-nil
                    // This is the common Go idiom: `resp, err := f(); if err != nil { return }`
                    if matches!(then_ty, Type::Never) && check.is_not_nil {
                        // check.expr_key is the error variable name
                        if let Some(companions) = self.error_companions.get(&check.expr_key) {
                            for companion in companions.clone() {
                                self.set_nil_state(companion, Nullability::NonNull);
                            }
                        }
                    }
                }

                // Pop the init scope if we pushed one
                if has_init {
                    self.pop_scope();
                }

                // If both branches diverge (return never), the if statement also diverges
                if matches!(then_ty, Type::Never) && matches!(else_ty, Type::Never) {
                    Ok(Type::never())
                } else {
                    Ok(Type::unit())
                }
            }

            StmtKind::Return { values } => {
                // Check return values against expected return types
                // Note: return statements always diverge, even if there are type errors
                if let Some(expected_types) = self.expected_return_types.clone() {
                    // Handle `return f()` where f() returns a tuple matching expected types
                    // This is the Go idiom: `return someFunc()` when both have same return signature
                    if values.len() == 1 && expected_types.len() > 1 {
                        let value_ty = self.infer_expr(&values[0]);
                        if !value_ty.is_error() {
                            let value_ty = self.substitute(value_ty);

                            // Check if it's a tuple type with matching arity
                            if let Type::Con { sym, args, .. } = &value_ty
                                && sym.name == "tuple"
                                && args.len() == expected_types.len()
                            {
                                // Unify each tuple element with expected type
                                for (arg_ty, expected) in args.iter().zip(expected_types.iter()) {
                                    self.unify(expected, arg_ty, &values[0].span);
                                }
                            } else {
                                // Not a matching tuple
                                self.emit_error(SoppoError::Type {
                                    message: format!(
                                        "Expected {} return value(s), got {}",
                                        expected_types.len(),
                                        values.len()
                                    ),
                                    span: stmt.span,
                                });
                            }
                        }
                        return Ok(Type::never());
                    }

                    if values.len() != expected_types.len() {
                        self.emit_error(SoppoError::Type {
                            message: format!(
                                "Expected {} return value(s), got {}",
                                expected_types.len(),
                                values.len()
                            ),
                            span: stmt.span,
                        });
                        // Still infer expressions for LSP support, etc.
                        for expr in values {
                            self.infer_expr(expr);
                        }
                        return Ok(Type::never());
                    }
                    for (expr, expected) in values.iter().zip(expected_types.iter()) {
                        // Check: returning nil to a non-nilable type is an error
                        if matches!(expr.kind, ExprKind::Nil)
                            && let Some(err) = Self::check_nil_to_non_nilable(expected, expr.span)
                        {
                            self.emit_error(err);
                            continue;
                        }

                        let value_ty = self.infer_expr(expr);
                        if !value_ty.is_error() {
                            self.unify(expected, &value_ty, &expr.span);
                            // After unification, set inferred type args on expressions that need them
                            self.set_inferred_type_args(expr, expected);
                        }
                    }
                } else if !values.is_empty() {
                    // Infer types but no expected types to check against
                    for expr in values {
                        self.infer_expr(expr);
                    }
                }
                // Return statements are diverging - they never produce a value
                Ok(Type::never())
            }

            StmtKind::Match { scrutinee, arms } => {
                // Expression-less match has no scrutinee
                let (scrutinee_ty, scrutinee_key) = if let Some(scrutinee) = scrutinee {
                    let ty = self.infer_expr(scrutinee);
                    let key = expr_to_key(scrutinee);
                    if ty.is_error() {
                        (None, key) // Continue checking arms even if scrutinee failed
                    } else {
                        (Some(self.substitute(ty)), key)
                    }
                } else {
                    (None, None)
                };

                // Annotate Variant patterns based on scrutinee type
                // If scrutinee is a soppo enum, patterns are soppo enums
                // Otherwise (e.g., byte, int), patterns are Go constants
                let is_soppo_enum_scrutinee = scrutinee_ty
                    .as_ref()
                    .map(|ty| {
                        if let Type::Con { sym, .. } = ty {
                            if sym.module.0.is_empty() {
                                // Local type in current module
                                self.global_state
                                    .lookup_type(&sym.name)
                                    .map(|td| matches!(td.kind, TypeDefKind::Enum { .. }))
                                    .unwrap_or(false)
                            } else {
                                // Cross-package type - use is_soppo_enum which checks both
                                // soppo imports and Go packages with soppo:enum markers
                                self.global_state.is_soppo_enum(&sym.module.0, &sym.name)
                            }
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);

                // Set the is_soppo_enum flag on all Variant patterns
                // Also set inferred type args from scrutinee type
                let scrutinee_type_args: Vec<String> = scrutinee_ty
                    .as_ref()
                    .and_then(|ty| {
                        if let Type::Con { args, .. } = ty {
                            if args.is_empty() {
                                None
                            } else {
                                Some(args.iter().map(|t| self.type_to_string(t)).collect())
                            }
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();

                for arm in arms.iter() {
                    for pattern in &arm.patterns {
                        match &pattern.kind {
                            PatternKind::Variant {
                                is_soppo_enum,
                                type_args,
                                ..
                            } => {
                                is_soppo_enum.set(is_soppo_enum_scrutinee);
                                // Set inferred type args if pattern has no explicit args
                                if type_args.is_empty() && !scrutinee_type_args.is_empty() {
                                    *pattern.inferred_type_args.borrow_mut() =
                                        Some(scrutinee_type_args.clone());
                                }
                            }
                            PatternKind::Destructor { type_args, .. }
                            | PatternKind::StructDestructor { type_args, .. } => {
                                // Set inferred type args if pattern has no explicit args
                                if type_args.is_empty() && !scrutinee_type_args.is_empty() {
                                    *pattern.inferred_type_args.borrow_mut() =
                                        Some(scrutinee_type_args.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Check if scrutinee is nilable (for nil narrowing in non-nil arms)
                let scrutinee_is_nilable = scrutinee_ty
                    .as_ref()
                    .map(|ty| ty.is_nullable())
                    .unwrap_or(false);

                // Track whether all arms diverge (for determining if match diverges)
                let mut all_arms_diverge = true;
                let mut has_default = false;

                // Check enum exhaustiveness upfront
                let is_exhaustive_enum = if let Some(ref scr_ty) = scrutinee_ty
                    && let Type::Con { sym: name, .. } = scr_ty
                    && let Some(type_def) = self.global_state.lookup_type(&name.name)
                    && let TypeDefKind::Enum { variants } = &type_def.kind
                {
                    let covered: HashSet<String> = arms
                        .iter()
                        .flat_map(|arm| arm.patterns.iter())
                        .filter_map(|pattern| match &pattern.kind {
                            PatternKind::Variant { name, .. } => {
                                Some(name.rsplit('.').next().unwrap_or(name).to_string())
                            }
                            PatternKind::Destructor { name, .. } => {
                                Some(name.rsplit('.').next().unwrap_or(name).to_string())
                            }
                            PatternKind::StructDestructor { name, .. } => {
                                Some(name.rsplit('.').next().unwrap_or(name).to_string())
                            }
                            _ => None,
                        })
                        .collect();

                    variants.iter().all(|v| {
                        let vname = match v {
                            EnumVariant::Unit { ident, .. } => &ident.name,
                            EnumVariant::Single { ident, .. } => &ident.name,
                            EnumVariant::Struct { ident, .. } => &ident.name,
                        };
                        covered.contains(vname)
                    })
                } else {
                    false
                };

                for arm in arms {
                    // Check if this arm has a default pattern
                    if arm
                        .patterns
                        .iter()
                        .any(|p| matches!(&p.kind, PatternKind::Default))
                    {
                        has_default = true;
                    }
                    // Create a new scope for pattern bindings
                    self.push_scope();

                    // Check if this arm matches nil
                    let arm_matches_nil = arm
                        .patterns
                        .iter()
                        .any(|p| matches!(&p.kind, PatternKind::Literal(Literal::Nil)));

                    // If scrutinee is nilable and this arm doesn't match nil,
                    // the scrutinee is known to be non-nil in this arm
                    let needs_nil_narrowing =
                        scrutinee_is_nilable && !arm_matches_nil && scrutinee_key.is_some();

                    if needs_nil_narrowing {
                        self.push_nil_scope();
                        if let Some(ref key) = scrutinee_key {
                            self.set_nil_state(key.clone(), Nullability::NonNull);
                        }
                    }

                    if let Some(ref scr_ty) = scrutinee_ty {
                        // Normal match with scrutinee
                        // Handle multiple patterns: validate bindings match across patterns
                        if arm.patterns.len() > 1 {
                            // Collect bindings from first pattern
                            match self.collect_pattern_bindings(&arm.patterns[0], scr_ty) {
                                Ok(first_bindings) => {
                                    // Validate subsequent patterns have matching bindings
                                    for pattern in &arm.patterns[1..] {
                                        match self.collect_pattern_bindings(pattern, scr_ty) {
                                            Ok(bindings) => {
                                                // Check binding names match
                                                let first_keys: HashSet<_> =
                                                    first_bindings.keys().collect();
                                                let other_keys: HashSet<_> =
                                                    bindings.keys().collect();
                                                if first_keys != other_keys {
                                                    self.emit_error(SoppoError::Type {
                                                        message: format!(
                                                            "Pattern bindings must match: expected {:?}, found {:?}",
                                                            first_keys, other_keys
                                                        ),
                                                        span: pattern.span,
                                                    });
                                                } else {
                                                    // Unify types of matching bindings
                                                    for (name, ty) in &bindings {
                                                        if let Some(first_ty) =
                                                            first_bindings.get(name)
                                                        {
                                                            self.unify(first_ty, ty, &pattern.span);
                                                        }
                                                    }
                                                }
                                            }
                                            Err(e) => self.emit_error(e),
                                        }
                                    }

                                    // Add first pattern's bindings to scope
                                    let pattern_span = arm.patterns.first().map(|p| p.span);
                                    for (name, ty) in first_bindings {
                                        if let Err(e) = self.insert_var(name, ty, pattern_span) {
                                            self.emit_error(e);
                                        }
                                    }
                                }
                                Err(e) => self.emit_error(e),
                            }
                        } else if let Some(pattern) = arm.patterns.first() {
                            // Single pattern
                            if let Err(e) = self.add_pattern_bindings(pattern, scr_ty) {
                                self.emit_error(e);
                            }
                        }
                    } else {
                        // Expression-less match: patterns must be Guard expressions
                        for pattern in &arm.patterns {
                            if let PatternKind::Guard(expr) = &pattern.kind {
                                let ty = self.infer_expr(expr);
                                if !ty.is_error() {
                                    self.unify(&ty, &Type::simple("bool"), &expr.span);
                                }
                            } else if !matches!(pattern.kind, PatternKind::Default) {
                                self.emit_error(SoppoError::Type {
                                    message:
                                        "Expression-less match requires boolean guard expressions"
                                            .to_string(),
                                    span: pattern.span,
                                });
                            }
                        }
                    }

                    // Type check the arm body
                    let arm_ty = self.infer_block(&arm.body);

                    // Track if this arm diverges
                    if !matches!(arm_ty, Type::Never) {
                        all_arms_diverge = false;
                    }

                    // Pop nil scope if we pushed one
                    if needs_nil_narrowing {
                        self.pop_nil_scope();
                    }

                    // Pop the scope after processing the arm
                    self.pop_scope();
                }

                // Exhaustiveness check for enum types (only for normal match)
                if let Some(ref scrutinee_ty) = scrutinee_ty
                    && let Type::Con { sym: name, .. } = scrutinee_ty
                    && let Some(type_def) = self.global_state.lookup_type(&name.name)
                    && let TypeDefKind::Enum { variants } = &type_def.kind
                {
                    // Check if any arm is Default (catch-all)
                    let has_default = arms.iter().any(|arm| {
                        arm.patterns
                            .iter()
                            .any(|p| matches!(&p.kind, PatternKind::Default))
                    });

                    if !has_default {
                        // Collect covered variants from all patterns in all arms
                        let covered: HashSet<String> = arms
                            .iter()
                            .flat_map(|arm| arm.patterns.iter())
                            .filter_map(|pattern| match &pattern.kind {
                                PatternKind::Variant { name, .. } => {
                                    // Extract variant name from qualified name like "Colour.Red"
                                    Some(name.rsplit('.').next().unwrap_or(name).to_string())
                                }
                                PatternKind::Destructor { name, .. } => {
                                    Some(name.rsplit('.').next().unwrap_or(name).to_string())
                                }
                                PatternKind::StructDestructor { name, .. } => {
                                    Some(name.rsplit('.').next().unwrap_or(name).to_string())
                                }
                                _ => None,
                            })
                            .collect();

                        // Find missing variants
                        let missing: Vec<String> = variants
                            .iter()
                            .filter_map(|v| {
                                let vname = match v {
                                    EnumVariant::Unit { ident, .. } => &ident.name,
                                    EnumVariant::Single { ident, .. } => &ident.name,
                                    EnumVariant::Struct { ident, .. } => &ident.name,
                                };
                                if !covered.contains(vname) {
                                    Some(vname.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();

                        if !missing.is_empty() {
                            self.emit_error(SoppoError::NonExhaustive {
                                missing,
                                span: stmt.span,
                            });
                        }
                    }
                }

                // If all arms diverge and the match is exhaustive, the whole match diverges
                if all_arms_diverge && (has_default || is_exhaustive_enum) {
                    Ok(Type::Never)
                } else {
                    Ok(Type::unit())
                }
            }

            StmtKind::Send { channel, value } => {
                // ch <- value: channel must be chan T, value must be T
                let channel_ty = self.infer_expr(channel);
                let value_ty = self.infer_expr(value);

                if !channel_ty.is_error() && !value_ty.is_error() {
                    let channel_ty = self.substitute(channel_ty);
                    let is_nil = matches!(value.kind, ExprKind::Nil);

                    // Extract element type from channel and check nil safety
                    if let Some(elem_ty) = Self::extract_channel_element(&channel_ty) {
                        // Check: sending nil to a channel with non-nilable element type is an error
                        if is_nil
                            && let Some(err) = Self::check_nil_to_non_nilable(&elem_ty, value.span)
                        {
                            self.emit_error(err);
                        } else {
                            self.unify(&elem_ty, &value_ty, &value.span);
                        }
                    }
                }

                Ok(Type::unit())
            }

            StmtKind::Select { cases } => {
                for case in cases {
                    self.push_scope();

                    match &case.kind {
                        SelectCaseKind::Recv { channel } => {
                            // <-ch: just infer the channel type
                            self.infer_expr(channel);
                        }
                        SelectCaseKind::RecvDecl { ident, channel } => {
                            // v := <-ch: infer channel type, declare v with element type
                            let channel_ty = self.infer_expr(channel);
                            let elem_ty = if channel_ty.is_error() {
                                Type::error()
                            } else {
                                let channel_ty = self.substitute(channel_ty);
                                Self::extract_channel_element(&channel_ty)
                                    .unwrap_or_else(|| self.fresh_ty_var())
                            };

                            if let Err(e) =
                                self.insert_var(ident.name.clone(), elem_ty, Some(ident.span))
                            {
                                self.emit_error(e);
                            }
                        }
                        SelectCaseKind::RecvDeclOk {
                            ident,
                            ok_ident,
                            channel,
                        } => {
                            // v, ok := <-ch: infer channel type, declare v and ok
                            let channel_ty = self.infer_expr(channel);
                            let elem_ty = if channel_ty.is_error() {
                                Type::error()
                            } else {
                                let channel_ty = self.substitute(channel_ty);
                                Self::extract_channel_element(&channel_ty)
                                    .unwrap_or_else(|| self.fresh_ty_var())
                            };

                            if let Err(e) =
                                self.insert_var(ident.name.clone(), elem_ty, Some(ident.span))
                            {
                                self.emit_error(e);
                            }
                            if let Err(e) = self.insert_var(
                                ok_ident.name.clone(),
                                Type::simple("bool"),
                                Some(ok_ident.span),
                            ) {
                                self.emit_error(e);
                            }
                        }
                        SelectCaseKind::Send { channel, value } => {
                            // ch <- value: same as Send statement
                            let channel_ty = self.infer_expr(channel);
                            let value_ty = self.infer_expr(value);
                            let is_nil = matches!(value.kind, ExprKind::Nil);

                            if !channel_ty.is_error() && !value_ty.is_error() {
                                let channel_ty = self.substitute(channel_ty);
                                if let Some(elem_ty) = Self::extract_channel_element(&channel_ty) {
                                    // Check nil safety
                                    if is_nil
                                        && let Some(err) =
                                            Self::check_nil_to_non_nilable(&elem_ty, value.span)
                                    {
                                        self.emit_error(err);
                                    } else {
                                        self.unify(&elem_ty, &value_ty, &value.span);
                                    }
                                }
                            }
                        }
                        SelectCaseKind::Default => {
                            // default: nothing to infer
                        }
                    }

                    // Infer body
                    self.infer_block(&case.body);

                    self.pop_scope();
                }

                Ok(Type::unit())
            }

            StmtKind::Go(expr) => {
                // go expr: expr should be a function call
                self.infer_expr(expr);
                Ok(Type::unit())
            }

            StmtKind::DeferStmt(expr) => {
                // defer expr: expr should be a function call
                self.infer_expr(expr);
                Ok(Type::unit())
            }

            StmtKind::Break | StmtKind::Continue => {
                // break/continue diverge from normal control flow
                Ok(Type::Never)
            }

            StmtKind::Expr(expr) => Ok(self.infer_expr(expr)),

            StmtKind::CompoundAssign {
                target,
                op: _,
                value,
            } => {
                // Compound assignment: x += value
                // Check that target and value types are compatible
                let target_ty = self.infer_expr(target);
                let value_ty = self.infer_expr(value);
                if !target_ty.is_error() && !value_ty.is_error() {
                    self.unify(&target_ty, &value_ty, &value.span);
                }
                Ok(Type::unit())
            }

            StmtKind::IncDec { target, is_inc: _ } => {
                // Increment/decrement: x++ or x--
                // Just infer the target type (should be numeric)
                self.infer_expr(target);
                Ok(Type::unit())
            }

            StmtKind::TryStmt {
                stmt: inner_stmt,
                error_name,
                handler,
                discard_count,
                ..
            } => {
                // Infer inner statement and extract expression type + span
                // For ? operator, we need to strip the error from tuple types
                let (expr_ty, expr_span) = match &inner_stmt.kind {
                    StmtKind::Decl { ident, value } => {
                        let value_ty = self.infer_expr(value);
                        if value_ty.is_error() {
                            // Still define the variable with error type
                            if let Err(e) =
                                self.insert_var(ident.name.clone(), Type::error(), Some(ident.span))
                            {
                                self.emit_error(e);
                            }
                            return Ok(Type::unit());
                        }
                        let value_ty_sub = self.substitute(value_ty.clone());

                        // If expression returns only `error`, can't assign to a variable with `?`
                        // The `?` strips the error, leaving nothing to assign
                        if self.is_error_type(&value_ty_sub) {
                            self.emit_error(SoppoError::TryCapturesError { span: ident.span });
                            if let Err(e) =
                                self.insert_var(ident.name.clone(), Type::error(), Some(ident.span))
                            {
                                self.emit_error(e);
                            }
                            return Ok(Type::unit());
                        }

                        // Strip error from tuple type for the variable
                        let var_ty = self.strip_error_from_tuple(&value_ty_sub);
                        if let Err(e) =
                            self.insert_var(ident.name.clone(), var_ty.clone(), Some(ident.span))
                        {
                            self.emit_error(e);
                        }
                        self.update_nil_state_for_assignment(&ident.name, value, &var_ty);

                        // Record variable definition for LSP
                        self.record_symbol(
                            ident.span,
                            SymbolInfo {
                                name: ident.name.clone(),
                                ty: var_ty,
                                definition_span: Some(stmt.span),
                                name_span: Some(ident.span),
                                kind: SymbolKind::Variable,
                                doc_comment: None,
                                go_location: None,
                            },
                        );

                        (value_ty_sub, value.span)
                    }
                    StmtKind::MultiDecl {
                        ident: names,
                        values,
                    } if values.len() == 1 => {
                        let value_ty = self.infer_expr(&values[0]);
                        if value_ty.is_error() {
                            // Still define the variables with error type
                            for var_ident in names {
                                if let Err(e) = self.insert_var(
                                    var_ident.name.clone(),
                                    Type::error(),
                                    Some(var_ident.span),
                                ) {
                                    self.emit_error(e);
                                }
                            }
                            return Ok(Type::unit());
                        }
                        let value_ty_sub = self.substitute(value_ty.clone());

                        // For multi-return, the type is a tuple
                        // Unpack and assign to each name, excluding the error
                        if let Type::Con {
                            sym: tname, args, ..
                        } = &value_ty_sub
                            && tname.name == "tuple"
                        {
                            // Exclude the last element (error) when assigning
                            let non_error_count = args.len().saturating_sub(1);

                            // Error if user provides more names than non-error values
                            // e.g., `result, err := foo() ?` when foo returns (T, error)
                            // The `?` already handles the error, so capturing it is wrong
                            if names.len() > non_error_count {
                                self.emit_error(SoppoError::TryCapturesError {
                                    span: names[non_error_count].span,
                                });
                                // Still define all variables with error type
                                for var_ident in names {
                                    if let Err(e) = self.insert_var(
                                        var_ident.name.clone(),
                                        Type::error(),
                                        Some(var_ident.span),
                                    ) {
                                        self.emit_error(e);
                                    }
                                }
                                return Ok(Type::unit());
                            }

                            for (i, var_ident) in names.iter().enumerate() {
                                if i < non_error_count
                                    && let Some(ty) = args.get(i)
                                {
                                    if let Err(e) = self.insert_var(
                                        var_ident.name.clone(),
                                        ty.clone(),
                                        Some(var_ident.span),
                                    ) {
                                        self.emit_error(e);
                                    }

                                    // Record variable definition for LSP
                                    self.record_symbol(
                                        var_ident.span,
                                        SymbolInfo {
                                            name: var_ident.name.clone(),
                                            ty: ty.clone(),
                                            definition_span: Some(stmt.span),
                                            name_span: Some(var_ident.span),
                                            kind: SymbolKind::Variable,
                                            doc_comment: None,
                                            go_location: None,
                                        },
                                    );
                                }
                            }
                        }
                        (value_ty_sub, values[0].span)
                    }
                    StmtKind::Assign { target, value } => {
                        let value_ty = self.infer_expr(value);
                        let target_ty = self.infer_expr(target);

                        if value_ty.is_error() || target_ty.is_error() {
                            return Ok(Type::unit());
                        }

                        let value_ty_sub = self.substitute(value_ty.clone());
                        // For assignment, we expect the target to already have the non-error type
                        let expected_ty = self.strip_error_from_tuple(&value_ty_sub);
                        self.unify(&target_ty, &expected_ty, &inner_stmt.span);
                        (value_ty_sub, value.span)
                    }
                    StmtKind::Expr(expr) => {
                        let expr_ty = self.infer_expr(expr);
                        if expr_ty.is_error() {
                            return Ok(Type::unit());
                        }
                        let expr_ty_sub = self.substitute(expr_ty);

                        // Calculate how many non-error values to discard
                        // For tuple[T, error] -> 1, for tuple[T, U, error] -> 2, for error -> 0
                        let count = if let Type::Con {
                            sym: name, args, ..
                        } = &expr_ty_sub
                            && name.name == "tuple"
                        {
                            // Count non-error args (all except the last one which is error)
                            args.len().saturating_sub(1)
                        } else {
                            // Single error return
                            0
                        };
                        discard_count.set(count);

                        (expr_ty_sub, expr.span)
                    }
                    _ => {
                        self.emit_error(SoppoError::Type {
                            message: "`?` can only be used with declarations, assignments, or expression statements".to_string(),
                            span: inner_stmt.span,
                        });
                        return Ok(Type::unit());
                    }
                };

                let expr_ty_sub = self.substitute(expr_ty.clone());

                // Verify expression returns error
                if !self.returns_error(&expr_ty_sub) {
                    self.emit_error(SoppoError::TryExprNoError { span: expr_span });
                    // Continue to check handler if present
                }

                // If handler present, infer it with error_name in scope
                if let Some(block) = handler {
                    self.push_scope();
                    if let Some(name) = error_name {
                        if let Err(e) =
                            self.insert_var(name.clone(), Type::simple("error"), Some(stmt.span))
                        {
                            self.emit_error(e);
                        }
                        // Error is known to be non-nil in the handler (handler only runs on error)
                        self.set_nil_state(name.clone(), Nullability::NonNull);
                    }
                    self.infer_block(block);
                    self.pop_scope();
                }

                // Mark assigned nilable variables as non-null (success implies valid result)
                // Use lookup_var_type to avoid marking the variable as "used"
                if let Some(var_name) = self.get_assigned_var_name(inner_stmt)
                    && let Some(var_type) = self.lookup_var_type(&var_name)
                {
                    let var_type_sub = self.substitute(var_type);
                    if Self::is_nilable_type(&var_type_sub) {
                        self.set_nil_state(var_name, Nullability::NonNull);
                    }
                }

                Ok(Type::unit())
            }

            StmtKind::LocalTypeDecl(type_decl) => {
                // Register the local type in the current module
                // This reuses the same type inference as top-level type declarations
                self.infer_type_decl(type_decl)?;
                Ok(Type::unit())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{Decl, FileId, Parser};

    #[test]
    fn test_infer_let_stmt() {
        let source = r#"
            func test() int {
                x := 42
                return x
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                assert!(infer.infer_func_decl(func).is_ok());
            }
        }
    }

    #[test]
    fn test_infer_multiple_lets() {
        let source = r#"
            func test() int {
                x := 1
                y := 2
                return x + y
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                assert!(infer.infer_func_decl(func).is_ok());
            }
        }
    }

    #[test]
    fn test_infer_return_stmt() {
        let source = r#"
            func test() string {
                return "hello"
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                assert!(infer.infer_func_decl(func).is_ok());
            }
        }
    }

    #[test]
    fn test_variable_redeclaration_error() {
        let source = r#"
            func test() int {
                x := 1
                x := 2
                return x
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                let _ = infer.infer_func_decl(func);
                assert!(
                    infer.has_errors(),
                    "Expected redeclaration error to be collected"
                );
                let errors = infer.errors();
                assert!(
                    errors.iter().any(|e| e.to_string().contains("redeclared")),
                    "Expected redeclaration error, got: {:?}",
                    errors
                );
            }
        }
    }

    #[test]
    fn test_array_index_type() {
        let source = r#"
            func test() int {
                x := []int{1, 2, 3}
                return x[0]
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                assert!(infer.infer_func_decl(func).is_ok());
            }
        }
    }

    #[test]
    fn test_array_element_type_checking() {
        let source = r#"
            func test() string {
                x := []int{1, 2, 3}
                return x[0]
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                let _ = infer.infer_func_decl(func);
                // Should fail: returning int where string expected
                assert!(
                    infer.has_errors(),
                    "Expected type mismatch error to be collected"
                );
            }
        }
    }
}
