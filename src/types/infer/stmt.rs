use std::collections::HashSet;

use super::Infer;
use crate::error::{SoppoError, SoppoResult};
use crate::syntax::{
    BinOp, EnumVariant, Expr, ExprKind, Literal, PatternKind, SelectCaseKind, Stmt, StmtKind,
};
use crate::types::ast::{
    TypedExprKind, TypedSelectCase, TypedSelectCaseKind, TypedStmt, TypedStmtKind,
};
use crate::types::ctx::TypeDefKind;
use crate::types::ty::Nullability;
use crate::types::{SymbolInfo, SymbolKind, Type, TypedExpr};

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

/// Convert a typed expression to a trackable key string
/// Returns Some(key) for identifiers and field access chains (e.g., "user.profile.address")
/// Returns None for complex expressions that can't be tracked
pub(super) fn typed_expr_to_key(expr: &TypedExpr) -> Option<String> {
    match &expr.kind {
        TypedExprKind::Ident(name) => Some(name.clone()),
        TypedExprKind::Field {
            expr: base, field, ..
        } => {
            let base_key = typed_expr_to_key(base)?;
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
    /// Infer a statement and return a TypedStmt directly.
    ///
    /// **Prefer `infer_stmt`** which collects errors and returns an error TypedStmt on failure.
    /// This version should only be used when you need to explicitly check if inference failed.
    ///
    /// Returns a TypedStmt containing the typed statement kind and its span.
    pub fn infer_stmt_inner(&mut self, stmt: &Stmt) -> SoppoResult<TypedStmt> {
        let kind = match &stmt.kind {
            StmtKind::Decl { ident, value } => {
                let typed_value = self.infer_expr(value);
                let value_ty = typed_value.ty.clone();
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
                        ty: value_ty.clone(),
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
                TypedStmtKind::Decl {
                    ident: ident.clone(),
                    var_ty: value_ty,
                    value: typed_value,
                }
            }

            StmtKind::MultiDecl {
                ident: names,
                values,
            } => {
                // Infer all values first
                let typed_values: Vec<_> = values.iter().map(|v| self.infer_expr(v)).collect();

                let var_tys = if values.len() == 1 && names.len() > 1 {
                    // a, b := f() (multi-return unpacking)
                    let value = &values[0];
                    let typed_value = &typed_values[0];

                    // Check for comma-ok idiom: v, ok := expr
                    if names.len() == 2 {
                        if let Some((value_ty, ok_ty)) = self.infer_comma_ok_expr(value) {
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
                                vec![Type::error(); names.len()]
                            } else {
                                let vars = vec![
                                    (names[0].name.clone(), value_ty.clone(), Some(names[0].span)),
                                    (names[1].name.clone(), ok_ty.clone(), Some(names[1].span)),
                                ];
                                if let Err(e) = self.insert_short_decl_vars(&vars) {
                                    self.emit_error(e);
                                }
                                vec![value_ty, ok_ty]
                            }
                        } else {
                            // Not comma-ok, handle as tuple unpacking below
                            self.handle_multi_decl_tuple_unpack(names, typed_value, value)
                        }
                    } else {
                        // More than 2 names, must be tuple unpacking
                        self.handle_multi_decl_tuple_unpack(names, typed_value, value)
                    }
                } else {
                    // a, b := expr1, expr2 (one value per name)
                    let mut vars = Vec::with_capacity(names.len());
                    let mut var_tys = Vec::with_capacity(names.len());
                    for (ident, typed_value) in names.iter().zip(typed_values.iter()) {
                        let value_ty = typed_value.ty.clone();
                        vars.push((ident.name.clone(), value_ty.clone(), Some(ident.span)));
                        var_tys.push(value_ty);
                    }
                    if let Err(e) = self.insert_short_decl_vars(&vars) {
                        self.emit_error(e);
                    }
                    var_tys
                };

                TypedStmtKind::MultiDecl {
                    idents: names.clone(),
                    var_tys,
                    values: typed_values,
                }
            }

            StmtKind::VarDecl { ident, ty, value } => {
                // Record type annotation for LSP if present
                if let Some(t) = ty {
                    self.record_type_annotation(t);
                }

                let has_explicit_type = ty.is_some();
                let (var_ty, typed_value) = match (ty, value) {
                    (Some(t), Some(expr)) => {
                        // var x type = value: unify declared with inferred
                        let declared_ty = self.resolve_type(t);
                        let typed_expr = self.infer_expr(expr);

                        // Check: assigning nil to a non-nilable type is an error
                        if matches!(expr.kind, ExprKind::Nil) {
                            if let Some(err) = Self::check_nil_to_non_nilable(&declared_ty, t.span)
                            {
                                self.emit_error(err);
                            }
                        } else if !typed_expr.ty.is_error() {
                            self.unify(&declared_ty, &typed_expr.ty, &expr.span);
                        }
                        (declared_ty, Some(typed_expr))
                    }
                    (Some(t), None) => {
                        // var x type: use declared type (zero value)
                        let declared_ty = self.resolve_type(t);

                        // Check: non-nilable types require initialisation
                        if declared_ty.is_nilable_kind() && !declared_ty.is_nullable() {
                            self.emit_error(SoppoError::NonNilableNoInit {
                                ty: declared_ty.to_string(),
                                span: stmt.span,
                            });
                        }
                        (declared_ty, None)
                    }
                    (None, Some(expr)) => {
                        // var x = value: infer from value
                        let typed_expr = self.infer_expr(expr);
                        (typed_expr.ty.clone(), Some(typed_expr))
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
                        ty: var_ty.clone(),
                        definition_span: Some(stmt.span),
                        name_span: Some(ident.span),
                        kind: SymbolKind::Variable,
                        doc_comment: None,
                        go_location: None,
                    },
                );

                // Track nil state for nilable types (only if not error type)
                if !var_ty_sub.is_error() {
                    if let Some(expr) = value {
                        self.update_nil_state_for_assignment(&ident.name, expr, &var_ty_sub);
                    } else if Self::is_nilable_type(&var_ty_sub) {
                        // Zero-initialised nilable types are nil
                        self.set_nil_state(
                            ident.name.clone(),
                            crate::types::ty::Nullability::Nullable,
                        );
                    }
                }
                TypedStmtKind::VarDecl {
                    ident: ident.clone(),
                    var_ty,
                    has_explicit_type,
                    value: typed_value,
                }
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

                let has_explicit_type = ty.is_some();
                let declared_ty = ty.as_ref().map(|t| self.resolve_type(t));

                // Infer all values
                let typed_values: Vec<_> = values.iter().map(|v| self.infer_expr(v)).collect();

                let var_ty = if values.is_empty() {
                    // var a, b, c type (zero values)
                    match &declared_ty {
                        Some(t) => t.clone(),
                        None => {
                            self.emit_error(SoppoError::Type {
                                message:
                                    "Multi-variable declaration without values requires a type"
                                        .to_string(),
                                span: stmt.span,
                            });
                            Type::error()
                        }
                    }
                } else if typed_values.len() == 1 {
                    // Single value - use its type or declared type
                    declared_ty
                        .clone()
                        .unwrap_or_else(|| typed_values[0].ty.clone())
                } else {
                    // Multiple values - use declared type or first value's type
                    declared_ty
                        .clone()
                        .unwrap_or_else(|| typed_values[0].ty.clone())
                };

                // Insert variables
                for ident in names {
                    if let Err(e) =
                        self.insert_var(ident.name.clone(), var_ty.clone(), Some(ident.span))
                    {
                        self.emit_error(e);
                    }
                }

                TypedStmtKind::MultiVarDecl {
                    idents: names.clone(),
                    var_ty,
                    has_explicit_type,
                    values: typed_values,
                }
            }

            StmtKind::ConstDecl { ident, ty, value } => {
                let has_explicit_type = ty.is_some();
                let typed_value = self.infer_expr(value);

                // Determine the constant's type
                let const_ty = if typed_value.ty.is_error() {
                    Type::error()
                } else if let Some(t) = ty {
                    // const x type = value: unify declared with inferred
                    let declared_ty = self.resolve_type(t);
                    self.unify(&declared_ty, &typed_value.ty, &value.span);
                    declared_ty
                } else {
                    // const x = value: infer from value
                    typed_value.ty.clone()
                };

                if let Err(e) =
                    self.insert_var(ident.name.clone(), const_ty.clone(), Some(ident.span))
                {
                    self.emit_error(e);
                }
                TypedStmtKind::ConstDecl {
                    ident: ident.clone(),
                    const_ty,
                    has_explicit_type,
                    value: typed_value,
                }
            }

            StmtKind::MultiConstDecl { idents, ty, values } => {
                let has_explicit_type = ty.is_some();
                let typed_values: Vec<_> = values.iter().map(|v| self.infer_expr(v)).collect();

                // const a, b = expr1, expr2 or const a, b type = expr1, expr2
                let const_ty = ty
                    .as_ref()
                    .map(|t| self.resolve_type(t))
                    .unwrap_or_else(|| {
                        typed_values
                            .first()
                            .map(|v| v.ty.clone())
                            .unwrap_or_else(Type::error)
                    });

                for (ident, typed_value) in idents.iter().zip(typed_values.iter()) {
                    let var_ty = if typed_value.ty.is_error() {
                        Type::error()
                    } else if let Some(t) = ty {
                        let declared_ty = self.resolve_type(t);
                        self.unify(&declared_ty, &typed_value.ty, &typed_value.span);
                        declared_ty
                    } else {
                        typed_value.ty.clone()
                    };
                    if let Err(e) = self.insert_var(ident.name.clone(), var_ty, Some(ident.span)) {
                        self.emit_error(e);
                    }
                }
                TypedStmtKind::MultiConstDecl {
                    idents: idents.clone(),
                    const_ty,
                    has_explicit_type,
                    values: typed_values,
                }
            }

            StmtKind::Assign { target, value } => {
                let typed_target = self.infer_expr(target);
                let typed_value = self.infer_expr(value);

                // Special case: blank identifier accepts any type
                let is_blank = matches!(&target.kind, ExprKind::Ident(name) if name == "_");

                if !is_blank && !typed_target.ty.is_error() && !typed_value.ty.is_error() {
                    let target_ty_sub = self.substitute(typed_target.ty.clone());

                    // Check: assigning nil to a non-nilable type is an error
                    if matches!(value.kind, ExprKind::Nil) {
                        if let Some(err) =
                            Self::check_nil_to_non_nilable(&target_ty_sub, value.span)
                        {
                            self.emit_error(err);
                        }
                    } else {
                        let value_ty_sub = self.substitute(typed_value.ty.clone());
                        self.unify(&typed_target.ty, &typed_value.ty, &stmt.span);
                        // Update nil state and variant state for reassignment
                        if let ExprKind::Ident(name) = &target.kind {
                            self.update_nil_state_for_assignment(name, value, &value_ty_sub);
                            self.update_variant_state_for_assignment(name, value);
                        }
                    }
                }
                TypedStmtKind::Assign {
                    target: typed_target,
                    value: typed_value,
                }
            }

            StmtKind::MultiAssign { targets, values } => {
                // Infer all targets and values
                let typed_targets: Vec<_> = targets.iter().map(|t| self.infer_expr(t)).collect();
                let typed_values: Vec<_> = values.iter().map(|v| self.infer_expr(v)).collect();

                // Perform unification as side effects
                if values.len() == 1 && targets.len() > 1 {
                    // a, b = f() (multi-return unpacking)
                    let value_ty = &typed_values[0].ty;
                    if !value_ty.is_error() {
                        let value_ty_sub = self.substitute(value_ty.clone());
                        if let Type::Con {
                            sym: type_name,
                            args,
                            ..
                        } = &value_ty_sub
                        {
                            if type_name.name == "tuple" && args.len() == targets.len() {
                                for (typed_target, expected_ty) in
                                    typed_targets.iter().zip(args.iter())
                                {
                                    if let ExprKind::Ident(name) = &targets[typed_targets
                                        .iter()
                                        .position(|t| t.span == typed_target.span)
                                        .unwrap_or(0)]
                                    .kind
                                        && name == "_"
                                    {
                                        continue;
                                    }
                                    if !typed_target.ty.is_error() {
                                        self.unify(
                                            &typed_target.ty,
                                            expected_ty,
                                            &typed_target.span,
                                        );
                                    }
                                }
                            } else {
                                self.emit_error(SoppoError::Type {
                                    message: format!(
                                        "Cannot unpack {} values from type `{}`",
                                        targets.len(),
                                        value_ty_sub
                                    ),
                                    span: values[0].span,
                                });
                            }
                        } else {
                            self.emit_error(SoppoError::Type {
                                message: format!(
                                    "Cannot unpack {} values from type `{}`",
                                    targets.len(),
                                    value_ty_sub
                                ),
                                span: values[0].span,
                            });
                        }
                    }
                } else {
                    // a, b = expr1, expr2 (one value per target)
                    for (typed_target, typed_value) in typed_targets.iter().zip(typed_values.iter())
                    {
                        if !typed_target.ty.is_error() && !typed_value.ty.is_error() {
                            self.unify(&typed_target.ty, &typed_value.ty, &typed_target.span);
                        }
                    }
                }

                TypedStmtKind::MultiAssign {
                    targets: typed_targets,
                    values: typed_values,
                }
            }

            StmtKind::For { condition, body } => {
                // Check condition is bool
                let typed_condition = self.infer_expr(condition);
                if !typed_condition.ty.is_error() {
                    self.unify(&Type::simple("bool"), &typed_condition.ty, &condition.span);
                }

                // Type check body
                let typed_body = self.infer_block(body);

                TypedStmtKind::For {
                    condition: typed_condition,
                    body: typed_body,
                }
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
                let typed_init = init
                    .as_ref()
                    .map(|init_stmt| Box::new(self.infer_stmt(init_stmt)));

                // Check condition is bool if present
                let typed_condition = condition.as_ref().map(|cond| {
                    let typed_cond = self.infer_expr(cond);
                    if !typed_cond.ty.is_error() {
                        self.unify(&Type::simple("bool"), &typed_cond.ty, &cond.span);
                    }
                    typed_cond
                });

                // Type check body
                let typed_body = self.infer_block(body);

                // Type check post statement if present
                let typed_post = post
                    .as_ref()
                    .map(|post_stmt| Box::new(self.infer_stmt(post_stmt)));

                self.pop_scope();

                TypedStmtKind::ForCStyle {
                    init: typed_init,
                    condition: typed_condition,
                    post: typed_post,
                    body: typed_body,
                }
            }

            StmtKind::ForRange {
                key,
                value,
                collection,
                body,
            } => {
                // Infer collection type
                let typed_collection = self.infer_expr(collection);
                let coll_ty = self.substitute(typed_collection.ty.clone());

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
                if let Err(e) = self.insert_var(key.name.clone(), key_ty.clone(), Some(key.span)) {
                    self.emit_error(e);
                }

                // Bind the value variable if present
                if let Some(val_ident) = value
                    && let Err(e) = self.insert_var(
                        val_ident.name.clone(),
                        value_ty.clone(),
                        Some(val_ident.span),
                    )
                {
                    self.emit_error(e);
                }

                // Type check body
                let typed_body = self.infer_block(body);

                self.pop_scope();

                TypedStmtKind::ForRange {
                    key: key.clone(),
                    key_ty,
                    value: value.clone(),
                    value_ty: value.as_ref().map(|_| value_ty),
                    collection: typed_collection,
                    body: typed_body,
                }
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
                let typed_init = init
                    .as_ref()
                    .map(|init_stmt| Box::new(self.infer_stmt(init_stmt)));

                // Check condition is bool
                let typed_condition = self.infer_expr(condition);
                if !typed_condition.ty.is_error() {
                    self.unify(&Type::simple("bool"), &typed_condition.ty, &condition.span);
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
                let typed_then = self.infer_block(then_block);
                let then_diverges = typed_then.diverges();
                self.pop_nil_scope();

                // Type check else block with opposite narrowing
                let (typed_else, else_diverges) = if let Some(else_blk) = else_block {
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
                    let typed_blk = self.infer_block(else_blk);
                    let diverges = typed_blk.diverges();
                    self.pop_nil_scope();
                    (Some(typed_blk), diverges)
                } else {
                    (None, false)
                };

                // Handle early return narrowing:
                // If then block diverges (returns/breaks) and condition was `x == nil`,
                // then after the if statement, x is known to be non-nil
                for check in &nil_checks {
                    if then_diverges && !check.is_not_nil {
                        // `if x == nil { return }` - x is non-nil after this point
                        self.set_nil_state(check.expr_key.clone(), Nullability::NonNull);
                    }
                    // Similarly, if else block diverges and condition was `x != nil`
                    if else_diverges && check.is_not_nil && else_block.is_some() {
                        // `if x != nil { ... } else { return }` - x is non-nil after
                        self.set_nil_state(check.expr_key.clone(), Nullability::NonNull);
                    }

                    // Handle error companion narrowing:
                    // `if err != nil { return }` - after this, err's companions are non-nil
                    // This is the common Go idiom: `resp, err := f(); if err != nil { return }`
                    if then_diverges && check.is_not_nil {
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

                TypedStmtKind::If {
                    init: typed_init,
                    condition: typed_condition,
                    then_block: typed_then,
                    else_block: typed_else,
                }
            }

            StmtKind::Return { values } => {
                // Infer all return values
                let typed_values: Vec<_> = values.iter().map(|v| self.infer_expr(v)).collect();

                // Check return values against expected return types
                if let Some(expected_types) = self.expected_return_types.clone() {
                    // Handle `return f()` where f() returns a tuple matching expected types
                    if typed_values.len() == 1 && expected_types.len() > 1 {
                        let value_ty = &typed_values[0].ty;
                        if !value_ty.is_error() {
                            let value_ty_sub = self.substitute(value_ty.clone());

                            // Check if it's a tuple type with matching arity
                            if let Type::Con { sym, args, .. } = &value_ty_sub
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
                    } else if typed_values.len() != expected_types.len() {
                        self.emit_error(SoppoError::Type {
                            message: format!(
                                "Expected {} return value(s), got {}",
                                expected_types.len(),
                                values.len()
                            ),
                            span: stmt.span,
                        });
                    } else {
                        for (i, (typed_expr, expected)) in
                            typed_values.iter().zip(expected_types.iter()).enumerate()
                        {
                            // Check: returning nil to a non-nilable type is an error
                            if matches!(values[i].kind, ExprKind::Nil)
                                && let Some(err) =
                                    Self::check_nil_to_non_nilable(expected, values[i].span)
                            {
                                self.emit_error(err);
                                continue;
                            }

                            if !typed_expr.ty.is_error() {
                                self.unify(expected, &typed_expr.ty, &typed_expr.span);
                                // After unification, set inferred type args on expressions that need them
                                self.set_inferred_type_args(&values[i], expected);
                            }
                        }
                    }
                }
                TypedStmtKind::Return {
                    values: typed_values,
                }
            }

            StmtKind::Match { scrutinee, arms } => {
                // Expression-less match has no scrutinee
                let (typed_scrutinee, scrutinee_ty, scrutinee_key) =
                    if let Some(scrutinee_expr) = scrutinee {
                        let typed_expr = self.infer_expr(scrutinee_expr);
                        let key = expr_to_key(scrutinee_expr);
                        if typed_expr.ty.is_error() {
                            (Some(typed_expr), None, key) // Continue checking arms even if scrutinee failed
                        } else {
                            let ty = self.substitute(typed_expr.ty.clone());
                            (Some(typed_expr), Some(ty), key)
                        }
                    } else {
                        (None, None, None)
                    };

                // Check if scrutinee is nilable (for nil narrowing in non-nil arms)
                let scrutinee_is_nilable = scrutinee_ty
                    .as_ref()
                    .map(|ty| ty.is_nullable())
                    .unwrap_or(false);

                // The matched type for building patterns
                let matched_ty = scrutinee_ty.clone().unwrap_or_else(Type::unit);

                // Build typed arms in a single pass
                let mut typed_arms = Vec::with_capacity(arms.len());
                let mut has_default = false;

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
                                let ty = self.infer_expr_ty(expr);
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

                    // Build typed patterns
                    let typed_patterns: Vec<_> = arm
                        .patterns
                        .iter()
                        .map(|p| self.build_typed_pattern(p, &matched_ty))
                        .collect();

                    // Type check the arm body
                    let typed_arm_body = self.infer_block(&arm.body);

                    // Pop nil scope if we pushed one
                    if needs_nil_narrowing {
                        self.pop_nil_scope();
                    }

                    // Pop the scope after processing the arm
                    self.pop_scope();

                    // Add the typed arm
                    typed_arms.push(crate::types::ast::TypedArm {
                        patterns: typed_patterns,
                        body: typed_arm_body,
                        span: arm.span,
                    });
                }

                // Exhaustiveness check for enum types (only for normal match)
                if let Some(ref scr_ty) = scrutinee_ty
                    && let Type::Con { sym: name, .. } = scr_ty
                    && let Some(type_def) = self.global_state.lookup_type(&name.name)
                    && let TypeDefKind::Enum { variants } = &type_def.kind
                    && !has_default
                {
                    // Collect covered variants from all patterns in all arms
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

                TypedStmtKind::Match {
                    scrutinee: typed_scrutinee,
                    scrutinee_ty,
                    arms: typed_arms,
                }
            }

            StmtKind::Send { channel, value } => {
                // ch <- value: channel must be chan T, value must be T
                let typed_channel = self.infer_expr(channel);
                let typed_value = self.infer_expr(value);

                if !typed_channel.ty.is_error() && !typed_value.ty.is_error() {
                    let channel_ty = self.substitute(typed_channel.ty.clone());
                    let is_nil = matches!(value.kind, ExprKind::Nil);

                    // Extract element type from channel and check nil safety
                    if let Some(elem_ty) = Self::extract_channel_element(&channel_ty) {
                        // Check: sending nil to a channel with non-nilable element type is an error
                        if is_nil
                            && let Some(err) = Self::check_nil_to_non_nilable(&elem_ty, value.span)
                        {
                            self.emit_error(err);
                        } else {
                            self.unify(&elem_ty, &typed_value.ty, &value.span);
                        }
                    }
                }

                TypedStmtKind::Send {
                    channel: typed_channel,
                    value: typed_value,
                }
            }

            StmtKind::Select { cases } => {
                let typed_cases: Vec<_> = cases
                    .iter()
                    .map(|case| {
                        self.push_scope();

                        let typed_kind = match &case.kind {
                            SelectCaseKind::Recv { channel } => {
                                // <-ch: infer the channel type
                                let typed_channel = self.infer_expr(channel);
                                let recv_ty = if typed_channel.ty.is_error() {
                                    Type::error()
                                } else {
                                    let channel_ty = self.substitute(typed_channel.ty.clone());
                                    Self::extract_channel_element(&channel_ty)
                                        .unwrap_or_else(|| self.fresh_ty_var())
                                };
                                TypedSelectCaseKind::Recv {
                                    channel: typed_channel,
                                    recv_ty,
                                }
                            }
                            SelectCaseKind::RecvDecl { ident, channel } => {
                                // v := <-ch: infer channel type, declare v with element type
                                let typed_channel = self.infer_expr(channel);
                                let recv_ty = if typed_channel.ty.is_error() {
                                    Type::error()
                                } else {
                                    let channel_ty = self.substitute(typed_channel.ty.clone());
                                    Self::extract_channel_element(&channel_ty)
                                        .unwrap_or_else(|| self.fresh_ty_var())
                                };

                                if let Err(e) = self.insert_var(
                                    ident.name.clone(),
                                    recv_ty.clone(),
                                    Some(ident.span),
                                ) {
                                    self.emit_error(e);
                                }

                                TypedSelectCaseKind::RecvDecl {
                                    ident: ident.clone(),
                                    channel: typed_channel,
                                    recv_ty,
                                }
                            }
                            SelectCaseKind::RecvDeclOk {
                                ident,
                                ok_ident,
                                channel,
                            } => {
                                // v, ok := <-ch: infer channel type, declare v and ok
                                let typed_channel = self.infer_expr(channel);
                                let recv_ty = if typed_channel.ty.is_error() {
                                    Type::error()
                                } else {
                                    let channel_ty = self.substitute(typed_channel.ty.clone());
                                    Self::extract_channel_element(&channel_ty)
                                        .unwrap_or_else(|| self.fresh_ty_var())
                                };

                                if let Err(e) = self.insert_var(
                                    ident.name.clone(),
                                    recv_ty.clone(),
                                    Some(ident.span),
                                ) {
                                    self.emit_error(e);
                                }
                                if let Err(e) = self.insert_var(
                                    ok_ident.name.clone(),
                                    Type::simple("bool"),
                                    Some(ok_ident.span),
                                ) {
                                    self.emit_error(e);
                                }

                                TypedSelectCaseKind::RecvDeclOk {
                                    ident: ident.clone(),
                                    ok_ident: ok_ident.clone(),
                                    channel: typed_channel,
                                    recv_ty,
                                }
                            }
                            SelectCaseKind::Send { channel, value } => {
                                // ch <- value: same as Send statement
                                let typed_channel = self.infer_expr(channel);
                                let typed_value = self.infer_expr(value);
                                let is_nil = matches!(value.kind, ExprKind::Nil);

                                if !typed_channel.ty.is_error() && !typed_value.ty.is_error() {
                                    let channel_ty = self.substitute(typed_channel.ty.clone());
                                    if let Some(elem_ty) =
                                        Self::extract_channel_element(&channel_ty)
                                    {
                                        // Check nil safety
                                        if is_nil
                                            && let Some(err) =
                                                Self::check_nil_to_non_nilable(&elem_ty, value.span)
                                        {
                                            self.emit_error(err);
                                        } else {
                                            self.unify(&elem_ty, &typed_value.ty, &value.span);
                                        }
                                    }
                                }

                                TypedSelectCaseKind::Send {
                                    channel: typed_channel,
                                    value: typed_value,
                                }
                            }
                            SelectCaseKind::Default => TypedSelectCaseKind::Default,
                        };

                        // Infer body
                        let typed_body = self.infer_block(&case.body);

                        self.pop_scope();

                        TypedSelectCase {
                            kind: typed_kind,
                            body: typed_body,
                            span: case.span,
                        }
                    })
                    .collect();

                TypedStmtKind::Select { cases: typed_cases }
            }

            StmtKind::Go(expr) => {
                // go expr: expr should be a function call
                let typed_expr = self.infer_expr(expr);
                TypedStmtKind::Go(typed_expr)
            }

            StmtKind::DeferStmt(expr) => {
                // defer expr: expr should be a function call
                let typed_expr = self.infer_expr(expr);
                TypedStmtKind::DeferStmt(typed_expr)
            }

            StmtKind::Break => TypedStmtKind::Break,

            StmtKind::Continue => TypedStmtKind::Continue,

            StmtKind::Expr(expr) => TypedStmtKind::Expr(self.infer_expr(expr)),

            StmtKind::CompoundAssign { target, op, value } => {
                // Compound assignment: x += value
                // Check that target and value types are compatible
                let typed_target = self.infer_expr(target);
                let typed_value = self.infer_expr(value);
                if !typed_target.ty.is_error() && !typed_value.ty.is_error() {
                    self.unify(&typed_target.ty, &typed_value.ty, &value.span);
                }
                TypedStmtKind::CompoundAssign {
                    target: typed_target,
                    op: *op,
                    value: typed_value,
                }
            }

            StmtKind::IncDec { target, is_inc } => {
                // Increment/decrement: x++ or x--
                // Just infer the target type (should be numeric)
                let typed_target = self.infer_expr(target);
                TypedStmtKind::IncDec {
                    target: typed_target,
                    is_inc: *is_inc,
                }
            }

            StmtKind::TryStmt {
                stmt: inner_stmt,
                error_name,
                handler,
                try_span,
            } => {
                // Build the typed inner statement and track types for the ? operator
                // For ? operator, we need to strip the error from tuple types
                // Returns: (typed_inner_stmt, discard_count, discard_types, expr_ty, expr_span)
                let (typed_inner_stmt, discard_count, discard_types, expr_ty, expr_span) =
                    match &inner_stmt.kind {
                        StmtKind::Decl { ident, value } => {
                            let typed_value = self.infer_expr(value);
                            let value_ty = typed_value.ty.clone();

                            if value_ty.is_error() {
                                // Still define the variable with error type
                                if let Err(e) = self.insert_var(
                                    ident.name.clone(),
                                    Type::error(),
                                    Some(ident.span),
                                ) {
                                    self.emit_error(e);
                                }
                                let inner = TypedStmt::new(
                                    TypedStmtKind::Decl {
                                        ident: ident.clone(),
                                        var_ty: Type::error(),
                                        value: typed_value,
                                    },
                                    inner_stmt.span,
                                );
                                (inner, 0, vec![], Type::error(), value.span)
                            } else {
                                let value_ty_sub = self.substitute(value_ty.clone());

                                // If expression returns only `error`, can't assign to a variable
                                if self.is_error_type(&value_ty_sub) {
                                    self.emit_error(SoppoError::TryCapturesError {
                                        span: ident.span,
                                    });
                                    if let Err(e) = self.insert_var(
                                        ident.name.clone(),
                                        Type::error(),
                                        Some(ident.span),
                                    ) {
                                        self.emit_error(e);
                                    }
                                    // Mark as used to suppress cascading unused variable error
                                    self.mark_var_used(&ident.name);
                                    let inner = TypedStmt::new(
                                        TypedStmtKind::Decl {
                                            ident: ident.clone(),
                                            var_ty: Type::error(),
                                            value: typed_value,
                                        },
                                        inner_stmt.span,
                                    );
                                    (inner, 0, vec![], value_ty_sub, value.span)
                                } else {
                                    // Strip error from tuple type for the variable
                                    let var_ty = self.strip_error_from_tuple(&value_ty_sub);
                                    if let Err(e) = self.insert_var(
                                        ident.name.clone(),
                                        var_ty.clone(),
                                        Some(ident.span),
                                    ) {
                                        self.emit_error(e);
                                    }
                                    self.update_nil_state_for_assignment(
                                        &ident.name,
                                        value,
                                        &var_ty,
                                    );

                                    // Record variable definition for LSP
                                    self.record_symbol(
                                        ident.span,
                                        SymbolInfo {
                                            name: ident.name.clone(),
                                            ty: var_ty.clone(),
                                            definition_span: Some(stmt.span),
                                            name_span: Some(ident.span),
                                            kind: SymbolKind::Variable,
                                            doc_comment: None,
                                            go_location: None,
                                        },
                                    );

                                    let inner = TypedStmt::new(
                                        TypedStmtKind::Decl {
                                            ident: ident.clone(),
                                            var_ty,
                                            value: typed_value,
                                        },
                                        inner_stmt.span,
                                    );
                                    (inner, 0, vec![], value_ty_sub, value.span)
                                }
                            }
                        }
                        StmtKind::MultiDecl {
                            ident: names,
                            values,
                        } if values.len() == 1 => {
                            let typed_value = self.infer_expr(&values[0]);
                            let value_ty = typed_value.ty.clone();

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
                                let inner = TypedStmt::new(
                                    TypedStmtKind::MultiDecl {
                                        idents: names.clone(),
                                        var_tys: vec![Type::error(); names.len()],
                                        values: vec![typed_value],
                                    },
                                    inner_stmt.span,
                                );
                                (inner, 0, vec![], Type::error(), values[0].span)
                            } else {
                                let value_ty_sub = self.substitute(value_ty.clone());
                                let mut var_tys = Vec::new();

                                // For multi-return, the type is a tuple
                                if let Type::Con {
                                    sym: tname, args, ..
                                } = &value_ty_sub
                                    && tname.name == "tuple"
                                {
                                    // Exclude the last element (error) when assigning
                                    let non_error_count = args.len().saturating_sub(1);

                                    if names.len() > non_error_count {
                                        self.emit_error(SoppoError::TryCapturesError {
                                            span: names[non_error_count].span,
                                        });
                                        for var_ident in names.iter() {
                                            if let Err(e) = self.insert_var(
                                                var_ident.name.clone(),
                                                Type::error(),
                                                Some(var_ident.span),
                                            ) {
                                                self.emit_error(e);
                                            }
                                            // Mark as used to suppress cascading unused variable error
                                            self.mark_var_used(&var_ident.name);
                                        }
                                        var_tys = vec![Type::error(); names.len()];
                                    } else {
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
                                                var_tys.push(ty.clone());

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
                                }

                                let inner = TypedStmt::new(
                                    TypedStmtKind::MultiDecl {
                                        idents: names.clone(),
                                        var_tys,
                                        values: vec![typed_value],
                                    },
                                    inner_stmt.span,
                                );
                                (inner, 0, vec![], value_ty_sub, values[0].span)
                            }
                        }
                        StmtKind::Assign { target, value } => {
                            let typed_target = self.infer_expr(target);
                            let typed_value = self.infer_expr(value);
                            let value_ty_sub = self.substitute(typed_value.ty.clone());

                            if !typed_value.ty.is_error() && !typed_target.ty.is_error() {
                                // Strip error from tuple and strip nilability - within a try statement,
                                // if the error is nil, the other values are assumed non-nil
                                let expected_ty =
                                    self.strip_error_from_tuple(&value_ty_sub).as_non_nullable();
                                self.unify(&typed_target.ty, &expected_ty, &inner_stmt.span);
                            }

                            let inner = TypedStmt::new(
                                TypedStmtKind::Assign {
                                    target: typed_target,
                                    value: typed_value.clone(),
                                },
                                inner_stmt.span,
                            );
                            (inner, 0, vec![], value_ty_sub, value.span)
                        }
                        StmtKind::Expr(expr) => {
                            let typed_expr = self.infer_expr(expr);
                            let expr_ty_sub = self.substitute(typed_expr.ty.clone());

                            // For expression statements, all non-error values are discarded
                            let (count, types) = if let Type::Con { sym, args, .. } = &expr_ty_sub
                                && sym.name == "tuple"
                            {
                                let non_error: Vec<_> = args
                                    .iter()
                                    .filter(|t| {
                                        !matches!(t, Type::Con { sym, .. } if sym.name == "error" || sym.name == "?error")
                                    })
                                    .cloned()
                                    .collect();
                                (non_error.len(), non_error)
                            } else {
                                (0, vec![])
                            };

                            let inner =
                                TypedStmt::new(TypedStmtKind::Expr(typed_expr), inner_stmt.span);
                            (inner, count, types, expr_ty_sub, expr.span)
                        }
                        _ => {
                            self.emit_error(SoppoError::Type {
                                message: "`?` can only be used with declarations, assignments, or expression statements".to_string(),
                                span: inner_stmt.span,
                            });
                            let inner = TypedStmt::error(inner_stmt.span);
                            (inner, 0, vec![], Type::error(), inner_stmt.span)
                        }
                    };

                // Verify expression returns error
                if !expr_ty.is_error() && !self.returns_error(&expr_ty) {
                    self.emit_error(SoppoError::TryExprNoError { span: expr_span });
                }

                // If handler present, infer it with error_name in scope
                let typed_handler = if let Some(block) = handler {
                    self.push_scope();
                    if let Some(name) = error_name {
                        if let Err(e) =
                            self.insert_var(name.clone(), Type::simple("error"), Some(stmt.span))
                        {
                            self.emit_error(e);
                        }
                        self.set_nil_state(name.clone(), Nullability::NonNull);
                    }
                    let typed_block = self.infer_block(block);
                    self.pop_scope();
                    Some(typed_block)
                } else {
                    None
                };

                // Mark assigned nilable variables as non-null
                if let Some(var_name) = self.get_assigned_var_name(inner_stmt)
                    && let Some(var_type) = self.lookup_var_type(&var_name)
                {
                    let var_type_sub = self.substitute(var_type);
                    if Self::is_nilable_type(&var_type_sub) {
                        self.set_nil_state(var_name, Nullability::NonNull);
                    }
                }

                TypedStmtKind::TryStmt {
                    stmt: Box::new(typed_inner_stmt),
                    error_name: error_name.clone(),
                    handler: typed_handler,
                    try_span: *try_span,
                    discard_count,
                    discard_types,
                }
            }

            StmtKind::LocalTypeDecl(type_decl) => {
                // Register the local type in the current module
                self.infer_type_decl(type_decl)?;

                // Convert TypeKind to TypedTypeKind
                let typed_kind = match &type_decl.kind {
                    crate::syntax::TypeKind::Alias { target } => {
                        crate::types::ast::TypedTypeKind::Alias {
                            target: self.resolve_type(target),
                        }
                    }
                    crate::syntax::TypeKind::Definition { target } => {
                        crate::types::ast::TypedTypeKind::Definition {
                            target: self.resolve_type(target),
                        }
                    }
                    crate::syntax::TypeKind::Enum { variants } => {
                        let typed_variants = variants
                            .iter()
                            .map(|v| match v {
                                EnumVariant::Unit { ident } => {
                                    crate::types::ast::TypedEnumVariant::Unit {
                                        ident: ident.clone(),
                                    }
                                }
                                EnumVariant::Single { ident, ty } => {
                                    crate::types::ast::TypedEnumVariant::Single {
                                        ident: ident.clone(),
                                        ty: self.resolve_type(ty),
                                    }
                                }
                                EnumVariant::Struct { ident, fields } => {
                                    crate::types::ast::TypedEnumVariant::Struct {
                                        ident: ident.clone(),
                                        fields: fields
                                            .iter()
                                            .map(|f| {
                                                (f.ident.name.clone(), self.resolve_type(&f.ty))
                                            })
                                            .collect(),
                                    }
                                }
                            })
                            .collect();
                        crate::types::ast::TypedTypeKind::Enum {
                            variants: typed_variants,
                        }
                    }
                    crate::syntax::TypeKind::Struct { fields } => {
                        let typed_fields = fields
                            .iter()
                            .map(|f| {
                                (
                                    f.ident.name.clone(),
                                    self.resolve_type(&f.ty),
                                    f.tag.clone(),
                                )
                            })
                            .collect();
                        crate::types::ast::TypedTypeKind::Struct {
                            fields: typed_fields,
                        }
                    }
                    crate::syntax::TypeKind::Interface { methods } => {
                        let typed_methods = methods
                            .iter()
                            .map(|m| crate::types::ast::TypedInterfaceMethod {
                                ident: m.ident.clone(),
                                params: m
                                    .params
                                    .iter()
                                    .map(|p| crate::types::ast::TypedParam {
                                        ident: p.ident.clone(),
                                        ty: self.resolve_type(&p.ty),
                                        nullable: false,
                                    })
                                    .collect(),
                                returns: m.returns.iter().map(|r| self.resolve_type(r)).collect(),
                            })
                            .collect();
                        crate::types::ast::TypedTypeKind::Interface {
                            methods: typed_methods,
                        }
                    }
                };

                TypedStmtKind::LocalTypeDecl(crate::types::ast::TypedTypeDecl {
                    ident: type_decl.ident.clone(),
                    generics: type_decl.generics.clone(),
                    kind: typed_kind,
                    span: type_decl.span,
                    doc_comment: type_decl.doc_comment.clone(),
                })
            }
        };
        Ok(TypedStmt::new(kind, stmt.span))
    }

    /// Helper for multi-decl tuple unpacking
    fn handle_multi_decl_tuple_unpack(
        &mut self,
        names: &[crate::syntax::Ident],
        typed_value: &TypedExpr,
        value: &Expr,
    ) -> Vec<Type> {
        let value_ty = typed_value.ty.clone();

        // If expression failed, insert vars with Error type
        if value_ty.is_error() {
            for ident in names {
                if let Err(e) = self.insert_var(ident.name.clone(), Type::error(), Some(ident.span))
                {
                    self.emit_error(e);
                }
            }
            return vec![Type::error(); names.len()];
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

            // Track error companions
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

            return args.clone();
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
            if let Err(e) = self.insert_var(ident.name.clone(), Type::error(), Some(ident.span)) {
                self.emit_error(e);
            }
        }
        vec![Type::error(); names.len()]
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
