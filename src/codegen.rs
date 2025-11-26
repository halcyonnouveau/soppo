use crate::ast::{
    BinOp, Block, ConstDecl, EnumVariant, Expr, ExprKind, File, FuncDecl, Stmt, StmtKind, TypeDecl,
    TypeKind,
};

/// Code generator for emitting Go code
pub struct Codegen {
    output: String,
    indent_level: usize,
    global_state: crate::module::GlobalState,
    current_func_return_type: Option<String>,
}

impl Codegen {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            indent_level: 0,
            global_state: crate::module::GlobalState::new(),
            current_func_return_type: None,
        }
    }

    pub fn with_global_state(global_state: crate::module::GlobalState) -> Self {
        Self {
            output: String::new(),
            indent_level: 0,
            global_state,
            current_func_return_type: None,
        }
    }

    /// Get the generated output
    pub fn output(&self) -> &str {
        &self.output
    }

    /// Emit a line with current indentation
    fn emit_line(&mut self, line: &str) {
        self.emit_indent();
        self.output.push_str(line);
        self.output.push('\n');
    }

    /// Emit text without newline
    fn emit(&mut self, text: &str) {
        self.output.push_str(text);
    }

    /// Emit current indentation
    fn emit_indent(&mut self) {
        for _ in 0..self.indent_level {
            self.output.push_str("    ");
        }
    }

    /// Increase indentation level
    fn indent(&mut self) {
        self.indent_level += 1;
    }

    /// Decrease indentation level
    fn dedent(&mut self) {
        if self.indent_level > 0 {
            self.indent_level -= 1;
        }
    }

    /// Format generic parameters with constraints: "T any, E any"
    fn format_generic_params(&self, generics: &[crate::ast::Generic]) -> String {
        generics
            .iter()
            .map(|g| format!("{} {}", g.name, g.constraint))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Format generic parameter names only: "T, E"
    fn format_generic_names(&self, generics: &[crate::ast::Generic]) -> String {
        generics
            .iter()
            .map(|g| g.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Format generic parameters in brackets if not empty: "[T any, E any]" or ""
    fn format_generic_brackets(&self, generics: &[crate::ast::Generic]) -> String {
        if generics.is_empty() {
            String::new()
        } else {
            format!("[{}]", self.format_generic_params(generics))
        }
    }

    /// Generate code for an entire file
    pub fn gen_file(&mut self, file: &File) {
        // Package declaration
        self.emit_line(&format!("package {}", file.package));
        self.emit_line("");

        // Generate imports
        if !file.imports.is_empty() {
            for import in &file.imports {
                self.emit_line(&format!("import \"{}\"", import.path));
            }
            self.emit_line("");
        }

        // Generate declarations
        for decl in &file.decls {
            match decl {
                crate::ast::Decl::Const(const_decl) => {
                    self.gen_const_decl(const_decl);
                    self.emit_line("");
                }
                crate::ast::Decl::Type(type_decl) => {
                    self.gen_type_decl(type_decl);
                    self.emit_line("");
                }
                crate::ast::Decl::Func(func) => {
                    self.gen_func_decl(func);
                    self.emit_line("");
                }
            }
        }
    }

    /// Generate a const declaration
    fn gen_const_decl(&mut self, const_decl: &ConstDecl) {
        self.emit(&format!(
            "const {} {} = ",
            const_decl.name,
            self.go_type(&const_decl.ty.name)
        ));
        self.gen_expr(&const_decl.value);
        self.emit("\n");
    }

    /// Generate a type declaration (enum or struct)
    fn gen_type_decl(&mut self, type_decl: &TypeDecl) {
        match &type_decl.kind {
            TypeKind::Alias { target } => {
                // Type alias: type Foo = Bar or type Foo int
                self.emit_line(&format!(
                    "type {} {}",
                    type_decl.name,
                    self.go_type(&target.name)
                ));
            }

            TypeKind::Enum { variants } => {
                // Generate the interface with generics if present
                let generic_params = self.format_generic_brackets(&type_decl.generics);

                self.emit_line(&format!(
                    "type {}{} interface {{",
                    type_decl.name, generic_params
                ));
                self.indent();
                self.emit_line(&format!("is{}()", type_decl.name));
                self.dedent();
                self.emit_line("}");
                self.emit_line("");

                // Generate each variant as a type with namespaced name (EnumName_VariantName)
                for variant in variants {
                    match variant {
                        EnumVariant::Unit { name, .. } => {
                            // Unit variant: empty struct with generics if present
                            let full_name = format!("{}_{}", type_decl.name, name);
                            let generic_params = self.format_generic_brackets(&type_decl.generics);
                            let generic_names = if !type_decl.generics.is_empty() {
                                format!("[{}]", self.format_generic_names(&type_decl.generics))
                            } else {
                                String::new()
                            };

                            self.emit_line(&format!("type {}{} struct {{}}", full_name, generic_params));
                            self.emit_line(&format!(
                                "func ({}{}) is{}() {{}}",
                                full_name, generic_names, type_decl.name
                            ));
                            self.emit_line(&format!(
                                "func ({}) String() string {{ return \"{}\" }}",
                                full_name, name
                            ));
                            self.emit_line("");
                        }
                        EnumVariant::Single { name, ty, .. } => {
                            // Single value variant: struct with Value field and generics if present
                            let full_name = format!("{}_{}", type_decl.name, name);
                            let generic_params = self.format_generic_brackets(&type_decl.generics);
                            let generic_names = if !type_decl.generics.is_empty() {
                                format!("[{}]", self.format_generic_names(&type_decl.generics))
                            } else {
                                String::new()
                            };

                            self.emit_line(&format!("type {}{} struct {{", full_name, generic_params));
                            self.indent();
                            self.emit_line(&format!("Value {}", self.go_type(&ty.name)));
                            self.dedent();
                            self.emit_line("}");
                            self.emit_line(&format!(
                                "func ({}{}) is{}() {{}}",
                                full_name, generic_names, type_decl.name
                            ));
                            self.emit_line("");
                        }
                        EnumVariant::Struct { name, fields, .. } => {
                            // Struct variant: struct with all fields and generics if present
                            let full_name = format!("{}_{}", type_decl.name, name);
                            let generic_params = self.format_generic_brackets(&type_decl.generics);
                            let generic_names = if !type_decl.generics.is_empty() {
                                format!("[{}]", self.format_generic_names(&type_decl.generics))
                            } else {
                                String::new()
                            };

                            self.emit_line(&format!("type {}{} struct {{", full_name, generic_params));
                            self.indent();
                            for field in fields {
                                self.emit_line(&format!(
                                    "{} {}",
                                    field.name,
                                    self.go_type(&field.ty.name)
                                ));
                            }
                            self.dedent();
                            self.emit_line("}");
                            self.emit_line(&format!(
                                "func ({}{}) is{}() {{}}",
                                full_name, generic_names, type_decl.name
                            ));
                            self.emit_line("");
                        }
                    }
                }

                // Generate constructors for unit variants and functions for variants with data
                let unit_variants: Vec<_> = variants
                    .iter()
                    .filter(|v| matches!(v, EnumVariant::Unit { .. }))
                    .collect();

                if !unit_variants.is_empty() && type_decl.generics.is_empty() {
                    // Only generate var block for non-generic unit variants
                    // Var names don't use underscore (to avoid collision with type names)
                    self.emit_line("var (");
                    self.indent();
                    for variant in unit_variants {
                        if let EnumVariant::Unit { name, .. } = variant {
                            let type_name = format!("{}_{}", type_decl.name, name);
                            let var_name = format!("{}{}", type_decl.name, name);
                            self.emit_line(&format!(
                                "{} {} = {}{{}}",
                                var_name, type_decl.name, type_name
                            ));
                        }
                    }
                    self.dedent();
                    self.emit_line(")");
                }

                // Generate constructor functions for variants with data
                for variant in variants {
                    match variant {
                        EnumVariant::Single { name, ty, .. } => {
                            // Generate: func MyResultOk[T any, E any](value T) MyResult[T, E] { return MyResult_Ok[T, E]{Value: value} }
                            // Function name without underscore, type name with underscore
                            let func_name = format!("{}{}", type_decl.name, name);
                            let type_name = format!("{}_{}", type_decl.name, name);
                            let generic_params = self.format_generic_brackets(&type_decl.generics);
                            let generic_names = if !type_decl.generics.is_empty() {
                                format!("[{}]", self.format_generic_names(&type_decl.generics))
                            } else {
                                String::new()
                            };

                            self.emit_line(&format!(
                                "func {}{}(value {}) {}{} {{",
                                func_name,
                                generic_params,
                                self.go_type(&ty.name),
                                type_decl.name,
                                generic_names
                            ));
                            self.indent();
                            self.emit_line(&format!(
                                "return {}{}{{Value: value}}",
                                type_name, generic_names
                            ));
                            self.dedent();
                            self.emit_line("}");
                        }
                        EnumVariant::Struct { .. } => {
                            // No constructor for struct variants - use struct literal syntax directly
                        }
                        EnumVariant::Unit { name, .. } => {
                            // For generic unit variants, generate a constructor function
                            // Function name without underscore, type name with underscore
                            if !type_decl.generics.is_empty() {
                                let func_name = format!("{}{}", type_decl.name, name);
                                let type_name = format!("{}_{}", type_decl.name, name);
                                let generic_params =
                                    self.format_generic_brackets(&type_decl.generics);
                                let generic_names =
                                    format!("[{}]", self.format_generic_names(&type_decl.generics));

                                self.emit_line(&format!(
                                    "func {}{}() {}{} {{",
                                    func_name,
                                    generic_params,
                                    type_decl.name,
                                    generic_names
                                ));
                                self.indent();
                                self.emit_line(&format!("return {}{}{{}}", type_name, generic_names));
                                self.dedent();
                                self.emit_line("}");
                            }
                        }
                    }
                }
            }
            TypeKind::Struct { fields } => {
                // Generate struct type with generics if present
                let generic_params = self.format_generic_brackets(&type_decl.generics);

                self.emit_line(&format!(
                    "type {}{} struct {{",
                    type_decl.name, generic_params
                ));
                self.indent();
                for field in fields {
                    self.emit_line(&format!("{} {}", field.name, self.go_type(&field.ty.name)));
                }
                self.dedent();
                self.emit_line("}");
            }
        }
    }

    /// Generate a function declaration
    fn gen_func_decl(&mut self, func: &FuncDecl) {
        // Function signature with optional receiver
        self.emit("func ");

        if let Some(receiver) = &func.receiver {
            self.emit(&format!(
                "({} {}) ",
                receiver.name,
                self.go_type(&receiver.ty.name)
            ));
        }

        self.emit(&func.name);

        // Generic parameters
        let generic_params = self.format_generic_brackets(&func.generics);
        if !generic_params.is_empty() {
            self.emit(&generic_params);
        }

        self.emit("(");

        // Parameters
        for (i, param) in func.params.iter().enumerate() {
            if i > 0 {
                self.emit(", ");
            }
            self.emit(&format!("{} {}", param.name, self.go_type(&param.ty.name)));
        }

        self.emit(")");

        // Return type
        if let Some(ret_ty) = &func.return_type {
            let go_type = self.go_type(&ret_ty.name).to_string();
            self.emit(&format!(" {}", go_type));
            // Store return type for use in return statements
            self.current_func_return_type = Some(go_type);
        } else {
            self.current_func_return_type = None;
        }

        self.emit(" ");

        // Body
        self.gen_block(&func.body);

        // Clear return type after function is done
        self.current_func_return_type = None;

        self.output.push('\n');
    }

    /// Generate a block
    fn gen_block(&mut self, block: &Block) {
        self.emit("{\n");
        self.indent();

        for stmt in &block.stmts {
            self.gen_stmt(stmt);
        }

        self.dedent();
        self.emit_indent();
        self.emit("}");
    }

    /// Generate a statement
    fn gen_stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Decl { name, value } => {
                self.emit_indent();
                self.emit(&format!("{} := ", name));
                self.gen_expr(value);
                self.emit("\n");
            }

            StmtKind::VarDecl { name, ty, value } => {
                self.emit_indent();
                if let Some(expr) = value {
                    self.emit(&format!("var {} {} = ", name, self.go_type(&ty.name)));
                    self.gen_expr(expr);
                } else {
                    self.emit(&format!("var {} {}", name, self.go_type(&ty.name)));
                }
                self.emit("\n");
            }

            StmtKind::Assign { target, value } => {
                self.emit_indent();
                self.gen_expr(target);
                self.emit(" = ");
                self.gen_expr(value);
                self.emit("\n");
            }

            StmtKind::For { condition, body } => {
                self.emit_indent();
                self.emit("for ");
                self.gen_expr(condition);
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

            StmtKind::Return { value } => {
                self.emit_indent();
                if let Some(expr) = value {
                    self.emit("return ");
                    self.gen_expr(expr);
                    self.emit("\n");
                } else {
                    self.emit("return\n");
                }
            }

            StmtKind::Match { scrutinee, arms } => {
                self.emit_indent();

                // Check if this is a type switch or value switch
                let is_type_switch = arms.iter().any(|arm| {
                    matches!(
                        &arm.pattern.kind,
                        crate::ast::PatternKind::Variant(_)
                            | crate::ast::PatternKind::Destructor { .. }
                            | crate::ast::PatternKind::StructDestructor { .. }
                    )
                });

                // Check if any arm needs the bound variable (Destructor or StructDestructor patterns)
                let needs_binding = arms.iter().any(|arm| {
                    matches!(
                        &arm.pattern.kind,
                        crate::ast::PatternKind::Destructor { .. }
                            | crate::ast::PatternKind::StructDestructor { .. }
                    )
                });

                if is_type_switch {
                    if needs_binding {
                        self.emit("switch __v := ");
                    } else {
                        self.emit("switch ");
                    }
                    self.gen_expr(scrutinee);
                    self.emit(".(type) {\n");
                } else {
                    self.emit("switch ");
                    self.gen_expr(scrutinee);
                    self.emit(" {\n");
                }

                for arm in arms {
                    self.emit_indent();

                    // Emit pattern - default is special (default:, not case default:)
                    if matches!(&arm.pattern.kind, crate::ast::PatternKind::Default) {
                        self.emit("default:\n");
                    } else {
                        self.emit("case ");
                        match &arm.pattern.kind {
                            crate::ast::PatternKind::Variant(name) => {
                                // Convert qualified name (Color.Red) to namespaced (Color_Red)
                                let full_name = name.replace('.', "_");
                                self.emit(&full_name);
                            }
                            crate::ast::PatternKind::Literal(lit) => match lit {
                                crate::ast::Literal::Integer(n) => self.emit(&n.to_string()),
                                crate::ast::Literal::String(s) => self.emit(&format!("\"{}\"", s)),
                                crate::ast::Literal::Bool(b) => self.emit(&b.to_string()),
                            },
                            crate::ast::PatternKind::Destructor { name, .. } => {
                                // Convert qualified name (MyResult.Ok) to namespaced (MyResult_Ok)
                                let full_name = name.replace('.', "_");
                                self.emit(&full_name);
                            }
                            crate::ast::PatternKind::StructDestructor { name, .. } => {
                                // Convert qualified name (Shape.Circle) to namespaced (Shape_Circle)
                                let full_name = name.replace('.', "_");
                                self.emit(&full_name);
                            }
                            crate::ast::PatternKind::Default => unreachable!(),
                        }
                        self.emit(":\n");
                    }
                    self.indent();

                    // Extract pattern bindings for destructor patterns
                    if let crate::ast::PatternKind::Destructor { name: _, binding } =
                        &arm.pattern.kind
                    {
                        // __v is already the concrete type from the switch statement
                        self.emit_indent();
                        self.emit(&format!("{} := __v.Value\n", binding));
                        // Add blank assignment to avoid unused variable warnings
                        self.emit_indent();
                        self.emit(&format!("_ = {}\n", binding));
                    }

                    // Extract pattern bindings for struct destructor patterns
                    if let crate::ast::PatternKind::StructDestructor { fields, .. } =
                        &arm.pattern.kind
                    {
                        // __v is already the concrete type from the switch statement
                        for (field_name, binding_name) in fields {
                            self.emit_indent();
                            self.emit(&format!("{} := __v.{}\n", binding_name, field_name));
                            // Add blank assignment to avoid unused variable warnings
                            self.emit_indent();
                            self.emit(&format!("_ = {}\n", binding_name));
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

            StmtKind::Expr(expr) => {
                self.emit_indent();
                self.gen_expr(expr);
                self.emit("\n");
            }
        }
    }

    /// Generate an expression
    fn gen_expr(&mut self, expr: &Expr) {
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

            ExprKind::Call { func, args } => {
                self.gen_expr(func);
                self.emit("(");
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(arg);
                }
                self.emit(")");
            }

            ExprKind::Field { expr, field } => {
                // Check if this is an enum constructor like Color.Red
                if let ExprKind::Ident(type_name) = &expr.kind {
                    // Check if it's a registered type (enum)
                    if self.global_state.has_type(type_name) {
                        // Enum values: Color.Red → ColorRed (var or function)
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
                // Generate [size]type{elements}
                self.emit("[");
                self.emit(&elements.len().to_string());
                self.emit("]");
                if let Some(ty) = ty {
                    self.emit(self.go_type(&ty.name));
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

            ExprKind::Block(block) => {
                self.gen_block(block);
            }
        }
    }

    /// Convert Soppo type to Go type
    fn go_type<'a>(&self, ty: &'a str) -> &'a str {
        match ty {
            "int" => "int",
            "string" => "string",
            "bool" => "bool",
            "()" => "",
            _ => ty,
        }
    }

    /// Convert binary operator to Go operator
    fn go_binop(&self, op: &BinOp) -> &str {
        match op {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
        }
    }
}

impl Default for Codegen {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;
    use crate::source::FileId;

    #[test]
    fn test_gen_simple_function() {
        let source = "func add(x int, y int) int { return x + y }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser.parse_func_decl().unwrap();

        let mut codegen = Codegen::new();
        codegen.gen_func_decl(&func);

        let output = codegen.output();
        assert!(output.contains("func add(x int, y int) int"));
        assert!(output.contains("return (x + y)"));
    }

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

    #[test]
    fn test_gen_complete_file() {
        let source = r#"
            func add(x int, y int) int {
                return x + y
            }

            func main() int {
                return add(1, 2)
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();

        let mut codegen = Codegen::new();
        codegen.gen_file(&file);

        let output = codegen.output();
        assert!(output.contains("package main"));
        assert!(output.contains("func add(x int, y int) int"));
        assert!(output.contains("func main() int"));
    }

    #[test]
    fn test_e2e_simple_program() {
        // End-to-end test: parse, type-check, and generate Go code
        let source = r#"
            func add(x int, y int) int {
                result := x + y
                return result
            }

            func main() int {
                a := 10
                b := 20
                sum := add(a, b)
                return sum
            }
        "#;

        // Parse
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();

        // Type check
        let mut infer = crate::infer::Infer::new();
        for decl in &file.decls {
            if let crate::ast::Decl::Func(func) = decl {
                infer.infer_func_decl(func).unwrap();
            }
        }

        // Generate Go code
        let mut codegen = Codegen::new();
        codegen.gen_file(&file);
        let output = codegen.output();

        // Verify output
        assert!(output.contains("package main"));
        assert!(output.contains("func add(x int, y int) int"));
        assert!(output.contains("result := (x + y)"));
        assert!(output.contains("func main() int"));
        assert!(output.contains("a := 10"));
        assert!(output.contains("b := 20"));
        assert!(output.contains("sum := add(a, b)"));
    }
}
