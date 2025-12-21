//! Lint rule: warn when a variable shadows another from an outer scope.

use std::collections::HashMap;

use crate::sniff::{Lint, LintWarning};
use crate::syntax::Span;
use crate::types::Type;
use crate::types::ast::{
    TypedBlock, TypedDecl, TypedExpr, TypedExprKind, TypedFieldPattern, TypedFile, TypedFuncDecl,
    TypedPatternKind, TypedSelectCaseKind, TypedStmt, TypedStmtKind,
};

pub struct Shadow;

impl Lint for Shadow {
    fn code(&self) -> &'static str {
        "shadow"
    }

    fn check(&self, file: &TypedFile, source_name: &str, source_code: &str) -> Vec<LintWarning> {
        let mut warnings = vec![];

        for decl in &file.decls {
            if let TypedDecl::Func(func) = decl {
                warnings.extend(check_function(func, source_name, source_code));
            }
        }

        warnings
    }
}

/// Track variable declarations with their spans.
struct Scope {
    vars: HashMap<String, Span>,
}

impl Scope {
    fn new() -> Self {
        Self {
            vars: HashMap::new(),
        }
    }
}

fn check_function(func: &TypedFuncDecl, source_name: &str, source_code: &str) -> Vec<LintWarning> {
    let mut scopes: Vec<Scope> = vec![Scope::new()];

    // Add function parameters to the initial scope
    for param in &func.params {
        scopes[0]
            .vars
            .insert(param.ident.name.clone(), param.ident.span);
    }

    check_block(&func.body, &mut scopes, source_name, source_code)
}

fn check_block(
    block: &TypedBlock,
    scopes: &mut Vec<Scope>,
    source_name: &str,
    source_code: &str,
) -> Vec<LintWarning> {
    let mut warnings = vec![];

    // Push a new scope for this block
    scopes.push(Scope::new());

    for stmt in &block.stmts {
        warnings.extend(check_stmt(stmt, scopes, source_name, source_code));
    }

    // Pop the scope
    scopes.pop();

    warnings
}

fn check_stmt(
    stmt: &TypedStmt,
    scopes: &mut Vec<Scope>,
    source_name: &str,
    source_code: &str,
) -> Vec<LintWarning> {
    let mut warnings = vec![];

    match &stmt.kind {
        TypedStmtKind::Decl {
            ident,
            var_ty,
            value,
        } => {
            if let Some(warning) = check_shadow(
                &ident.name,
                ident.span,
                Some(var_ty),
                scopes,
                source_name,
                source_code,
            ) {
                warnings.push(warning);
            }
            // Check the value expression for lambdas
            warnings.extend(check_expr(value, scopes, source_name, source_code));
            // Add to current scope
            if let Some(scope) = scopes.last_mut() {
                scope.vars.insert(ident.name.clone(), ident.span);
            }
        }

        TypedStmtKind::MultiDecl {
            idents,
            var_tys,
            values,
        } => {
            for (ident, ty) in idents.iter().zip(var_tys.iter()) {
                if let Some(warning) = check_shadow(
                    &ident.name,
                    ident.span,
                    Some(ty),
                    scopes,
                    source_name,
                    source_code,
                ) {
                    warnings.push(warning);
                }
            }
            for value in values {
                warnings.extend(check_expr(value, scopes, source_name, source_code));
            }
            for ident in idents {
                if let Some(scope) = scopes.last_mut() {
                    scope.vars.insert(ident.name.clone(), ident.span);
                }
            }
        }

        TypedStmtKind::VarDecl {
            ident,
            var_ty,
            value,
            ..
        } => {
            if let Some(warning) = check_shadow(
                &ident.name,
                ident.span,
                Some(var_ty),
                scopes,
                source_name,
                source_code,
            ) {
                warnings.push(warning);
            }
            if let Some(v) = value {
                warnings.extend(check_expr(v, scopes, source_name, source_code));
            }
            if let Some(scope) = scopes.last_mut() {
                scope.vars.insert(ident.name.clone(), ident.span);
            }
        }

        TypedStmtKind::MultiVarDecl {
            idents,
            var_ty,
            values,
            ..
        } => {
            for ident in idents {
                if let Some(warning) = check_shadow(
                    &ident.name,
                    ident.span,
                    Some(var_ty),
                    scopes,
                    source_name,
                    source_code,
                ) {
                    warnings.push(warning);
                }
            }
            for value in values {
                warnings.extend(check_expr(value, scopes, source_name, source_code));
            }
            for ident in idents {
                if let Some(scope) = scopes.last_mut() {
                    scope.vars.insert(ident.name.clone(), ident.span);
                }
            }
        }

        TypedStmtKind::ConstDecl {
            ident,
            const_ty,
            value,
            ..
        } => {
            if let Some(warning) = check_shadow(
                &ident.name,
                ident.span,
                Some(const_ty),
                scopes,
                source_name,
                source_code,
            ) {
                warnings.push(warning);
            }
            warnings.extend(check_expr(value, scopes, source_name, source_code));
            if let Some(scope) = scopes.last_mut() {
                scope.vars.insert(ident.name.clone(), ident.span);
            }
        }

        TypedStmtKind::MultiConstDecl {
            idents,
            const_ty,
            values,
            ..
        } => {
            for ident in idents {
                if let Some(warning) = check_shadow(
                    &ident.name,
                    ident.span,
                    Some(const_ty),
                    scopes,
                    source_name,
                    source_code,
                ) {
                    warnings.push(warning);
                }
            }
            for value in values {
                warnings.extend(check_expr(value, scopes, source_name, source_code));
            }
            for ident in idents {
                if let Some(scope) = scopes.last_mut() {
                    scope.vars.insert(ident.name.clone(), ident.span);
                }
            }
        }

        TypedStmtKind::If {
            init,
            condition,
            then_block,
            else_block,
        } => {
            // If with init has its own scope
            if init.is_some() {
                scopes.push(Scope::new());
            }

            if let Some(init_stmt) = init {
                warnings.extend(check_stmt(init_stmt, scopes, source_name, source_code));
            }
            warnings.extend(check_expr(condition, scopes, source_name, source_code));
            warnings.extend(check_block(then_block, scopes, source_name, source_code));
            if let Some(else_b) = else_block {
                warnings.extend(check_block(else_b, scopes, source_name, source_code));
            }

            if init.is_some() {
                scopes.pop();
            }
        }

        TypedStmtKind::For { condition, body } => {
            warnings.extend(check_expr(condition, scopes, source_name, source_code));
            warnings.extend(check_block(body, scopes, source_name, source_code));
        }

        TypedStmtKind::ForCStyle {
            init,
            condition,
            post,
            body,
        } => {
            // C-style for loop has its own scope
            scopes.push(Scope::new());

            if let Some(init_stmt) = init {
                warnings.extend(check_stmt(init_stmt, scopes, source_name, source_code));
            }
            if let Some(cond) = condition {
                warnings.extend(check_expr(cond, scopes, source_name, source_code));
            }
            if let Some(post_stmt) = post {
                warnings.extend(check_stmt(post_stmt, scopes, source_name, source_code));
            }

            warnings.extend(check_block(body, scopes, source_name, source_code));

            scopes.pop();
        }

        TypedStmtKind::ForRange {
            key,
            key_ty,
            value,
            value_ty,
            collection,
            body,
        } => {
            warnings.extend(check_expr(collection, scopes, source_name, source_code));

            // For-range has its own scope
            scopes.push(Scope::new());

            // key is always present
            if let Some(warning) = check_shadow(
                &key.name,
                key.span,
                Some(key_ty),
                scopes,
                source_name,
                source_code,
            ) {
                warnings.push(warning);
            }
            if let Some(scope) = scopes.last_mut() {
                scope.vars.insert(key.name.clone(), key.span);
            }

            if let Some(v) = value {
                if let Some(warning) = check_shadow(
                    &v.name,
                    v.span,
                    value_ty.as_ref(),
                    scopes,
                    source_name,
                    source_code,
                ) {
                    warnings.push(warning);
                }
                if let Some(scope) = scopes.last_mut() {
                    scope.vars.insert(v.name.clone(), v.span);
                }
            }

            warnings.extend(check_block(body, scopes, source_name, source_code));

            scopes.pop();
        }

        TypedStmtKind::Match {
            scrutinee, arms, ..
        } => {
            if let Some(expr) = scrutinee {
                warnings.extend(check_expr(expr, scopes, source_name, source_code));
            }

            for arm in arms {
                // Each arm has its own scope for pattern bindings
                scopes.push(Scope::new());

                // Extract bindings from patterns
                for pattern in &arm.patterns {
                    match &pattern.kind {
                        TypedPatternKind::Destructor {
                            binding,
                            binding_ty,
                            ..
                        } => {
                            if let Some(warning) = check_shadow(
                                &binding.name,
                                binding.span,
                                Some(binding_ty),
                                scopes,
                                source_name,
                                source_code,
                            ) {
                                warnings.push(warning);
                            }
                            if let Some(scope) = scopes.last_mut() {
                                scope.vars.insert(binding.name.clone(), binding.span);
                            }
                        }
                        TypedPatternKind::StructDestructor { fields, .. } => {
                            for (_, field_pattern) in fields {
                                if let TypedFieldPattern::Bind(ident, ty) = field_pattern {
                                    if let Some(warning) = check_shadow(
                                        &ident.name,
                                        ident.span,
                                        Some(ty),
                                        scopes,
                                        source_name,
                                        source_code,
                                    ) {
                                        warnings.push(warning);
                                    }
                                    if let Some(scope) = scopes.last_mut() {
                                        scope.vars.insert(ident.name.clone(), ident.span);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                warnings.extend(check_block(&arm.body, scopes, source_name, source_code));

                scopes.pop();
            }
        }

        TypedStmtKind::Select { cases } => {
            for case in cases {
                scopes.push(Scope::new());

                match &case.kind {
                    TypedSelectCaseKind::RecvDecl {
                        ident,
                        channel,
                        recv_ty,
                    } => {
                        warnings.extend(check_expr(channel, scopes, source_name, source_code));
                        if let Some(warning) = check_shadow(
                            &ident.name,
                            ident.span,
                            Some(recv_ty),
                            scopes,
                            source_name,
                            source_code,
                        ) {
                            warnings.push(warning);
                        }
                        if let Some(scope) = scopes.last_mut() {
                            scope.vars.insert(ident.name.clone(), ident.span);
                        }
                    }
                    TypedSelectCaseKind::RecvDeclOk {
                        ident,
                        ok_ident,
                        channel,
                        recv_ty,
                    } => {
                        warnings.extend(check_expr(channel, scopes, source_name, source_code));
                        // Check ident with recv_ty
                        if let Some(warning) = check_shadow(
                            &ident.name,
                            ident.span,
                            Some(recv_ty),
                            scopes,
                            source_name,
                            source_code,
                        ) {
                            warnings.push(warning);
                        }
                        if let Some(scope) = scopes.last_mut() {
                            scope.vars.insert(ident.name.clone(), ident.span);
                        }
                        // Check ok_ident (bool type, never error)
                        if let Some(warning) = check_shadow(
                            &ok_ident.name,
                            ok_ident.span,
                            None,
                            scopes,
                            source_name,
                            source_code,
                        ) {
                            warnings.push(warning);
                        }
                        if let Some(scope) = scopes.last_mut() {
                            scope.vars.insert(ok_ident.name.clone(), ok_ident.span);
                        }
                    }
                    TypedSelectCaseKind::Recv { channel, .. } => {
                        warnings.extend(check_expr(channel, scopes, source_name, source_code));
                    }
                    TypedSelectCaseKind::Send { channel, value } => {
                        warnings.extend(check_expr(channel, scopes, source_name, source_code));
                        warnings.extend(check_expr(value, scopes, source_name, source_code));
                    }
                    TypedSelectCaseKind::Default => {}
                }

                warnings.extend(check_block(&case.body, scopes, source_name, source_code));

                scopes.pop();
            }
        }

        TypedStmtKind::Expr(expr) => {
            warnings.extend(check_expr(expr, scopes, source_name, source_code));
        }

        TypedStmtKind::Assign { value, .. } => {
            warnings.extend(check_expr(value, scopes, source_name, source_code));
        }

        TypedStmtKind::MultiAssign { values, .. } => {
            for value in values {
                warnings.extend(check_expr(value, scopes, source_name, source_code));
            }
        }

        TypedStmtKind::CompoundAssign { value, .. } => {
            warnings.extend(check_expr(value, scopes, source_name, source_code));
        }

        TypedStmtKind::Return { values } => {
            for v in values {
                warnings.extend(check_expr(v, scopes, source_name, source_code));
            }
        }

        TypedStmtKind::Send { channel, value } => {
            warnings.extend(check_expr(channel, scopes, source_name, source_code));
            warnings.extend(check_expr(value, scopes, source_name, source_code));
        }

        TypedStmtKind::Go(expr) | TypedStmtKind::DeferStmt(expr) => {
            warnings.extend(check_expr(expr, scopes, source_name, source_code));
        }

        TypedStmtKind::TryStmt {
            stmt,
            handler,
            error_name,
            ..
        } => {
            warnings.extend(check_stmt(stmt, scopes, source_name, source_code));

            if let Some(handler_block) = handler {
                scopes.push(Scope::new());

                // error_name is always error type - skip shadow check (reusing err is idiomatic)
                if let Some(err_ident) = error_name
                    && let Some(scope) = scopes.last_mut()
                {
                    scope.vars.insert(err_ident.name.clone(), err_ident.span);
                }

                warnings.extend(check_block(handler_block, scopes, source_name, source_code));

                scopes.pop();
            }
        }

        _ => {}
    }

    warnings
}

fn check_expr(
    expr: &TypedExpr,
    scopes: &mut Vec<Scope>,
    source_name: &str,
    source_code: &str,
) -> Vec<LintWarning> {
    let mut warnings = vec![];

    match &expr.kind {
        TypedExprKind::FuncLit { params, body, .. } => {
            scopes.push(Scope::new());

            for param in params {
                if let Some(warning) = check_shadow(
                    &param.ident.name,
                    param.ident.span,
                    Some(&param.ty),
                    scopes,
                    source_name,
                    source_code,
                ) {
                    warnings.push(warning);
                }
                if let Some(scope) = scopes.last_mut() {
                    scope
                        .vars
                        .insert(param.ident.name.clone(), param.ident.span);
                }
            }

            warnings.extend(check_block(body, scopes, source_name, source_code));

            scopes.pop();
        }

        TypedExprKind::Call { func, args, .. } => {
            warnings.extend(check_expr(func, scopes, source_name, source_code));
            for (_, arg, _) in args {
                warnings.extend(check_expr(arg, scopes, source_name, source_code));
            }
        }

        TypedExprKind::Binary { left, right, .. } => {
            warnings.extend(check_expr(left, scopes, source_name, source_code));
            warnings.extend(check_expr(right, scopes, source_name, source_code));
        }

        TypedExprKind::Unary { operand, .. } | TypedExprKind::Deref { operand } => {
            warnings.extend(check_expr(operand, scopes, source_name, source_code));
        }

        TypedExprKind::Index { expr, index, .. } => {
            warnings.extend(check_expr(expr, scopes, source_name, source_code));
            warnings.extend(check_expr(index, scopes, source_name, source_code));
        }

        TypedExprKind::Field { expr, .. } => {
            warnings.extend(check_expr(expr, scopes, source_name, source_code));
        }

        TypedExprKind::Slice {
            expr,
            low,
            high,
            cap,
        } => {
            warnings.extend(check_expr(expr, scopes, source_name, source_code));
            if let Some(l) = low {
                warnings.extend(check_expr(l, scopes, source_name, source_code));
            }
            if let Some(h) = high {
                warnings.extend(check_expr(h, scopes, source_name, source_code));
            }
            if let Some(c) = cap {
                warnings.extend(check_expr(c, scopes, source_name, source_code));
            }
        }

        TypedExprKind::StructLit { fields, .. } | TypedExprKind::AnonStructLit { fields, .. } => {
            for (_, field_expr) in fields {
                warnings.extend(check_expr(field_expr, scopes, source_name, source_code));
            }
        }

        TypedExprKind::ArrayLit { elements, .. } => {
            for elem in elements {
                warnings.extend(check_expr(elem, scopes, source_name, source_code));
            }
        }

        TypedExprKind::MapLit { entries, .. } => {
            for (k, v) in entries {
                warnings.extend(check_expr(k, scopes, source_name, source_code));
                warnings.extend(check_expr(v, scopes, source_name, source_code));
            }
        }

        TypedExprKind::TypeAssert { expr, .. } | TypedExprKind::NilAssert { expr } => {
            warnings.extend(check_expr(expr, scopes, source_name, source_code));
        }

        TypedExprKind::TypeConversion { value, .. } => {
            warnings.extend(check_expr(value, scopes, source_name, source_code));
        }

        TypedExprKind::Paren(inner) => {
            warnings.extend(check_expr(inner, scopes, source_name, source_code));
        }

        TypedExprKind::Block(block) => {
            warnings.extend(check_block(block, scopes, source_name, source_code));
        }

        TypedExprKind::StringInterpolation(parts) => {
            for part in parts {
                if let crate::types::ast::TypedStringPart::Expr { expr, .. } = part {
                    warnings.extend(check_expr(expr, scopes, source_name, source_code));
                }
            }
        }

        _ => {}
    }

    warnings
}

/// Check if declaring `name` at `span` would shadow a variable in an outer scope.
fn check_shadow(
    name: &str,
    span: Span,
    ty: Option<&Type>,
    scopes: &[Scope],
    source_name: &str,
    source_code: &str,
) -> Option<LintWarning> {
    // Skip underscore - it's meant to be discarded
    if name == "_" {
        return None;
    }

    // Skip error types - reusing `err` is idiomatic
    if let Some(t) = ty
        && is_error_type(t)
    {
        return None;
    }

    // Check outer scopes (not the current one)
    if scopes.len() > 1 {
        for scope in &scopes[..scopes.len() - 1] {
            if scope.vars.contains_key(name) {
                return Some(
                    LintWarning::new(
                        "shadow",
                        format!("`{}` shadows a variable from an outer scope", name),
                        span,
                        "shadows outer variable",
                        source_name,
                        source_code,
                    )
                    .with_help("consider using a different name".to_string()),
                );
            }
        }
    }

    None
}

fn is_error_type(ty: &Type) -> bool {
    match ty {
        Type::Con { sym, args, .. } if args.is_empty() => sym.name == "error",
        _ => false,
    }
}
