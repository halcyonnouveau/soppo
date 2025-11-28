use std::collections::HashSet;

use super::Infer;
use crate::error::{Result, SoppoError};
use crate::syntax::{EnumVariant, PatternKind, SelectCaseKind, Stmt, StmtKind};
use crate::types::Type;
use crate::types::ctx::TypeDefKind;

impl Infer {
    /// Infer the type of a statement
    /// Returns the type of the statement (unit for most, or the type of the expression)
    pub fn infer_stmt(&mut self, stmt: &Stmt) -> Result<Type> {
        match &stmt.kind {
            StmtKind::Decl { name, value } => {
                let value_ty = self.infer_expr(value)?;
                self.insert_var(name.clone(), value_ty.clone());
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
                let var_ty = match (ty, value) {
                    (Some(t), Some(expr)) => {
                        // var x type = value: unify declared with inferred
                        let declared_ty = Type::from_ast(t);
                        let value_ty = self.infer_expr(expr)?;
                        self.unify(&declared_ty, &value_ty, &expr.span)?;
                        declared_ty
                    }
                    (Some(t), None) => {
                        // var x type: use declared type (zero value)
                        Type::from_ast(t)
                    }
                    (None, Some(expr)) => {
                        // var x = value: infer from value
                        self.infer_expr(expr)?
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
                self.insert_var(name.clone(), var_ty);
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
                let target_ty = self.infer_expr(target)?;
                let value_ty = self.infer_expr(value)?;
                self.unify(&target_ty, &value_ty, &stmt.span)?;
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

                // Type check then block
                let then_ty = self.infer_block(then_block)?;

                // Type check else block if present
                let else_ty = if let Some(else_block) = else_block {
                    self.infer_block(else_block)?
                } else {
                    Type::unit()
                };

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
