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

                self.emit_line(&format!("type {}{} interface {{", type_decl.name, generic_params));
                self.indent();
                self.emit_line(&format!("is{}()", type_decl.name));
                self.dedent();
                self.emit_line("}");
                self.emit_line("");

                // Generate each variant as a type
                for variant in variants {
                    match variant {
                        EnumVariant::Unit { name, .. } => {
                            // Unit variant: empty struct with generics if present
                            let generic_params = self.format_generic_brackets(&type_decl.generics);
                            let generic_names = if !type_decl.generics.is_empty() {
                                format!("[{}]", self.format_generic_names(&type_decl.generics))
                            } else {
                                String::new()
                            };

                            self.emit_line(&format!("type {}{} struct {{}}", name, generic_params));
                            self.emit_line(&format!("func ({}{}) is{}() {{}}", name, generic_names, type_decl.name));
                            self.emit_line(&format!(
                                "func ({}) String() string {{ return \"{}\" }}",
                                name, name
                            ));
                            self.emit_line("");
                        }
                        EnumVariant::Single { name, ty, .. } => {
                            // Single value variant: struct with Value field and generics if present
                            let generic_params = self.format_generic_brackets(&type_decl.generics);
                            let generic_names = if !type_decl.generics.is_empty() {
                                format!("[{}]", self.format_generic_names(&type_decl.generics))
                            } else {
                                String::new()
                            };

                            self.emit_line(&format!("type {}{} struct {{", name, generic_params));
                            self.indent();
                            self.emit_line(&format!("Value {}", self.go_type(&ty.name)));
                            self.dedent();
                            self.emit_line("}");
                            self.emit_line(&format!("func ({}{}) is{}() {{}}", name, generic_names, type_decl.name));
                            self.emit_line("");
                        }
                        EnumVariant::Struct { name, fields, .. } => {
                            // Struct variant: struct with all fields and generics if present
                            let generic_params = self.format_generic_brackets(&type_decl.generics);
                            let generic_names = if !type_decl.generics.is_empty() {
                                format!("[{}]", self.format_generic_names(&type_decl.generics))
                            } else {
                                String::new()
                            };

                            self.emit_line(&format!("type {}{} struct {{", name, generic_params));
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
                            self.emit_line(&format!("func ({}{}) is{}() {{}}", name, generic_names, type_decl.name));
                            self.emit_line("");
                        }
                    }
                }

                // Generate constructors for unit variants and functions for variants with data
                let unit_variants: Vec<_> = variants.iter().filter(|v| matches!(v, EnumVariant::Unit { .. })).collect();

                if !unit_variants.is_empty() && type_decl.generics.is_empty() {
                    // Only generate var block for non-generic unit variants
                    self.emit_line("var (");
                    self.indent();
                    for variant in unit_variants {
                        if let EnumVariant::Unit { name, .. } = variant {
                            self.emit_line(&format!(
                                "{}{} {} = {}{{}}",
                                type_decl.name, name, type_decl.name, name
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
                            // Generate: func ResultOk[T any, E any](value T) Result[T, E] { return Ok[T, E]{Value: value} }
                            let generic_params = self.format_generic_brackets(&type_decl.generics);
                            let generic_names = if !type_decl.generics.is_empty() {
                                format!("[{}]", self.format_generic_names(&type_decl.generics))
                            } else {
                                String::new()
                            };

                            self.emit_line(&format!(
                                "func {}{}{}(value {}) {}{} {{",
                                type_decl.name, name,
                                generic_params,
                                self.go_type(&ty.name),
                                type_decl.name,
                                generic_names
                            ));
                            self.indent();
                            self.emit_line(&format!("return {}{}{{Value: value}}", name, generic_names));
                            self.dedent();
                            self.emit_line("}");
                        }
                        EnumVariant::Struct { name, fields, .. } => {
                            // Generate constructor with all fields as parameters
                            let params: Vec<String> = fields.iter()
                                .map(|f| format!("{} {}", f.name, self.go_type(&f.ty.name)))
                                .collect();
                            let field_inits: Vec<String> = fields.iter()
                                .map(|f| format!("{}: {}", f.name, f.name))
                                .collect();

                            let generic_params = self.format_generic_brackets(&type_decl.generics);
                            let generic_names = if !type_decl.generics.is_empty() {
                                format!("[{}]", self.format_generic_names(&type_decl.generics))
                            } else {
                                String::new()
                            };

                            self.emit_line(&format!(
                                "func {}{}{}({}) {}{} {{",
                                type_decl.name, name,
                                generic_params,
                                params.join(", "),
                                type_decl.name,
                                generic_names
                            ));
                            self.indent();
                            self.emit_line(&format!("return {}{}{{ {} }}", name, generic_names, field_inits.join(", ")));
                            self.dedent();
                            self.emit_line("}");
                        }
                        EnumVariant::Unit { name, .. } => {
                            // For generic unit variants, generate a constructor function
                            if !type_decl.generics.is_empty() {
                                let generic_params = self.format_generic_brackets(&type_decl.generics);
                                let generic_names = format!("[{}]", self.format_generic_names(&type_decl.generics));

                                self.emit_line(&format!(
                                    "func {}{}{}() {}{} {{",
                                    type_decl.name, name,
                                    generic_params,
                                    type_decl.name,
                                    generic_names
                                ));
                                self.indent();
                                self.emit_line(&format!("return {}{}{{}}", name, generic_names));
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

                self.emit_line(&format!("type {}{} struct {{", type_decl.name, generic_params));
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

    /// Generate a match expression as assignment to a variable
    fn gen_match_as_assignment(&mut self, var_name: &str, scrutinee: &Expr, arms: &[crate::ast::Arm]) {
        // Check if this is a type switch or value switch
        let is_type_switch = arms.iter().any(|arm| {
            matches!(
                &arm.pattern.kind,
                crate::ast::PatternKind::Variant(_)
                    | crate::ast::PatternKind::TuplePattern { .. }
            )
        });

        self.emit_indent();
        if is_type_switch {
            self.emit("switch __v := ");
            self.gen_expr(scrutinee);
            self.emit(".(type) {\n");
        } else {
            self.emit("switch ");
            self.gen_expr(scrutinee);
            self.emit(" {\n");
        }

        for arm in arms {
            self.emit_indent();

            // Emit pattern
            if matches!(&arm.pattern.kind, crate::ast::PatternKind::Wildcard) {
                self.emit("default:\n");
            } else {
                self.emit("case ");
                match &arm.pattern.kind {
                    crate::ast::PatternKind::Variant(name) => {
                        // Extract just the variant name from qualified names like "Color.Red"
                        let variant_name = name.rsplit('.').next().unwrap_or(name);
                        self.emit(variant_name);
                    }
                    crate::ast::PatternKind::Literal(lit) => match lit {
                        crate::ast::Literal::Integer(n) => self.emit(&n.to_string()),
                        crate::ast::Literal::String(s) => {
                            self.emit(&format!("\"{}\"", s))
                        }
                        crate::ast::Literal::Bool(b) => self.emit(&b.to_string()),
                    },
                    crate::ast::PatternKind::TuplePattern { name, .. } => {
                        // Extract just the variant name from qualified names like "Result.Ok"
                        let variant_name = name.rsplit('.').next().unwrap_or(name);
                        self.emit(variant_name);
                    }
                    crate::ast::PatternKind::Wildcard => unreachable!(),
                }
                self.emit(":\n");
            }
            self.indent();

            // Extract pattern bindings for tuple patterns
            if let crate::ast::PatternKind::TuplePattern { name: _, elements } = &arm.pattern.kind {
                for elem in elements.iter() {
                    if let crate::ast::PatternKind::Variant(binding_name) = &elem.kind {
                        self.emit_indent();
                        self.emit(&format!("{} := __v.Value\n", binding_name));
                        self.emit_indent();
                        self.emit(&format!("_ = {}\n", binding_name));
                    }
                }
            }

            // Emit assignment to variable
            match &arm.body.kind {
                ExprKind::Block(block) => {
                    for stmt in &block.stmts {
                        self.gen_stmt(stmt);
                    }
                }
                _ => {
                    self.emit_indent();
                    self.emit(&format!("{} = ", var_name));
                    self.gen_expr(&arm.body);
                    self.emit("\n");
                }
            }

            self.dedent();
        }

        self.emit_indent();
        self.emit("}\n");
    }

    /// Generate a statement
    fn gen_stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Declare { name, value } => {
                // Special case: match expression becomes var + switch
                if let ExprKind::Match { scrutinee, arms } = &value.kind {
                    // Declare variable (we'd need type inference here, using string for now)
                    self.emit_indent();
                    self.emit(&format!("var {} string\n", name));
                    self.emit_indent();

                    // Check if this is a type switch (matching on enum variants) or value switch (literals)
                    let is_type_switch = arms.iter().any(|arm| {
                        matches!(
                            &arm.pattern.kind,
                            crate::ast::PatternKind::Variant(_)
                                | crate::ast::PatternKind::TuplePattern { .. }
                        )
                    });

                    if is_type_switch {
                        self.emit("switch ");
                        self.gen_expr(scrutinee);
                        self.emit(".(type) {\n");
                    } else {
                        self.emit("switch ");
                        self.gen_expr(scrutinee);
                        self.emit(" {\n");
                    }

                    for arm in arms {
                        self.emit_indent();

                        // Emit pattern
                        if matches!(&arm.pattern.kind, crate::ast::PatternKind::Wildcard) {
                            self.emit("default:\n");
                        } else {
                            self.emit("case ");
                            match &arm.pattern.kind {
                                crate::ast::PatternKind::Variant(var_name) => {
                                    // Extract just the variant name from qualified names
                                    let variant_name = var_name.rsplit('.').next().unwrap_or(var_name);
                                    self.emit(variant_name);
                                }
                                crate::ast::PatternKind::Literal(lit) => match lit {
                                    crate::ast::Literal::Integer(n) => self.emit(&n.to_string()),
                                    crate::ast::Literal::String(s) => {
                                        self.emit(&format!("\"{}\"", s))
                                    }
                                    crate::ast::Literal::Bool(b) => self.emit(&b.to_string()),
                                },
                                crate::ast::PatternKind::TuplePattern {
                                    name: var_name, ..
                                } => {
                                    self.emit(var_name);
                                }
                                crate::ast::PatternKind::Wildcard => unreachable!(),
                            }
                            self.emit(":\n");
                        }
                        self.indent();

                        // Emit assignment to variable
                        match &arm.body.kind {
                            ExprKind::Block(block) => {
                                // For block bodies, emit statements without braces
                                for stmt in &block.stmts {
                                    self.gen_stmt(stmt);
                                }
                            }
                            _ => {
                                self.emit_indent();
                                self.emit(&format!("{} = ", name));
                                self.gen_expr(&arm.body);
                                self.emit("\n");
                            }
                        }

                        self.dedent();
                    }

                    self.emit_indent();
                    self.emit("}\n");
                } else {
                    self.emit_indent();
                    self.emit(&format!("{} := ", name));
                    self.gen_expr(value);
                    self.emit("\n");
                }
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
                if let Some(expr) = value {
                    // Check if returning a match expression
                    if let ExprKind::Match { scrutinee, arms } = &expr.kind {
                        // Generate match as assignment to temp variable, then return it
                        // var __result T
                        // switch ... { case X: __result = ... }
                        // return __result

                        self.emit_indent();
                        // Declare result variable with function's return type
                        if let Some(ret_type) = &self.current_func_return_type {
                            self.emit(&format!("var __result {}\n", ret_type));
                        } else {
                            self.emit("var __result interface{}\n");
                        }

                        // Generate the match as assignment to __result
                        self.gen_match_as_assignment("__result", scrutinee, arms);

                        self.emit_indent();
                        self.emit("return __result\n");
                    } else {
                        self.emit_indent();
                        self.emit("return ");
                        self.gen_expr(expr);
                        self.emit("\n");
                    }
                } else {
                    self.emit_indent();
                    self.emit("return\n");
                }
            }

            StmtKind::Expr(expr) => {
                // Special case: match expressions in statement context
                // Should be plain switch statements, not IIFEs
                if let ExprKind::Match { scrutinee, arms } = &expr.kind {
                    self.emit_indent();

                    // Check if this is a type switch or value switch
                    let is_type_switch = arms.iter().any(|arm| {
                        matches!(
                            &arm.pattern.kind,
                            crate::ast::PatternKind::Variant(_)
                                | crate::ast::PatternKind::TuplePattern { .. }
                        )
                    });

                    if is_type_switch {
                        self.emit("switch __v := ");
                        self.gen_expr(scrutinee);
                        self.emit(".(type) {\n");
                    } else {
                        self.emit("switch ");
                        self.gen_expr(scrutinee);
                        self.emit(" {\n");
                    }

                    for arm in arms {
                        self.emit_indent();

                        // Emit pattern - wildcard is special (default:, not case default:)
                        if matches!(&arm.pattern.kind, crate::ast::PatternKind::Wildcard) {
                            self.emit("default:\n");
                        } else {
                            self.emit("case ");
                            match &arm.pattern.kind {
                                crate::ast::PatternKind::Variant(name) => {
                                    self.emit(name);
                                }
                                crate::ast::PatternKind::Literal(lit) => match lit {
                                    crate::ast::Literal::Integer(n) => self.emit(&n.to_string()),
                                    crate::ast::Literal::String(s) => {
                                        self.emit(&format!("\"{}\"", s))
                                    }
                                    crate::ast::Literal::Bool(b) => self.emit(&b.to_string()),
                                },
                                crate::ast::PatternKind::TuplePattern { name, .. } => {
                                    // Extract just the variant name from qualified names
                                    let variant_name = name.rsplit('.').next().unwrap_or(name);
                                    self.emit(variant_name);
                                }
                                crate::ast::PatternKind::Wildcard => unreachable!(),
                            }
                            self.emit(":\n");
                        }
                        self.indent();

                        // Extract pattern bindings for tuple patterns
                        if let crate::ast::PatternKind::TuplePattern { name: _, elements } = &arm.pattern.kind {
                            // __v is already the concrete type from the switch statement
                            // Extract bound variables from the pattern
                            for elem in elements.iter() {
                                if let crate::ast::PatternKind::Variant(binding_name) = &elem.kind {
                                    self.emit_indent();
                                    self.emit(&format!("{} := __v.Value\n", binding_name));
                                    // Add blank assignment to avoid unused variable warnings
                                    self.emit_indent();
                                    self.emit(&format!("_ = {}\n", binding_name));
                                }
                            }
                        }

                        // Emit body directly
                        match &arm.body.kind {
                            ExprKind::Block(block) => {
                                // For block bodies, emit statements without braces
                                for stmt in &block.stmts {
                                    self.gen_stmt(stmt);
                                }
                            }
                            _ => {
                                self.emit_indent();
                                self.gen_expr(&arm.body);
                                self.emit("\n");
                            }
                        }

                        self.dedent();
                    }

                    self.emit_indent();
                    self.emit("}\n");
                } else {
                    self.emit_indent();
                    self.gen_expr(expr);
                    self.emit("\n");
                }
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
                        // Enum constructors: Color.Red → ColorRed
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
                self.emit(self.go_type(&ty.name));
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

            ExprKind::Match { .. } => {
                // Match expressions should only appear in Let statements (handled in gen_stmt)
                // or as statement-level expressions (also handled in gen_stmt)
                // If we get here, it's an error - match in unsupported position
                self.emit("/* ERROR: match in unsupported position */");
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
