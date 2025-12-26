use std::collections::HashSet;

use super::{Codegen, type_to_go_string};
use crate::syntax::{FileId, LineColumn, Span};
use crate::types::ast::{
    TypedCallArg, TypedConstDecl, TypedDecl, TypedEnumVariant, TypedExpr, TypedExprKind, TypedFile,
    TypedFuncDecl, TypedImportKind, TypedTypeDecl, TypedTypeKind, TypedVarDecl,
};

impl Codegen {
    /// Generate code for an entire typed file
    pub fn gen_file(&mut self, file: &TypedFile) -> crate::error::SoppoResult<()> {
        // Find the earliest position in the file (first import or first declaration)
        let first_line = file
            .imports
            .first()
            .map(|i| i.span.start.line)
            .into_iter()
            .chain(file.decls.first().map(|d| self.get_decl_span(d).start.line))
            .min()
            .unwrap_or(usize::MAX);

        // Separate file-level comments from the rest
        let (file_comments, other_comments): (Vec<_>, Vec<_>) = file
            .comments
            .iter()
            .cloned()
            .partition(|c| c.span.start.line < first_line);

        // Set up only non-file-level comments for emission during declarations
        self.set_comments(other_comments);

        // Generate declarations first to discover needed imports
        let decls_output = self.gen_declarations(file)?;

        // Now build the final output with header, imports, and declarations
        // Soppo generated marker with version - always first
        self.emit_line(&format!("//soppo:generated {}", super::MARKER_VERSION));

        // Emit file-level comments (after marker, before package)
        for comment in &file_comments {
            self.output.push_str(&comment.text);
            self.output.push('\n');
        }

        // Package declaration
        self.emit_line(&format!("package {}", file.package));
        self.emit_line("");

        // Collect explicit import paths
        let explicit_imports: HashSet<_> = file.imports.iter().map(|i| i.path.as_str()).collect();

        // Generate imports (explicit + auto-detected)
        let has_imports = !file.imports.is_empty()
            || self
                .needed_imports
                .iter()
                .any(|i| !explicit_imports.contains(i.as_str()));

        if has_imports {
            // Add auto-detected imports that aren't already explicit
            let auto_imports: Vec<_> = self
                .needed_imports
                .iter()
                .filter(|i| !explicit_imports.contains(i.as_str()))
                .cloned()
                .collect();
            for needed in auto_imports {
                self.emit_line(&format!("import \"{}\"", needed));
            }

            for import in &file.imports {
                self.emit_comments_before(import.span.byte_start, import.span.start.line);

                // Transform Soppo imports if needed
                let go_path = match (&import.kind, &self.module_path, &self.output_dir) {
                    (TypedImportKind::Soppo(_), Some(module_path), Some(out_dir)) => {
                        // Get local path from import
                        if let Some(local_path) =
                            crate::deps::get_local_package_path(&import.path, module_path)
                        {
                            // "." means root package - just use module_path/out_dir
                            if local_path == "." {
                                format!("{}/{}", module_path, out_dir)
                            } else {
                                format!("{}/{}/{}", module_path, out_dir, local_path)
                            }
                        } else {
                            import.path.clone()
                        }
                    }
                    _ => import.path.clone(),
                };

                if let Some(alias) = &import.alias {
                    self.emit_line(&format!("import {} \"{}\"", alias, go_path));
                } else {
                    self.emit_line(&format!("import \"{}\"", go_path));
                }
            }
            self.emit_line("");
        }

        // Append the pre-generated declarations
        self.output.push_str(&decls_output);

        // Emit any remaining comments at the end
        self.emit_remaining_comments();

        Ok(())
    }

    /// Generate a constant declaration
    pub(crate) fn gen_const_decl(&mut self, decl: &TypedConstDecl) {
        // Doc comments are emitted via emit_comments_before in gen_typed_file
        self.emit_indent();
        // Use Go's const for compile-time constant expressions,
        // otherwise use var (immutability is enforced by Soppo)
        if Self::is_go_const_expr(&decl.value) {
            self.emit("const ");
            self.emit(&decl.ident.name);
            if decl.has_explicit_type {
                self.emit(" ");
                self.emit(type_to_go_string(&decl.const_ty));
            }
            self.emit(" = ");
        } else {
            // Runtime value: emit as var (Soppo prevents reassignment)
            self.emit("var ");
            self.emit(&decl.ident.name);
            if decl.has_explicit_type {
                self.emit(" ");
                self.emit(type_to_go_string(&decl.const_ty));
            }
            self.emit(" = ");
        }
        self.gen_expr(&decl.value);
        self.emit("\n");
    }

    /// Generate a variable declaration
    pub(crate) fn gen_var_decl(&mut self, decl: &TypedVarDecl) {
        self.emit_indent();
        self.emit("var ");
        self.emit(&decl.ident.name);
        if decl.has_explicit_type {
            self.emit(" ");
            self.emit(type_to_go_string(&decl.var_ty));
        }
        if let Some(value) = &decl.value {
            self.emit(" = ");
            self.gen_expr(value);
        }
        self.emit("\n");
    }

    /// Generate a type declaration
    pub(crate) fn gen_type_decl(&mut self, decl: &TypedTypeDecl) {
        // Doc comments are emitted via emit_comments_before in gen_typed_file
        match &decl.kind {
            TypedTypeKind::Alias { target } => {
                self.emit_line(&format!(
                    "type {}{} = {}",
                    decl.ident,
                    self.format_generic_brackets(&decl.generics),
                    type_to_go_string(target)
                ));
            }
            TypedTypeKind::Definition { target } => {
                self.emit_line(&format!(
                    "type {}{} {}",
                    decl.ident,
                    self.format_generic_brackets(&decl.generics),
                    type_to_go_string(target)
                ));
            }
            TypedTypeKind::Struct { fields } => {
                self.emit_line(&format!(
                    "type {}{} struct {{",
                    decl.ident,
                    self.format_generic_brackets(&decl.generics)
                ));
                self.indent();
                for (name, ty, tag, _attrs, is_const) in fields {
                    let tag_str = tag.as_ref().map(|t| format!("`{}`", t)).unwrap_or_default();
                    // Build soppo markers (comma-separated if multiple)
                    let mut markers = Vec::new();
                    if ty.is_nullable() {
                        markers.push("nilable");
                    }
                    if *is_const {
                        markers.push("const");
                    }
                    let soppo_comment = if markers.is_empty() {
                        String::new()
                    } else {
                        format!(" //soppo:{}", markers.join(","))
                    };
                    let go_type = type_to_go_string(ty);
                    // Check if this is an anonymous struct type (needs multiline formatting)
                    if go_type.starts_with("*struct { ") && go_type.ends_with(" }") {
                        self.emit_struct_field_with_anon_struct(
                            name,
                            &go_type,
                            &tag_str,
                            &soppo_comment,
                        );
                    } else {
                        let tag_with_space = if tag_str.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", tag_str)
                        };
                        self.emit_line(&format!(
                            "{} {}{}{}",
                            name, go_type, tag_with_space, soppo_comment
                        ));
                    }
                }
                self.dedent();
                self.emit_line("}");
            }
            TypedTypeKind::Enum { variants } => {
                // Generate enum as interface + variant structs
                let generic_params = self.format_generic_brackets(&decl.generics);
                let generic_names = self.format_generic_name_brackets(&decl.generics);

                // Emit soppo:enum marker comment for Go tools to recognize this as a Soppo enum
                self.emit_soppo_enum_marker(decl, variants);

                // Interface type
                self.emit_line(&format!(
                    "type {}{} interface {{",
                    decl.ident, generic_params
                ));
                self.indent();
                self.emit_line(&format!("is{}()", decl.ident));
                self.dedent();
                self.emit_line("}");
                self.emit_line("");

                // Variant structs
                for variant in variants {
                    let variant_ident = match variant {
                        TypedEnumVariant::Unit { ident, .. } => ident,
                        TypedEnumVariant::Single { ident, .. } => ident,
                        TypedEnumVariant::Struct { ident, .. } => ident,
                    };
                    let full_name = format!("{}_{}", decl.ident, variant_ident);

                    match variant {
                        TypedEnumVariant::Unit { .. } => {
                            self.emit_line(&format!(
                                "type {}{} struct {{}}",
                                full_name, generic_params
                            ));
                        }
                        TypedEnumVariant::Single { ty, .. } => {
                            self.emit_line(&format!(
                                "type {}{} struct {{",
                                full_name, generic_params
                            ));
                            self.indent();
                            self.emit_line(&format!("Value {}", type_to_go_string(ty)));
                            self.dedent();
                            self.emit_line("}");
                        }
                        TypedEnumVariant::Struct { fields, .. } => {
                            self.emit_line(&format!(
                                "type {}{} struct {{",
                                full_name, generic_params
                            ));
                            self.indent();
                            for (name, ty) in fields {
                                self.emit_line(&format!("{} {}", name, type_to_go_string(ty)));
                            }
                            self.dedent();
                            self.emit_line("}");
                        }
                    }

                    // Interface implementation
                    self.emit_line(&format!(
                        "func ({}{}) is{}() {{}}",
                        full_name, generic_names, decl.ident
                    ));

                    // String() method for unit variants
                    if matches!(variant, TypedEnumVariant::Unit { .. }) {
                        self.emit_line(&format!(
                            "func ({}{}) String() string {{ return \"{}\" }}",
                            full_name, generic_names, variant_ident
                        ));
                    }

                    self.emit_line("");
                }

                // Generate var block for non-generic unit variants
                let unit_variants: Vec<_> = variants
                    .iter()
                    .filter(|v| matches!(v, TypedEnumVariant::Unit { .. }))
                    .collect();

                if !unit_variants.is_empty() && decl.generics.is_empty() {
                    self.emit_line("var (");
                    self.indent();
                    for variant in unit_variants {
                        if let TypedEnumVariant::Unit { ident, .. } = variant {
                            let type_name = format!("{}_{}", decl.ident, ident);
                            let var_name = format!("{}{}", decl.ident, ident);
                            self.emit_line(&format!(
                                "{} {} = {}{{}}",
                                var_name, decl.ident, type_name
                            ));
                        }
                    }
                    self.dedent();
                    self.emit_line(")");
                }

                // Generate constructor functions for variants with data
                for variant in variants {
                    match variant {
                        TypedEnumVariant::Single { ident, ty, .. } => {
                            let func_name = format!("{}{}", decl.ident, ident);
                            let type_name = format!("{}_{}", decl.ident, ident);
                            self.emit_line(&format!(
                                "func {}{}(value {}) {}{} {{",
                                func_name,
                                generic_params,
                                type_to_go_string(ty),
                                decl.ident,
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
                        TypedEnumVariant::Unit { ident, .. } if !decl.generics.is_empty() => {
                            // Generic unit variants need constructor functions
                            let func_name = format!("{}{}", decl.ident, ident);
                            let type_name = format!("{}_{}", decl.ident, ident);
                            self.emit_line(&format!(
                                "func {}{}() {}{} {{",
                                func_name, generic_params, decl.ident, generic_names
                            ));
                            self.indent();
                            self.emit_line(&format!("return {}{}{{}}", type_name, generic_names));
                            self.dedent();
                            self.emit_line("}");
                        }
                        _ => {}
                    }
                }
            }
            TypedTypeKind::Interface { methods } => {
                self.emit_line(&format!(
                    "type {}{} interface {{",
                    decl.ident,
                    self.format_generic_brackets(&decl.generics)
                ));
                self.indent();
                for method in methods {
                    let params: Vec<String> = method
                        .params
                        .iter()
                        .map(|p| format!("{} {}", p.ident.name, type_to_go_string(&p.ty)))
                        .collect();
                    let returns: Vec<String> =
                        method.returns.iter().map(type_to_go_string).collect();
                    let ret_str = if returns.is_empty() {
                        String::new()
                    } else if returns.len() == 1 {
                        format!(" {}", returns[0])
                    } else {
                        format!(" ({})", returns.join(", "))
                    };
                    self.emit_line(&format!(
                        "{}({}){}",
                        method.ident.name,
                        params.join(", "),
                        ret_str
                    ));
                }
                self.dedent();
                self.emit_line("}");
            }
        }
    }

    /// Generate a function declaration
    pub(crate) fn gen_func_decl(&mut self, decl: &TypedFuncDecl) {
        self.reset_error_vars();

        // Emit //soppo:nilable comment if any parameters or return types are nullable
        // Format: //soppo:nilable p q : 0 1
        //         (params before :, return indices after :)
        let nilable_params: Vec<&str> = decl
            .params
            .iter()
            .filter(|p| p.nullable)
            .map(|p| p.ident.name.as_str())
            .collect();
        let nilable_returns: Vec<String> = decl
            .returns
            .iter()
            .enumerate()
            .filter(|(_, p)| p.nullable)
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

        // Doc comments are emitted via emit_comments_before in gen_typed_file
        self.emit_indent();
        self.emit("func ");

        // Receiver
        if let Some(receiver) = &decl.receiver {
            self.emit("(");
            self.emit(&receiver.ident.name);
            self.emit(" ");
            let recv_ty = type_to_go_string(&receiver.ty);
            self.emit(self.go_receiver_type(&recv_ty));
            self.emit(") ");
        }

        // Function name and generics
        self.emit(&decl.ident.name);
        self.emit(self.format_generic_brackets(&decl.generics));

        // Parameters
        self.emit("(");
        for (i, param) in decl.params.iter().enumerate() {
            if i > 0 {
                self.emit(", ");
            }
            self.emit(&param.ident.name);
            self.emit(" ");
            self.emit(type_to_go_string(&param.ty));
        }
        self.emit(")");

        // Set up return types for try statement expansion
        self.current_return_types = decl
            .returns
            .iter()
            .map(|p| type_to_go_string(&p.ty))
            .collect();

        // Return types
        if !decl.returns.is_empty() {
            let is_named = !decl.returns[0].ident.name.is_empty();
            if decl.returns.len() == 1 && !is_named {
                self.emit(" ");
                self.emit(type_to_go_string(&decl.returns[0].ty));
            } else {
                self.emit(" (");
                for (i, ret) in decl.returns.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    if !ret.ident.name.is_empty() {
                        self.emit(&ret.ident.name);
                        self.emit(" ");
                    }
                    self.emit(type_to_go_string(&ret.ty));
                }
                self.emit(")");
            }
        }

        self.emit(" ");
        self.gen_block(&decl.body);
        self.emit("\n");

        // Clear return types after function
        self.current_return_types.clear();
    }

    /// Generate a declaration
    pub(crate) fn gen_decl(&mut self, decl: &TypedDecl) {
        match decl {
            TypedDecl::Const(c) => self.gen_const_decl(c),
            TypedDecl::ConstBlock(consts) => {
                self.emit_line("const (");
                self.indent();
                for (i, c) in consts.iter().enumerate() {
                    self.emit_indent();
                    self.emit(&c.ident.name);
                    if c.has_explicit_type {
                        self.emit(" ");
                        self.emit(type_to_go_string(&c.const_ty));
                        self.emit(" = ");
                        self.gen_expr(&c.value);
                    } else {
                        // Check if value is just "iota" - for continuation lines, skip emitting it
                        let is_iota =
                            matches!(&c.value.kind, TypedExprKind::Ident(name) if name == "iota");
                        if !is_iota || i == 0 {
                            self.emit(" = ");
                            self.gen_expr(&c.value);
                        }
                    }
                    self.emit("\n");
                }
                self.dedent();
                self.emit_line(")");
            }
            TypedDecl::Var(v) => self.gen_var_decl(v),
            TypedDecl::Type(t) => self.gen_type_decl(t),
            TypedDecl::Func(f) => self.gen_func_decl(f),
        }
    }

    /// Generate typed declarations to a separate buffer (to discover needed imports first)
    fn gen_declarations(&mut self, file: &TypedFile) -> crate::error::SoppoResult<String> {
        let saved_output = std::mem::take(&mut self.output);

        for decl in &file.decls {
            let span = self.get_decl_span(decl);
            self.emit_comments_before(span.byte_start, span.start.line);
            self.gen_decl(decl);
            self.emit_line("");
        }

        // Generate init() function for attribute registration if there are non-builtin attributes
        self.gen_attr_init(file)?;

        let decls = std::mem::replace(&mut self.output, saved_output);
        Ok(decls)
    }

    /// Builtin attributes that the compiler handles directly (no runtime registration)
    const BUILTIN_ATTRS: &'static [&'static str] = &["MustUse", "Deprecated"];

    /// Check if an attribute is a builtin (handled by compiler, not registered at runtime)
    fn is_builtin_attr(name: &str) -> bool {
        Self::BUILTIN_ATTRS.contains(&name)
    }

    /// Generate init() function for attribute registration
    fn gen_attr_init(&mut self, file: &TypedFile) -> crate::error::SoppoResult<()> {
        // Collect all non-builtin attributes from functions and types
        let mut registrations: Vec<(String, String, String)> = Vec::new();
        // Collect enum variant registrations: (enumTarget, variantName, valueType)
        let mut enum_variants: Vec<(String, String, String)> = Vec::new();

        for decl in &file.decls {
            match decl {
                TypedDecl::Func(func) => {
                    for attr in &func.attributes {
                        if !Self::is_builtin_attr(&attr.name) {
                            // Function: target is "pkg.FuncName", field is ""
                            let target = format!("{}.{}", file.package, func.ident.name);
                            let attr_literal = self.format_attr_literal(attr)?;
                            registrations.push((target, String::new(), attr_literal));
                        }
                    }
                }
                TypedDecl::Type(type_decl) => {
                    for attr in &type_decl.attributes {
                        if !Self::is_builtin_attr(&attr.name) {
                            // Type: target is "pkg.TypeName", field is ""
                            let target = format!("{}.{}", file.package, type_decl.ident.name);
                            let attr_literal = self.format_attr_literal(attr)?;
                            registrations.push((target, String::new(), attr_literal));
                        }
                    }

                    // Struct field attributes
                    if let TypedTypeKind::Struct { fields } = &type_decl.kind {
                        for (field_name, _, _, attrs, _) in fields {
                            for attr in attrs {
                                if !Self::is_builtin_attr(&attr.name) {
                                    // Field: target is "pkg.TypeName", field is "FieldName"
                                    let target =
                                        format!("{}.{}", file.package, type_decl.ident.name);
                                    let attr_literal = self.format_attr_literal(attr)?;
                                    registrations.push((target, field_name.clone(), attr_literal));
                                }
                            }
                        }
                    }

                    // Enum variant attributes and registration
                    if let TypedTypeKind::Enum { variants } = &type_decl.kind {
                        let enum_target = format!("{}.{}", file.package, type_decl.ident.name);

                        // Check if enum has Subcommands attribute (for slap CLI)
                        let has_subcommands_attr = type_decl
                            .attributes
                            .iter()
                            .any(|a| a.name == "Subcommands" || a.name.ends_with(".Subcommands"));

                        for variant in variants {
                            let (variant_name, attrs, value_type) = match variant {
                                TypedEnumVariant::Unit { ident, attributes } => (
                                    ident.name.clone(),
                                    attributes,
                                    format!("{}_{}{{}}", type_decl.ident.name, ident.name),
                                ),
                                TypedEnumVariant::Single {
                                    ident, attributes, ..
                                } => (
                                    ident.name.clone(),
                                    attributes,
                                    format!("{}_{}{{}}", type_decl.ident.name, ident.name),
                                ),
                                TypedEnumVariant::Struct {
                                    ident, attributes, ..
                                } => (
                                    ident.name.clone(),
                                    attributes,
                                    format!("{}_{}{{}}", type_decl.ident.name, ident.name),
                                ),
                            };

                            // Only register enum variant if enum has Subcommands attribute
                            if has_subcommands_attr {
                                enum_variants.push((
                                    enum_target.clone(),
                                    variant_name.clone(),
                                    value_type,
                                ));
                            }

                            for attr in attrs {
                                if !Self::is_builtin_attr(&attr.name) {
                                    // Variant: target is "pkg.EnumName", field is "VariantName"
                                    let attr_literal = self.format_attr_literal(attr)?;
                                    registrations.push((
                                        enum_target.clone(),
                                        variant_name.clone(),
                                        attr_literal,
                                    ));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // If no registrations, skip init() generation
        if registrations.is_empty() && enum_variants.is_empty() {
            return Ok(());
        }

        // Add runtime import
        self.needed_imports
            .insert("github.com/halcyonnouveau/soppo/runtime".to_string());

        // Generate init() function
        self.emit_line("func init() {");
        self.indent();
        for (target, field, attr_literal) in registrations {
            self.emit_indent();
            self.emit(format!(
                "runtime.RegisterAttr(\"{}\", \"{}\", {})\n",
                target, field, attr_literal
            ));
        }
        for (enum_target, variant_name, value_type) in enum_variants {
            self.emit_indent();
            self.emit(format!(
                "runtime.RegisterAttr(\"{}\", \"{}\", runtime.EnumVariant{{WrapperType: {}}})\n",
                enum_target, variant_name, value_type
            ));
        }
        self.dedent();
        self.emit_line("}");

        Ok(())
    }

    /// Format an attribute as a Go struct literal
    fn format_attr_literal(
        &mut self,
        attr: &crate::syntax::Attribute,
    ) -> crate::error::SoppoResult<String> {
        if attr.args.is_empty() {
            Ok(format!("{}{{}}", attr.name))
        } else {
            let mut args = Vec::new();
            for (k, v) in &attr.args {
                let expr_str = self.format_expr_to_string(v)?;
                args.push(format!("{}: {}", k, expr_str));
            }
            Ok(format!("{}{{{}}}", attr.name, args.join(", ")))
        }
    }

    /// Format an expression to a Go string (for attribute arguments)
    fn format_expr_to_string(
        &mut self,
        expr: &crate::syntax::Expr,
    ) -> crate::error::SoppoResult<String> {
        // Save current output, generate expr, then restore
        let saved = std::mem::take(&mut self.output);
        // Create a typed expr for the literal values
        let kind = self.expr_kind_to_typed(&expr.kind, expr.span)?;
        let typed = crate::types::ast::TypedExpr {
            kind,
            ty: crate::types::Type::simple("any"),
            span: expr.span,
        };
        self.gen_expr(&typed);
        let result = std::mem::replace(&mut self.output, saved);
        Ok(result)
    }

    /// Convert untyped ExprKind to TypedExprKind (for attribute arguments)
    fn expr_kind_to_typed(
        &self,
        kind: &crate::syntax::ExprKind,
        span: Span,
    ) -> crate::error::SoppoResult<crate::types::ast::TypedExprKind> {
        match kind {
            crate::syntax::ExprKind::Integer(n, fmt) => {
                Ok(crate::types::ast::TypedExprKind::Integer(*n, *fmt))
            }
            crate::syntax::ExprKind::Float(f) => Ok(crate::types::ast::TypedExprKind::Float(*f)),
            crate::syntax::ExprKind::String(s) => {
                Ok(crate::types::ast::TypedExprKind::String(s.clone()))
            }
            crate::syntax::ExprKind::Bool(b) => Ok(crate::types::ast::TypedExprKind::Bool(*b)),
            crate::syntax::ExprKind::Ident(name) => {
                Ok(crate::types::ast::TypedExprKind::Ident(name.clone()))
            }
            crate::syntax::ExprKind::Field {
                expr: inner,
                field,
                span: field_span,
            } => {
                // Check if this is an enum variant like Method.Get
                if let crate::syntax::ExprKind::Ident(type_name) = &inner.kind
                    && self.global_state.is_enum(type_name)
                {
                    // Enum variant: Method.Get -> MethodGet
                    let variant_name = format!("{}{}", type_name, field);
                    return Ok(crate::types::ast::TypedExprKind::Ident(variant_name));
                }

                // Check if this is a cross-package enum variant like pkg.Method.Get
                if let crate::syntax::ExprKind::Field {
                    expr: pkg_expr,
                    field: type_name,
                    ..
                } = &inner.kind
                    && let crate::syntax::ExprKind::Ident(pkg_name) = &pkg_expr.kind
                    && self.global_state.is_soppo_enum(pkg_name, type_name)
                {
                    // Cross-package enum: pkg.Method.Get -> pkg.MethodGet
                    let variant_name = format!("{}{}", type_name, field);
                    let pkg_typed = Box::new(crate::types::ast::TypedExpr {
                        kind: crate::types::ast::TypedExprKind::Ident(pkg_name.clone()),
                        ty: crate::types::Type::simple("package"),
                        span: pkg_expr.span,
                    });
                    return Ok(crate::types::ast::TypedExprKind::Field {
                        expr: pkg_typed,
                        field: variant_name,
                        span: *field_span,
                    });
                }

                // Qualified access like pkg.Const
                let inner_kind = self.expr_kind_to_typed(&inner.kind, inner.span)?;
                let inner_typed = Box::new(crate::types::ast::TypedExpr {
                    kind: inner_kind,
                    ty: crate::types::Type::simple("any"),
                    span: inner.span,
                });
                Ok(crate::types::ast::TypedExprKind::Field {
                    expr: inner_typed,
                    field: field.clone(),
                    span: *field_span,
                })
            }
            crate::syntax::ExprKind::Unary { op, operand } => {
                // Unary like -1
                let inner_kind = self.expr_kind_to_typed(&operand.kind, operand.span)?;
                let inner_typed = Box::new(crate::types::ast::TypedExpr {
                    kind: inner_kind,
                    ty: crate::types::Type::simple("any"),
                    span: operand.span,
                });
                Ok(crate::types::ast::TypedExprKind::Unary {
                    op: *op,
                    operand: inner_typed,
                })
            }
            _ => Err(crate::error::SoppoError::Type {
                message: "unsupported expression in attribute argument".to_string(),
                span,
            }),
        }
    }

    /// Get the span of a typed declaration
    fn get_decl_span(&self, decl: &TypedDecl) -> crate::syntax::Span {
        let zero_span = Span {
            byte_start: 0,
            byte_end: 0,
            start: LineColumn { line: 0, col: 0 },
            end: LineColumn { line: 0, col: 0 },
            file: FileId(0),
        };
        match decl {
            TypedDecl::Const(c) => c.span,
            TypedDecl::ConstBlock(consts) => consts.first().map(|c| c.span).unwrap_or(zero_span),
            TypedDecl::Var(v) => v.span,
            TypedDecl::Type(t) => t.span,
            TypedDecl::Func(f) => f.span,
        }
    }

    /// Reorder function call arguments based on named arguments
    /// Named args are placed in their designated positions, positional args fill remaining slots
    pub(crate) fn reorder_call_args<'a>(
        &self,
        func: &TypedExpr,
        args: &'a [TypedCallArg],
    ) -> Vec<(&'a TypedExpr, bool)> {
        let has_named = args.iter().any(|(name, _, _)| name.is_some());

        // If no named args, just return all in order
        if !has_named {
            return args.iter().map(|(_, arg, spread)| (arg, *spread)).collect();
        }

        // Look up parameter names (exclude variadic params)
        let param_names: Option<Vec<String>> = if let TypedExprKind::Ident(func_name) = &func.kind {
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
            // - Named args reserve their specific slots first
            // - Positional args fill remaining slots in order
            // - Extra positional args go to variadic
            let mut result: Vec<Option<(&TypedExpr, bool)>> = vec![None; param_names.len()];
            let mut variadic_args: Vec<(&TypedExpr, bool)> = Vec::new();
            let mut positional_args: Vec<(&TypedExpr, bool)> = Vec::new();

            // First pass: process named args to reserve slots, collect positional args
            for (name, arg, spread) in args {
                match name {
                    Some((n, _)) => {
                        if let Some(idx) = param_names.iter().position(|p| p == n) {
                            result[idx] = Some((arg, *spread));
                        }
                    }
                    None => {
                        positional_args.push((arg, *spread));
                    }
                }
            }

            // Second pass: fill remaining slots with positional args
            let mut positional_iter = positional_args.into_iter();
            for slot in result.iter_mut() {
                if slot.is_none()
                    && let Some(arg) = positional_iter.next()
                {
                    *slot = Some(arg);
                }
            }

            // Any remaining positional args go to variadic
            variadic_args.extend(positional_iter);

            // Collect results (type checker already validated all are filled)
            let mut ordered: Vec<(&TypedExpr, bool)> = result.into_iter().flatten().collect();

            // Add variadic args at the end
            ordered.extend(variadic_args);

            ordered
        } else {
            // Unknown function - just use positional order (type checker would have errored)
            args.iter().map(|(_, arg, spread)| (arg, *spread)).collect()
        }
    }

    /// Emit a soppo:enum marker block comment for an enum type
    fn emit_soppo_enum_marker(&mut self, decl: &TypedTypeDecl, variants: &[TypedEnumVariant]) {
        // Format: /*soppo:enum\nEnumName[T, E] {\n    Ok T\n    Err E\n}\n*/
        self.emit_line("/*soppo:enum");

        // Enum name with generics (Soppo-style, just names)
        let generic_names = self.format_generic_name_brackets(&decl.generics);
        self.emit_line(&format!("{}{} {{", decl.ident, generic_names));

        // Variants
        for variant in variants {
            match variant {
                TypedEnumVariant::Unit { ident, .. } => {
                    self.emit_line(&format!("    {}", ident));
                }
                TypedEnumVariant::Single { ident, ty, .. } => {
                    self.emit_line(&format!("    {} {}", ident, type_to_go_string(ty)));
                }
                TypedEnumVariant::Struct { ident, fields, .. } => {
                    self.emit_line(&format!("    {} {{", ident));
                    for (name, ty) in fields {
                        self.emit_line(&format!("        {} {}", name, type_to_go_string(ty)));
                    }
                    self.emit_line("    }");
                }
            }
        }

        self.emit_line("}");
        self.emit_line("*/");
    }
}
