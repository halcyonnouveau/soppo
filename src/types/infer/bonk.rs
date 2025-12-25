//! Bonking (aka Zonking) - substituting type variables with their solutions.
//!
//! After type inference produces a TypedFile with potentially unresolved
//! type variables (Type::Var), bonking walks the tree and substitutes
//! all type variables with their solutions from the substitution map.

use std::collections::HashMap;

use crate::types::Type;
use crate::types::ast::{
    TypedArm, TypedBlock, TypedCallArg, TypedConstDecl, TypedDecl, TypedEnumVariant, TypedExpr,
    TypedExprKind, TypedFieldPattern, TypedFile, TypedFuncDecl, TypedInterfaceMethod, TypedParam,
    TypedPattern, TypedPatternKind, TypedSelectCase, TypedSelectCaseKind, TypedStmt, TypedStmtKind,
    TypedStringPart, TypedTypeDecl, TypedTypeKind, TypedVarDecl,
};

/// Bonk a TypedFile, substituting all type variables.
pub fn bonk_file(file: &mut TypedFile, subs: &HashMap<i32, Type>) {
    for decl in &mut file.decls {
        bonk_decl(decl, subs);
    }
}

/// Bonk a TypedDecl
fn bonk_decl(decl: &mut TypedDecl, subs: &HashMap<i32, Type>) {
    match decl {
        TypedDecl::Const(c) => bonk_const_decl(c, subs),
        TypedDecl::ConstBlock(consts) => {
            for c in consts {
                bonk_const_decl(c, subs);
            }
        }
        TypedDecl::Var(v) => bonk_var_decl(v, subs),
        TypedDecl::Type(t) => bonk_type_decl(t, subs),
        TypedDecl::Func(f) => bonk_func_decl(f, subs),
    }
}

fn bonk_const_decl(c: &mut TypedConstDecl, subs: &HashMap<i32, Type>) {
    bonk_type(&mut c.const_ty, subs);
    bonk_expr(&mut c.value, subs);
}

fn bonk_var_decl(v: &mut TypedVarDecl, subs: &HashMap<i32, Type>) {
    bonk_type(&mut v.var_ty, subs);
    if let Some(value) = &mut v.value {
        bonk_expr(value, subs);
    }
}

fn bonk_type_decl(t: &mut TypedTypeDecl, subs: &HashMap<i32, Type>) {
    match &mut t.kind {
        TypedTypeKind::Alias { target } => bonk_type(target, subs),
        TypedTypeKind::Definition { target } => bonk_type(target, subs),
        TypedTypeKind::Enum { variants } => {
            for v in variants {
                bonk_enum_variant(v, subs);
            }
        }
        TypedTypeKind::Struct { fields } => {
            for (_, ty, _, _) in fields {
                bonk_type(ty, subs);
            }
        }
        TypedTypeKind::Interface { methods } => {
            for m in methods {
                bonk_interface_method(m, subs);
            }
        }
    }
}

fn bonk_enum_variant(v: &mut TypedEnumVariant, subs: &HashMap<i32, Type>) {
    match v {
        TypedEnumVariant::Unit { .. } => {}
        TypedEnumVariant::Single { ty, .. } => bonk_type(ty, subs),
        TypedEnumVariant::Struct { fields, .. } => {
            for (_, ty) in fields {
                bonk_type(ty, subs);
            }
        }
    }
}

fn bonk_interface_method(m: &mut TypedInterfaceMethod, subs: &HashMap<i32, Type>) {
    for p in &mut m.params {
        bonk_param(p, subs);
    }
    for ret in &mut m.returns {
        bonk_type(ret, subs);
    }
}

fn bonk_func_decl(f: &mut TypedFuncDecl, subs: &HashMap<i32, Type>) {
    if let Some(recv) = &mut f.receiver {
        bonk_param(recv, subs);
    }
    for p in &mut f.params {
        bonk_param(p, subs);
    }
    for r in &mut f.returns {
        bonk_param(r, subs);
    }
    bonk_block(&mut f.body, subs);
}

fn bonk_param(p: &mut TypedParam, subs: &HashMap<i32, Type>) {
    bonk_type(&mut p.ty, subs);
}

fn bonk_block(block: &mut TypedBlock, subs: &HashMap<i32, Type>) {
    for stmt in &mut block.stmts {
        bonk_stmt(stmt, subs);
    }
}

/// bonk a TypedStmt
fn bonk_stmt(stmt: &mut TypedStmt, subs: &HashMap<i32, Type>) {
    match &mut stmt.kind {
        TypedStmtKind::Decl { var_ty, value, .. } => {
            bonk_type(var_ty, subs);
            bonk_expr(value, subs);
        }

        TypedStmtKind::MultiDecl {
            var_tys, values, ..
        } => {
            for ty in var_tys {
                bonk_type(ty, subs);
            }
            for v in values {
                bonk_expr(v, subs);
            }
        }

        TypedStmtKind::VarDecl { var_ty, value, .. } => {
            bonk_type(var_ty, subs);
            if let Some(v) = value {
                bonk_expr(v, subs);
            }
        }

        TypedStmtKind::MultiVarDecl { var_ty, values, .. } => {
            bonk_type(var_ty, subs);
            for v in values {
                bonk_expr(v, subs);
            }
        }

        TypedStmtKind::ConstDecl {
            const_ty, value, ..
        } => {
            bonk_type(const_ty, subs);
            bonk_expr(value, subs);
        }

        TypedStmtKind::MultiConstDecl {
            const_ty, values, ..
        } => {
            bonk_type(const_ty, subs);
            for v in values {
                bonk_expr(v, subs);
            }
        }

        TypedStmtKind::Assign { target, value } => {
            bonk_expr(target, subs);
            bonk_expr(value, subs);
        }

        TypedStmtKind::MultiAssign { targets, values } => {
            for t in targets {
                bonk_expr(t, subs);
            }
            for v in values {
                bonk_expr(v, subs);
            }
        }

        TypedStmtKind::CompoundAssign { target, value, .. } => {
            bonk_expr(target, subs);
            bonk_expr(value, subs);
        }

        TypedStmtKind::IncDec { target, .. } => {
            bonk_expr(target, subs);
        }

        TypedStmtKind::For { condition, body } => {
            bonk_expr(condition, subs);
            bonk_block(body, subs);
        }

        TypedStmtKind::ForCStyle {
            init,
            condition,
            post,
            body,
        } => {
            if let Some(s) = init {
                bonk_stmt(s, subs);
            }
            if let Some(e) = condition {
                bonk_expr(e, subs);
            }
            if let Some(s) = post {
                bonk_stmt(s, subs);
            }
            bonk_block(body, subs);
        }

        TypedStmtKind::ForRange {
            key_ty,
            value_ty,
            collection,
            body,
            ..
        } => {
            bonk_type(key_ty, subs);
            if let Some(vt) = value_ty {
                bonk_type(vt, subs);
            }
            bonk_expr(collection, subs);
            bonk_block(body, subs);
        }

        TypedStmtKind::If {
            init,
            condition,
            then_block,
            else_block,
        } => {
            if let Some(s) = init {
                bonk_stmt(s, subs);
            }
            bonk_expr(condition, subs);
            bonk_block(then_block, subs);
            if let Some(b) = else_block {
                bonk_block(b, subs);
            }
        }

        TypedStmtKind::Return { values } => {
            for v in values {
                bonk_expr(v, subs);
            }
        }

        TypedStmtKind::Match {
            scrutinee,
            scrutinee_ty,
            arms,
        } => {
            if let Some(e) = scrutinee {
                bonk_expr(e, subs);
            }
            if let Some(ty) = scrutinee_ty {
                bonk_type(ty, subs);
            }
            for arm in arms {
                bonk_arm(arm, subs);
            }
        }

        TypedStmtKind::Send { channel, value } => {
            bonk_expr(channel, subs);
            bonk_expr(value, subs);
        }

        TypedStmtKind::Select { cases } => {
            for case in cases {
                bonk_select_case(case, subs);
            }
        }

        TypedStmtKind::Go(e) => bonk_expr(e, subs),
        TypedStmtKind::DeferStmt(e) => bonk_expr(e, subs),

        TypedStmtKind::Break | TypedStmtKind::Continue => {}

        TypedStmtKind::Expr(e) => bonk_expr(e, subs),

        TypedStmtKind::TryStmt {
            stmt,
            handler,
            discard_types,
            ..
        } => {
            bonk_stmt(stmt, subs);
            if let Some(h) = handler {
                bonk_block(h, subs);
            }
            for ty in discard_types {
                bonk_type(ty, subs);
            }
        }

        TypedStmtKind::LocalTypeDecl(t) => bonk_type_decl(t, subs),
    }
}

fn bonk_arm(arm: &mut TypedArm, subs: &HashMap<i32, Type>) {
    for p in &mut arm.patterns {
        bonk_pattern(p, subs);
    }
    bonk_block(&mut arm.body, subs);
}

fn bonk_pattern(pattern: &mut TypedPattern, subs: &HashMap<i32, Type>) {
    bonk_type(&mut pattern.matched_ty, subs);

    match &mut pattern.kind {
        TypedPatternKind::Default => {}

        TypedPatternKind::Variant {
            enum_ty, type_args, ..
        } => {
            bonk_type(enum_ty, subs);
            for ta in type_args {
                bonk_type(ta, subs);
            }
        }

        TypedPatternKind::Literal(_) => {}

        TypedPatternKind::Destructor {
            enum_ty,
            type_args,
            binding_ty,
            ..
        } => {
            bonk_type(enum_ty, subs);
            for ta in type_args {
                bonk_type(ta, subs);
            }
            bonk_type(binding_ty, subs);
        }

        TypedPatternKind::StructDestructor {
            struct_ty,
            type_args,
            fields,
            ..
        } => {
            bonk_type(struct_ty, subs);
            for ta in type_args {
                bonk_type(ta, subs);
            }
            for (_, fp) in fields {
                bonk_field_pattern(fp, subs);
            }
        }

        TypedPatternKind::Guard(e) => bonk_expr(e, subs),
    }
}

fn bonk_field_pattern(fp: &mut TypedFieldPattern, subs: &HashMap<i32, Type>) {
    match fp {
        TypedFieldPattern::Bind(_, ty) => bonk_type(ty, subs),
        TypedFieldPattern::Literal(_) => {}
    }
}

fn bonk_select_case(case: &mut TypedSelectCase, subs: &HashMap<i32, Type>) {
    match &mut case.kind {
        TypedSelectCaseKind::Recv { channel, recv_ty } => {
            bonk_expr(channel, subs);
            bonk_type(recv_ty, subs);
        }
        TypedSelectCaseKind::RecvDecl {
            channel, recv_ty, ..
        } => {
            bonk_expr(channel, subs);
            bonk_type(recv_ty, subs);
        }
        TypedSelectCaseKind::RecvDeclOk {
            channel, recv_ty, ..
        } => {
            bonk_expr(channel, subs);
            bonk_type(recv_ty, subs);
        }
        TypedSelectCaseKind::Send { channel, value } => {
            bonk_expr(channel, subs);
            bonk_expr(value, subs);
        }
        TypedSelectCaseKind::Default => {}
    }
    bonk_block(&mut case.body, subs);
}

/// bonk a TypedExpr
fn bonk_expr(expr: &mut TypedExpr, subs: &HashMap<i32, Type>) {
    // bonk the expression's type
    bonk_type(&mut expr.ty, subs);

    // bonk child expressions
    match &mut expr.kind {
        TypedExprKind::Integer(_, _)
        | TypedExprKind::Float(_)
        | TypedExprKind::String(_)
        | TypedExprKind::RawString(_)
        | TypedExprKind::Rune(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Nil
        | TypedExprKind::Ident(_)
        | TypedExprKind::Error => {}

        TypedExprKind::StringInterpolation(parts) => {
            for part in parts {
                if let TypedStringPart::Expr { expr, .. } = part {
                    bonk_expr(expr, subs);
                }
            }
        }

        TypedExprKind::Binary { left, right, .. } => {
            bonk_expr(left, subs);
            bonk_expr(right, subs);
        }

        TypedExprKind::Call {
            func,
            type_args,
            args,
        } => {
            bonk_expr(func, subs);
            for ta in type_args {
                bonk_type(ta, subs);
            }
            for arg in args {
                bonk_call_arg(arg, subs);
            }
        }

        TypedExprKind::TypeConversion { target_ty, value } => {
            bonk_type(target_ty, subs);
            bonk_expr(value, subs);
        }

        TypedExprKind::TypeInst { ty } => {
            bonk_type(ty, subs);
        }

        TypedExprKind::PackageMember { .. } => {
            // No child expressions to bonk
        }

        TypedExprKind::EnumVariant { enum_ty, .. } => {
            bonk_type(enum_ty, subs);
        }

        TypedExprKind::Field { expr, .. } => {
            bonk_expr(expr, subs);
        }

        TypedExprKind::Index { expr, index } => {
            bonk_expr(expr, subs);
            bonk_expr(index, subs);
        }

        TypedExprKind::Slice {
            expr,
            low,
            high,
            cap,
        } => {
            bonk_expr(expr, subs);
            if let Some(l) = low {
                bonk_expr(l, subs);
            }
            if let Some(h) = high {
                bonk_expr(h, subs);
            }
            if let Some(c) = cap {
                bonk_expr(c, subs);
            }
        }

        TypedExprKind::TypeAssert {
            expr, target_ty, ..
        } => {
            bonk_expr(expr, subs);
            bonk_type(target_ty, subs);
        }

        TypedExprKind::NilAssert { expr } => {
            bonk_expr(expr, subs);
        }

        TypedExprKind::ArrayLit { elem_ty, elements } => {
            bonk_type(elem_ty, subs);
            for e in elements {
                bonk_expr(e, subs);
            }
        }

        TypedExprKind::StructLit {
            struct_ty, fields, ..
        } => {
            bonk_type(struct_ty, subs);
            for (_, e) in fields {
                bonk_expr(e, subs);
            }
        }

        TypedExprKind::AnonStructLit { struct_ty, fields } => {
            bonk_type(struct_ty, subs);
            for (_, e) in fields {
                bonk_expr(e, subs);
            }
        }

        TypedExprKind::MapLit { map_ty, entries } => {
            bonk_type(map_ty, subs);
            for (k, v) in entries {
                bonk_expr(k, subs);
                bonk_expr(v, subs);
            }
        }

        TypedExprKind::Unary { operand, .. } => {
            bonk_expr(operand, subs);
        }

        TypedExprKind::Deref { operand } => {
            bonk_expr(operand, subs);
        }

        TypedExprKind::FuncLit {
            params,
            returns,
            body,
        } => {
            for p in params {
                bonk_param(p, subs);
            }
            for r in returns {
                bonk_param(r, subs);
            }
            bonk_block(body, subs);
        }

        TypedExprKind::Block(block) => {
            bonk_block(block, subs);
        }

        TypedExprKind::Paren(inner) => {
            bonk_expr(inner, subs);
        }
    }
}

fn bonk_call_arg(arg: &mut TypedCallArg, subs: &HashMap<i32, Type>) {
    bonk_expr(&mut arg.1, subs);
}

/// bonk a Type, replacing type variables with their solutions.
fn bonk_type(ty: &mut Type, subs: &HashMap<i32, Type>) {
    match ty {
        Type::Var(v) => {
            if let Some(solution) = subs.get(v) {
                let mut resolved = solution.clone();
                // Recursively bonk the solution (it might contain more variables)
                bonk_type(&mut resolved, subs);
                *ty = resolved;
            }
        }
        Type::Con { args, .. } => {
            for arg in args {
                bonk_type(arg, subs);
            }
        }
        Type::Func { args, ret, .. } => {
            for (_, arg_ty) in args {
                bonk_type(arg_ty, subs);
            }
            bonk_type(ret, subs);
        }
        Type::Never | Type::Error => {}
    }
}
