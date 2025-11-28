mod decl;
mod expr;
mod stmt;

use crate::syntax::{BinOp, Comment, Decl, EnumVariant, File, Generic, TypeDecl};
use crate::types::GlobalCtxt;

/// Code generator for emitting Go code
pub struct Codegen {
    pub(crate) output: String,
    indent_level: usize,
    pub(crate) global_state: GlobalCtxt,
    pub(crate) current_func_return_type: Option<String>,
    comments: Vec<Comment>,
    comment_idx: usize,
}

impl Codegen {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            indent_level: 0,
            global_state: GlobalCtxt::new(),
            current_func_return_type: None,
            comments: Vec::new(),
            comment_idx: 0,
        }
    }

    pub fn with_global_state(global_state: GlobalCtxt) -> Self {
        Self {
            output: String::new(),
            indent_level: 0,
            global_state,
            current_func_return_type: None,
            comments: Vec::new(),
            comment_idx: 0,
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
            let text = self.comments[self.comment_idx].text.clone();
            self.emit_indent();
            self.output.push_str(&text);
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
                let text = comment.text.clone();
                self.output.push(' ');
                self.output.push_str(&text);
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
                        self.emit_line(&format!(
                            "        {} {}",
                            field.name,
                            self.go_type(&field.ty.name)
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
    pub fn gen_file(&mut self, file: &File) {
        // Set up comments for emission
        self.set_comments(file.comments.clone());

        // Package declaration
        self.emit_line(&format!("package {}", file.package));
        self.emit_line("");

        // Generate imports
        if !file.imports.is_empty() {
            for import in &file.imports {
                self.emit_comments_before(import.span.byte_start, import.span.start.line);
                self.emit_line(&format!("import \"{}\"", import.path));
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
    }

    /// Convert Soppo type to Go type
    pub(crate) fn go_type<'a>(&self, ty: &'a str) -> &'a str {
        match ty {
            "()" => "", // Unit type
            _ => ty,
        }
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
        }
    }
}

impl Default for Codegen {
    fn default() -> Self {
        Self::new()
    }
}
