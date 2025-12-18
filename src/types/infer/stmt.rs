use std::collections::HashSet;

use super::Infer;
use crate::error::{Result, SoppoError};
use crate::syntax::{
    BinOp, EnumVariant, Expr, ExprKind, PatternKind, SelectCaseKind, Stmt, StmtKind,
};
use crate::types::ctx::TypeDefKind;
use crate::types::ty::Nullability;
use crate::types::{SymbolKind, Type};

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
    /// Infer the type of a statement
    /// Returns the type of the statement (unit for most, or the type of the expression)
    pub fn infer_stmt(&mut self, stmt: &Stmt) -> Result<Type> {
        match &stmt.kind {
            StmtKind::Decl { ident, value } => {
                let value_ty = self.infer_expr(value)?;
                let value_ty_sub = self.substitute(value_ty.clone());
                self.insert_var(ident.name.clone(), value_ty.clone(), Some(ident.span));

                // Record variable definition for LSP
                self.record_symbol(
                    ident.span,
                    ident.name.clone(),
                    value_ty,
                    Some(stmt.span),
                    Some(ident.span),
                    SymbolKind::Variable,
                    None,
                );

                // Track nil state for pointer types
                self.update_nil_state_for_assignment(&ident.name, value, &value_ty_sub);
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
                        && let Some((value_ty, ok_ty)) = self.infer_comma_ok_expr(value)?
                    {
                        self.insert_var(names[0].name.clone(), value_ty, Some(names[0].span));
                        self.insert_var(names[1].name.clone(), ok_ty, Some(names[1].span));
                        return Ok(Type::unit());
                    }

                    let value_ty = self.infer_expr(value)?;
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
                        for (ident, ty) in names.iter().zip(args.iter()) {
                            self.insert_var(ident.name.clone(), ty.clone(), Some(ident.span));
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

                    // Not a tuple type or wrong arity
                    Err(SoppoError::Type {
                        message: format!(
                            "Cannot unpack {} values from type `{}`",
                            names.len(),
                            value_ty
                        ),
                        span: value.span,
                    })
                } else {
                    // a, b := expr1, expr2 (one value per name)
                    for (ident, value) in names.iter().zip(values.iter()) {
                        let value_ty = self.infer_expr(value)?;
                        self.insert_var(ident.name.clone(), value_ty, Some(ident.span));
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
                            return Err(err);
                        }

                        let value_ty = self.infer_expr(expr)?;
                        self.unify(&declared_ty, &value_ty, &expr.span)?;
                        (declared_ty, Some(expr))
                    }
                    (Some(t), None) => {
                        // var x type: use declared type (zero value)
                        let declared_ty = Type::from_ast(t);

                        // Check: non-nilable types require initialisation
                        // Zero value for pointer/slice/map/etc is nil, which violates non-nilable
                        if declared_ty.is_nilable_kind() && !declared_ty.is_nullable() {
                            return Err(SoppoError::NonNilableNoInit {
                                ty: declared_ty.to_string(),
                                span: stmt.span,
                            });
                        }

                        (declared_ty, None)
                    }
                    (None, Some(expr)) => {
                        // var x = value: infer from value
                        let ty = self.infer_expr(expr)?;
                        (ty, Some(expr))
                    }
                    (None, None) => {
                        // var x: error (should be caught by parser)
                        return Err(SoppoError::Type {
                            message:
                                "Variable declaration requires either a type or an initialiser"
                                    .to_string(),
                            span: stmt.span,
                        });
                    }
                };
                let var_ty_sub = self.substitute(var_ty.clone());
                self.insert_var(ident.name.clone(), var_ty.clone(), Some(ident.span));

                // Record variable definition for LSP
                self.record_symbol(
                    ident.span,
                    ident.name.clone(),
                    var_ty,
                    Some(stmt.span),
                    Some(ident.span),
                    SymbolKind::Variable,
                    None,
                );

                // Track nil state for nilable types
                if let Some(expr) = init_expr {
                    self.update_nil_state_for_assignment(&ident.name, expr, &var_ty_sub);
                } else if Self::is_nilable_type(&var_ty_sub) {
                    // Zero-initialized nilable types are nil
                    self.set_nil_state(ident.name.clone(), crate::types::ty::Nullability::Nullable);
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
                    let declared_ty =
                        ty.as_ref()
                            .map(Type::from_ast)
                            .ok_or_else(|| SoppoError::Type {
                                message:
                                    "Multi-variable declaration without values requires a type"
                                        .to_string(),
                                span: stmt.span,
                            })?;
                    for ident in names {
                        self.insert_var(ident.name.clone(), declared_ty.clone(), Some(ident.span));
                    }
                } else if values.len() == 1 && names.len() > 1 {
                    // var a, b = f() (multi-return unpacking)
                    let value = &values[0];

                    // Check for comma-ok idiom: v, ok := expr
                    // Applies to: type assertions, map access, channel receive
                    if names.len() == 2
                        && let Some((value_ty, ok_ty)) = self.infer_comma_ok_expr(value)?
                    {
                        // First variable gets the value type
                        let var_ty = if let Some(t) = ty {
                            let declared_ty = Type::from_ast(t);
                            self.unify(&declared_ty, &value_ty, &value.span)?;
                            declared_ty
                        } else {
                            value_ty
                        };
                        self.insert_var(names[0].name.clone(), var_ty, Some(names[0].span));
                        // Second variable gets the ok type (bool)
                        self.insert_var(names[1].name.clone(), ok_ty, Some(names[1].span));
                        return Ok(Type::unit());
                    }

                    let value_ty = self.infer_expr(value)?;
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
                                self.unify(&declared_ty, arg_ty, &value.span)?;
                                declared_ty
                            } else {
                                arg_ty.clone()
                            };
                            self.insert_var(ident.name.clone(), var_ty, Some(ident.span));
                        }
                        return Ok(Type::unit());
                    }

                    return Err(SoppoError::Type {
                        message: format!(
                            "Cannot unpack {} values from type `{}`",
                            names.len(),
                            value_ty
                        ),
                        span: value.span,
                    });
                } else {
                    // var a, b = expr1, expr2 or var a, b type = expr1, expr2
                    for (ident, value) in names.iter().zip(values.iter()) {
                        let value_ty = self.infer_expr(value)?;
                        let var_ty = if let Some(t) = ty {
                            let declared_ty = Type::from_ast(t);
                            self.unify(&declared_ty, &value_ty, &value.span)?;
                            declared_ty
                        } else {
                            value_ty
                        };
                        self.insert_var(ident.name.clone(), var_ty, Some(ident.span));
                    }
                }
                Ok(Type::unit())
            }

            StmtKind::ConstDecl { ident, ty, value } => {
                // Infer the type of the value
                let value_ty = self.infer_expr(value)?;

                // Determine the constant's type
                let const_ty = if let Some(t) = ty {
                    // const x type = value: unify declared with inferred
                    let declared_ty = Type::from_ast(t);
                    self.unify(&declared_ty, &value_ty, &value.span)?;
                    declared_ty
                } else {
                    // const x = value: infer from value
                    value_ty
                };

                self.insert_var(ident.name.clone(), const_ty, Some(ident.span));
                Ok(Type::unit())
            }

            StmtKind::MultiConstDecl { idents, ty, values } => {
                // const a, b = expr1, expr2 or const a, b type = expr1, expr2
                for (ident, value) in idents.iter().zip(values.iter()) {
                    let value_ty = self.infer_expr(value)?;
                    let const_ty = if let Some(t) = ty {
                        let declared_ty = Type::from_ast(t);
                        self.unify(&declared_ty, &value_ty, &value.span)?;
                        declared_ty
                    } else {
                        value_ty
                    };
                    self.insert_var(ident.name.clone(), const_ty, Some(ident.span));
                }
                Ok(Type::unit())
            }

            StmtKind::Assign { target, value } => {
                // Special case: blank identifier accepts any type
                if let ExprKind::Ident(name) = &target.kind
                    && name == "_"
                {
                    // Just infer the value type, don't unify
                    self.infer_expr(value)?;
                    return Ok(Type::unit());
                }
                let target_ty = self.infer_expr(target)?;
                let target_ty_sub = self.substitute(target_ty.clone());

                // Check: assigning nil to a non-nilable type is an error
                if matches!(value.kind, ExprKind::Nil)
                    && let Some(err) = Self::check_nil_to_non_nilable(&target_ty_sub, value.span)
                {
                    return Err(err);
                }

                let value_ty = self.infer_expr(value)?;
                let value_ty_sub = self.substitute(value_ty.clone());
                self.unify(&target_ty, &value_ty, &stmt.span)?;
                // Update nil state for reassignment
                if let ExprKind::Ident(name) = &target.kind {
                    self.update_nil_state_for_assignment(name, value, &value_ty_sub);
                }
                Ok(Type::unit())
            }

            StmtKind::MultiAssign { targets, values } => {
                if values.len() == 1 && targets.len() > 1 {
                    // a, b = f() (multi-return unpacking)
                    let value = &values[0];
                    let value_ty = self.infer_expr(value)?;
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
                            let target_ty = self.infer_expr(target)?;
                            self.unify(&target_ty, expected_ty, &target.span)?;
                        }
                        return Ok(Type::unit());
                    }

                    // Not a tuple type or wrong arity
                    Err(SoppoError::Type {
                        message: format!(
                            "Cannot unpack {} values from type `{}`",
                            targets.len(),
                            value_ty
                        ),
                        span: value.span,
                    })
                } else {
                    // a, b = expr1, expr2 (one value per target)
                    for (target, value) in targets.iter().zip(values.iter()) {
                        // Special case: blank identifier accepts any type
                        if let ExprKind::Ident(name) = &target.kind
                            && name == "_"
                        {
                            self.infer_expr(value)?;
                            continue;
                        }
                        let target_ty = self.infer_expr(target)?;
                        let value_ty = self.infer_expr(value)?;
                        self.unify(&target_ty, &value_ty, &target.span)?;
                    }
                    Ok(Type::unit())
                }
            }

            StmtKind::For { condition, body } => {
                // Check condition is bool
                let cond_ty = self.infer_expr(condition)?;
                self.unify(&Type::simple("bool"), &cond_ty, &condition.span)?;

                // Type check body
                self.infer_block(body)?;

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
                    self.infer_stmt(init_stmt)?;
                }

                // Check condition is bool if present
                if let Some(cond) = condition {
                    let cond_ty = self.infer_expr(cond)?;
                    self.unify(&Type::simple("bool"), &cond_ty, &cond.span)?;
                }

                // Type check body
                self.infer_block(body)?;

                // Type check post statement if present
                if let Some(post_stmt) = post {
                    self.infer_stmt(post_stmt)?;
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
                let coll_ty = self.infer_expr(collection)?;
                let coll_ty = self.substitute(coll_ty);

                // Determine key and value types based on collection type
                let (key_ty, value_ty) =
                    if let Some(elem_ty) = Self::extract_slice_element(&coll_ty) {
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

                // Bind the key variable
                self.insert_var(key.name.clone(), key_ty, Some(key.span));

                // Bind the value variable if present
                if let Some(val_ident) = value {
                    self.insert_var(val_ident.name.clone(), value_ty, Some(val_ident.span));
                }

                // Type check body
                self.infer_block(body)?;

                Ok(Type::unit())
            }

            StmtKind::If {
                init,
                condition,
                then_block,
                else_block,
            } => {
                // Process init statement if present (Go-style: if x := expr; cond { })
                if let Some(init_stmt) = init {
                    self.infer_stmt(init_stmt)?;
                }

                // Check condition is bool
                let cond_ty = self.infer_expr(condition)?;
                self.unify(&Type::simple("bool"), &cond_ty, &condition.span)?;

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
                let then_ty = self.infer_block(then_block)?;
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
                    let ty = self.infer_block(else_block)?;
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

                // If both branches diverge (return never), the if statement also diverges
                if matches!(then_ty, Type::Never) && matches!(else_ty, Type::Never) {
                    Ok(Type::never())
                } else {
                    Ok(Type::unit())
                }
            }

            StmtKind::Return { values } => {
                // Check return values against expected return types
                if let Some(expected_types) = self.expected_return_types.clone() {
                    // Handle `return f()` where f() returns a tuple matching expected types
                    // This is the Go idiom: `return someFunc()` when both have same return signature
                    if values.len() == 1 && expected_types.len() > 1 {
                        let value_ty = self.infer_expr(&values[0])?;
                        let value_ty = self.substitute(value_ty);

                        // Check if it's a tuple type with matching arity
                        if let Type::Con { sym, args, .. } = &value_ty
                            && sym.name == "tuple"
                            && args.len() == expected_types.len()
                        {
                            // Unify each tuple element with expected type
                            for (arg_ty, expected) in args.iter().zip(expected_types.iter()) {
                                self.unify(expected, arg_ty, &values[0].span)?;
                            }
                            return Ok(Type::never());
                        }

                        // Not a matching tuple - fall through to error
                        return Err(SoppoError::Type {
                            message: format!(
                                "Expected {} return value(s), got {}",
                                expected_types.len(),
                                values.len()
                            ),
                            span: stmt.span,
                        });
                    }

                    if values.len() != expected_types.len() {
                        return Err(SoppoError::Type {
                            message: format!(
                                "Expected {} return value(s), got {}",
                                expected_types.len(),
                                values.len()
                            ),
                            span: stmt.span,
                        });
                    }
                    for (expr, expected) in values.iter().zip(expected_types.iter()) {
                        // Check: returning nil to a non-nilable type is an error
                        if matches!(expr.kind, ExprKind::Nil)
                            && let Some(err) = Self::check_nil_to_non_nilable(expected, expr.span)
                        {
                            return Err(err);
                        }

                        let value_ty = self.infer_expr(expr)?;
                        self.unify(expected, &value_ty, &expr.span)?;
                    }
                } else if !values.is_empty() {
                    // Infer types but no expected types to check against
                    for expr in values {
                        self.infer_expr(expr)?;
                    }
                }
                // Return statements are diverging - they never produce a value
                Ok(Type::never())
            }

            StmtKind::Match { scrutinee, arms } => {
                // Expression-less match has no scrutinee
                let scrutinee_ty = if let Some(scrutinee) = scrutinee {
                    let ty = self.infer_expr(scrutinee)?;
                    Some(self.substitute(ty))
                } else {
                    None
                };

                for arm in arms {
                    // Create a new scope for pattern bindings
                    self.push_scope();

                    if let Some(ref scr_ty) = scrutinee_ty {
                        // Normal match with scrutinee
                        // Handle multiple patterns: validate bindings match across patterns
                        if arm.patterns.len() > 1 {
                            // Collect bindings from first pattern
                            let first_bindings =
                                self.collect_pattern_bindings(&arm.patterns[0], scr_ty)?;

                            // Validate subsequent patterns have matching bindings
                            for pattern in &arm.patterns[1..] {
                                let bindings = self.collect_pattern_bindings(pattern, scr_ty)?;

                                // Check binding names match
                                let first_keys: HashSet<_> = first_bindings.keys().collect();
                                let other_keys: HashSet<_> = bindings.keys().collect();
                                if first_keys != other_keys {
                                    return Err(SoppoError::Type {
                                        message: format!(
                                            "Pattern bindings must match: expected {:?}, found {:?}",
                                            first_keys, other_keys
                                        ),
                                        span: pattern.span,
                                    });
                                }

                                // Unify types of matching bindings
                                for (name, ty) in &bindings {
                                    if let Some(first_ty) = first_bindings.get(name) {
                                        self.unify(first_ty, ty, &pattern.span)?;
                                    }
                                }
                            }

                            // Add first pattern's bindings to scope
                            let pattern_span = arm.patterns.first().map(|p| p.span);
                            for (name, ty) in first_bindings {
                                self.insert_var(name, ty, pattern_span);
                            }
                        } else if let Some(pattern) = arm.patterns.first() {
                            // Single pattern
                            self.add_pattern_bindings(pattern, scr_ty)?;
                        }
                    } else {
                        // Expression-less match: patterns must be Guard expressions
                        for pattern in &arm.patterns {
                            if let PatternKind::Guard(expr) = &pattern.kind {
                                let ty = self.infer_expr(expr)?;
                                self.unify(&ty, &Type::simple("bool"), &expr.span)?;
                            } else if !matches!(pattern.kind, PatternKind::Default) {
                                return Err(SoppoError::Type {
                                    message:
                                        "Expression-less match requires boolean guard expressions"
                                            .to_string(),
                                    span: pattern.span,
                                });
                            }
                        }
                    }

                    // Type check the arm body
                    self.infer_block(&arm.body)?;

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
                                PatternKind::Variant(v) => {
                                    // Extract variant name from qualified name like "Colour.Red"
                                    Some(v.rsplit('.').next().unwrap_or(v).to_string())
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
                            return Err(SoppoError::NonExhaustive {
                                missing,
                                span: stmt.span,
                            });
                        }
                    }
                }

                Ok(Type::unit())
            }

            StmtKind::Send { channel, value } => {
                // ch <- value: channel must be chan T, value must be T
                let channel_ty = self.infer_expr(channel)?;
                let channel_ty = self.substitute(channel_ty);
                let value_ty = self.infer_expr(value)?;
                let is_nil = matches!(value.kind, ExprKind::Nil);

                // Extract element type from channel and check nil safety
                if let Some(elem_ty) = Self::extract_channel_element(&channel_ty) {
                    // Check: sending nil to a channel with non-nilable element type is an error
                    if is_nil
                        && let Some(err) = Self::check_nil_to_non_nilable(&elem_ty, value.span)
                    {
                        return Err(err);
                    }
                    self.unify(&elem_ty, &value_ty, &value.span)?;
                }

                Ok(Type::unit())
            }

            StmtKind::Select { cases } => {
                for case in cases {
                    self.push_scope();

                    match &case.kind {
                        SelectCaseKind::Recv { channel } => {
                            // <-ch: just infer the channel type
                            self.infer_expr(channel)?;
                        }
                        SelectCaseKind::RecvDecl { ident, channel } => {
                            // v := <-ch: infer channel type, declare v with element type
                            let channel_ty = self.infer_expr(channel)?;
                            let channel_ty = self.substitute(channel_ty);

                            // Extract element type from channel
                            let elem_ty = Self::extract_channel_element(&channel_ty)
                                .unwrap_or_else(|| self.fresh_ty_var());

                            self.insert_var(ident.name.clone(), elem_ty, Some(ident.span));
                        }
                        SelectCaseKind::RecvDeclOk {
                            ident,
                            ok_ident,
                            channel,
                        } => {
                            // v, ok := <-ch: infer channel type, declare v and ok
                            let channel_ty = self.infer_expr(channel)?;
                            let channel_ty = self.substitute(channel_ty);

                            // Extract element type from channel
                            let elem_ty = Self::extract_channel_element(&channel_ty)
                                .unwrap_or_else(|| self.fresh_ty_var());

                            self.insert_var(ident.name.clone(), elem_ty, Some(ident.span));
                            self.insert_var(
                                ok_ident.name.clone(),
                                Type::simple("bool"),
                                Some(ok_ident.span),
                            );
                        }
                        SelectCaseKind::Send { channel, value } => {
                            // ch <- value: same as Send statement
                            let channel_ty = self.infer_expr(channel)?;
                            let channel_ty = self.substitute(channel_ty);
                            let value_ty = self.infer_expr(value)?;
                            let is_nil = matches!(value.kind, ExprKind::Nil);

                            if let Some(elem_ty) = Self::extract_channel_element(&channel_ty) {
                                // Check nil safety
                                if is_nil
                                    && let Some(err) =
                                        Self::check_nil_to_non_nilable(&elem_ty, value.span)
                                {
                                    return Err(err);
                                }
                                self.unify(&elem_ty, &value_ty, &value.span)?;
                            }
                        }
                        SelectCaseKind::Default => {
                            // default: nothing to infer
                        }
                    }

                    // Infer body
                    self.infer_block(&case.body)?;

                    self.pop_scope();
                }

                Ok(Type::unit())
            }

            StmtKind::Go(expr) => {
                // go expr: expr should be a function call
                self.infer_expr(expr)?;
                Ok(Type::unit())
            }

            StmtKind::DeferStmt(expr) => {
                // defer expr: expr should be a function call
                self.infer_expr(expr)?;
                Ok(Type::unit())
            }

            StmtKind::Break | StmtKind::Continue => {
                // break/continue don't have types, just return unit
                Ok(Type::unit())
            }

            StmtKind::Expr(expr) => self.infer_expr(expr),

            StmtKind::CompoundAssign {
                target,
                op: _,
                value,
            } => {
                // Compound assignment: x += value
                // Check that target and value types are compatible
                let target_ty = self.infer_expr(target)?;
                let value_ty = self.infer_expr(value)?;
                self.unify(&target_ty, &value_ty, &value.span)?;
                Ok(Type::unit())
            }

            StmtKind::IncDec { target, is_inc: _ } => {
                // Increment/decrement: x++ or x--
                // Just infer the target type (should be numeric)
                self.infer_expr(target)?;
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
                        let value_ty = self.infer_expr(value)?;
                        let value_ty_sub = self.substitute(value_ty.clone());

                        // Strip error from tuple type for the variable
                        let var_ty = self.strip_error_from_tuple(&value_ty_sub);
                        self.insert_var(ident.name.clone(), var_ty.clone(), Some(ident.span));
                        self.update_nil_state_for_assignment(&ident.name, value, &var_ty);
                        (value_ty_sub, value.span)
                    }
                    StmtKind::MultiDecl {
                        ident: names,
                        values,
                    } if values.len() == 1 => {
                        let value_ty = self.infer_expr(&values[0])?;
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
                            for (i, var_ident) in names.iter().enumerate() {
                                if i < non_error_count
                                    && let Some(ty) = args.get(i)
                                {
                                    self.insert_var(
                                        var_ident.name.clone(),
                                        ty.clone(),
                                        Some(var_ident.span),
                                    );
                                }
                            }
                        }
                        (value_ty_sub, values[0].span)
                    }
                    StmtKind::Assign { target, value } => {
                        let value_ty = self.infer_expr(value)?;
                        let value_ty_sub = self.substitute(value_ty.clone());

                        // For assignment, we expect the target to already have the non-error type
                        let target_ty = self.infer_expr(target)?;
                        let expected_ty = self.strip_error_from_tuple(&value_ty_sub);
                        self.unify(&target_ty, &expected_ty, &inner_stmt.span)?;
                        (value_ty_sub, value.span)
                    }
                    StmtKind::Expr(expr) => {
                        let expr_ty = self.infer_expr(expr)?;
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
                        return Err(SoppoError::Type {
                            message: "`?` can only be used with declarations, assignments, or expression statements".to_string(),
                            span: inner_stmt.span,
                        });
                    }
                };

                let expr_ty_sub = self.substitute(expr_ty.clone());

                // Verify expression returns error
                if !self.returns_error(&expr_ty_sub) {
                    return Err(SoppoError::TryExprNoError { span: expr_span });
                }

                // If handler present, infer it with error_name in scope
                if let Some(block) = handler {
                    self.push_scope();
                    if let Some(name) = error_name {
                        self.insert_var(name.clone(), Type::simple("error"), Some(stmt.span));
                        // Error is known to be non-nil in the handler (handler only runs on error)
                        self.set_nil_state(name.clone(), Nullability::NonNull);
                    }
                    self.infer_block(block)?;
                    self.pop_scope();
                }

                // Mark assigned nilable variables as non-null (success implies valid result)
                if let Some(var_name) = self.get_assigned_var_name(inner_stmt)
                    && let Some((var_type, _)) = self.lookup_var(&var_name)
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
    fn test_variable_shadowing() {
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
                // Shadowing is allowed in Soppo
                assert!(infer.infer_func_decl(func).is_ok());
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
                // Should fail: returning int where string expected
                assert!(infer.infer_func_decl(func).is_err());
            }
        }
    }
}
