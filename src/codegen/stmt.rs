use super::{Codegen, type_to_go_string};
use crate::syntax::{Ident, IntFormat, Literal};
use crate::types::Type;
use crate::types::ast::{
    TypedArm, TypedBlock, TypedExpr, TypedExprKind, TypedFieldPattern, TypedPatternKind,
    TypedSelectCase, TypedSelectCaseKind, TypedStmt, TypedStmtKind,
};

impl Codegen {
    /// Emit statement ending with optional trailing comment and newline
    fn emit_stmt_end(&mut self, line: usize) {
        self.emit_trailing_comment(line);
        self.emit("\n");
    }

    /// Generate a statement from TypedStmt
    pub(crate) fn gen_stmt(&mut self, stmt: &TypedStmt) {
        // Emit any comments that should appear before this statement
        self.emit_comments_before(stmt.span.byte_start, stmt.span.start.line);

        // Track the statement's line for trailing comments
        let stmt_line = stmt.span.start.line;

        match &stmt.kind {
            TypedStmtKind::Decl { ident, value, .. } => {
                self.emit_indent();
                // Check if assigning an enum variant struct literal
                // If so, declare with interface type so type assertions work in Go
                if let TypedExprKind::StructLit { struct_ty, .. } = &value.kind {
                    let type_str = type_to_go_string(struct_ty);
                    // Check for enum variant pattern: EnumType.Variant or EnumType_Variant
                    if let Some((enum_name, _variant)) = type_str.split_once('.')
                        && self.global_state.is_enum(enum_name)
                    {
                        // This is an enum variant like Shape.Circle
                        // Generate: var name EnumInterface = Variant{...}
                        self.emit("var ");
                        self.emit(&ident.name);
                        self.emit(" ");
                        self.emit(enum_name);
                        self.emit(" = ");
                        self.gen_expr(value);
                        self.emit_stmt_end(stmt_line);
                        return;
                    }
                }
                self.emit(&ident.name);
                self.emit(" := ");
                self.gen_expr(value);
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::MultiDecl { idents, values, .. } => {
                self.emit_indent();
                for (i, ident) in idents.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.emit(&ident.name);
                }
                self.emit(" := ");
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(value);
                }
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::VarDecl {
                ident,
                var_ty,
                has_explicit_type,
                value,
            } => {
                self.emit_indent();
                self.emit("var ");
                self.emit(&ident.name);
                if *has_explicit_type {
                    self.emit(" ");
                    self.emit(type_to_go_string(var_ty));
                }
                if let Some(v) = value {
                    self.emit(" = ");
                    self.gen_expr(v);
                }
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::MultiVarDecl {
                idents,
                var_ty,
                has_explicit_type,
                values,
            } => {
                self.emit_indent();
                self.emit("var ");
                for (i, ident) in idents.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.emit(&ident.name);
                }
                if *has_explicit_type {
                    self.emit(" ");
                    self.emit(type_to_go_string(var_ty));
                }
                if !values.is_empty() {
                    self.emit(" = ");
                    for (i, value) in values.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.gen_expr(value);
                    }
                }
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::ConstDecl {
                ident,
                const_ty,
                has_explicit_type,
                value,
            } => {
                self.emit_indent();
                // Use Go's const for compile-time constant expressions,
                // otherwise use var (immutability is enforced by Soppo)
                if Self::is_go_const_expr(value) {
                    self.emit("const ");
                    self.emit(&ident.name);
                    if *has_explicit_type {
                        self.emit(" ");
                        self.emit(type_to_go_string(const_ty));
                    }
                    self.emit(" = ");
                    self.gen_expr(value);
                } else {
                    // Runtime value: emit as var (Soppo prevents reassignment)
                    self.emit(&ident.name);
                    self.emit(" := ");
                    self.gen_expr(value);
                }
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::MultiConstDecl {
                idents,
                const_ty,
                has_explicit_type,
                values,
            } => {
                self.emit_indent();
                // Use Go's const only if ALL values are compile-time constants
                let all_const = values.iter().all(Self::is_go_const_expr);
                if all_const {
                    self.emit("const ");
                    for (i, ident) in idents.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.emit(&ident.name);
                    }
                    if *has_explicit_type {
                        self.emit(" ");
                        self.emit(type_to_go_string(const_ty));
                    }
                    self.emit(" = ");
                    for (i, value) in values.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.gen_expr(value);
                    }
                } else {
                    // Runtime values: emit as var (Soppo prevents reassignment)
                    for (i, ident) in idents.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.emit(&ident.name);
                    }
                    self.emit(" := ");
                    for (i, value) in values.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.gen_expr(value);
                    }
                }
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::Assign { target, value } => {
                self.emit_indent();
                self.gen_expr(target);
                self.emit(" = ");
                self.gen_expr(value);
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::MultiAssign { targets, values } => {
                self.emit_indent();
                for (i, target) in targets.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(target);
                }
                self.emit(" = ");
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(value);
                }
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::CompoundAssign { target, op, value } => {
                self.emit_indent();
                self.gen_expr(target);
                self.emit(format!(" {} ", self.go_assign_op(op)));
                self.gen_expr(value);
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::IncDec { target, is_inc } => {
                self.emit_indent();
                self.gen_expr(target);
                self.emit(if *is_inc { "++" } else { "--" });
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::For { condition, body } => {
                self.emit_indent();
                self.emit("for ");
                self.gen_expr(condition);
                self.emit(" ");
                self.gen_block(body);
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::ForCStyle {
                init,
                condition,
                post,
                body,
            } => {
                self.emit_indent();
                self.emit("for ");

                // If all parts are None, generate simple infinite loop: for { ... }
                if init.is_none() && condition.is_none() && post.is_none() {
                    self.gen_block(body);
                    self.emit("\n");
                } else {
                    if let Some(init) = init {
                        self.gen_stmt_inline(init);
                    }
                    self.emit("; ");
                    if let Some(cond) = condition {
                        self.gen_expr(cond);
                    }
                    self.emit("; ");
                    if let Some(post) = post {
                        self.gen_stmt_inline(post);
                    }
                    self.emit(" ");
                    self.gen_block(body);
                    self.emit_stmt_end(stmt_line);
                }
            }

            TypedStmtKind::ForRange {
                key,
                value,
                collection,
                body,
                ..
            } => {
                self.emit_indent();
                self.emit("for ");
                self.emit(&key.name);
                if let Some(val) = value {
                    self.emit(", ");
                    self.emit(&val.name);
                }
                self.emit(" := range ");
                self.gen_expr(collection);
                self.emit(" ");
                self.gen_block(body);
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::If {
                init,
                condition,
                then_block,
                else_block,
            } => {
                self.emit_indent();
                self.emit("if ");
                if let Some(init) = init {
                    self.gen_stmt_inline(init);
                    self.emit("; ");
                }
                self.gen_expr(condition);
                self.emit(" ");
                self.gen_block(then_block);
                if let Some(else_blk) = else_block {
                    self.emit(" else ");
                    self.gen_block(else_blk);
                }
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::Return { values } => {
                self.emit_indent();
                self.emit("return");
                if !values.is_empty() {
                    self.emit(" ");
                    for (i, value) in values.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.gen_expr(value);
                    }
                }
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::Match {
                scrutinee, arms, ..
            } => {
                // Check for struct matching (regular structs, not enum variants)
                // This needs if/else chains instead of type switches
                let is_struct_match = scrutinee.is_some()
                    && arms.iter().any(|arm| {
                        arm.patterns.iter().any(|p| {
                            if let TypedPatternKind::StructDestructor { pattern_name, .. } = &p.kind
                            {
                                // It's a struct match if pattern_name has no dot (not an enum variant)
                                !pattern_name.contains('.')
                            } else {
                                false
                            }
                        })
                    });

                if is_struct_match {
                    self.gen_struct_match(scrutinee.as_ref().unwrap(), arms);
                    return;
                }

                // Check if this is a type switch (for Soppo enums)
                let is_type_switch = scrutinee.is_some()
                    && arms.iter().any(|arm| {
                        arm.patterns.iter().any(|p| {
                            matches!(
                                &p.kind,
                                TypedPatternKind::Variant {
                                    is_soppo_enum: true,
                                    ..
                                } | TypedPatternKind::Destructor { .. }
                                    | TypedPatternKind::StructDestructor { .. }
                            )
                        })
                    });

                // Check if any arm needs the bound variable (Destructor or StructDestructor patterns)
                let needs_binding = arms.iter().any(|arm| {
                    arm.patterns.iter().any(|p| {
                        matches!(
                            &p.kind,
                            TypedPatternKind::Destructor { .. }
                                | TypedPatternKind::StructDestructor { .. }
                        )
                    })
                });

                self.emit_indent();
                if let Some(expr) = scrutinee {
                    if is_type_switch {
                        if needs_binding {
                            self.emit("switch __v := ");
                        } else {
                            self.emit("switch ");
                        }
                        self.gen_expr(expr);
                        self.emit(".(type) {\n");
                    } else {
                        self.emit("switch ");
                        self.gen_expr(expr);
                        self.emit(" {\n");
                    }
                } else {
                    self.emit("switch {\n");
                }
                // Check if any arm is a default case
                let has_default = arms.iter().any(|arm| {
                    arm.patterns.len() == 1
                        && matches!(arm.patterns[0].kind, TypedPatternKind::Default)
                });

                // Check if all arms diverge (return, panic, etc.)
                let all_arms_diverge = arms.iter().all(|arm| {
                    arm.body
                        .stmts
                        .last()
                        .map(Self::check_stmt_diverges)
                        .unwrap_or(false)
                });

                for arm in arms {
                    self.gen_arm_with_mode(arm, is_type_switch);
                }
                self.emit_indent();
                self.emit("}\n");

                // For type switches without default where all arms diverge,
                // add panic("unreachable") for Go compiler
                if is_type_switch && !has_default && all_arms_diverge {
                    self.emit_indent();
                    self.emit("panic(\"unreachable\")\n");
                }
            }

            TypedStmtKind::Send { channel, value } => {
                self.emit_indent();
                self.gen_expr(channel);
                self.emit(" <- ");
                self.gen_expr(value);
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::Select { cases } => {
                self.emit_indent();
                self.emit("select {\n");
                for case in cases {
                    self.gen_select_case(case);
                }
                self.emit_indent();
                self.emit("}\n");
            }

            TypedStmtKind::Go(expr) => {
                self.emit_indent();
                self.emit("go ");
                self.gen_expr(expr);
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::DeferStmt(expr) => {
                self.emit_indent();
                self.emit("defer ");
                self.gen_expr(expr);
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::Break => {
                self.emit_indent();
                self.emit("break");
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::Continue => {
                self.emit_indent();
                self.emit("continue");
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::Expr(expr) => {
                self.emit_indent();
                self.gen_expr(expr);
                self.emit_stmt_end(stmt_line);
            }

            TypedStmtKind::TryStmt {
                stmt,
                error_name,
                handler,
                discard_count,
                discard_types,
                ..
            } => {
                // Generate the try operator expansion
                self.gen_try_stmt(stmt, error_name, handler, *discard_count, discard_types);
            }

            TypedStmtKind::LocalTypeDecl(type_decl) => {
                self.gen_type_decl(type_decl);
            }
        }
    }

    /// Generate a statement inline (without newline at end)
    fn gen_stmt_inline(&mut self, stmt: &TypedStmt) {
        match &stmt.kind {
            TypedStmtKind::Decl { ident, value, .. } => {
                self.emit(&ident.name);
                self.emit(" := ");
                // Special handling for type assertions on enum variants
                // Go type assertions return structs directly, but we need
                // to compare to nil, so wrap in a closure returning pointer
                if let TypedExprKind::TypeAssert {
                    expr, target_ty, ..
                } = &value.kind
                {
                    let type_name = type_to_go_string(target_ty).replace('.', "_");
                    self.emit(format!("func() *{} {{ if _v, _ok := ", type_name));
                    self.gen_expr(expr);
                    self.emit(format!(
                        ".({}); _ok {{ return &_v }}; return nil }}()",
                        type_name
                    ));
                } else {
                    self.gen_expr(value);
                }
            }
            TypedStmtKind::MultiDecl { idents, values, .. } => {
                for (i, ident) in idents.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.emit(&ident.name);
                }
                self.emit(" := ");
                for (i, val) in values.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(val);
                }
            }
            TypedStmtKind::Assign { target, value } => {
                self.gen_expr(target);
                self.emit(" = ");
                self.gen_expr(value);
            }
            TypedStmtKind::IncDec { target, is_inc } => {
                self.gen_expr(target);
                self.emit(if *is_inc { "++" } else { "--" });
            }
            TypedStmtKind::CompoundAssign { target, op, value } => {
                self.gen_expr(target);
                self.emit(" ");
                self.emit(match op {
                    crate::syntax::AssignOp::Add => "+=",
                    crate::syntax::AssignOp::Sub => "-=",
                    crate::syntax::AssignOp::Mul => "*=",
                    crate::syntax::AssignOp::Div => "/=",
                    crate::syntax::AssignOp::Mod => "%=",
                    crate::syntax::AssignOp::BitAnd => "&=",
                    crate::syntax::AssignOp::BitOr => "|=",
                    crate::syntax::AssignOp::BitXor => "^=",
                    crate::syntax::AssignOp::Shl => "<<=",
                    crate::syntax::AssignOp::Shr => ">>=",
                });
                self.emit(" ");
                self.gen_expr(value);
            }
            TypedStmtKind::If {
                init,
                condition,
                then_block,
                else_block,
            } => {
                self.emit("if ");
                if let Some(init) = init {
                    self.gen_stmt_inline(init);
                    self.emit("; ");
                }
                self.gen_expr(condition);
                self.emit(" ");
                self.gen_block(then_block);
                if let Some(else_blk) = else_block {
                    self.emit(" else ");
                    self.gen_block(else_blk);
                }
            }
            _ => {
                // For other statements, generate normally but strip trailing newline
                let saved = std::mem::take(&mut self.output);
                self.gen_stmt(stmt);
                let generated = std::mem::replace(&mut self.output, saved);
                self.output.push_str(generated.trim_end());
            }
        }
    }

    /// Generate a block from TypedBlock
    pub(crate) fn gen_block(&mut self, block: &TypedBlock) {
        self.emit("{\n");
        self.indent();

        let mut prev_end_line = 0;
        for stmt in &block.stmts {
            // Preserve blank lines between statements
            if prev_end_line > 0 && stmt.span.start.line > prev_end_line + 1 {
                self.emit("\n");
            }
            self.gen_stmt(stmt);
            prev_end_line = stmt.span.end.line;
        }

        self.dedent();
        self.emit_indent();
        self.emit("}");
    }

    /// Check if a typed statement diverges (never falls through)
    fn check_stmt_diverges(stmt: &TypedStmt) -> bool {
        match &stmt.kind {
            TypedStmtKind::Return { .. } => true,
            TypedStmtKind::Break | TypedStmtKind::Continue => true,
            TypedStmtKind::Expr(expr) => {
                // Check for panic() call
                if let TypedExprKind::Call { func, .. } = &expr.kind
                    && let TypedExprKind::Ident(name) = &func.kind
                {
                    return name == "panic";
                }
                false
            }
            _ => false,
        }
    }

    /// Generate a match arm with mode (type switch vs value switch)
    fn gen_arm_with_mode(&mut self, arm: &TypedArm, _is_type_switch: bool) {
        use crate::syntax::Literal;
        use crate::types::ast::TypedPatternKind;

        // Check if this is a default case first
        if arm.patterns.len() == 1
            && let TypedPatternKind::Default = &arm.patterns[0].kind
        {
            self.emit_indent();
            self.emit("default:\n");
            self.indent();
            for stmt in &arm.body.stmts {
                self.gen_stmt(stmt);
            }
            self.dedent();
            return;
        }

        self.emit_indent();
        self.emit("case ");

        for (i, pattern) in arm.patterns.iter().enumerate() {
            if i > 0 {
                self.emit(", ");
            }
            match &pattern.kind {
                TypedPatternKind::Default => {
                    // Should not reach here - handled above
                }
                TypedPatternKind::Variant {
                    variant_name,
                    type_args,
                    is_soppo_enum,
                    ..
                } => {
                    if *is_soppo_enum {
                        // Use explicit type_args, or infer from matched_ty
                        let effective_args = if type_args.is_empty() {
                            if let Type::Con { args, .. } = &pattern.matched_ty {
                                args.clone()
                            } else {
                                vec![]
                            }
                        } else {
                            type_args.clone()
                        };
                        // Convert enum pattern: Type.Variant -> Type_Variant
                        // or pkg.Type.Variant -> pkg.Type_Variant
                        let converted =
                            self.convert_enum_pattern_name(variant_name, &effective_args);
                        self.emit(&converted);
                    } else {
                        // Go constant
                        self.emit(variant_name);
                    }
                }
                TypedPatternKind::Literal(lit) => {
                    // Format literal properly
                    match lit {
                        Literal::Integer(val, _) => self.emit(val.to_string()),
                        Literal::String(s) => self.emit(format!("\"{}\"", s)),
                        Literal::Bool(b) => self.emit(if *b { "true" } else { "false" }),
                        Literal::Nil => self.emit("nil"),
                    }
                }
                TypedPatternKind::Guard(expr) => {
                    self.gen_expr(expr);
                }
                TypedPatternKind::Destructor {
                    variant_name,
                    type_args,
                    binding,
                    ..
                } => {
                    // Use explicit type_args, or infer from matched_ty
                    let effective_args = if type_args.is_empty() {
                        if let Type::Con { args, .. } = &pattern.matched_ty {
                            args.clone()
                        } else {
                            vec![]
                        }
                    } else {
                        type_args.clone()
                    };
                    // Type switch with binding: case Type_Variant
                    let converted = self.convert_enum_pattern_name(variant_name, &effective_args);
                    self.emit(&converted);
                    // Binding variable will be created in the body
                    let _ = binding; // We'll handle binding in body generation
                }
                TypedPatternKind::StructDestructor {
                    pattern_name,
                    type_args,
                    ..
                } => {
                    // Use explicit type_args, or infer from matched_ty
                    let effective_args = if type_args.is_empty() {
                        if let Type::Con { args, .. } = &pattern.matched_ty {
                            args.clone()
                        } else {
                            vec![]
                        }
                    } else {
                        type_args.clone()
                    };
                    // Type switch with struct pattern - convert Shape.Circle -> Shape_Circle
                    let converted = self.convert_enum_pattern_name(pattern_name, &effective_args);
                    self.emit(&converted);
                }
            }
        }

        self.emit(":\n");
        self.indent();

        // For destructor patterns, emit the binding assignment
        if arm.patterns.len() == 1 {
            match &arm.patterns[0].kind {
                TypedPatternKind::Destructor { binding, .. } => {
                    // For single-value enum variant: value := __v.Value
                    self.emit_indent();
                    self.emit(&binding.name);
                    self.emit(" := __v.Value\n");
                    // Add blank assignment to avoid unused variable warnings
                    self.emit_indent();
                    self.emit("_ = ");
                    self.emit(&binding.name);
                    self.emit("\n");
                }
                TypedPatternKind::StructDestructor { fields, .. } => {
                    use crate::types::ast::TypedFieldPattern;
                    // For struct variant: extract each field binding
                    for (field_name, field_pattern) in fields {
                        if let TypedFieldPattern::Bind(binding, _) = field_pattern {
                            self.emit_indent();
                            self.emit(&binding.name);
                            self.emit(" := __v.");
                            self.emit(field_name);
                            self.emit("\n");
                            // Add blank assignment to avoid unused variable warnings
                            self.emit_indent();
                            self.emit("_ = ");
                            self.emit(&binding.name);
                            self.emit("\n");
                        }
                    }
                }
                _ => {}
            }
        }

        for stmt in &arm.body.stmts {
            self.gen_stmt(stmt);
        }
        self.dedent();
    }

    /// Generate a select case
    fn gen_select_case(&mut self, case: &TypedSelectCase) {
        self.emit_indent();
        match &case.kind {
            TypedSelectCaseKind::Recv { channel, .. } => {
                self.emit("case <-");
                self.gen_expr(channel);
            }
            TypedSelectCaseKind::RecvDecl { ident, channel, .. } => {
                self.emit("case ");
                self.emit(&ident.name);
                self.emit(" := <-");
                self.gen_expr(channel);
            }
            TypedSelectCaseKind::RecvDeclOk {
                ident,
                ok_ident,
                channel,
                ..
            } => {
                self.emit("case ");
                self.emit(&ident.name);
                self.emit(", ");
                self.emit(&ok_ident.name);
                self.emit(" := <-");
                self.gen_expr(channel);
            }
            TypedSelectCaseKind::Send { channel, value } => {
                self.emit("case ");
                self.gen_expr(channel);
                self.emit(" <- ");
                self.gen_expr(value);
            }
            TypedSelectCaseKind::Default => {
                self.emit("default");
            }
        }
        self.emit(":\n");
        self.indent();
        for stmt in &case.body.stmts {
            self.gen_stmt(stmt);
        }
        self.dedent();
    }

    /// Generate try statement expansion
    fn gen_try_stmt(
        &mut self,
        stmt: &TypedStmt,
        error_name: &Option<Ident>,
        handler: &Option<TypedBlock>,
        discard_count: usize,
        _discard_types: &[Type],
    ) {
        // Generate the inner statement and capture error
        let err_var = self.fresh_error_var();

        // For assignments (not declarations), we need to declare the error variable first
        // because Go doesn't allow mixing := and = in the same statement
        let needs_err_decl = matches!(
            &stmt.kind,
            TypedStmtKind::Assign { .. } | TypedStmtKind::MultiAssign { .. }
        );

        if needs_err_decl {
            self.emit_indent();
            self.emit(format!("var {} error\n", err_var));
        }

        // Emit the inner statement with error variable added
        self.emit_indent();
        self.gen_try_inner_stmt(stmt, &err_var, discard_count);
        self.emit("\n");

        // Generate error check
        self.emit_indent();
        self.emit("if ");
        self.emit(&err_var);
        self.emit(" != nil ");
        if let Some(handler_block) = handler {
            // Named error and handler
            if let Some(ident) = error_name {
                self.emit("{\n");
                self.indent();
                self.emit_indent();
                self.emit(&ident.name);
                self.emit(" := ");
                self.emit(&err_var);
                self.emit("\n");
                for stmt in &handler_block.stmts {
                    self.gen_stmt(stmt);
                }
                self.dedent();
                self.emit_indent();
                self.emit("}\n");
            } else {
                self.gen_block(handler_block);
                self.emit("\n");
            }
        } else {
            // Default: return zero values (+ error if function returns error)
            self.emit("{\n");
            self.indent();

            let return_types = self.current_return_types.clone();
            let returns_error = return_types.last().is_some_and(|ty| {
                let name = type_to_go_string(ty);
                name == "error" || name.ends_with(".error")
            });

            // The error is propagated as-is; the rest need zero values
            let zeroed = if returns_error {
                &return_types[..return_types.len() - 1]
            } else {
                &return_types[..]
            };

            // Types with no literal zero value (enums, interfaces, generic params)
            // get a `var` declaration and Go works the zero value out for us.
            let zero_values: Vec<String> = zeroed
                .iter()
                .enumerate()
                .map(|(i, ty)| {
                    self.zero_value(ty).unwrap_or_else(|| {
                        let name = format!("_zero{}", i);
                        self.emit_indent();
                        self.emit(format!("var {} {}\n", name, type_to_go_string(ty)));
                        name
                    })
                })
                .collect();

            self.emit_indent();
            self.emit("return ");
            self.emit(zero_values.join(", "));

            if returns_error {
                if !zero_values.is_empty() {
                    self.emit(", ");
                }
                self.emit(&err_var);
            }

            self.emit("\n");
            self.dedent();
            self.emit_indent();
            self.emit("}\n");
        }
    }

    /// Generate inner statement with error capture for ? operator
    /// Transforms: x := f() -> x, _err := f()
    /// Transforms: x = f()  -> x, _err = f()
    fn gen_try_inner_stmt(&mut self, stmt: &TypedStmt, err_var: &str, discard_count: usize) {
        match &stmt.kind {
            TypedStmtKind::Decl { ident, value, .. } => {
                // x := f() -> x, _err := f()
                self.emit(format!("{}, {} := ", ident.name, err_var));
                self.gen_expr(value);
            }
            TypedStmtKind::MultiDecl { idents, values, .. } if values.len() == 1 => {
                // x, y := f() -> x, y, _err := f()
                self.emit(
                    idents
                        .iter()
                        .map(|n| n.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                self.emit(format!(", {} := ", err_var));
                self.gen_expr(&values[0]);
            }
            TypedStmtKind::Assign { target, value } => {
                // x = f() -> x, _err = f()
                self.gen_expr(target);
                self.emit(format!(", {} = ", err_var));
                self.gen_expr(value);
            }
            TypedStmtKind::MultiAssign { targets, values } if values.len() == 1 => {
                // x, y = f() -> x, y, _err = f()
                for (i, target) in targets.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(target);
                }
                self.emit(format!(", {} = ", err_var));
                self.gen_expr(&values[0]);
            }
            TypedStmtKind::Expr(expr) => {
                // f() -> _err := f() for error-only returns
                // f() -> _, _err := f() for (T, error) returns
                // f() -> _, _, _err := f() for (T, U, error) returns
                if discard_count > 0 {
                    let blanks = vec!["_"; discard_count].join(", ");
                    self.emit(format!("{}, {} := ", blanks, err_var));
                } else {
                    self.emit(format!("{} := ", err_var));
                }
                self.gen_expr(expr);
            }
            _ => {
                // Fallback - shouldn't happen after type checking
                self.emit("/* unsupported try statement */");
            }
        }
    }

    /// Generate struct matching with if/else chains
    fn gen_struct_match(&mut self, scrutinee: &TypedExpr, arms: &[TypedArm]) {
        let mut first_arm = true;

        for arm in arms {
            let is_default = arm
                .patterns
                .iter()
                .any(|p| matches!(&p.kind, TypedPatternKind::Default));

            self.emit_indent();
            if is_default {
                if first_arm {
                    self.emit("{\n");
                } else {
                    self.emit("} else {\n");
                }
            } else if let Some(pattern) = arm.patterns.first()
                && let TypedPatternKind::StructDestructor { fields, .. } = &pattern.kind
            {
                // Collect literal conditions
                let conditions: Vec<_> = fields
                    .iter()
                    .filter_map(|(field_name, field_pattern)| {
                        if let TypedFieldPattern::Literal(lit) = field_pattern {
                            Some((field_name.clone(), lit.clone()))
                        } else {
                            None
                        }
                    })
                    .collect();

                if first_arm {
                    self.emit("if ");
                } else {
                    self.emit("} else if ");
                }

                // Generate condition from literal patterns
                if conditions.is_empty() {
                    // No literal conditions - this matches anything (like a default)
                    self.emit("true");
                } else {
                    for (i, (field_name, lit)) in conditions.iter().enumerate() {
                        if i > 0 {
                            self.emit(" && ");
                        }
                        self.gen_expr(scrutinee);
                        self.emit(format!(".{} == ", field_name));
                        match lit {
                            Literal::Integer(n, fmt) => match fmt {
                                IntFormat::Decimal => self.emit(format!("{}", n)),
                                IntFormat::Octal => self.emit(format!("0o{:o}", n)),
                                IntFormat::Hex => self.emit(format!("0x{:x}", n)),
                                IntFormat::Binary => self.emit(format!("0b{:b}", n)),
                            },
                            Literal::String(s) => self.emit(format!("\"{}\"", s)),
                            Literal::Bool(b) => self.emit(format!("{}", b)),
                            Literal::Nil => self.emit("nil"),
                        }
                    }
                }

                self.emit(" {\n");
            }

            self.indent();

            // Extract bindings
            if let Some(pattern) = arm.patterns.first()
                && let TypedPatternKind::StructDestructor { fields, .. } = &pattern.kind
            {
                for (field_name, field_pattern) in fields {
                    if let TypedFieldPattern::Bind(binding_ident, _) = field_pattern {
                        self.emit_indent();
                        self.emit(format!("{} := ", binding_ident.name));
                        self.gen_expr(scrutinee);
                        self.emit(format!(".{}\n", field_name));
                        self.emit_indent();
                        self.emit(format!("_ = {}\n", binding_ident.name));
                    }
                }
            }

            // Emit arm body
            for arm_stmt in &arm.body.stmts {
                self.gen_stmt(arm_stmt);
            }

            self.dedent();
            first_arm = false;
        }

        self.emit_indent();
        self.emit("}\n");
    }

    /// Convert an enum pattern name to Go format
    /// - `Type.Variant` → `Type_Variant`
    /// - `pkg.Type.Variant` → `pkg.Type_Variant`
    pub(crate) fn convert_enum_pattern_name(&self, name: &str, type_args: &[Type]) -> String {
        let type_params_str = if type_args.is_empty() {
            String::new()
        } else {
            let args: Vec<String> = type_args.iter().map(type_to_go_string).collect();
            format!("[{}]", args.join(", "))
        };

        let parts: Vec<&str> = name.split('.').collect();
        match parts.len() {
            0 | 1 => name.to_string(),
            2 => {
                // Type.Variant → Type_Variant
                format!("{}_{}{}", parts[0], parts[1], type_params_str)
            }
            _ => {
                // pkg.Type.Variant or pkg.subpkg.Type.Variant
                // Keep everything except last two parts as prefix
                let prefix = parts[..parts.len() - 2].join(".");
                let type_name = parts[parts.len() - 2];
                let variant = parts[parts.len() - 1];
                format!("{}.{}_{}{}", prefix, type_name, variant, type_params_str)
            }
        }
    }
}
