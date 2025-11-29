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

            ExprKind::Rune(r) => {
                self.emit(&format!("'{}'", r));
            }

            ExprKind::StringInterpolation(parts) => {
                // Generate fmt.Sprintf("format", args...)
                self.emit("fmt.Sprintf(\"");

                // Build the format string and collect expressions
                let mut exprs: Vec<&crate::syntax::Expr> = Vec::new();
                for part in parts {
                    match part {
                        crate::syntax::StringPart::Literal(s) => {
                            // Escape % characters for fmt.Sprintf
                            self.emit(&s.replace('%', "%%"));
                        }
                        crate::syntax::StringPart::Expr(expr) => {
                            self.emit("%v");
                            exprs.push(expr);
                        }
                    }
                }
                self.emit("\"");

                // Add the expressions as arguments
                for expr in exprs {
                    self.emit(", ");
                    self.gen_expr(expr);
                }
                self.emit(")");
            }

            ExprKind::Bool(b) => {
                self.emit(if *b { "true" } else { "false" });
            }

            ExprKind::Nil => {
                self.emit("nil");
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
                    for (_, arg) in args {
                        self.emit(", ");
                        self.gen_expr(arg);
                    }
                    self.emit(")");
                    return;
                }

                // Reorder args based on named arguments if needed
                let ordered_args = self.reorder_call_args(func, args);

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
                for (i, arg) in ordered_args.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(arg);
                }
                self.emit(")");
            }

            ExprKind::Field { expr, field, .. } => {
                // Check if this is an enum constructor like Colour.Red or pkg.Status.Active
                if let ExprKind::Ident(type_name) = &expr.kind {
                    // Check if it's a registered local enum type
                    if self.global_state.is_local_enum(type_name) {
                        // Enum values: Colour.Red → ColourRed (var or function)
                        self.emit(&format!("{}{}", type_name, field));
                    } else {
                        // Regular field access (like fmt.Printf)
                        self.emit(type_name);
                        self.emit(".");
                        self.emit(field);
                    }
                } else if let ExprKind::Field {
                    expr: inner_expr,
                    field: type_name,
                    ..
                } = &expr.kind
                {
                    // Check for cross-package enum: pkg.Type.Variant
                    if let ExprKind::Ident(pkg_name) = &inner_expr.kind {
                        if self.global_state.is_soppo_enum(pkg_name, type_name) {
                            // Cross-package enum: types.Status.Active → types.StatusActive
                            self.emit(&format!("{}.{}{}", pkg_name, type_name, field));
                        } else {
                            // Regular nested field access
                            self.gen_expr(expr);
                            self.emit(".");
                            self.emit(field);
                        }
                    } else {
                        // Regular nested field access
                        self.gen_expr(expr);
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

            ExprKind::Slice {
                expr,
                low,
                high,
                cap,
            } => {
                self.gen_expr(expr);
                self.emit("[");
                if let Some(l) = low {
                    self.gen_expr(l);
                }
                self.emit(":");
                if let Some(h) = high {
                    self.gen_expr(h);
                }
                if let Some(c) = cap {
                    self.emit(":");
                    self.gen_expr(c);
                }
                self.emit("]");
            }

            ExprKind::TypeAssert { expr, ty } => {
                // Type assertion returns a pointer: nil if failed, &value if succeeded
                // Generate: func() *Type { if _v, _ok := expr.(Type); _ok { return &_v }; return nil }()
                let type_name = ty.name.replace('.', "_");
                self.emit("func() *");
                self.emit(&type_name);
                self.emit(" { if _v, _ok := ");
                self.gen_expr(expr);
                self.emit(".(");
                self.emit(&type_name);
                self.emit("); _ok { return &_v }; return nil }()");
            }

            ExprKind::NilAssert { expr } => {
                // Nil assertion is compile-time only - just emit the inner expression
                self.gen_expr(expr);
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
                // For cross-package types like types.User, keep as types.User
                // For cross-package enum variants like pkg.Type.Variant, convert to pkg.Type_Variant
                let type_name = convert_struct_type_name(&ty.name);
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

            ExprKind::AnonStructLit { field_defs, fields } => {
                // Generate struct { Name Type; ... }{Name: value, ...}
                self.emit("struct { ");
                for (i, field) in field_defs.iter().enumerate() {
                    if i > 0 {
                        self.emit("; ");
                    }
                    self.emit(&field.name);
                    self.emit(" ");
                    self.emit(self.go_type(&field.ty.name));
                }
                self.emit(" }{");
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

    /// Reorder function call arguments based on named arguments
    fn reorder_call_args<'a>(
        &self,
        func: &Expr,
        args: &'a [(Option<(String, crate::syntax::Span)>, Expr)],
    ) -> Vec<&'a Expr> {
        // Check if any args are named
        let has_named = args.iter().any(|(name, _)| name.is_some());

        // If no named args, just return all in order
        if !has_named {
            return args.iter().map(|(_, arg)| arg).collect();
        }

        // Look up parameter names (exclude variadic params)
        let param_names: Option<Vec<String>> = if let ExprKind::Ident(func_name) = &func.kind {
            self.global_state.lookup_function(func_name).map(|f| {
                f.params
                    .iter()
                    .filter(|(_, ty)| !ty.to_string().starts_with("variadic"))
                    .map(|(name, _)| name.clone())
                    .collect()
            })
        } else {
            None
        };

        if let Some(param_names) = param_names {
            // Reorder based on parameter names
            // Rules:
            // - Positional args before any named arg fill fixed params in order
            // - Named args fill their named slots
            // - Positional args after a named arg go to variadic
            let mut result: Vec<Option<&Expr>> = vec![None; param_names.len()];
            let mut variadic_args: Vec<&Expr> = Vec::new();
            let mut seen_named = false;
            let mut next_positional_idx = 0;

            for (name, arg) in args {
                match name {
                    Some((n, _)) => {
                        seen_named = true;
                        if let Some(idx) = param_names.iter().position(|p| p == n) {
                            result[idx] = Some(arg);
                        }
                    }
                    None => {
                        if seen_named {
                            // Positional after named goes to variadic
                            variadic_args.push(arg);
                        } else {
                            // Positional before any named fills fixed params
                            if next_positional_idx < param_names.len() {
                                result[next_positional_idx] = Some(arg);
                                next_positional_idx += 1;
                            } else {
                                // Extra positional goes to variadic
                                variadic_args.push(arg);
                            }
                        }
                    }
                }
            }

            // Collect results (type checker already validated all are filled)
            let mut ordered: Vec<&Expr> = result.into_iter().flatten().collect();

            // Add variadic args at the end
            ordered.extend(variadic_args);

            ordered
        } else {
            // Unknown function - just use positional order (type checker would have errored)
            args.iter().map(|(_, arg)| arg).collect()
        }
    }

    /// Generate a comma-ok expression (type assertion, map index, channel receive)
    /// Returns Some(code) if this is a comma-ok expression, None otherwise
    pub(crate) fn gen_comma_ok_expr(&mut self, expr: &Expr) -> Option<String> {
        match &expr.kind {
            // Type assertion: x.(T) -> native Go comma-ok
            ExprKind::TypeAssert { expr: inner, ty } => {
                let mut code = String::new();
                let mut temp = Codegen::new();
                temp.gen_expr(inner);
                code.push_str(temp.output());
                code.push_str(".(");
                code.push_str(&ty.name.replace('.', "_"));
                code.push(')');
                Some(code)
            }

            // Map index and channel receive work natively with comma-ok
            // Just generate the expression normally
            ExprKind::Index { .. } => {
                let mut temp = Codegen::new();
                temp.gen_expr(expr);
                Some(temp.output().to_string())
            }

            ExprKind::Unary {
                op: UnaryOp::Recv, ..
            } => {
                let mut temp = Codegen::new();
                temp.gen_expr(expr);
                Some(temp.output().to_string())
            }

            _ => None,
        }
    }
}

/// Convert a struct type name for codegen:
/// - `Type` → `Type` (simple type)
/// - `pkg.Type` → `pkg.Type` (cross-package type)
/// - `Type.Variant` → `Type_Variant` (enum variant)
/// - `pkg.Type.Variant` → `pkg.Type_Variant` (cross-package enum variant)
fn convert_struct_type_name(name: &str) -> String {
    let parts: Vec<&str> = name.split('.').collect();
    match parts.len() {
        0 | 1 => name.to_string(), // Simple type
        2 => {
            // Could be pkg.Type or Type.Variant
            // If first char of first part is lowercase, assume package
            // If first char is uppercase, assume enum variant
            let first_char = parts[0].chars().next().unwrap_or('a');
            if first_char.is_lowercase() {
                // pkg.Type - keep as is
                name.to_string()
            } else {
                // Type.Variant - convert to Type_Variant
                format!("{}_{}", parts[0], parts[1])
            }
        }
        _ => {
            // pkg.Type.Variant or deeper - join all but last two with dots,
            // then underscore the last dot
            let prefix = parts[..parts.len() - 2].join(".");
            let type_name = parts[parts.len() - 2];
            let variant = parts[parts.len() - 1];
            if prefix.is_empty() {
                format!("{}_{}", type_name, variant)
            } else {
                format!("{}.{}_{}", prefix, type_name, variant)
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
