//! Lint rule: suggest using the `?` operator instead of `if err != nil`.

use crate::sniff::{Lint, LintWarning};
use crate::syntax::{BinOp, Span};
use crate::types::Type;
use crate::types::ast::{
    TypedBlock, TypedDecl, TypedExpr, TypedExprKind, TypedFile, TypedFuncDecl, TypedStmt,
    TypedStmtKind,
};

pub struct TryOperator;

impl Lint for TryOperator {
    fn code(&self) -> &'static str {
        "try_operator"
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

fn check_function(func: &TypedFuncDecl, source_name: &str, source_code: &str) -> Vec<LintWarning> {
    check_block(&func.body, source_name, source_code)
}

fn check_block(block: &TypedBlock, source_name: &str, source_code: &str) -> Vec<LintWarning> {
    let mut warnings = vec![];

    // Look at pairs of consecutive statements
    for i in 0..block.stmts.len() {
        // Check if this statement assigns an error variable
        let err_var_name = get_error_var_from_stmt(&block.stmts[i]);

        if let Some(err_name) = err_var_name {
            // Check if next statement is `if err != nil`
            if i + 1 < block.stmts.len()
                && let Some(if_span) =
                    check_if_err_nil(&block.stmts[i + 1], &err_name, source_name, source_code)
            {
                let warning = LintWarning::new(
                    "try_operator",
                    "Consider using the `?` operator",
                    if_span,
                    "use `?` instead",
                    source_name,
                    source_code,
                )
                .with_help(format!(
                    "replace `if {} != nil {{ ... }}` with `?`",
                    err_name
                ));

                warnings.push(warning);
            }
        }

        // Check for short-circuit if: `if err := call(); err != nil { ... }`
        if let Some(warning) = check_short_circuit_if(&block.stmts[i], source_name, source_code) {
            warnings.push(warning);
        }

        // Recursively check nested blocks
        warnings.extend(check_stmt_nested(&block.stmts[i], source_name, source_code));
    }

    warnings
}

/// Get the error variable name if this statement assigns to an error type.
fn get_error_var_from_stmt(stmt: &TypedStmt) -> Option<String> {
    match &stmt.kind {
        // x, err := call()
        TypedStmtKind::MultiDecl {
            idents, var_tys, ..
        } => {
            for (ident, ty) in idents.iter().zip(var_tys.iter()) {
                if is_error_type(ty) {
                    return Some(ident.name.clone());
                }
            }
            None
        }
        // err := call()
        TypedStmtKind::Decl { ident, var_ty, .. } => {
            if is_error_type(var_ty) {
                Some(ident.name.clone())
            } else {
                None
            }
        }
        // x, err = call()
        TypedStmtKind::MultiAssign { targets, .. } => {
            for target in targets {
                if is_error_type(&target.ty)
                    && let TypedExprKind::Ident(name) = &target.kind
                {
                    return Some(name.clone());
                }
            }
            None
        }
        // err = call()
        TypedStmtKind::Assign { target, .. } => {
            if is_error_type(&target.ty)
                && let TypedExprKind::Ident(name) = &target.kind
            {
                return Some(name.clone());
            }
            None
        }
        _ => None,
    }
}

/// Check if this statement is `if err != nil` and return the if span.
fn check_if_err_nil(
    stmt: &TypedStmt,
    err_name: &str,
    _source_name: &str,
    _source_code: &str,
) -> Option<Span> {
    if let TypedStmtKind::If { condition, .. } = &stmt.kind
        && is_err_nil_check(condition, err_name)
    {
        return Some(stmt.span);
    }
    None
}

/// Check for short-circuit if: `if err := call(); err != nil { ... }`
fn check_short_circuit_if(
    stmt: &TypedStmt,
    source_name: &str,
    source_code: &str,
) -> Option<LintWarning> {
    if let TypedStmtKind::If {
        init: Some(init),
        condition,
        ..
    } = &stmt.kind
    {
        // Check if init assigns an error variable
        if let Some(err_name) = get_error_var_from_stmt(init) {
            // Check if condition is `err != nil`
            if is_err_nil_check(condition, &err_name) {
                return Some(
                    LintWarning::new(
                        "try_operator",
                        "Consider using the `?` operator",
                        stmt.span,
                        "use `?` instead",
                        source_name,
                        source_code,
                    )
                    .with_help(format!(
                        "replace `if {} := ...; {} != nil {{ ... }}` with `? {} {{ ... }}`",
                        err_name, err_name, err_name
                    )),
                );
            }
        }
    }
    None
}

/// Check if the condition is `err != nil` or `nil != err`.
fn is_err_nil_check(expr: &TypedExpr, err_name: &str) -> bool {
    if let TypedExprKind::Binary {
        op: BinOp::Ne,
        left,
        right,
    } = &expr.kind
    {
        // Check err != nil
        if is_ident(left, err_name) && is_nil(right) {
            return true;
        }
        // Check nil != err
        if is_nil(left) && is_ident(right, err_name) {
            return true;
        }
    }
    false
}

fn is_ident(expr: &TypedExpr, name: &str) -> bool {
    matches!(&expr.kind, TypedExprKind::Ident(n) if n == name)
}

fn is_nil(expr: &TypedExpr) -> bool {
    matches!(&expr.kind, TypedExprKind::Nil)
}

fn is_error_type(ty: &Type) -> bool {
    match ty {
        Type::Con {
            sym,
            args,
            nullable: false,
        } if args.is_empty() => sym.name == "error",
        _ => false,
    }
}

/// Recursively check nested blocks in a statement.
fn check_stmt_nested(stmt: &TypedStmt, source_name: &str, source_code: &str) -> Vec<LintWarning> {
    let mut warnings = vec![];

    match &stmt.kind {
        TypedStmtKind::If {
            then_block,
            else_block,
            ..
        } => {
            warnings.extend(check_block(then_block, source_name, source_code));
            if let Some(else_b) = else_block {
                warnings.extend(check_block(else_b, source_name, source_code));
            }
        }
        TypedStmtKind::For { body, .. } => {
            warnings.extend(check_block(body, source_name, source_code));
        }
        TypedStmtKind::ForRange { body, .. } => {
            warnings.extend(check_block(body, source_name, source_code));
        }
        TypedStmtKind::Match { arms, .. } => {
            for arm in arms {
                warnings.extend(check_block(&arm.body, source_name, source_code));
            }
        }
        _ => {}
    }

    warnings
}
