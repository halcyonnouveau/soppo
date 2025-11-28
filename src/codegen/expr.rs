use super::Codegen;
use crate::syntax::{Expr, ExprKind, UnaryOp};

impl Codegen {
    /// Generate an expression
    pub(crate) fn gen_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Integer(n) => {
                self.emit(&n.to_string());
            }

            ExprKind::Float(f) => {
                let s = f.to_string();
                // Ensure we have a decimal point for Go
                if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                    self.emit(&format!("{}.0", s));
                } else {
                    self.emit(&s);
                }
            }

            ExprKind::String(s) => {
                self.emit(&format!("\"{}\"", s));
            }

            ExprKind::Bool(b) => {
                self.emit(if *b { "true" } else { "false" });
            }

            ExprKind::Ident(name) => {
                self.emit(name);
            }

            ExprKind::Binary { op, left, right } => {
                self.emit("(");
                self.gen_expr(left);
                self.emit(&format!(" {} ", self.go_binop(op)));
                self.gen_expr(right);
                self.emit(")");
            }

            ExprKind::Call {
                func,
                type_args,
                args,
            } => {
                // Special handling for make and new built-ins
                if let ExprKind::Ident(name) = &func.kind
                    && (name == "make" || name == "new")
                    && !type_args.is_empty()
                {
                    // make(type, args...) or new(type)
                    self.emit(name);
                    self.emit("(");
                    // Type is first argument
                    self.emit(&type_args[0].name);
                    // Additional arguments
                    for arg in args {
                        self.emit(", ");
                        self.gen_expr(arg);
                    }
                    self.emit(")");
                    return;
                }

                self.gen_expr(func);
                // Emit type arguments if present: func[int, string](args)
                if !type_args.is_empty() {
                    self.emit("[");
                    for (i, ty) in type_args.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.emit(self.go_type(&ty.name));
                    }
                    self.emit("]");
                }
                self.emit("(");
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(arg);
                }
                self.emit(")");
            }

            ExprKind::Field { expr, field, .. } => {
                // Check if this is an enum constructor like Colour.Red
                if let ExprKind::Ident(type_name) = &expr.kind {
                    // Check if it's a registered type (enum)
                    if self.global_state.has_type(type_name) {
                        // Enum values: Colour.Red → ColourRed (var or function)
                        self.emit(&format!("{}{}", type_name, field));
                    } else {
                        // Regular field access (like fmt.Printf)
                        self.emit(type_name);
                        self.emit(".");
                        self.emit(field);
                    }
                } else {
                    // Regular field access on expression
                    self.gen_expr(expr);
                    self.emit(".");
                    self.emit(field);
                }
            }

            ExprKind::Index { expr, index } => {
                self.gen_expr(expr);
                self.emit("[");
                self.gen_expr(index);
                self.emit("]");
            }

            ExprKind::ArrayLit { ty, elements } => {
                // Generate []type{elements} for slices or [size]type{elements} for arrays
                if let Some(ty) = ty {
                    let type_name = &ty.name;
                    if type_name.starts_with("[]") {
                        // Slice literal: []type{elements}
                        self.emit(self.go_type(type_name));
                    } else {
                        // Array literal: [size]type{elements}
                        self.emit("[");
                        self.emit(&elements.len().to_string());
                        self.emit("]");
                        self.emit(self.go_type(type_name));
                    }
                } else {
                    // No type - infer as array with size
                    self.emit("[");
                    self.emit(&elements.len().to_string());
                    self.emit("]");
                }
                self.emit("{");
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(elem);
                }
                self.emit("}");
            }

            ExprKind::StructLit { ty, fields } => {
                // Generate Type{field: value, ...}
                // For enum variants like Shape.Circle, convert to Shape_Circle
                let type_name = if ty.name.contains('.') {
                    ty.name.replace('.', "_")
                } else {
                    ty.name.clone()
                };
                self.emit(self.go_type(&type_name));
                self.emit("{");
                for (i, (field_name, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.emit(field_name);
                    self.emit(": ");
                    self.gen_expr(value);
                }
                self.emit("}");
            }

            ExprKind::MapLit { ty, entries } => {
                // Generate map[K]V{key: value, ...}
                self.emit(&ty.name);
                self.emit("{");
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(key);
                    self.emit(": ");
                    self.gen_expr(value);
                }
                self.emit("}");
            }

            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Neg => {
                    self.emit("(-");
                    self.gen_expr(operand);
                    self.emit(")");
                }
                UnaryOp::Not => {
                    self.emit("(!");
                    self.gen_expr(operand);
                    self.emit(")");
                }
                UnaryOp::Ref => {
                    self.emit("(&");
                    self.gen_expr(operand);
                    self.emit(")");
                }
                UnaryOp::Deref => {
                    self.emit("(*");
                    self.gen_expr(operand);
                    self.emit(")");
                }
                UnaryOp::Recv => {
                    self.emit("(<-");
                    self.gen_expr(operand);
                    self.emit(")");
                }
            },

            ExprKind::FuncLit {
                params,
                return_types,
                body,
            } => {
                self.emit("func(");
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.emit(&param.name);
                    self.emit(" ");
                    self.emit(self.go_type(&param.ty.name));
                }
                self.emit(")");

                // Return types
                if return_types.len() == 1 {
                    self.emit(" ");
                    self.emit(self.go_type(&return_types[0].name));
                } else if return_types.len() > 1 {
                    self.emit(" (");
                    for (i, ty) in return_types.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.emit(self.go_type(&ty.name));
                    }
                    self.emit(")");
                }

                self.emit(" ");
                self.gen_block(body);
            }

            ExprKind::Block(block) => {
                self.gen_block(block);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{FileId, Parser};

    #[test]
    fn test_gen_literals() {
        let source = r#"func test() int { return 42 }"#;
        let mut parser = Parser::new(source, FileId(0));
        let func = parser.parse_func_decl().unwrap();

        let mut codegen = Codegen::new();
        codegen.gen_func_decl(&func);

        assert!(codegen.output().contains("return 42"));
    }

    #[test]
    fn test_gen_binary_ops() {
        let source = "func test() int { return 1 + 2 * 3 }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser.parse_func_decl().unwrap();

        let mut codegen = Codegen::new();
        codegen.gen_func_decl(&func);

        let output = codegen.output();
        assert!(output.contains("(1 + (2 * 3))"));
    }

    #[test]
    fn test_gen_function_call() {
        let source = "func main() int { return add(1, 2) }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser.parse_func_decl().unwrap();

        let mut codegen = Codegen::new();
        codegen.gen_func_decl(&func);

        let output = codegen.output();
        assert!(output.contains("add(1, 2)"));
    }
}
