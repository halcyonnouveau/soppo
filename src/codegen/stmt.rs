use super::Codegen;
use crate::syntax::{
    Arm, Expr, ExprKind, FieldPattern, IntFormat, Literal, PatternKind, SelectCaseKind, Stmt,
    StmtKind,
};

impl Codegen {
    /// Emit a statement ending with optional trailing comment and newline
    pub(crate) fn emit_stmt_end(&mut self, line: usize) {
        self.emit_trailing_comment(line);
        self.emit("\n");
    }

    /// Generate a statement inline (no indent, no newline) - for C-style for loop parts
    pub(crate) fn gen_stmt_inline(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Decl { ident: name, value } => {
                self.emit(format!("{} := ", name));
                self.gen_expr(value);
            }
            StmtKind::Assign { target, value } => {
                self.gen_expr(target);
                self.emit(" = ");
                self.gen_expr(value);
            }
            StmtKind::IncDec { target, is_inc } => {
                self.gen_expr(target);
                self.emit(if *is_inc { "++" } else { "--" });
            }
            StmtKind::CompoundAssign { target, op, value } => {
                let op_str = self.go_assign_op(op).to_string();
                self.gen_expr(target);
                self.emit(" ");
                self.emit(&op_str);
                self.emit(" ");
                self.gen_expr(value);
            }
            StmtKind::Expr(expr) => {
                self.gen_expr(expr);
            }
            // For other statement types, fall back to full generation (shouldn't happen in for loop)
            _ => self.gen_stmt(stmt),
        }
    }

    /// Generate a statement
    pub(crate) fn gen_stmt(&mut self, stmt: &Stmt) {
        // Emit any comments that appear before this statement
        self.emit_comments_before(stmt.span.byte_start, stmt.span.start.line);

        // Track the statement's line for trailing comments
        let stmt_line = stmt.span.start.line;

        match &stmt.kind {
            StmtKind::Decl { ident: name, value } => {
                self.emit_indent();
                self.emit(format!("{} := ", name));
                self.gen_expr(value);
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::MultiDecl {
                ident: names,
                values,
            } => {
                self.emit_indent();
                self.emit(
                    names
                        .iter()
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                self.emit(" := ");

                // Check for comma-ok idiom: v, ok := expr
                // For these expressions, generate native Go comma-ok syntax
                if names.len() == 2
                    && values.len() == 1
                    && let Some(raw) = self.gen_comma_ok_expr(&values[0])
                {
                    self.emit(&raw);
                    self.emit_stmt_end(stmt_line);
                    return;
                }

                for (i, val) in values.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(val);
                }
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::VarDecl {
                ident: name,
                ty,
                value,
            } => {
                self.emit_indent();
                match (ty, value) {
                    (Some(t), Some(expr)) => {
                        // var x type = value
                        self.emit(format!("var {} {} = ", name, self.go_type(&t.name)));
                        self.gen_expr(expr);
                    }
                    (Some(t), None) => {
                        // var x type (zero value)
                        self.emit(format!("var {} {}", name, self.go_type(&t.name)));
                    }
                    (None, Some(expr)) => {
                        // var x = value (type inference)
                        self.emit(format!("var {} = ", name));
                        self.gen_expr(expr);
                    }
                    // INVARIANT: type checker ensures var has type or value
                    (None, None) => unreachable!("var declaration without type or value"),
                }
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::ConstDecl {
                ident: name,
                ty,
                value,
            } => {
                self.emit_indent();
                if let Some(t) = ty {
                    // const x type = value
                    self.emit(format!("const {} {} = ", name, self.go_type(&t.name)));
                } else {
                    // const x = value (type inference)
                    self.emit(format!("const {} = ", name));
                }
                self.gen_expr(value);
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::MultiVarDecl {
                ident: names,
                ty,
                values,
            } => {
                self.emit_indent();
                let names_str = names
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                if values.is_empty() {
                    // var a, b, c type (zero values)
                    let ty = ty.as_ref().expect("MultiVarDecl without values needs type");
                    self.emit(format!("var {} {}", names_str, self.go_type(&ty.name)));
                } else if let Some(t) = ty {
                    // var a, b type = expr1, expr2
                    self.emit(format!("var {} {} = ", names_str, self.go_type(&t.name)));
                    for (i, val) in values.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.gen_expr(val);
                    }
                } else {
                    // var a, b = expr1, expr2
                    self.emit(format!("var {} = ", names_str));
                    for (i, val) in values.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.gen_expr(val);
                    }
                }
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::MultiConstDecl {
                idents: names,
                ty,
                values,
            } => {
                self.emit_indent();
                let names_str = names
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                if let Some(t) = ty {
                    // const a, b type = expr1, expr2
                    self.emit(format!("const {} {} = ", names_str, self.go_type(&t.name)));
                } else {
                    // const a, b = expr1, expr2
                    self.emit(format!("const {} = ", names_str));
                }
                for (i, val) in values.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(val);
                }
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::Assign { target, value } => {
                self.emit_indent();
                self.gen_expr(target);
                self.emit(" = ");
                self.gen_expr(value);
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::MultiAssign { targets, values } => {
                self.emit_indent();
                for (i, target) in targets.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(target);
                }
                self.emit(" = ");
                for (i, val) in values.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(val);
                }
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::CompoundAssign { target, op, value } => {
                self.emit_indent();
                self.gen_expr(target);
                self.emit(format!(" {} ", self.go_assign_op(op)));
                self.gen_expr(value);
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::IncDec { target, is_inc } => {
                self.emit_indent();
                self.gen_expr(target);
                if *is_inc {
                    self.emit("++");
                } else {
                    self.emit("--");
                }
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::For { condition, body } => {
                self.emit_indent();
                self.emit("for ");
                self.gen_expr(condition);
                self.emit(" ");
                self.gen_block(body);
                self.output.push('\n');
            }

            StmtKind::ForCStyle {
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
                    self.output.push('\n');
                } else {
                    // Generate init statement (without newline/indent)
                    if let Some(init_stmt) = init {
                        self.gen_stmt_inline(init_stmt);
                    }
                    self.emit("; ");

                    // Generate condition
                    if let Some(cond) = condition {
                        self.gen_expr(cond);
                    }
                    self.emit("; ");

                    // Generate post statement (without newline/indent)
                    if let Some(post_stmt) = post {
                        self.gen_stmt_inline(post_stmt);
                    }

                    self.emit(" ");
                    self.gen_block(body);
                    self.output.push('\n');
                }
            }

            StmtKind::ForRange {
                key,
                value,
                collection,
                body,
            } => {
                self.emit_indent();
                self.emit("for ");
                self.emit(key);
                if let Some(val) = value {
                    self.emit(", ");
                    self.emit(val);
                }
                self.emit(" := range ");
                self.gen_expr(collection);
                self.emit(" ");
                self.gen_block(body);
                self.output.push('\n');
            }

            StmtKind::If {
                init,
                condition,
                then_block,
                else_block,
            } => {
                self.emit_indent();
                self.emit("if ");
                // Generate init statement if present: if x := expr; cond { }
                if let Some(init_stmt) = init {
                    // Generate the init statement inline (without indent/newline)
                    match &init_stmt.kind {
                        StmtKind::Decl { ident: name, value } => {
                            self.emit(name);
                            self.emit(" := ");
                            self.gen_expr(value);
                        }
                        StmtKind::MultiDecl {
                            ident: names,
                            values,
                        } => {
                            self.emit(
                                names
                                    .iter()
                                    .map(|n| n.to_string())
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            );
                            self.emit(" := ");
                            for (i, v) in values.iter().enumerate() {
                                if i > 0 {
                                    self.emit(", ");
                                }
                                self.gen_expr(v);
                            }
                        }
                        _ => {
                            // For other statement types, just generate them inline
                            // This shouldn't happen in practice
                        }
                    }
                    self.emit("; ");
                }
                self.gen_expr(condition);
                self.emit(" ");
                self.gen_block(then_block);

                if let Some(else_block) = else_block {
                    self.emit(" else ");
                    self.gen_block(else_block);
                }
                self.output.push('\n');
            }

            StmtKind::Return { values } => {
                self.emit_indent();
                if values.is_empty() {
                    self.emit("return");
                } else {
                    self.emit("return ");
                    for (i, expr) in values.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.gen_expr(expr);
                    }
                }
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::Match { scrutinee, arms } => self.gen_match(scrutinee, arms),

            StmtKind::Send { channel, value } => {
                self.emit_indent();
                self.gen_expr(channel);
                self.emit(" <- ");
                self.gen_expr(value);
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::Select { cases } => {
                self.emit_indent();
                self.emit("select {\n");

                for case in cases {
                    self.emit_indent();

                    match &case.kind {
                        SelectCaseKind::Recv { channel } => {
                            self.emit("case <-");
                            self.gen_expr(channel);
                            self.emit(":\n");
                        }
                        SelectCaseKind::RecvDecl {
                            ident: name,
                            channel,
                        } => {
                            self.emit(format!("case {} := <-", name));
                            self.gen_expr(channel);
                            self.emit(":\n");
                        }
                        SelectCaseKind::RecvDeclOk {
                            ident: name,
                            ok_ident: ok_name,
                            channel,
                        } => {
                            self.emit(format!("case {}, {} := <-", name, ok_name));
                            self.gen_expr(channel);
                            self.emit(":\n");
                        }
                        SelectCaseKind::Send { channel, value } => {
                            self.emit("case ");
                            self.gen_expr(channel);
                            self.emit(" <- ");
                            self.gen_expr(value);
                            self.emit(":\n");
                        }
                        SelectCaseKind::Default => {
                            self.emit("default:\n");
                        }
                    }

                    self.indent();
                    for stmt in &case.body.stmts {
                        self.gen_stmt(stmt);
                    }
                    self.dedent();
                }

                self.emit_indent();
                self.emit("}\n");
            }

            StmtKind::Go(expr) => {
                self.emit_indent();
                self.emit("go ");
                self.gen_expr(expr);
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::DeferStmt(expr) => {
                self.emit_indent();
                self.emit("defer ");
                self.gen_expr(expr);
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::Break => {
                self.emit_indent();
                self.emit("break");
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::Continue => {
                self.emit_indent();
                self.emit("continue");
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::Expr(expr) => {
                self.emit_indent();
                self.gen_expr(expr);
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::TryStmt {
                stmt: inner_stmt,
                error_name,
                handler,
                discard_count,
                ..
            } => {
                // Generate the inner statement with error capture
                let err_var = self.fresh_error_var();
                let discard = discard_count.get();

                // For assignments (not declarations), we need to declare the error variable first
                // because Go doesn't allow mixing := and = in the same statement
                let needs_err_decl = matches!(
                    &inner_stmt.kind,
                    StmtKind::Assign { .. } | StmtKind::MultiAssign { .. }
                );

                if needs_err_decl {
                    self.emit_indent();
                    self.emit(format!("var {} error\n", err_var));
                }

                // Emit the inner statement with error variable added
                self.emit_indent();
                self.gen_try_inner_stmt(inner_stmt, &err_var, discard);
                self.emit("\n");

                // Generate error check: if err != nil { ... }
                self.emit_indent();
                self.emit(format!("if {} != nil ", err_var));
                self.emit("{\n");
                self.indent();

                if let Some(block) = handler {
                    // Custom handler block
                    if let Some(name) = error_name {
                        // Bind error to the named variable
                        self.emit_indent();
                        self.emit(format!("{} := {}\n", name, err_var));
                    }
                    // Emit handler body
                    for handler_stmt in &block.stmts {
                        self.gen_stmt(handler_stmt);
                    }
                } else {
                    // Default: return zero values (+ error if function returns error)
                    self.emit_indent();
                    self.emit("return ");

                    let return_types = self.current_return_types.clone();
                    let returns_error = return_types
                        .last()
                        .is_some_and(|ty| ty == "error" || ty.ends_with(".error"));

                    if returns_error {
                        // Generate zero values for all return types except last (error)
                        let zero_values: Vec<String> = return_types
                            .iter()
                            .take(return_types.len().saturating_sub(1))
                            .map(|ty| self.zero_value(ty))
                            .collect();

                        self.emit(zero_values.join(", "));

                        // Add error variable
                        if !zero_values.is_empty() {
                            self.emit(", ");
                        }
                        self.emit(&err_var);
                    } else {
                        // No error return - just return zero values for all return types
                        let zero_values: Vec<String> =
                            return_types.iter().map(|ty| self.zero_value(ty)).collect();

                        self.emit(zero_values.join(", "));
                    }
                    self.emit("\n");
                }

                self.dedent();
                self.emit_indent();
                self.emit("}\n");
            }

            StmtKind::LocalTypeDecl(type_decl) => {
                // Generate local type declaration
                // Reuse the top-level type declaration generator
                self.gen_type_decl(type_decl);
            }
        }
    }

    /// Generate inner statement with error capture for ? operator
    /// Transforms: x := f() -> x, _err := f()
    /// Transforms: x = f()  -> x, _err = f()
    /// Transforms: f()      -> _, _err := f() (with appropriate number of _ for multi-return)
    fn gen_try_inner_stmt(&mut self, stmt: &Stmt, err_var: &str, discard_count: usize) {
        match &stmt.kind {
            StmtKind::Decl { ident: name, value } => {
                // x := f() -> x, _err := f()
                self.emit(format!("{}, {} := ", name, err_var));
                self.gen_expr(value);
            }
            StmtKind::MultiDecl {
                ident: names,
                values,
            } if values.len() == 1 => {
                // x, y := f() -> x, y, _err := f()
                self.emit(
                    names
                        .iter()
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                self.emit(format!(", {} := ", err_var));
                self.gen_expr(&values[0]);
            }
            StmtKind::Assign { target, value } => {
                // x = f() -> x, _err = f()
                self.gen_expr(target);
                self.emit(format!(", {} = ", err_var));
                self.gen_expr(value);
            }
            StmtKind::MultiAssign { targets, values } if values.len() == 1 => {
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
            StmtKind::Expr(expr) => {
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

    /// Generate code for a match statement
    fn gen_match(&mut self, scrutinee: &Option<Expr>, arms: &[Arm]) {
        // Expression-less match: `match { case x > 0: ... }` -> `switch { case x > 0: ... }`
        let is_expression_less = scrutinee.is_none();

        // Check if this is struct matching (regular structs, not enum variants)
        // Struct patterns have names without dots (e.g., "Point"), enum variants have dots (e.g., "Shape.Circle")
        let is_struct_match = !is_expression_less
            && arms.iter().any(|arm| {
                arm.patterns.iter().any(|p| {
                    if let PatternKind::StructDestructor { name, .. } = &p.kind {
                        !name.contains('.')
                    } else {
                        false
                    }
                })
            });

        // Handle struct matching with if/else chains
        if is_struct_match {
            let scrutinee_expr = scrutinee.as_ref().unwrap();
            let mut first_arm = true;

            for arm in arms {
                let is_default = arm
                    .patterns
                    .iter()
                    .any(|p| matches!(&p.kind, PatternKind::Default));

                self.emit_indent();
                if is_default {
                    if first_arm {
                        self.emit("{\n");
                    } else {
                        self.emit("} else {\n");
                    }
                } else if let Some(pattern) = arm.patterns.first()
                    && let PatternKind::StructDestructor { fields, .. } = &pattern.kind
                {
                    // Collect literal conditions
                    let conditions: Vec<_> = fields
                        .iter()
                        .filter_map(|(field_name, field_pattern)| {
                            if let FieldPattern::Literal(lit) = field_pattern {
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
                            self.gen_expr(scrutinee_expr);
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
                    && let PatternKind::StructDestructor { fields, .. } = &pattern.kind
                {
                    for (field_name, field_pattern) in fields {
                        if let FieldPattern::Bind(binding_name) = field_pattern {
                            self.emit_indent();
                            self.emit(format!("{} := ", binding_name));
                            self.gen_expr(scrutinee_expr);
                            self.emit(format!(".{}\n", field_name));
                            self.emit_indent();
                            self.emit(format!("_ = {}\n", binding_name));
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
            return;
        }

        self.emit_indent();

        // Check if this is a type switch or value switch
        // Type switch is needed for soppo enums (which are interface types)
        // Value switch is used for Go constants
        let is_type_switch = !is_expression_less
            && arms.iter().any(|arm| {
                arm.patterns.iter().any(|p| match &p.kind {
                    PatternKind::Variant(_, is_soppo_enum) => is_soppo_enum.get(),
                    PatternKind::Destructor { .. } | PatternKind::StructDestructor { .. } => true,
                    _ => false,
                })
            });

        // Check if any arm needs the bound variable (Destructor or StructDestructor patterns)
        let needs_binding = arms.iter().any(|arm| {
            arm.patterns.iter().any(|p| {
                matches!(
                    &p.kind,
                    PatternKind::Destructor { .. } | PatternKind::StructDestructor { .. }
                )
            })
        });

        if is_expression_less {
            // Expression-less match: `switch { ... }`
            self.emit("switch {\n");
        } else if is_type_switch {
            if needs_binding {
                self.emit("switch __v := ");
            } else {
                self.emit("switch ");
            }
            self.gen_expr(scrutinee.as_ref().unwrap());
            self.emit(".(type) {\n");
        } else {
            self.emit("switch ");
            self.gen_expr(scrutinee.as_ref().unwrap());
            self.emit(" {\n");
        }

        for arm in arms {
            self.emit_indent();

            // Check if this arm is a default case
            let is_default = arm
                .patterns
                .iter()
                .any(|p| matches!(&p.kind, PatternKind::Default));

            if is_default {
                self.emit("default:\n");
            } else {
                self.emit("case ");

                // Emit comma-separated patterns
                for (i, pattern) in arm.patterns.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_pattern(pattern);
                }

                self.emit(":\n");
            }
            self.indent();

            // Extract pattern bindings for destructor patterns (from first pattern only)
            if let Some(first_pattern) = arm.patterns.first() {
                if let PatternKind::Destructor { binding, .. } = &first_pattern.kind {
                    // __v is already the concrete type from the switch statement
                    self.emit_indent();
                    self.emit(format!("{} := __v.Value\n", binding));
                    // Add blank assignment to avoid unused variable warnings
                    self.emit_indent();
                    self.emit(format!("_ = {}\n", binding));
                }

                // Extract pattern bindings for struct destructor patterns
                if let PatternKind::StructDestructor { fields, .. } = &first_pattern.kind {
                    // __v is already the concrete type from the switch statement
                    for (field_name, field_pattern) in fields {
                        // Only emit bindings, not literal matches
                        if let FieldPattern::Bind(binding_name) = field_pattern {
                            self.emit_indent();
                            self.emit(format!("{} := __v.{}\n", binding_name, field_name));
                            // Add blank assignment to avoid unused variable warnings
                            self.emit_indent();
                            self.emit(format!("_ = {}\n", binding_name));
                        }
                    }
                }
            }

            // Emit arm body statements
            for arm_stmt in &arm.body.stmts {
                self.gen_stmt(arm_stmt);
            }

            self.dedent();
        }

        // Check if there's a default case
        let has_default = arms.iter().any(|arm| {
            arm.patterns
                .iter()
                .any(|p| matches!(&p.kind, PatternKind::Default))
        });

        // Check if all arms diverge (end with return/break/continue/panic)
        let all_arms_diverge = arms.iter().all(|arm| {
            arm.body
                .stmts
                .last()
                .map(Self::stmt_diverges)
                .unwrap_or(false)
        });

        self.emit_indent();
        self.emit("}\n");

        // For type switches without default where all arms return,
        // add panic("unreachable") for Go compiler (Go doesn't know the switch is exhaustive)
        if is_type_switch && !has_default && all_arms_diverge {
            self.emit_indent();
            self.emit("panic(\"unreachable\")\n");
        }
    }

    /// Check if a statement diverges (never falls through)
    fn stmt_diverges(stmt: &Stmt) -> bool {
        match &stmt.kind {
            StmtKind::Return { .. } => true,
            StmtKind::Break | StmtKind::Continue => true,
            StmtKind::Expr(expr) => {
                // Check for panic() call
                if let ExprKind::Call { func, .. } = &expr.kind
                    && let ExprKind::Ident(name) = &func.kind
                {
                    return name == "panic";
                }
                false
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{FileId, Parser};

    #[test]
    fn test_gen_let_statement() {
        let source = "func test() int { x := 42\nreturn x }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser.parse_func_decl().unwrap();

        let mut codegen = Codegen::new();
        codegen.gen_func_decl(&func);

        let output = codegen.output();
        assert!(output.contains("x := 42"));
        assert!(output.contains("return x"));
    }
}
