//! Lint rule: warn when code is unreachable.

use crate::sniff::{Lint, LintWarning};
use crate::syntax::Span;
use crate::types::ast::{
    TypedBlock, TypedDecl, TypedExpr, TypedExprKind, TypedFile, TypedFuncDecl, TypedStmt,
    TypedStmtKind,
};

pub struct Unreachable;

impl Lint for Unreachable {
    fn code(&self) -> &'static str {
        "unreachable"
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

    for (i, stmt) in block.stmts.iter().enumerate() {
        // Check nested blocks within this statement
        warnings.extend(check_stmt(stmt, source_name, source_code));

        // Check if this is a terminating statement
        if is_terminating(stmt) && i + 1 < block.stmts.len() {
            // There are statements after this terminating statement
            let next_stmt = &block.stmts[i + 1];
            let unreachable_span = Span {
                start: next_stmt.span.start,
                end: block.stmts.last().unwrap().span.end,
                ..next_stmt.span
            };

            warnings.push(LintWarning::new(
                "unreachable",
                "Unreachable code".to_string(),
                unreachable_span,
                "this code will never execute",
                source_name,
                source_code,
            ));

            // Don't report multiple unreachable warnings for the same block
            break;
        }
    }

    warnings
}

/// Check if a statement is terminating (control flow never continues past it).
fn is_terminating(stmt: &TypedStmt) -> bool {
    match &stmt.kind {
        TypedStmtKind::Return { .. } => true,
        TypedStmtKind::Break | TypedStmtKind::Continue => true,
        // A match is terminating if all arms are terminating
        TypedStmtKind::Match { arms, .. } => {
            !arms.is_empty() && arms.iter().all(|arm| block_terminates(&arm.body))
        }
        // An if is terminating if both branches exist and both terminate
        TypedStmtKind::If {
            then_block,
            else_block: Some(else_b),
            ..
        } => block_terminates(then_block) && block_terminates(else_b),
        _ => false,
    }
}

/// Check if a block always terminates.
fn block_terminates(block: &TypedBlock) -> bool {
    block.stmts.last().is_some_and(is_terminating)
}

/// Recursively check statements for unreachable code in nested blocks.
fn check_stmt(stmt: &TypedStmt, source_name: &str, source_code: &str) -> Vec<LintWarning> {
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

        TypedStmtKind::For { body, .. }
        | TypedStmtKind::ForCStyle { body, .. }
        | TypedStmtKind::ForRange { body, .. } => {
            warnings.extend(check_block(body, source_name, source_code));
        }

        TypedStmtKind::Match { arms, .. } => {
            for arm in arms {
                warnings.extend(check_block(&arm.body, source_name, source_code));
            }
        }

        TypedStmtKind::Select { cases } => {
            for case in cases {
                warnings.extend(check_block(&case.body, source_name, source_code));
            }
        }

        TypedStmtKind::TryStmt { stmt, handler, .. } => {
            warnings.extend(check_stmt(stmt, source_name, source_code));
            if let Some(h) = handler {
                warnings.extend(check_block(h, source_name, source_code));
            }
        }

        // Check expressions that contain blocks (function literals)
        TypedStmtKind::Decl { value, .. } => {
            warnings.extend(check_expr(value, source_name, source_code));
        }

        TypedStmtKind::Expr(expr) => {
            warnings.extend(check_expr(expr, source_name, source_code));
        }

        _ => {}
    }

    warnings
}

fn check_expr(expr: &TypedExpr, source_name: &str, source_code: &str) -> Vec<LintWarning> {
    let mut warnings = vec![];

    match &expr.kind {
        TypedExprKind::FuncLit { body, .. } => {
            warnings.extend(check_block(body, source_name, source_code));
        }

        TypedExprKind::Call { func, args, .. } => {
            warnings.extend(check_expr(func, source_name, source_code));
            for (_, arg, _) in args {
                warnings.extend(check_expr(arg, source_name, source_code));
            }
        }

        TypedExprKind::Binary { left, right, .. } => {
            warnings.extend(check_expr(left, source_name, source_code));
            warnings.extend(check_expr(right, source_name, source_code));
        }

        TypedExprKind::Block(block) => {
            warnings.extend(check_block(block, source_name, source_code));
        }

        _ => {}
    }

    warnings
}
