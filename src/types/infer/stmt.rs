use std::collections::HashSet;

use super::Infer;
use crate::error::{Result, SoppoError};
use crate::syntax::{
    BinOp, EnumVariant, Expr, ExprKind, PatternKind, SelectCaseKind, Stmt, StmtKind,
};
use crate::types::Type;
use crate::types::ctx::TypeDefKind;
use crate::types::ty::Nullability;

/// Result of analyzing a nil check condition
#[derive(Debug)]
struct NilCheck {
    /// The expression key being checked (e.g., "user" or "user.profile")
    expr_key: String,
    /// True if the check is `expr != nil`, false if `expr == nil`
    is_not_nil: bool,
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

impl Infer {
    /// Infer the type of a statement
    /// Returns the type of the statement (unit for most, or the type of the expression)
    pub fn infer_stmt(&mut self, stmt: &Stmt) -> Result<Type> {
        match &stmt.kind {
            StmtKind::Decl { name, value } => {
                let value_ty = self.infer_expr(value)?;
                let value_ty_sub = self.substitute(value_ty.clone());
                self.insert_var(name.clone(), value_ty.clone());
                // Track nil state for pointer types
                self.update_nil_state_for_assignment(name, value, &value_ty_sub);
                Ok(Type::unit())
            }

            StmtKind::MultiDecl { names, values } => {
                if values.len() == 1 && names.len() > 1 {
                    // a, b := f() (multi-return unpacking)
                    let value = &values[0];
                    let value_ty = self.infer_expr(value)?;
                    let value_ty = self.substitute(value_ty);

                    // The value should be a tuple type with matching arity
                    if let Type::Con {
                        name: type_name,
                        args,
                    } = &value_ty
                        && type_name.name == "tuple"
                        && args.len() == names.len()
                    {
                        for (name, ty) in names.iter().zip(args.iter()) {
                            self.insert_var(name.clone(), ty.clone());
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
                    for (name, value) in names.iter().zip(values.iter()) {
                        let value_ty = self.infer_expr(value)?;
                        self.insert_var(name.clone(), value_ty);
                    }
                    Ok(Type::unit())
                }
            }

            StmtKind::VarDecl { name, ty, value } => {
                let (var_ty, init_expr) = match (ty, value) {
                    (Some(t), Some(expr)) => {
                        // var x type = value: unify declared with inferred
                        let declared_ty = Type::from_ast(t);
                        let value_ty = self.infer_expr(expr)?;
                        self.unify(&declared_ty, &value_ty, &expr.span)?;
                        (declared_ty, Some(expr))
                    }
                    (Some(t), None) => {
                        // var x type: use declared type (zero value)
                        // Zero value for pointer is nil, so it's nullable
                        (Type::from_ast(t), None)
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
                self.insert_var(name.clone(), var_ty);
                // Track nil state for pointer types
                if let Some(expr) = init_expr {
                    self.update_nil_state_for_assignment(name, expr, &var_ty_sub);
                } else if Self::is_pointer_type(&var_ty_sub) {
                    // Zero-initialized pointers are nil
                    self.set_nil_state(name.clone(), crate::types::ty::Nullability::Nullable);
                }
                Ok(Type::unit())
            }

            StmtKind::MultiVarDecl { names, ty, values } => {
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
                    for name in names {
                        self.insert_var(name.clone(), declared_ty.clone());
                    }
                } else if values.len() == 1 && names.len() > 1 {
                    // var a, b = f() (multi-return unpacking)
                    let value = &values[0];
                    let value_ty = self.infer_expr(value)?;
                    let value_ty = self.substitute(value_ty);

                    // The value should be a tuple type with matching arity
                    if let Type::Con {
                        name: type_name,
                        args,
                    } = &value_ty
                        && type_name.name == "tuple"
                        && args.len() == names.len()
                    {
                        for (name, arg_ty) in names.iter().zip(args.iter()) {
                            let var_ty = if let Some(t) = ty {
                                let declared_ty = Type::from_ast(t);
                                self.unify(&declared_ty, arg_ty, &value.span)?;
                                declared_ty
                            } else {
                                arg_ty.clone()
                            };
                            self.insert_var(name.clone(), var_ty);
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
                    for (name, value) in names.iter().zip(values.iter()) {
                        let value_ty = self.infer_expr(value)?;
                        let var_ty = if let Some(t) = ty {
                            let declared_ty = Type::from_ast(t);
                            self.unify(&declared_ty, &value_ty, &value.span)?;
                            declared_ty
                        } else {
                            value_ty
                        };
                        self.insert_var(name.clone(), var_ty);
                    }
                }
                Ok(Type::unit())
            }

            StmtKind::ConstDecl { name, ty, value } => {
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

                self.insert_var(name.clone(), const_ty);
                Ok(Type::unit())
            }

            StmtKind::MultiConstDecl { names, ty, values } => {
                // const a, b = expr1, expr2 or const a, b type = expr1, expr2
                for (name, value) in names.iter().zip(values.iter()) {
                    let value_ty = self.infer_expr(value)?;
                    let const_ty = if let Some(t) = ty {
                        let declared_ty = Type::from_ast(t);
                        self.unify(&declared_ty, &value_ty, &value.span)?;
                        declared_ty
                    } else {
                        value_ty
                    };
                    self.insert_var(name.clone(), const_ty);
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
                        name: type_name,
                        args,
                    } = &value_ty
                        && type_name.name == "tuple"
                        && args.len() == targets.len()
                    {
                        for (target, expected_ty) in targets.iter().zip(args.iter()) {
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
                let (key_ty, value_ty) = if let Type::Con { name, args } = &coll_ty {
                    if name.name.starts_with("[]") {
                        // Slice: key is int, value is element type
                        let elem_ty = if args.len() == 1 {
                            args[0].clone()
                        } else {
                            let elem_name = &name.name[2..];
                            Type::simple(elem_name)
                        };
                        (Type::simple("int"), elem_ty)
                    } else if name.name.starts_with("map[") {
                        // Map: key is key type, value is value type
                        if args.len() == 2 {
                            (args[0].clone(), args[1].clone())
                        } else {
                            (self.fresh_ty_var(), self.fresh_ty_var())
                        }
                    } else if name.name.starts_with("chan ") {
                        // Channel: only one variable (value type)
                        let elem_ty = if args.len() == 1 {
                            args[0].clone()
                        } else {
                            let elem_name = &name.name[5..];
                            Type::simple(elem_name)
                        };
                        (elem_ty.clone(), elem_ty)
                    } else if name.name == "string" {
                        // String: key is int (index), value is rune
                        (Type::simple("int"), Type::simple("rune"))
                    } else {
                        (self.fresh_ty_var(), self.fresh_ty_var())
                    }
                } else {
                    (self.fresh_ty_var(), self.fresh_ty_var())
                };

                // Bind the key variable
                self.insert_var(key.clone(), key_ty);

                // Bind the value variable if present
                if let Some(val_name) = value {
                    self.insert_var(val_name.clone(), value_ty);
                }

                // Type check body
                self.infer_block(body)?;

                Ok(Type::unit())
            }

            StmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                // Check condition is bool
                let cond_ty = self.infer_expr(condition)?;
                self.unify(&Type::simple("bool"), &cond_ty, &condition.span)?;

                // Extract nil check from condition for flow-sensitive narrowing
                let nil_check = extract_nil_check(condition);

                // Type check then block with narrowed nil state
                self.push_nil_scope();
                if let Some(ref check) = nil_check {
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
                    if let Some(ref check) = nil_check {
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
                if let Some(ref check) = nil_check {
                    if matches!(then_ty, Type::Never) && !check.is_not_nil {
                        // `if x == nil { return }` - x is non-nil after this point
                        self.set_nil_state(check.expr_key.clone(), Nullability::NonNull);
                    }
                    // Similarly, if else block diverges and condition was `x != nil`
                    if matches!(else_ty, Type::Never) && check.is_not_nil && else_block.is_some() {
                        // `if x != nil { ... } else { return }` - x is non-nil after
                        self.set_nil_state(check.expr_key.clone(), Nullability::NonNull);
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
                            for (name, ty) in first_bindings {
                                self.insert_var(name, ty);
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
                    && let Type::Con { name, .. } = scrutinee_ty
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
                                    EnumVariant::Unit { name, .. } => name,
                                    EnumVariant::Single { name, .. } => name,
                                    EnumVariant::Struct { name, .. } => name,
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

                // Extract element type from channel
                if let Type::Con { name, args } = &channel_ty {
                    if name.name.starts_with("chan ") && args.len() == 1 {
                        self.unify(&args[0], &value_ty, &value.span)?;
                    } else if name.name.starts_with("chan ") {
                        let elem_name = &name.name[5..]; // skip "chan "
                        let elem_ty = Type::simple(elem_name);
                        self.unify(&elem_ty, &value_ty, &value.span)?;
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
                            self.infer_expr(channel)?;
                        }
                        SelectCaseKind::RecvDecl { name, channel } => {
                            // v := <-ch: infer channel type, declare v with element type
                            let channel_ty = self.infer_expr(channel)?;
                            let channel_ty = self.substitute(channel_ty);

                            // Extract element type from channel
                            let elem_ty = if let Type::Con { name, args } = &channel_ty {
                                if name.name.starts_with("chan ") && args.len() == 1 {
                                    args[0].clone()
                                } else if name.name.starts_with("chan ") {
                                    let elem_name = &name.name[5..];
                                    Type::simple(elem_name)
                                } else {
                                    self.fresh_ty_var()
                                }
                            } else {
                                self.fresh_ty_var()
                            };

                            self.insert_var(name.clone(), elem_ty);
                        }
                        SelectCaseKind::RecvDeclOk {
                            name,
                            ok_name,
                            channel,
                        } => {
                            // v, ok := <-ch: infer channel type, declare v and ok
                            let channel_ty = self.infer_expr(channel)?;
                            let channel_ty = self.substitute(channel_ty);

                            // Extract element type from channel
                            let elem_ty = if let Type::Con { name, args } = &channel_ty {
                                if name.name.starts_with("chan ") && args.len() == 1 {
                                    args[0].clone()
                                } else if name.name.starts_with("chan ") {
                                    let elem_name = &name.name[5..];
                                    Type::simple(elem_name)
                                } else {
                                    self.fresh_ty_var()
                                }
                            } else {
                                self.fresh_ty_var()
                            };

                            self.insert_var(name.clone(), elem_ty);
                            self.insert_var(ok_name.clone(), Type::simple("bool"));
                        }
                        SelectCaseKind::Send { channel, value } => {
                            // ch <- value: same as Send statement
                            let channel_ty = self.infer_expr(channel)?;
                            let channel_ty = self.substitute(channel_ty);
                            let value_ty = self.infer_expr(value)?;

                            if let Type::Con { name, args } = &channel_ty {
                                if name.name.starts_with("chan ") && args.len() == 1 {
                                    self.unify(&args[0], &value_ty, &value.span)?;
                                } else if name.name.starts_with("chan ") {
                                    let elem_name = &name.name[5..];
                                    let elem_ty = Type::simple(elem_name);
                                    self.unify(&elem_ty, &value_ty, &value.span)?;
                                }
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
                try_span,
            } => {
                // Check current function returns error as last type
                let return_types = self
                    .expected_return_types
                    .as_ref()
                    .ok_or(SoppoError::TryNoErrorReturn { span: *try_span })?;

                let last_type = return_types
                    .last()
                    .ok_or(SoppoError::TryNoErrorReturn { span: *try_span })?;

                if !self.is_error_type(last_type) {
                    return Err(SoppoError::TryNoErrorReturn { span: *try_span });
                }

                // Infer inner statement and extract expression type + span
                // For ? operator, we need to strip the error from tuple types
                let (expr_ty, expr_span) = match &inner_stmt.kind {
                    StmtKind::Decl { name, value } => {
                        let value_ty = self.infer_expr(value)?;
                        let value_ty_sub = self.substitute(value_ty.clone());

                        // Strip error from tuple type for the variable
                        let var_ty = self.strip_error_from_tuple(&value_ty_sub);
                        self.insert_var(name.clone(), var_ty.clone());
                        self.update_nil_state_for_assignment(name, value, &var_ty);
                        (value_ty_sub, value.span)
                    }
                    StmtKind::MultiDecl { names, values } if values.len() == 1 => {
                        let value_ty = self.infer_expr(&values[0])?;
                        let value_ty_sub = self.substitute(value_ty.clone());

                        // For multi-return, the type is a tuple
                        // Unpack and assign to each name, excluding the error
                        if let Type::Con { name: tname, args } = &value_ty_sub
                            && tname.name == "tuple"
                        {
                            // Exclude the last element (error) when assigning
                            let non_error_count = args.len().saturating_sub(1);
                            for (i, var_name) in names.iter().enumerate() {
                                if i < non_error_count
                                    && let Some(ty) = args.get(i)
                                {
                                    self.insert_var(var_name.clone(), ty.clone());
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
                        (self.substitute(expr_ty), expr.span)
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
                        self.insert_var(name.clone(), Type::simple("error"));
                    }
                    self.infer_block(block)?;
                    self.pop_scope();
                }

                // Mark assigned pointer variables as non-null (success implies valid result)
                if let Some(var_name) = self.get_assigned_var_name(inner_stmt)
                    && let Some(var_type) = self.lookup_var(&var_name)
                {
                    let var_type_sub = self.substitute(var_type);
                    if Self::is_pointer_type(&var_type_sub) {
                        self.set_nil_state(var_name, Nullability::NonNull);
                    }
                }

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
