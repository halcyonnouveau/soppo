use super::Codegen;
use crate::syntax::{PatternKind, SelectCaseKind, Stmt, StmtKind};

impl Codegen {
    /// Emit a statement ending with optional trailing comment and newline
    pub(crate) fn emit_stmt_end(&mut self, line: usize) {
        self.emit_trailing_comment(line);
        self.emit("\n");
    }

    /// Generate a statement
    pub(crate) fn gen_stmt(&mut self, stmt: &Stmt) {
        // Emit any comments that appear before this statement
        self.emit_comments_before(stmt.span.byte_start, stmt.span.start.line);

        // Track the statement's line for trailing comments
        let stmt_line = stmt.span.start.line;

        match &stmt.kind {
            StmtKind::Decl { name, value } => {
                self.emit_indent();
                self.emit(&format!("{} := ", name));
                self.gen_expr(value);
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::MultiDecl { names, values } => {
                self.emit_indent();
                self.emit(&names.join(", "));
                self.emit(" := ");
                for (i, val) in values.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(val);
                }
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::VarDecl { name, ty, value } => {
                self.emit_indent();
                match (ty, value) {
                    (Some(t), Some(expr)) => {
                        // var x type = value
                        self.emit(&format!("var {} {} = ", name, self.go_type(&t.name)));
                        self.gen_expr(expr);
                    }
                    (Some(t), None) => {
                        // var x type (zero value)
                        self.emit(&format!("var {} {}", name, self.go_type(&t.name)));
                    }
                    (None, Some(expr)) => {
                        // var x = value (type inference)
                        self.emit(&format!("var {} = ", name));
                        self.gen_expr(expr);
                    }
                    (None, None) => {
                        // Should be caught by type checker
                        unreachable!("var declaration without type or value")
                    }
                }
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::ConstDecl { name, ty, value } => {
                self.emit_indent();
                if let Some(t) = ty {
                    // const x type = value
                    self.emit(&format!("const {} {} = ", name, self.go_type(&t.name)));
                } else {
                    // const x = value (type inference)
                    self.emit(&format!("const {} = ", name));
                }
                self.gen_expr(value);
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::MultiVarDecl { names, ty, values } => {
                self.emit_indent();
                if values.is_empty() {
                    // var a, b, c type (zero values)
                    let ty = ty.as_ref().expect("MultiVarDecl without values needs type");
                    self.emit(&format!(
                        "var {} {}",
                        names.join(", "),
                        self.go_type(&ty.name)
                    ));
                } else if let Some(t) = ty {
                    // var a, b type = expr1, expr2
                    self.emit(&format!(
                        "var {} {} = ",
                        names.join(", "),
                        self.go_type(&t.name)
                    ));
                    for (i, val) in values.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.gen_expr(val);
                    }
                } else {
                    // var a, b = expr1, expr2
                    self.emit(&format!("var {} = ", names.join(", ")));
                    for (i, val) in values.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.gen_expr(val);
                    }
                }
                self.emit_stmt_end(stmt_line);
            }

            StmtKind::MultiConstDecl { names, ty, values } => {
                self.emit_indent();
                if let Some(t) = ty {
                    // const a, b type = expr1, expr2
                    self.emit(&format!(
                        "const {} {} = ",
                        names.join(", "),
                        self.go_type(&t.name)
                    ));
                } else {
                    // const a, b = expr1, expr2
                    self.emit(&format!("const {} = ", names.join(", ")));
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

            StmtKind::For { condition, body } => {
                self.emit_indent();
                self.emit("for ");
                self.gen_expr(condition);
                self.emit(" ");
                self.gen_block(body);
                self.output.push('\n');
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
                condition,
                then_block,
                else_block,
            } => {
                self.emit_indent();
                self.emit("if ");
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

            StmtKind::Match { scrutinee, arms } => {
                self.emit_indent();

                // Expression-less match: `match { case x > 0: ... }` -> `switch { case x > 0: ... }`
                let is_expression_less = scrutinee.is_none();

                // Check if this is a type switch or value switch
                let is_type_switch = !is_expression_less
                    && arms.iter().any(|arm| {
                        arm.patterns.iter().any(|p| {
                            matches!(
                                &p.kind,
                                PatternKind::Variant(_)
                                    | PatternKind::Destructor { .. }
                                    | PatternKind::StructDestructor { .. }
                            )
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
                            self.emit(&format!("{} := __v.Value\n", binding));
                            // Add blank assignment to avoid unused variable warnings
                            self.emit_indent();
                            self.emit(&format!("_ = {}\n", binding));
                        }

                        // Extract pattern bindings for struct destructor patterns
                        if let PatternKind::StructDestructor { fields, .. } = &first_pattern.kind {
                            // __v is already the concrete type from the switch statement
                            for (field_name, binding_name) in fields {
                                self.emit_indent();
                                self.emit(&format!("{} := __v.{}\n", binding_name, field_name));
                                // Add blank assignment to avoid unused variable warnings
                                self.emit_indent();
                                self.emit(&format!("_ = {}\n", binding_name));
                            }
                        }
                    }

                    // Emit arm body statements
                    for arm_stmt in &arm.body.stmts {
                        self.gen_stmt(arm_stmt);
                    }

                    self.dedent();
                }

                self.emit_indent();
                self.emit("}\n");
            }

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
                        SelectCaseKind::RecvDecl { name, channel } => {
                            self.emit(&format!("case {} := <-", name));
                            self.gen_expr(channel);
                            self.emit(":\n");
                        }
                        SelectCaseKind::RecvDeclOk {
                            name,
                            ok_name,
                            channel,
                        } => {
                            self.emit(&format!("case {}, {} := <-", name, ok_name));
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
