mod decl;
mod expr;
mod stmt;

use std::collections::HashSet;
use std::path::PathBuf;

use crate::error::Result;
use crate::syntax::{AssignOp, BinOp, Comment, Decl, EnumVariant, File, Generic, TypeDecl};
use crate::types::GlobalCtxt;

/// Code generator for emitting Go code
pub struct Codegen {
    pub(crate) output: String,
    indent_level: usize,
    pub(crate) global_state: GlobalCtxt,
    pub(crate) current_func_return_type: Option<String>,
    /// Current function's return types (for ? operator zero value generation)
    pub(crate) current_return_types: Vec<String>,
    /// Counter for generating unique error variable names (_err0, _err1, etc.)
    pub(crate) error_var_counter: usize,
    comments: Vec<Comment>,
    comment_idx: usize,
    /// Go module path for resolving local imports (e.g., "github.com/user/project")
    module_path: Option<String>,
    /// Output directory relative to project root (e.g., "gen")
    output_dir: Option<String>,
    /// Project root for checking if imports are Soppo packages
    project_root: Option<PathBuf>,
    /// Imports needed by generated code (e.g., "fmt" for string interpolation)
    pub(crate) needed_imports: HashSet<String>,
}

impl Codegen {
    /// Create base Codegen with given global state
    fn base(global_state: GlobalCtxt) -> Self {
        Self {
            output: String::new(),
            indent_level: 0,
            global_state,
            current_func_return_type: None,
            current_return_types: Vec::new(),
            error_var_counter: 0,
            comments: Vec::new(),
            comment_idx: 0,
            module_path: None,
            output_dir: None,
            project_root: None,
            needed_imports: HashSet::new(),
        }
    }

    pub fn new() -> Self {
        Self::base(GlobalCtxt::new())
    }

    pub fn with_global_state(global_state: GlobalCtxt) -> Self {
        Self::base(global_state)
    }

    /// Create codegen with module info for resolving local Soppo imports
    pub fn with_module_info(
        global_state: GlobalCtxt,
        module_path: String,
        output_dir: Option<String>,
        project_root: PathBuf,
    ) -> Self {
        Self {
            module_path: Some(module_path),
            output_dir,
            project_root: Some(project_root),
            ..Self::base(global_state)
        }
    }

    /// Generate a fresh error variable name (_err0, _err1, etc.)
    pub(crate) fn fresh_error_var(&mut self) -> String {
        let name = format!("_err{}", self.error_var_counter);
        self.error_var_counter += 1;
        name
    }

    /// Reset the error variable counter (called at start of each function)
    pub(crate) fn reset_error_vars(&mut self) {
        self.error_var_counter = 0;
    }

    /// Generate zero value for a type
    pub(crate) fn zero_value(&self, ty: &str) -> String {
        match ty {
            "int" | "int8" | "int16" | "int32" | "int64" => "0".to_string(),
            "uint" | "uint8" | "uint16" | "uint32" | "uint64" | "uintptr" | "byte" | "rune" => {
                "0".to_string()
            }
            "float32" | "float64" => "0".to_string(),
            "bool" => "false".to_string(),
            "string" => "\"\"".to_string(),
            "" | "()" => "".to_string(), // unit type
            _ if ty.starts_with('*')
                || ty.starts_with("[]")
                || ty.starts_with("map[")
                || ty.starts_with("chan ")
                || ty == "error"
                || ty.starts_with("func(") =>
            {
                // Pointers, slices, maps, channels, error, functions -> nil
                "nil".to_string()
            }
            _ => {
                // Struct types -> TypeName{}
                format!("{}{{}}", ty)
            }
        }
    }

    /// Set comments for the codegen (sorted by byte position)
    pub(crate) fn set_comments(&mut self, mut comments: Vec<Comment>) {
        comments.sort_by_key(|c| c.span.byte_start);
        self.comments = comments;
        self.comment_idx = 0;
    }

    /// Emit all comments that appear before the given position
    /// Only emits comments that are on lines strictly before the given line
    pub(crate) fn emit_comments_before(&mut self, byte_pos: usize, line: usize) {
        while self.comment_idx < self.comments.len() {
            let comment = &self.comments[self.comment_idx];
            // Only emit if comment is before the byte position AND on an earlier line
            // (comments on the same line are trailing comments, handled separately)
            if comment.span.byte_start < byte_pos && comment.span.start.line < line {
                let text = comment.text.clone();
                self.emit_indent();
                self.output.push_str(&text);
                self.output.push('\n');
                self.comment_idx += 1;
            } else {
                break;
            }
        }
    }

    /// Emit all remaining comments
    pub(crate) fn emit_remaining_comments(&mut self) {
        while self.comment_idx < self.comments.len() {
            self.emit_indent();
            self.output.push_str(&self.comments[self.comment_idx].text);
            self.output.push('\n');
            self.comment_idx += 1;
        }
    }

    /// Emit a trailing comment if there's one on the given line, returns true if a comment was emitted
    pub(crate) fn emit_trailing_comment(&mut self, line: usize) -> bool {
        if self.comment_idx < self.comments.len() {
            let comment = &self.comments[self.comment_idx];
            // Check if comment is on the same line (trailing comment)
            if comment.span.start.line == line && !comment.is_block {
                self.output.push(' ');
                self.output.push_str(&comment.text);
                self.comment_idx += 1;
                return true;
            }
        }
        false
    }

    /// Get the generated output
    pub fn output(&self) -> &str {
        &self.output
    }

    /// Emit a line with current indentation
    pub(crate) fn emit_line(&mut self, line: &str) {
        self.emit_indent();
        self.output.push_str(line);
        self.output.push('\n');
    }

    /// Emit text without newline
    pub(crate) fn emit(&mut self, text: impl AsRef<str>) {
        self.output.push_str(text.as_ref());
    }

    /// Emit current indentation
    pub(crate) fn emit_indent(&mut self) {
        for _ in 0..self.indent_level {
            self.output.push_str("    ");
        }
    }

    /// Increase indentation level
    pub(crate) fn indent(&mut self) {
        self.indent_level += 1;
    }

    /// Decrease indentation level
    pub(crate) fn dedent(&mut self) {
        if self.indent_level > 0 {
            self.indent_level -= 1;
        }
    }

    /// Format generic parameters with constraints: "T any, E any"
    pub(crate) fn format_generic_params(&self, generics: &[Generic]) -> String {
        generics
            .iter()
            .map(|g| format!("{} {}", g.ident, g.constraint))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Format generic parameter names only: "T, E"
    pub(crate) fn format_generic_names(&self, generics: &[Generic]) -> String {
        generics
            .iter()
            .map(|g| g.ident.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Format generic parameters in brackets if not empty: "[T any, E any]" or ""
    pub(crate) fn format_generic_brackets(&self, generics: &[Generic]) -> String {
        if generics.is_empty() {
            String::new()
        } else {
            format!("[{}]", self.format_generic_params(generics))
        }
    }

    /// Format generic names in brackets if not empty: "[T, E]" or ""
    pub(crate) fn format_generic_name_brackets(&self, generics: &[Generic]) -> String {
        if generics.is_empty() {
            String::new()
        } else {
            format!("[{}]", self.format_generic_names(generics))
        }
    }

    /// Emit a soppo:enum marker block comment for an enum type
    pub(crate) fn emit_soppo_enum_marker(
        &mut self,
        type_decl: &TypeDecl,
        variants: &[EnumVariant],
    ) {
        // Format: /*soppo:enum\nEnumName[T, E] {\n    Ok T\n    Err E\n}\n*/
        self.emit_line("/*soppo:enum");

        // Enum name with generics (Soppo-style, just names)
        let generic_names = self.format_generic_name_brackets(&type_decl.generics);
        self.emit_line(&format!("{}{} {{", type_decl.ident, generic_names));

        // Variants
        for variant in variants {
            match variant {
                EnumVariant::Unit { ident: name, .. } => {
                    self.emit_line(&format!("    {}", name));
                }
                EnumVariant::Single {
                    ident: name, ty, ..
                } => {
                    self.emit_line(&format!("    {} {}", name, self.go_type(&ty.name)));
                }
                EnumVariant::Struct {
                    ident: name,
                    fields,
                    ..
                } => {
                    self.emit_line(&format!("    {} {{", name));
                    for field in fields {
                        let tag = field
                            .tag
                            .as_ref()
                            .map(|t| format!(" `{}`", t))
                            .unwrap_or_default();
                        self.emit_line(&format!(
                            "        {} {}{}",
                            field.ident,
                            self.go_type(&field.ty.name),
                            tag
                        ));
                    }
                    self.emit_line("    }");
                }
            }
        }

        self.emit_line("}");
        self.emit_line("*/");
    }

    /// Generate code for an entire file
    pub fn gen_file(&mut self, file: &File) -> Result<()> {
        // Find the earliest position in the file (first import or first declaration)
        let first_line = file
            .imports
            .first()
            .map(|i| i.span.start.line)
            .into_iter()
            .chain(file.decls.first().map(|d| d.span().start.line))
            .min()
            .unwrap_or(usize::MAX);

        // Separate file-level comments from the rest
        let (file_comments, other_comments): (Vec<_>, Vec<_>) = file
            .comments
            .iter()
            .cloned()
            .partition(|c| c.span.start.line < first_line);

        // Set up only non-file-level comments for emission during gen_declarations
        self.set_comments(other_comments);

        // Generate declarations first to discover needed imports
        let decls_output = self.gen_declarations(file)?;

        // Now build the final output with header, imports, and declarations
        // Soppo generated marker - always first
        self.emit_line("//soppo:generated");

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

                // Check if this is a local Soppo import that needs transformation
                // Local Soppo imports: github.com/user/project/helpers -> github.com/user/project/gen/helpers
                let is_soppo = match (&self.module_path, &self.project_root) {
                    (Some(module_path), Some(project_root)) => {
                        crate::deps::is_soppo_import(&import.path, module_path, project_root)
                    }
                    _ => false,
                };

                let go_path = if is_soppo {
                    let module_path = self.module_path.as_ref().unwrap();
                    // Get the local path portion (e.g., "helpers" from "github.com/user/project/helpers")
                    let local_path =
                        crate::deps::get_local_package_path(&import.path, module_path).unwrap();

                    // Build Go import path: {module_path}/{output_dir}/{local_path}
                    match &self.output_dir {
                        Some(out_dir) => {
                            format!("{}/{}/{}", module_path, out_dir, local_path)
                        }
                        None => {
                            // No output_dir, keep original path
                            import.path.clone()
                        }
                    }
                } else {
                    // Go import - keep as-is
                    import.path.clone()
                };

                // Generate with alias if present
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

    /// Generate declarations to a separate buffer (to discover needed imports first)
    fn gen_declarations(&mut self, file: &File) -> Result<String> {
        let saved_output = std::mem::take(&mut self.output);

        for decl in &file.decls {
            match decl {
                Decl::Const(const_decl) => {
                    self.emit_comments_before(
                        const_decl.span.byte_start,
                        const_decl.span.start.line,
                    );
                    self.gen_const_decl(const_decl);
                    self.emit_line("");
                }
                Decl::ConstBlock(consts) => {
                    if let Some(first) = consts.first() {
                        self.emit_comments_before(first.span.byte_start, first.span.start.line);
                    }
                    self.gen_const_block(consts);
                    self.emit_line("");
                }
                Decl::Var(var_decl) => {
                    self.emit_comments_before(var_decl.span.byte_start, var_decl.span.start.line);
                    self.gen_var_decl(var_decl);
                    self.emit_line("");
                }
                Decl::Type(type_decl) => {
                    self.emit_comments_before(type_decl.span.byte_start, type_decl.span.start.line);
                    self.gen_type_decl(type_decl);
                    self.emit_line("");
                }
                Decl::Func(func) => {
                    self.emit_comments_before(func.span.byte_start, func.span.start.line);
                    self.gen_func_decl(func);
                    self.emit_line("");
                }
            }
        }

        let decls = std::mem::replace(&mut self.output, saved_output);
        Ok(decls)
    }

    /// Convert Soppo type to Go type
    pub(crate) fn go_type<'a>(&self, ty: &'a str) -> &'a str {
        match ty {
            "()" => "", // Unit type
            _ => ty,
        }
    }

    /// Convert Soppo type to Go type, stripping the ? prefix for nullable types
    /// Go doesn't have nullable syntax - nullability is tracked by Soppo's type checker
    pub(crate) fn go_type_from_ast(&self, ty: &crate::syntax::TypeAnnotation) -> String {
        let name = &ty.name;
        // Strip ? prefix if present (nullable marker in Soppo syntax)
        let go_name = name.strip_prefix('?').unwrap_or(name);
        match go_name {
            "()" => String::new(), // Unit type
            _ => go_name.to_string(),
        }
    }

    /// Convert enum variant type syntax (EnumName.Variant) to Go struct name (EnumName_Variant)
    /// Handles pointer types like *EnumName.Variant -> *EnumName_Variant
    pub(crate) fn go_receiver_type(&self, ty: &str) -> String {
        // Handle pointer prefix
        let (prefix, base) = if let Some(rest) = ty.strip_prefix('*') {
            ("*", rest)
        } else {
            ("", ty)
        };

        // Convert EnumName.Variant to EnumName_Variant
        let converted = base.replace('.', "_");

        format!("{}{}", prefix, converted)
    }

    /// Generate nilable comment if the type is nullable
    /// Returns " //soppo:nilable" if nullable, empty string otherwise
    pub(crate) fn nilable_comment(&self, ty: &crate::syntax::TypeAnnotation) -> &'static str {
        if ty.nullable { " //soppo:nilable" } else { "" }
    }

    /// Emit a struct field with an anonymous struct type, formatting it as multiline
    /// Input: go_type = "*struct { bio string; ptr ?*int }"
    /// Output (if outer type is nilable):
    ///   field_name *struct { //soppo:nilable
    ///       bio string
    ///       ptr *int //soppo:nilable
    ///   }
    pub(crate) fn emit_struct_field_with_anon_struct(
        &mut self,
        field_name: impl AsRef<str>,
        go_type: &str,
        tag: &str,
        nilable_comment: &str,
    ) {
        // Extract struct body from "*struct { ... }"
        let struct_body = go_type
            .strip_prefix("*struct { ")
            .and_then(|s| s.strip_suffix(" }"))
            .unwrap_or("");

        // Emit opening line with nilable comment if applicable
        self.emit_line(&format!(
            "{} *struct {{{}",
            field_name.as_ref(),
            nilable_comment
        ));
        self.indent();

        // Parse and emit each field, handling inner nilable types
        for field_def in struct_body.split("; ") {
            if !field_def.is_empty() {
                // Check if field type has ? prefix (nilable)
                // Format: "name ?*Type" or "name Type"
                if let Some((name, ty)) = field_def.split_once(' ') {
                    if let Some(inner_ty) = ty.strip_prefix('?') {
                        // Nilable inner field
                        self.emit_line(&format!("{} {} //soppo:nilable", name, inner_ty));
                    } else {
                        self.emit_line(field_def);
                    }
                } else {
                    self.emit_line(field_def);
                }
            }
        }

        self.dedent();
        self.emit_line(&format!("}}{}", tag));
    }

    /// Convert binary operator to Go operator
    pub(crate) fn go_binop(&self, op: &BinOp) -> &str {
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
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
        }
    }

    /// Convert compound assignment operator to Go operator
    pub(crate) fn go_assign_op(&self, op: &AssignOp) -> &str {
        match op {
            AssignOp::Add => "+=",
            AssignOp::Sub => "-=",
            AssignOp::Mul => "*=",
            AssignOp::Div => "/=",
            AssignOp::Mod => "%=",
            AssignOp::BitAnd => "&=",
            AssignOp::BitOr => "|=",
            AssignOp::BitXor => "^=",
            AssignOp::Shl => "<<=",
            AssignOp::Shr => ">>=",
        }
    }
}

impl Default for Codegen {
    fn default() -> Self {
        Self::new()
    }
}
