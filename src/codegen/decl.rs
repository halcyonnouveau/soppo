use super::Codegen;
use crate::syntax::{
    Block, ConstDecl, EnumVariant, FuncDecl, Literal, Pattern, PatternKind, TypeDecl, TypeKind,
};

impl Codegen {
    /// Generate a const declaration
    pub(crate) fn gen_const_decl(&mut self, const_decl: &ConstDecl) {
        if let Some(ty) = &const_decl.ty {
            // const X type = value
            self.emit(format!(
                "const {} {} = ",
                const_decl.ident,
                self.go_type(&ty.name)
            ));
        } else {
            // const X = value (type inference)
            self.emit(format!("const {} = ", const_decl.ident));
        }
        self.gen_expr(&const_decl.value);
        self.emit("\n");
    }

    /// Generate a grouped const block (for iota support)
    pub(crate) fn gen_const_block(&mut self, consts: &[ConstDecl]) {
        self.emit_line("const (");
        self.indent();
        for const_decl in consts {
            self.emit_indent();
            if let Some(ty) = &const_decl.ty {
                // X type = value
                self.emit(format!(
                    "{} {} = ",
                    const_decl.ident,
                    self.go_type(&ty.name)
                ));
                self.gen_expr(&const_decl.value);
            } else {
                // X = value or just X (for implicit iota continuation)
                self.emit(&const_decl.ident);
                // Only emit " = value" if the value isn't just "iota" on continuation lines
                // For the first entry with iota, we need to emit it
                // For subsequent entries, Go handles implicit continuation
                let is_iota = matches!(
                    &const_decl.value.kind,
                    crate::syntax::ExprKind::Ident(name) if name == "iota"
                );
                if !is_iota || consts.first() == Some(const_decl) {
                    self.emit(" = ");
                    self.gen_expr(&const_decl.value);
                }
            }
            self.emit("\n");
        }
        self.dedent();
        self.emit_line(")");
    }

    /// Generate a type declaration (enum or struct)
    pub(crate) fn gen_type_decl(&mut self, type_decl: &TypeDecl) {
        match &type_decl.kind {
            TypeKind::Alias { target } => {
                // Type alias: type Foo = Bar (Foo is exactly Bar)
                self.emit_line(&format!(
                    "type {} = {}",
                    type_decl.ident,
                    self.go_type_from_ast(target)
                ));
            }

            TypeKind::Definition { target } => {
                // Type definition: type Foo Bar (Foo is a new distinct type)
                self.emit_line(&format!(
                    "type {} {}",
                    type_decl.ident,
                    self.go_type_from_ast(target)
                ));
            }

            TypeKind::Enum { variants } => {
                // Generate the interface with generics if present
                let generic_params = self.format_generic_brackets(&type_decl.generics);

                self.emit_soppo_enum_marker(type_decl, variants);
                self.emit_line(&format!(
                    "type {}{} interface {{",
                    type_decl.ident, generic_params
                ));
                self.indent();
                self.emit_line(&format!("is{}()", type_decl.ident));
                self.dedent();
                self.emit_line("}");
                self.emit_line("");

                // Generate each variant as a type with namespaced name (EnumName_VariantName)
                for variant in variants {
                    match variant {
                        EnumVariant::Unit { ident: name, .. } => {
                            // Unit variant: empty struct with generics if present
                            let full_name = format!("{}_{}", type_decl.ident, name);
                            let generic_params = self.format_generic_brackets(&type_decl.generics);
                            let generic_names =
                                self.format_generic_name_brackets(&type_decl.generics);

                            self.emit_line(&format!(
                                "type {}{} struct {{}}",
                                full_name, generic_params
                            ));
                            self.emit_line(&format!(
                                "func ({}{}) is{}() {{}}",
                                full_name, generic_names, type_decl.ident
                            ));
                            self.emit_line(&format!(
                                "func ({}{}) String() string {{ return \"{}\" }}",
                                full_name, generic_names, name
                            ));
                            self.emit_line("");
                        }
                        EnumVariant::Single {
                            ident: name, ty, ..
                        } => {
                            // Single value variant: struct with Value field and generics if present
                            let full_name = format!("{}_{}", type_decl.ident, name);
                            let generic_params = self.format_generic_brackets(&type_decl.generics);
                            let generic_names =
                                self.format_generic_name_brackets(&type_decl.generics);

                            self.emit_line(&format!(
                                "type {}{} struct {{",
                                full_name, generic_params
                            ));
                            self.indent();
                            self.emit_line(&format!("Value {}", self.go_type(&ty.name)));
                            self.dedent();
                            self.emit_line("}");
                            self.emit_line(&format!(
                                "func ({}{}) is{}() {{}}",
                                full_name, generic_names, type_decl.ident
                            ));
                            self.emit_line("");
                        }
                        EnumVariant::Struct {
                            ident: name,
                            fields,
                            ..
                        } => {
                            // Struct variant: struct with all fields and generics if present
                            let full_name = format!("{}_{}", type_decl.ident, name);
                            let generic_params = self.format_generic_brackets(&type_decl.generics);
                            let generic_names =
                                self.format_generic_name_brackets(&type_decl.generics);

                            self.emit_line(&format!(
                                "type {}{} struct {{",
                                full_name, generic_params
                            ));
                            self.indent();
                            for field in fields {
                                let tag = field
                                    .tag
                                    .as_ref()
                                    .map(|t| format!(" `{}`", t))
                                    .unwrap_or_default();
                                self.emit_line(&format!(
                                    "{} {}{}",
                                    field.ident,
                                    self.go_type(&field.ty.name),
                                    tag
                                ));
                            }
                            self.dedent();
                            self.emit_line("}");
                            self.emit_line(&format!(
                                "func ({}{}) is{}() {{}}",
                                full_name, generic_names, type_decl.ident
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
                        if let EnumVariant::Unit { ident: name, .. } = variant {
                            let type_name = format!("{}_{}", type_decl.ident, name);
                            let var_name = format!("{}{}", type_decl.ident, name);
                            self.emit_line(&format!(
                                "{} {} = {}{{}}",
                                var_name, type_decl.ident, type_name
                            ));
                        }
                    }
                    self.dedent();
                    self.emit_line(")");
                }

                // Generate constructor functions for variants with data
                for variant in variants {
                    match variant {
                        EnumVariant::Single {
                            ident: name, ty, ..
                        } => {
                            // Generate: func MyResultOk[T any, E any](value T) MyResult[T, E] { return MyResult_Ok[T, E]{Value: value} }
                            // Function name without underscore, type name with underscore
                            let func_name = format!("{}{}", type_decl.ident, name);
                            let type_name = format!("{}_{}", type_decl.ident, name);
                            let generic_params = self.format_generic_brackets(&type_decl.generics);
                            let generic_names =
                                self.format_generic_name_brackets(&type_decl.generics);

                            self.emit_line(&format!(
                                "func {}{}(value {}) {}{} {{",
                                func_name,
                                generic_params,
                                self.go_type(&ty.name),
                                type_decl.ident,
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
                        EnumVariant::Unit { ident: name, .. } => {
                            // For generic unit variants, generate a constructor function
                            // Function name without underscore, type name with underscore
                            if !type_decl.generics.is_empty() {
                                let func_name = format!("{}{}", type_decl.ident, name);
                                let type_name = format!("{}_{}", type_decl.ident, name);
                                let generic_params =
                                    self.format_generic_brackets(&type_decl.generics);
                                let generic_names =
                                    self.format_generic_name_brackets(&type_decl.generics);

                                self.emit_line(&format!(
                                    "func {}{}() {}{} {{",
                                    func_name, generic_params, type_decl.ident, generic_names
                                ));
                                self.indent();
                                self.emit_line(&format!(
                                    "return {}{}{{}}",
                                    type_name, generic_names
                                ));
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
                    type_decl.ident, generic_params
                ));
                self.indent();
                for field in fields {
                    let go_type = self.go_type_from_ast(&field.ty);
                    let nilable_comment = self.nilable_comment(&field.ty);
                    let tag = field
                        .tag
                        .as_ref()
                        .map(|t| format!(" `{}`", t))
                        .unwrap_or_default();

                    // Special handling for nilable anonymous struct pointers
                    // Format as multiline: *struct { //soppo:nilable\n  field1\n  field2\n}
                    if !nilable_comment.is_empty() && go_type.starts_with("*struct { ") {
                        self.emit_struct_field_with_anon_struct(
                            &field.ident,
                            &go_type,
                            &tag,
                            nilable_comment,
                        );
                    } else {
                        self.emit_line(&format!(
                            "{} {}{}{}",
                            field.ident, go_type, tag, nilable_comment
                        ));
                    }
                }
                self.dedent();
                self.emit_line("}");
            }

            TypeKind::Interface { methods } => {
                // Generate Go interface directly
                let generic_params = self.format_generic_brackets(&type_decl.generics);

                self.emit_line(&format!(
                    "type {}{} interface {{",
                    type_decl.ident, generic_params
                ));
                self.indent();
                for method in methods {
                    self.emit_indent();
                    self.emit(format!("{}(", method.ident));

                    // Parameters
                    for (i, param) in method.params.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.emit(format!("{} {}", param.ident, self.go_type(&param.ty.name)));
                    }

                    self.emit(")");

                    // Return types
                    if !method.returns.is_empty() {
                        if method.returns.len() == 1 {
                            self.emit(format!(" {}", self.go_type(&method.returns[0].name)));
                        } else {
                            self.emit(" (");
                            for (i, ty) in method.returns.iter().enumerate() {
                                if i > 0 {
                                    self.emit(", ");
                                }
                                self.emit(self.go_type(&ty.name));
                            }
                            self.emit(")");
                        }
                    }

                    self.emit("\n");
                }
                self.dedent();
                self.emit_line("}");
            }
        }
    }

    /// Generate a function declaration
    pub(crate) fn gen_func_decl(&mut self, func: &FuncDecl) {
        // Emit //soppo:nilable comment if any parameters or return types are nullable
        // Format: //soppo:nilable p q : 0 1
        //         (params before :, return indices after :)
        let nilable_params: Vec<&str> = func
            .params
            .iter()
            .filter(|p| p.ty.nullable)
            .map(|p| p.ident.name.as_str())
            .collect();
        let nilable_returns: Vec<String> = func
            .returns
            .iter()
            .enumerate()
            .filter(|(_, p)| p.ty.nullable)
            .map(|(i, _)| i.to_string())
            .collect();

        if !nilable_params.is_empty() || !nilable_returns.is_empty() {
            let params_part = nilable_params.join(" ");
            let returns_part = nilable_returns.join(" ");
            let annotation = match (nilable_params.is_empty(), nilable_returns.is_empty()) {
                (false, true) => params_part,
                (true, false) => format!(": {}", returns_part),
                (false, false) => format!("{} : {}", params_part, returns_part),
                // INVARIANT: outer if ensures at least one is non-empty
                (true, true) => {
                    unreachable!("nilable annotation with no nilable params or returns")
                }
            };
            self.emit_line(&format!("//soppo:nilable {}", annotation));
        }

        // Function signature with optional receiver
        self.emit("func ");

        if let Some(receiver) = &func.receiver {
            // Use go_receiver_type to convert EnumName.Variant -> EnumName_Variant
            let receiver_type = self.go_receiver_type(&receiver.ty.name);
            self.emit(format!("({} {}) ", receiver.ident, receiver_type));
        }

        self.emit(&func.ident);

        // Generic parameters
        let generic_params = self.format_generic_brackets(&func.generics);
        if !generic_params.is_empty() {
            self.emit(&generic_params);
        }

        self.emit("(");

        // Parameters
        // Note: //soppo:nilable comments on params would make Go syntax invalid
        // Instead, we just strip the ? prefix and rely on type checking having validated nullability
        for (i, param) in func.params.iter().enumerate() {
            if i > 0 {
                self.emit(", ");
            }
            let go_type = self.go_type_from_ast(&param.ty);
            self.emit(format!("{} {}", param.ident, go_type));
        }

        self.emit(")");

        // Return type(s) - handles both named (x int, y string) and unnamed (int, string)
        // Use go_type_from_ast to strip ? prefix from nullable types
        self.current_return_types = func
            .returns
            .iter()
            .map(|p| self.go_type_from_ast(&p.ty))
            .collect();

        if !func.returns.is_empty() {
            // Check if returns are named (first return has non-empty name)
            let is_named = !func.returns[0].ident.name.is_empty();

            if func.returns.len() == 1 && !is_named {
                // Single unnamed return type
                let go_type = self.go_type_from_ast(&func.returns[0].ty);
                self.emit(format!(" {}", go_type));
                self.current_func_return_type = Some(go_type);
            } else {
                // Multi-value return or named returns: (type1, type2, ...) or (x int, y string)
                let returns_str: Vec<String> = func
                    .returns
                    .iter()
                    .map(|p| {
                        let go_type = self.go_type_from_ast(&p.ty);
                        if p.ident.name.is_empty() {
                            go_type
                        } else {
                            format!("{} {}", p.ident.name, go_type)
                        }
                    })
                    .collect();
                self.emit(format!(" ({})", returns_str.join(", ")));
                let types_only: Vec<String> = func
                    .returns
                    .iter()
                    .map(|p| self.go_type_from_ast(&p.ty))
                    .collect();
                self.current_func_return_type = Some(types_only.join(", "));
            }
        } else {
            self.current_func_return_type = None;
        }

        self.emit(" ");

        // Reset error variable counter for this function
        self.reset_error_vars();

        // Body
        self.gen_block(&func.body);

        // Clear return type after function is done
        self.current_func_return_type = None;
        self.current_return_types.clear();

        self.output.push('\n');
    }

    /// Generate a block
    pub(crate) fn gen_block(&mut self, block: &Block) {
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

    /// Generate a pattern for match arms
    pub(crate) fn gen_pattern(&mut self, pattern: &Pattern) {
        match &pattern.kind {
            PatternKind::Variant(name, is_soppo_enum) => {
                if is_soppo_enum.get() {
                    // Soppo enum: convert qualified name to Go type name
                    // Colour.Red → Colour_Red
                    // pkg.Status.Active → pkg.Status_Active
                    self.emit(Self::convert_enum_pattern(name));
                } else {
                    // Go constant: keep as-is (e.g., tar.TypeDir)
                    self.emit(name);
                }
            }
            PatternKind::Literal(lit) => match lit {
                Literal::Integer(n) => self.emit(n.to_string()),
                Literal::String(s) => self.emit(format!("\"{}\"", s)),
                Literal::Bool(b) => self.emit(b.to_string()),
                Literal::Nil => self.emit("nil"),
            },
            PatternKind::Destructor { name, .. } => {
                // Convert qualified name to Go type name
                self.emit(Self::convert_enum_pattern(name));
            }
            PatternKind::StructDestructor { name, .. } => {
                // Convert qualified name to Go type name
                self.emit(Self::convert_enum_pattern(name));
            }
            PatternKind::Guard(expr) => {
                // For expression-less match, emit the boolean expression
                self.gen_expr(expr.as_ref());
            }
            PatternKind::Default => {
                // Default is handled separately, shouldn't reach here
                self.emit("default");
            }
        }
    }

    /// Convert a qualified enum pattern name to Go type name
    /// - `Colour.Red` → `Colour_Red` (soppo enum)
    /// - `pkg.Status.Active` → `pkg.Status_Active` (soppo enum in package)
    /// - `tar.TypeDir` → `tar.TypeDir` (Go constant, unchanged)
    fn convert_enum_pattern(name: &str) -> String {
        let parts: Vec<&str> = name.split('.').collect();
        match parts.len() {
            0 => name.to_string(),
            1 => name.to_string(),
            2 => {
                // Check if first part is a Go package (lowercase) or soppo type (PascalCase)
                // Go packages start with lowercase, soppo types start with uppercase
                let first_char = parts[0].chars().next().unwrap_or('a');
                if first_char.is_lowercase() {
                    // Go constant like tar.TypeDir - keep as-is
                    name.to_string()
                } else {
                    // Soppo enum like Result.Ok → Result_Ok
                    format!("{}_{}", parts[0], parts[1])
                }
            }
            _ => {
                // pkg.Type.Variant or pkg.subpkg.Type.Variant
                // Keep all but last two as package prefix with dots
                // Join last two with underscore
                let prefix = parts[..parts.len() - 2].join(".");
                let type_name = parts[parts.len() - 2];
                let variant = parts[parts.len() - 1];
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
        codegen.gen_file(&file).unwrap();

        let output = codegen.output();
        assert!(output.contains("package main"));
        assert!(output.contains("func add(x int, y int) int"));
        assert!(output.contains("func main() int"));
    }

    #[test]
    fn test_convert_enum_pattern() {
        // Simple Type.Variant
        assert_eq!(Codegen::convert_enum_pattern("Colour.Red"), "Colour_Red");
        assert_eq!(
            Codegen::convert_enum_pattern("Status.Active"),
            "Status_Active"
        );

        // Package-qualified pkg.Type.Variant
        assert_eq!(
            Codegen::convert_enum_pattern("types.Status.Pending"),
            "types.Status_Pending"
        );
        assert_eq!(
            Codegen::convert_enum_pattern("pkg.MyEnum.Variant"),
            "pkg.MyEnum_Variant"
        );

        // Nested package pkg.subpkg.Type.Variant
        assert_eq!(
            Codegen::convert_enum_pattern("foo.bar.Status.Active"),
            "foo.bar.Status_Active"
        );

        // Edge cases
        assert_eq!(Codegen::convert_enum_pattern("Single"), "Single");
        assert_eq!(Codegen::convert_enum_pattern(""), "");
    }
}
