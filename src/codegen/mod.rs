mod decl;
mod expr;
mod stmt;

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
    pub(crate) fn emit(&mut self, text: &str) {
        self.output.push_str(text);
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
            .map(|g| format!("{} {}", g.name, g.constraint))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Format generic parameter names only: "T, E"
    pub(crate) fn format_generic_names(&self, generics: &[Generic]) -> String {
        generics
            .iter()
            .map(|g| g.name.as_str())
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
        self.emit_line(&format!("{}{} {{", type_decl.name, generic_names));

        // Variants
        for variant in variants {
            match variant {
                EnumVariant::Unit { name, .. } => {
                    self.emit_line(&format!("    {}", name));
                }
                EnumVariant::Single { name, ty, .. } => {
                    self.emit_line(&format!("    {} {}", name, self.go_type(&ty.name)));
                }
                EnumVariant::Struct { name, fields, .. } => {
                    self.emit_line(&format!("    {} {{", name));
                    for field in fields {
                        let tag = field
                            .tag
                            .as_ref()
                            .map(|t| format!(" `{}`", t))
                            .unwrap_or_default();
                        self.emit_line(&format!(
                            "        {} {}{}",
                            field.name,
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
        // Set up comments for emission
        self.set_comments(file.comments.clone());

        // Soppo generated marker - allows re-importing with proper nil safety
        self.emit_line("//soppo:generated");

        // Package declaration
        self.emit_line(&format!("package {}", file.package));
        self.emit_line("");

        // Generate imports
        if !file.imports.is_empty() {
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

        // Generate declarations
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

        // Emit any remaining comments at the end
        self.emit_remaining_comments();
        Ok(())
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
    pub(crate) fn go_type_from_ast(&self, ty: &crate::syntax::Type) -> String {
        let name = &ty.name;
        // Strip ? prefix if present (nullable marker in Soppo syntax)
        let go_name = name.strip_prefix('?').unwrap_or(name);
        match go_name {
            "()" => String::new(), // Unit type
            _ => go_name.to_string(),
        }
    }

    /// Generate nilable comment if the type is nullable
    /// Returns " //soppo:nilable" if nullable, empty string otherwise
    pub(crate) fn nilable_comment(&self, ty: &crate::syntax::Type) -> &'static str {
        if ty.nullable { " //soppo:nilable" } else { "" }
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
