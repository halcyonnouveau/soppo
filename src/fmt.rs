use crate::error::SoppoError;
use crate::syntax::{
    Arm, AssignOp, Attribute, BinOp, Block, Comment, ConstDecl, Decl, EnumVariant, Expr, ExprKind,
    Field, FieldPattern, File, FileId, FuncDecl, Generic, Import, IntFormat, InterfaceMethod,
    Literal, Param, Parser, Pattern, PatternKind, SelectCase, SelectCaseKind, Stmt, StmtKind,
    StringPart, TypeAnnotation, TypeDecl, TypeKind, UnaryOp,
};

/// Formatter for emitting formatted Soppo code
pub struct Formatter {
    output: String,
    indent_level: usize,
    comments: Vec<Comment>,
    comment_idx: usize,
}

impl Formatter {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            indent_level: 0,
            comments: Vec::new(),
            comment_idx: 0,
        }
    }

    /// Get the formatted output
    pub fn output(&self) -> &str {
        &self.output
    }

    /// Set comments for emission (sorted by byte position)
    fn set_comments(&mut self, mut comments: Vec<Comment>) {
        comments.sort_by_key(|c| c.span.byte_start);
        self.comments = comments;
        self.comment_idx = 0;
    }

    /// Emit all comments that appear before the given line
    fn emit_comments_before(&mut self, line: usize) {
        while self.comment_idx < self.comments.len() {
            let comment = &self.comments[self.comment_idx];
            if comment.span.start.line < line {
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

    /// Emit comments before a declaration, but skip doc comments if the decl has one
    fn emit_comments_before_decl(&mut self, decl_line: usize, has_doc_comment: bool) {
        if !has_doc_comment {
            self.emit_comments_before(decl_line);
            return;
        }

        // Find where the doc comment block starts by scanning backward from decl_line
        // Doc comments are consecutive comment lines ending just before the declaration
        let mut doc_start_line = decl_line;
        let mut expected_line = decl_line - 1;
        for comment in self.comments.iter().rev() {
            if comment.span.start.line >= decl_line {
                continue; // Skip comments at or after declaration
            }
            if comment.span.end.line == expected_line {
                doc_start_line = comment.span.start.line;
                expected_line = comment.span.start.line.saturating_sub(1);
            } else if comment.span.end.line < expected_line {
                break; // Gap found, stop looking
            }
        }

        // Emit comments before the doc comment block, skip doc comments
        while self.comment_idx < self.comments.len() {
            let comment = &self.comments[self.comment_idx];
            if comment.span.start.line < doc_start_line {
                // Before doc comment block - emit it
                let text = comment.text.clone();
                self.emit_indent();
                self.output.push_str(&text);
                self.output.push('\n');
                self.comment_idx += 1;
            } else if comment.span.start.line < decl_line {
                // Part of doc comment block - skip it
                self.comment_idx += 1;
            } else {
                break;
            }
        }
    }

    /// Emit a trailing comment if there's one on the given line
    fn emit_trailing_comment(&mut self, line: usize) -> bool {
        if self.comment_idx < self.comments.len() {
            let comment = &self.comments[self.comment_idx];
            if comment.span.start.line == line && !comment.is_block {
                self.output.push(' ');
                self.output.push_str(&comment.text);
                self.comment_idx += 1;
                return true;
            }
        }
        false
    }

    /// Emit all remaining comments
    fn emit_remaining_comments(&mut self) {
        while self.comment_idx < self.comments.len() {
            self.emit_indent();
            self.output.push_str(&self.comments[self.comment_idx].text);
            self.output.push('\n');
            self.comment_idx += 1;
        }
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

    /// Emit current indentation (tabs, like gofmt)
    fn emit_indent(&mut self) {
        for _ in 0..self.indent_level {
            self.output.push('\t');
        }
    }

    fn indent(&mut self) {
        self.indent_level += 1;
    }

    fn dedent(&mut self) {
        if self.indent_level > 0 {
            self.indent_level -= 1;
        }
    }

    /// Format a single attribute: `[Name]` or `[pkg.Name{field: value}]`
    fn format_attribute(&self, attr: &Attribute) -> String {
        let mut result = format!("[{}", attr.name);
        if !attr.args.is_empty() {
            result.push('{');
            let args: Vec<String> = attr
                .args
                .iter()
                .map(|(k, v)| format!("{}: {}", k, self.format_expr(v)))
                .collect();
            result.push_str(&args.join(", "));
            result.push('}');
        }
        result.push(']');
        result
    }

    /// Emit attributes, each on its own line
    fn emit_attributes(&mut self, attrs: &[Attribute]) {
        for attr in attrs {
            self.emit_line(&self.format_attribute(attr));
        }
    }

    /// Format an entire file
    pub fn format_file(&mut self, file: &File) {
        // Comments before the package line are file-level comments
        let package_line = file.package.span.start.line;

        // Separate file-level comments from the rest
        let (file_comments, other_comments): (Vec<_>, Vec<_>) = file
            .comments
            .iter()
            .cloned()
            .partition(|c| c.span.start.line < package_line);

        // Emit file-level comments first
        for comment in &file_comments {
            self.output.push_str(&comment.text);
            self.output.push('\n');
        }

        // Package declaration
        self.emit_line(&format!("package {}", file.package.name));
        self.emit_line("");

        // Set up remaining comments for emission
        self.set_comments(other_comments);

        // Imports
        if !file.imports.is_empty() {
            if file.imports.len() == 1 {
                let import = &file.imports[0];
                self.emit_comments_before(import.span.start.line);
                self.format_import(import);
            } else {
                self.emit_line("import (");
                self.indent();
                let mut prev_line: Option<usize> = None;
                for import in &file.imports {
                    // Preserve blank lines between import groups
                    if let Some(prev) = prev_line
                        && import.span.start.line > prev + 1
                    {
                        self.output.push('\n');
                    }
                    self.emit_comments_before(import.span.start.line);
                    self.emit_indent();
                    if let Some(alias) = &import.alias {
                        self.emit(&format!("{} \"{}\"", alias, import.path));
                    } else {
                        self.emit(&format!("\"{}\"", import.path));
                    }
                    self.emit_trailing_comment(import.span.start.line);
                    self.output.push('\n');
                    prev_line = Some(import.span.start.line);
                }
                self.dedent();
                self.emit_line(")");
            }
            self.emit_line("");
        }

        // Declarations
        for decl in &file.decls {
            let has_doc = match decl {
                Decl::Const(c) => c.doc_comment.is_some(),
                Decl::ConstBlock(_) => false,
                Decl::Var(_) => false,
                Decl::Type(t) => t.doc_comment.is_some(),
                Decl::Func(f) => f.doc_comment.is_some(),
            };
            self.emit_comments_before_decl(decl.span().start.line, has_doc);
            self.format_decl(decl);
            self.emit_line("");
        }

        self.emit_remaining_comments();

        // Remove trailing blank lines
        while self.output.ends_with("\n\n") {
            self.output.pop();
        }
    }

    fn format_import(&mut self, import: &Import) {
        if let Some(alias) = &import.alias {
            self.emit_line(&format!("import {} \"{}\"", alias, import.path));
        } else {
            self.emit_line(&format!("import \"{}\"", import.path));
        }
    }

    fn format_decl(&mut self, decl: &Decl) {
        match decl {
            Decl::Const(c) => self.format_const_decl(c),
            Decl::ConstBlock(cs) => self.format_const_block(cs),
            Decl::Var(v) => self.format_var_decl(v),
            Decl::Type(t) => self.format_type_decl(t),
            Decl::Func(f) => self.format_func_decl(f),
        }
    }

    fn format_const_decl(&mut self, c: &ConstDecl) {
        if let Some(doc) = &c.doc_comment {
            for line in doc.lines() {
                self.emit_line(&format!("//{}", line));
            }
        }
        self.emit_indent();
        self.emit(&format!("const {}", c.ident.name));
        if let Some(ty) = &c.ty {
            self.emit(" ");
            self.emit(&Self::format_type_annotation(ty));
        }
        self.emit(" = ");
        self.emit(&self.format_expr(&c.value));
        self.emit_trailing_comment(c.span.start.line);
        self.output.push('\n');
    }

    fn format_const_block(&mut self, consts: &[ConstDecl]) {
        self.emit_line("const (");
        self.indent();
        for c in consts {
            self.emit_comments_before(c.span.start.line);
            if let Some(doc) = &c.doc_comment {
                for line in doc.lines() {
                    self.emit_line(&format!("//{}", line));
                }
            }
            self.emit_indent();
            self.emit(&c.ident.name);
            if let Some(ty) = &c.ty {
                self.emit(" ");
                self.emit(&Self::format_type_annotation(ty));
            }
            self.emit(" = ");
            self.emit(&self.format_expr(&c.value));
            self.emit_trailing_comment(c.span.start.line);
            self.output.push('\n');
        }
        self.dedent();
        self.emit_line(")");
    }

    fn format_var_decl(&mut self, v: &crate::syntax::VarDecl) {
        self.emit_indent();
        self.emit(&format!("var {}", v.ident.name));
        if let Some(ty) = &v.ty {
            self.emit(" ");
            self.emit(&Self::format_type_annotation(ty));
        }
        if let Some(value) = &v.value {
            self.emit(" = ");
            self.emit(&self.format_expr(value));
        }
        self.emit_trailing_comment(v.span.start.line);
        self.output.push('\n');
    }

    fn format_type_decl(&mut self, t: &TypeDecl) {
        // Attributes first
        self.emit_attributes(&t.attributes);

        if let Some(doc) = &t.doc_comment {
            for line in doc.lines() {
                self.emit_line(&format!("//{}", line));
            }
        }

        self.emit_indent();
        self.emit(&format!("type {}", t.ident.name));

        if !t.generics.is_empty() {
            self.emit("[");
            self.emit(&self.format_generics(&t.generics));
            self.emit("]");
        }

        match &t.kind {
            TypeKind::Alias { target } => {
                self.emit(" = ");
                self.emit(&Self::format_type_annotation(target));
                self.output.push('\n');
            }
            TypeKind::Definition { target } => {
                self.emit(" ");
                self.emit(&Self::format_type_annotation(target));
                self.output.push('\n');
            }
            TypeKind::Enum { variants } => {
                self.emit(" enum {\n");
                self.indent();

                // Group variants by contiguous blocks
                // A new group starts when there's a blank line, comment, or attribute
                // Each group is aligned independently
                let groups = self.group_enum_variants(variants);

                for (group_idx, group) in groups.iter().enumerate() {
                    // Emit blank line between groups (except before first)
                    if group_idx > 0 {
                        self.output.push('\n');
                    }

                    // Calculate max name length for this group only
                    let max_name_len = group
                        .iter()
                        .map(|v| match v {
                            EnumVariant::Unit { ident, .. } => ident.name.len(),
                            EnumVariant::Single { ident, .. } => ident.name.len(),
                            EnumVariant::Struct { ident, .. } => ident.name.len(),
                        })
                        .max()
                        .unwrap_or(0);

                    for variant in group {
                        self.format_enum_variant_aligned(variant, max_name_len);
                    }
                }

                self.dedent();
                self.emit_line("}");
            }
            TypeKind::Struct { fields } => {
                // Empty struct: use single-line format (no space before braces)
                if fields.is_empty() {
                    if t.is_const {
                        self.emit(" const struct{}\n");
                    } else {
                        self.emit(" struct{}\n");
                    }
                } else {
                    // Emit "const struct" if the whole type is const
                    if t.is_const {
                        self.emit(" const struct {\n");
                    } else {
                        self.emit(" struct {\n");
                    }
                    self.indent();

                    // Group fields by contiguous blocks
                    // A new group starts when there's a blank line, comment, or attribute
                    // Each group is aligned independently (like gofmt)
                    let groups = self.group_fields(fields);

                    for (group_idx, group) in groups.iter().enumerate() {
                        // Emit blank line between groups (except before first)
                        if group_idx > 0 {
                            self.output.push('\n');
                        }

                        // Calculate alignment for this group only
                        let (max_name_len, max_type_len) = self.calc_field_alignment_refs(group);

                        for field in group {
                            self.format_field_aligned(field, max_name_len, max_type_len);
                        }
                    }

                    self.dedent();
                    self.emit_line("}");
                }
            }
            TypeKind::Interface { methods } => {
                self.emit(" interface {\n");
                self.indent();
                for method in methods {
                    self.format_interface_method(method);
                }
                self.dedent();
                self.emit_line("}");
            }
        }
    }

    fn format_generics(&self, generics: &[Generic]) -> String {
        generics
            .iter()
            .map(|g| format!("{} {}", g.ident.name, g.constraint))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn format_enum_variant_aligned(&mut self, variant: &EnumVariant, max_name_len: usize) {
        match variant {
            EnumVariant::Unit { ident, attributes } => {
                self.emit_attributes(attributes);
                self.emit_line(&ident.name);
            }
            EnumVariant::Single {
                ident,
                ty,
                attributes,
            } => {
                self.emit_attributes(attributes);
                self.emit_indent();
                self.emit(&ident.name);
                // Pad name to align types
                let padding = max_name_len - ident.name.len() + 1;
                self.emit(&" ".repeat(padding));
                self.emit(&Self::format_type_annotation(ty));
                self.output.push('\n');
            }
            EnumVariant::Struct {
                ident,
                fields,
                attributes,
            } => {
                self.emit_attributes(attributes);
                self.emit_indent();
                self.emit(&format!("{} struct {{\n", ident.name));
                self.indent();
                let (field_max_name_len, max_type_len) = self.calc_field_alignment(fields);
                for field in fields {
                    self.format_field_aligned(field, field_max_name_len, max_type_len);
                }
                self.dedent();
                self.emit_line("}");
            }
        }
    }

    /// Calculate alignment widths for a slice of fields
    fn calc_field_alignment(&self, fields: &[Field]) -> (usize, usize) {
        let mut max_name_len = 0;
        let mut max_type_len = 0;
        for field in fields {
            // Account for "const " prefix (6 chars) if the field is const
            let name_len = if field.is_const {
                "const ".len() + field.ident.name.len()
            } else {
                field.ident.name.len()
            };
            max_name_len = max_name_len.max(name_len);
            let type_str = Self::format_type_annotation(&field.ty);
            max_type_len = max_type_len.max(type_str.len());
        }
        (max_name_len, max_type_len)
    }

    /// Calculate alignment widths for a slice of field references
    fn calc_field_alignment_refs(&self, fields: &[&Field]) -> (usize, usize) {
        let mut max_name_len = 0;
        let mut max_type_len = 0;
        for field in fields {
            // Account for "const " prefix (6 chars) if the field is const
            let name_len = if field.is_const {
                "const ".len() + field.ident.name.len()
            } else {
                field.ident.name.len()
            };
            max_name_len = max_name_len.max(name_len);
            let type_str = Self::format_type_annotation(&field.ty);
            max_type_len = max_type_len.max(type_str.len());
        }
        (max_name_len, max_type_len)
    }

    /// Group fields into alignment groups.
    /// A new group starts when there's a blank line, comment, or attribute before a field.
    fn group_fields<'a>(&self, fields: &'a [Field]) -> Vec<Vec<&'a Field>> {
        let mut groups: Vec<Vec<&'a Field>> = Vec::new();
        let mut current_group: Vec<&'a Field> = Vec::new();

        for (i, field) in fields.iter().enumerate() {
            let starts_new_group = if i == 0 {
                false // First field never starts a new group
            } else {
                let prev_field = &fields[i - 1];
                let prev_end = prev_field.ident.span.end.line;

                // Check for attribute on this field
                let has_attribute = !field.attributes.is_empty();

                // Check for blank line or comment between previous field and this one
                let field_start = field
                    .attributes
                    .first()
                    .map(|a| a.span.start.line)
                    .unwrap_or(field.ident.span.start.line);

                let has_gap = field_start > prev_end + 1;

                // Check for comments between previous field and this one
                let has_comment = self.comments.iter().any(|c| {
                    c.span.start.line > prev_end && c.span.start.line < field.ident.span.start.line
                });

                has_attribute || has_gap || has_comment
            };

            if starts_new_group && !current_group.is_empty() {
                groups.push(current_group);
                current_group = Vec::new();
            }

            current_group.push(field);
        }

        if !current_group.is_empty() {
            groups.push(current_group);
        }

        groups
    }

    /// Group enum variants into alignment groups.
    /// A new group starts when there's a blank line, comment, or attribute before a variant.
    fn group_enum_variants<'a>(&self, variants: &'a [EnumVariant]) -> Vec<Vec<&'a EnumVariant>> {
        let mut groups: Vec<Vec<&'a EnumVariant>> = Vec::new();
        let mut current_group: Vec<&'a EnumVariant> = Vec::new();

        for (i, variant) in variants.iter().enumerate() {
            let (attrs, ident_line) = match variant {
                EnumVariant::Unit { ident, attributes } => {
                    (attributes.as_slice(), ident.span.start.line)
                }
                EnumVariant::Single {
                    ident, attributes, ..
                } => (attributes.as_slice(), ident.span.start.line),
                EnumVariant::Struct {
                    ident, attributes, ..
                } => (attributes.as_slice(), ident.span.start.line),
            };

            let starts_new_group = if i == 0 {
                false // First variant never starts a new group
            } else {
                let prev_variant = &variants[i - 1];
                let prev_end = match prev_variant {
                    EnumVariant::Unit { ident, .. } => ident.span.end.line,
                    EnumVariant::Single { ident, .. } => ident.span.end.line,
                    EnumVariant::Struct { ident, .. } => ident.span.end.line,
                };

                // Check for attribute on this variant
                let has_attribute = !attrs.is_empty();

                // Check for blank line between previous variant and this one
                let variant_start = attrs
                    .first()
                    .map(|a| a.span.start.line)
                    .unwrap_or(ident_line);

                let has_gap = variant_start > prev_end + 1;

                // Check for comments between previous variant and this one
                let has_comment = self
                    .comments
                    .iter()
                    .any(|c| c.span.start.line > prev_end && c.span.start.line < ident_line);

                has_attribute || has_gap || has_comment
            };

            if starts_new_group && !current_group.is_empty() {
                groups.push(current_group);
                current_group = Vec::new();
            }

            current_group.push(variant);
        }

        if !current_group.is_empty() {
            groups.push(current_group);
        }

        groups
    }

    /// Format a field with alignment (used for struct fields)
    fn format_field_aligned(&mut self, field: &Field, max_name_len: usize, max_type_len: usize) {
        let line = field.ident.span.start.line;
        self.emit_comments_before(line);

        // Emit field attributes
        self.emit_attributes(&field.attributes);

        self.emit_indent();

        // Emit const keyword if field is const
        if field.is_const {
            self.emit("const ");
        }

        let name = &field.ident.name;
        let type_str = Self::format_type_annotation(&field.ty);

        // Calculate effective name length (including "const " prefix if present)
        let effective_name_len = if field.is_const {
            "const ".len() + name.len()
        } else {
            name.len()
        };

        // Pad name to align types
        self.emit(name);
        if max_name_len > 0 {
            let name_padding = max_name_len - effective_name_len + 1;
            self.emit(&" ".repeat(name_padding));
        } else {
            self.emit(" ");
        }

        // Emit type
        self.emit(&type_str);

        // Check if this field has a trailing comment
        let has_trailing_comment = self.comment_idx < self.comments.len()
            && self.comments[self.comment_idx].span.start.line == line
            && !self.comments[self.comment_idx].text.starts_with('\n');

        // Pad type to align tags/comments (if needed)
        if (field.tag.is_some() || has_trailing_comment)
            && max_type_len > 0
            && type_str.len() < max_type_len
        {
            let type_padding = max_type_len - type_str.len();
            self.emit(&" ".repeat(type_padding));
        }

        if let Some(tag) = &field.tag {
            self.emit(&format!(" `{}`", tag));
        }

        self.emit_trailing_comment(line);
        self.output.push('\n');
    }

    fn format_interface_method(&mut self, method: &InterfaceMethod) {
        self.emit_indent();
        self.emit(&method.ident.name);
        self.emit("(");
        self.emit(&self.format_params(&method.params));
        self.emit(")");
        if !method.returns.is_empty() {
            self.emit(" ");
            if method.returns.len() == 1 {
                self.emit(&Self::format_type_annotation(&method.returns[0]));
            } else {
                self.emit("(");
                let returns: Vec<_> = method
                    .returns
                    .iter()
                    .map(Self::format_type_annotation)
                    .collect();
                self.emit(&returns.join(", "));
                self.emit(")");
            }
        }
        self.output.push('\n');
    }

    fn format_func_decl(&mut self, f: &FuncDecl) {
        // Attributes first
        self.emit_attributes(&f.attributes);

        if let Some(doc) = &f.doc_comment {
            for line in doc.lines() {
                self.emit_line(&format!("//{}", line));
            }
        }

        self.emit_indent();
        self.emit("func ");

        // Receiver
        if let Some(recv) = &f.receiver {
            self.emit(&format!(
                "({} {}) ",
                recv.ident.name,
                Self::format_type_annotation(&recv.ty)
            ));
        }

        self.emit(&f.ident.name);

        // Generics
        if !f.generics.is_empty() {
            self.emit("[");
            self.emit(&self.format_generics(&f.generics));
            self.emit("]");
        }

        // Parameters
        self.emit("(");
        self.emit(&self.format_params(&f.params));
        self.emit(")");

        // Return types - handle named and unnamed
        if !f.returns.is_empty() {
            self.emit(" ");
            let is_named = !f.returns[0].ident.name.is_empty();
            if f.returns.len() == 1 && !is_named {
                self.emit(&Self::format_type_annotation(&f.returns[0].ty));
            } else {
                self.emit("(");
                self.emit(&self.format_params_grouped(&f.returns, true));
                self.emit(")");
            }
        }

        // Body
        self.emit(" {\n");
        self.indent();
        self.format_block_contents(&f.body);
        self.dedent();
        self.emit_line("}");
    }

    fn format_params(&self, params: &[Param]) -> String {
        self.format_params_grouped(params, false)
    }

    /// Format parameters with type grouping (a, b int instead of a int, b int)
    /// If `allow_unnamed` is true, params without names just emit the type
    fn format_params_grouped(&self, params: &[Param], allow_unnamed: bool) -> String {
        if params.is_empty() {
            return String::new();
        }

        // Group consecutive parameters with the same type
        let mut groups: Vec<(Vec<&str>, String)> = Vec::new();

        for param in params {
            let ty_str = Self::format_type_annotation(&param.ty);
            let name = &param.ident.name;

            // Unnamed parameter (just type)
            if name.is_empty() && allow_unnamed {
                groups.push((vec![], ty_str));
                continue;
            }

            if let Some((names, last_ty)) = groups.last_mut()
                && *last_ty == ty_str
                && !names.is_empty()
            {
                names.push(name);
                continue;
            }
            groups.push((vec![name.as_str()], ty_str));
        }

        groups
            .into_iter()
            .map(|(names, ty)| {
                if names.is_empty() {
                    ty
                } else {
                    format!("{} {}", names.join(", "), ty)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn format_type_annotation(ty: &TypeAnnotation) -> String {
        Self::format_type_annotation_with_indent(ty, 0)
    }

    fn format_type_annotation_with_indent(ty: &TypeAnnotation, indent: usize) -> String {
        let mut result = String::new();

        if ty.nullable {
            result.push('?');
        }

        // Handle anonymous struct types specially
        if ty.name.starts_with("struct {") || ty.name.starts_with("struct{") {
            result.push_str(&Self::format_anon_struct_type(&ty.name, indent));
        } else if ty.name.starts_with("[]") {
            // Slice type - check if element is anonymous struct
            let elem = &ty.name[2..];
            if elem.starts_with("struct {") || elem.starts_with("struct{") {
                result.push_str("[]");
                result.push_str(&Self::format_anon_struct_type(elem, indent));
            } else {
                result.push_str(&ty.name);
            }
        } else {
            result.push_str(&ty.name);
        }

        // Only add [args] for generic types, not for built-in compound types
        // (slices, arrays, pointers, maps, channels, variadics, funcs)
        // whose element types are already embedded in the name
        let is_builtin_compound = ty.name.starts_with("[]")
            || ty.name.starts_with("[")
            || ty.name.starts_with('*')
            || ty.name.starts_with("map[")
            || ty.name.starts_with("chan ")
            || ty.name.starts_with("...")
            || ty.name.starts_with("func(")
            || ty.name.starts_with("struct {");

        if !ty.args.is_empty() && !is_builtin_compound {
            result.push('[');
            let args: Vec<_> = ty
                .args
                .iter()
                .map(|a| Self::format_type_annotation_with_indent(a, indent))
                .collect();
            result.push_str(&args.join(", "));
            result.push(']');
        }

        result
    }

    /// Format anonymous struct type with proper multiline layout
    fn format_anon_struct_type(s: &str, indent: usize) -> String {
        let inner = s
            .strip_prefix("struct {")
            .or_else(|| s.strip_prefix("struct{"))
            .and_then(|s| s.strip_suffix('}'))
            .map(|s| s.trim());

        let Some(inner) = inner else {
            return s.to_string();
        };

        if inner.is_empty() {
            return "struct {}".to_string();
        }

        // Parse fields - split on last space to handle grouped fields like "a, b, c int"
        let mut fields = Vec::new();
        for field_def in inner.split(';') {
            let field_def = field_def.trim();
            if field_def.is_empty() {
                continue;
            }
            // Find the last space to separate names from type
            if let Some(last_space) = field_def.rfind(' ') {
                let names = field_def[..last_space].trim();
                let ty = field_def[last_space + 1..].trim();
                fields.push((names, ty));
            }
        }

        // Single field: one line
        if fields.len() == 1 {
            return format!("struct {{ {} {} }}", fields[0].0, fields[0].1);
        }

        // Multiple fields: multiline with alignment
        let max_name_len = fields.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
        let indent_str = "\t".repeat(indent + 1);
        let close_indent = "\t".repeat(indent);

        let mut result = String::from("struct {\n");
        for (name, ty) in &fields {
            result.push_str(&indent_str);
            result.push_str(name);
            // Pad for alignment
            for _ in 0..(max_name_len - name.len() + 1) {
                result.push(' ');
            }
            result.push_str(ty);
            result.push('\n');
        }
        result.push_str(&close_indent);
        result.push('}');
        result
    }

    fn format_block_contents(&mut self, block: &Block) {
        let mut prev_end_line = 0;
        for stmt in &block.stmts {
            // Find the first comment line before this statement (if any)
            let first_comment_line = if self.comment_idx < self.comments.len()
                && self.comments[self.comment_idx].span.start.line < stmt.span.start.line
            {
                Some(self.comments[self.comment_idx].span.start.line)
            } else {
                None
            };

            // Find the last comment's end line before this statement
            let mut last_comment_end_line: Option<usize> = None;
            for i in self.comment_idx..self.comments.len() {
                let comment = &self.comments[i];
                if comment.span.start.line >= stmt.span.start.line {
                    break;
                }
                last_comment_end_line = Some(comment.span.end.line);
            }

            // Check for blank line before first comment (or before statement if no comments)
            let first_line = first_comment_line.unwrap_or(stmt.span.start.line);
            if prev_end_line > 0 && first_line > prev_end_line + 1 {
                self.output.push('\n');
            }

            // Emit comments
            self.emit_comments_before(stmt.span.start.line);

            // Check for blank line after last comment (before statement)
            if let Some(last_end) = last_comment_end_line
                && stmt.span.start.line > last_end + 1
            {
                self.output.push('\n');
            }

            self.format_stmt(stmt);
            prev_end_line = stmt.span.end.line;
        }

        // Emit any comments at the end of the block (e.g., in an otherwise empty block)
        self.emit_comments_before(block.span.end.line);
    }

    fn format_stmt(&mut self, stmt: &Stmt) {
        let line = stmt.span.start.line;
        match &stmt.kind {
            StmtKind::Decl { ident, value } => {
                self.emit_indent();
                self.emit(&format!("{} := ", ident.name));
                if self.is_multiline_array(value) {
                    self.format_array_lit_multiline(value);
                } else {
                    self.emit(&self.format_expr(value));
                }
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::MultiDecl { ident, values } => {
                self.emit_indent();
                let names: Vec<_> = ident.iter().map(|i| i.name.as_str()).collect();
                self.emit(&names.join(", "));
                self.emit(" := ");
                let vals: Vec<_> = values.iter().map(|v| self.format_expr(v)).collect();
                self.emit(&vals.join(", "));
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::VarDecl { ident, ty, value } => {
                self.emit_indent();
                self.emit(&format!("var {}", ident.name));
                if let Some(t) = ty {
                    self.emit(" ");
                    self.emit(&Self::format_type_annotation(t));
                }
                if let Some(v) = value {
                    self.emit(" = ");
                    if self.is_multiline_array(v) {
                        self.format_array_lit_multiline(v);
                    } else {
                        self.emit(&self.format_expr(v));
                    }
                }
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::MultiVarDecl { ident, ty, values } => {
                self.emit_indent();
                self.emit("var ");
                let names: Vec<_> = ident.iter().map(|i| i.name.as_str()).collect();
                self.emit(&names.join(", "));
                if let Some(t) = ty {
                    self.emit(" ");
                    self.emit(&Self::format_type_annotation(t));
                }
                if !values.is_empty() {
                    self.emit(" = ");
                    let vals: Vec<_> = values.iter().map(|v| self.format_expr(v)).collect();
                    self.emit(&vals.join(", "));
                }
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::ConstDecl { ident, ty, value } => {
                self.emit_indent();
                self.emit(&format!("const {}", ident.name));
                if let Some(t) = ty {
                    self.emit(" ");
                    self.emit(&Self::format_type_annotation(t));
                }
                self.emit(" = ");
                self.emit(&self.format_expr(value));
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::MultiConstDecl { idents, ty, values } => {
                self.emit_indent();
                self.emit("const ");
                let names: Vec<_> = idents.iter().map(|i| i.name.as_str()).collect();
                self.emit(&names.join(", "));
                if let Some(t) = ty {
                    self.emit(" ");
                    self.emit(&Self::format_type_annotation(t));
                }
                self.emit(" = ");
                let vals: Vec<_> = values.iter().map(|v| self.format_expr(v)).collect();
                self.emit(&vals.join(", "));
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::Assign { target, value } => {
                self.emit_indent();
                self.emit(&self.format_expr(target));
                self.emit(" = ");
                if self.is_multiline_array(value) {
                    self.format_array_lit_multiline(value);
                } else {
                    self.emit(&self.format_expr(value));
                }
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::MultiAssign { targets, values } => {
                self.emit_indent();
                let tgts: Vec<_> = targets.iter().map(|t| self.format_expr(t)).collect();
                self.emit(&tgts.join(", "));
                self.emit(" = ");
                let vals: Vec<_> = values.iter().map(|v| self.format_expr(v)).collect();
                self.emit(&vals.join(", "));
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::CompoundAssign { target, op, value } => {
                self.emit_indent();
                self.emit(&self.format_expr(target));
                self.emit(&format!(" {} ", self.format_assign_op(*op)));
                self.emit(&self.format_expr(value));
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::IncDec { target, is_inc } => {
                self.emit_indent();
                self.emit(&self.format_expr(target));
                self.emit(if *is_inc { "++" } else { "--" });
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::For { condition, body } => {
                self.emit_indent();
                self.emit("for ");
                self.emit(&self.format_expr(condition));
                self.emit(" {\n");
                self.indent();
                self.format_block_contents(body);
                self.dedent();
                self.emit_line("}");
            }
            StmtKind::ForCStyle {
                init,
                condition,
                post,
                body,
            } => {
                self.emit_indent();
                // Infinite loop: for { } (no semicolons)
                if init.is_none() && condition.is_none() && post.is_none() {
                    self.emit("for {\n");
                } else {
                    self.emit("for ");
                    if let Some(i) = init {
                        self.emit(&self.format_stmt_inline(i));
                    }
                    self.emit("; ");
                    if let Some(c) = condition {
                        self.emit(&self.format_expr(c));
                    }
                    self.emit("; ");
                    if let Some(p) = post {
                        self.emit(&self.format_stmt_inline(p));
                    }
                    self.emit(" {\n");
                }
                self.indent();
                self.format_block_contents(body);
                self.dedent();
                self.emit_line("}");
            }
            StmtKind::ForRange {
                key,
                value,
                collection,
                body,
            } => {
                self.emit_indent();
                self.emit(&format!("for {}", key.name));
                if let Some(v) = value {
                    self.emit(&format!(", {}", v.name));
                }
                self.emit(" := range ");
                self.emit(&self.format_expr(collection));
                self.emit(" {\n");
                self.indent();
                self.format_block_contents(body);
                self.dedent();
                self.emit_line("}");
            }
            StmtKind::If {
                init,
                condition,
                then_block,
                else_block,
            } => {
                self.emit_indent();
                self.format_if_chain(init.as_deref(), condition, then_block, else_block.as_ref());
            }
            StmtKind::Return { values } => {
                self.emit_indent();
                if values.is_empty() {
                    self.emit("return");
                } else {
                    self.emit("return ");
                    let vals: Vec<_> = values.iter().map(|v| self.format_expr(v)).collect();
                    self.emit(&vals.join(", "));
                }
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::Match { scrutinee, arms } => {
                self.emit_indent();
                self.emit("match ");
                if let Some(s) = scrutinee {
                    self.emit(&self.format_expr(s));
                    self.emit(" ");
                }
                self.emit("{\n");
                for arm in arms {
                    self.format_arm(arm);
                }
                self.emit_line("}");
            }
            StmtKind::Send { channel, value } => {
                self.emit_indent();
                self.emit(&self.format_expr(channel));
                self.emit(" <- ");
                self.emit(&self.format_expr(value));
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::Select { cases } => {
                self.emit_line("select {");
                for case in cases {
                    self.format_select_case(case);
                }
                self.emit_line("}");
            }
            StmtKind::Go(expr) => {
                self.emit_indent();
                self.emit("go ");
                self.emit(&self.format_expr(expr));
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::DeferStmt(expr) => {
                self.emit_indent();
                self.emit("defer ");
                self.emit(&self.format_expr(expr));
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::Break => {
                self.emit_line("break");
            }
            StmtKind::Continue => {
                self.emit_line("continue");
            }
            StmtKind::Expr(expr) => {
                self.emit_indent();
                self.emit(&self.format_expr(expr));
                self.emit_trailing_comment(line);
                self.output.push('\n');
            }
            StmtKind::TryStmt {
                stmt,
                error_name,
                handler,
                ..
            } => {
                // Format the inner statement without newline
                let inner = self.format_stmt_inline(stmt);
                self.emit_indent();
                self.emit(&inner);
                self.emit(" ?");
                if let Some(name) = error_name {
                    self.emit(&format!(" {}", name));
                }
                if let Some(h) = handler {
                    self.emit(" {\n");
                    self.indent();
                    self.format_block_contents(h);
                    self.dedent();
                    self.emit_line("}");
                } else {
                    self.emit_trailing_comment(line);
                    self.output.push('\n');
                }
            }
            StmtKind::LocalTypeDecl(t) => {
                self.format_type_decl(t);
            }
        }
    }

    /// Format an if statement with proper else-if chain handling
    fn format_if_chain(
        &mut self,
        init: Option<&Stmt>,
        condition: &Expr,
        then_block: &Block,
        else_block: Option<&Block>,
    ) {
        self.emit("if ");
        if let Some(i) = init {
            self.emit(&self.format_stmt_inline(i));
            self.emit("; ");
        }
        self.emit(&self.format_expr(condition));
        self.emit(" {\n");
        self.indent();
        self.format_block_contents(then_block);
        self.dedent();

        if let Some(else_b) = else_block {
            // Check if this is an else-if chain (block span == if stmt span means no braces)
            if else_b.stmts.len() == 1
                && let StmtKind::If {
                    init: else_init,
                    condition: else_cond,
                    then_block: else_then,
                    else_block: else_else,
                } = &else_b.stmts[0].kind
                && else_b.span == else_b.stmts[0].span
            {
                // This was originally "else if" - format as such
                self.emit_indent();
                self.emit("} else ");
                self.format_if_chain(
                    else_init.as_deref(),
                    else_cond,
                    else_then,
                    else_else.as_ref(),
                );
                return;
            }

            // Regular else block with explicit braces
            self.emit_line("} else {");
            self.indent();
            self.format_block_contents(else_b);
            self.dedent();
        }
        self.emit_line("}");
    }

    /// Format a statement without indentation or newline (for inline use)
    fn format_stmt_inline(&self, stmt: &Stmt) -> String {
        match &stmt.kind {
            StmtKind::Decl { ident, value } => {
                format!("{} := {}", ident.name, self.format_expr(value))
            }
            StmtKind::MultiDecl { ident, values } => {
                let names = ident
                    .iter()
                    .map(|i| i.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let vals = values
                    .iter()
                    .map(|v| self.format_expr(v))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} := {}", names, vals)
            }
            StmtKind::Assign { target, value } => {
                format!("{} = {}", self.format_expr(target), self.format_expr(value))
            }
            StmtKind::MultiAssign { targets, values } => {
                let tgts = targets
                    .iter()
                    .map(|t| self.format_expr(t))
                    .collect::<Vec<_>>()
                    .join(", ");
                let vals = values
                    .iter()
                    .map(|v| self.format_expr(v))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} = {}", tgts, vals)
            }
            StmtKind::IncDec { target, is_inc } => {
                format!(
                    "{}{}",
                    self.format_expr(target),
                    if *is_inc { "++" } else { "--" }
                )
            }
            StmtKind::Expr(expr) => self.format_expr(expr),
            _ => String::new(), // Other statements shouldn't appear inline
        }
    }

    fn format_arm(&mut self, arm: &Arm) {
        self.emit_comments_before(arm.span.start.line);
        self.emit_indent();
        // Check if this is a default case
        let is_default =
            arm.patterns.len() == 1 && matches!(arm.patterns[0].kind, PatternKind::Default);
        if is_default {
            self.emit("default:\n");
        } else {
            self.emit("case ");
            let patterns: Vec<_> = arm
                .patterns
                .iter()
                .map(|p| self.format_pattern(p))
                .collect();
            self.emit(&patterns.join(", "));
            self.emit(":\n");
        }
        self.indent();
        self.format_block_contents(&arm.body);
        self.dedent();
    }

    fn format_pattern(&self, pat: &Pattern) -> String {
        match &pat.kind {
            PatternKind::Default => "default".to_string(),
            PatternKind::Variant {
                name, type_args, ..
            } => {
                if type_args.is_empty() {
                    name.clone()
                } else {
                    // Format: BaseType[type_args].Variant
                    let parts: Vec<&str> = name.split('.').collect();
                    if parts.len() >= 2 {
                        let base = parts[0];
                        let rest = parts[1..].join(".");
                        let args: Vec<String> =
                            type_args.iter().map(|ta| ta.name.clone()).collect();
                        format!("{}[{}].{}", base, args.join(", "), rest)
                    } else {
                        name.clone()
                    }
                }
            }
            PatternKind::Literal(lit) => self.format_literal(lit),
            PatternKind::Destructor {
                name,
                type_args,
                binding,
            } => {
                let formatted_name = if type_args.is_empty() {
                    name.clone()
                } else {
                    let parts: Vec<&str> = name.split('.').collect();
                    if parts.len() >= 2 {
                        let base = parts[0];
                        let rest = parts[1..].join(".");
                        let args: Vec<String> =
                            type_args.iter().map(|ta| ta.name.clone()).collect();
                        format!("{}[{}].{}", base, args.join(", "), rest)
                    } else {
                        name.clone()
                    }
                };
                format!("{}({})", formatted_name, binding.name)
            }
            PatternKind::StructDestructor {
                name,
                type_args,
                fields,
                rest,
            } => {
                let formatted_name = if type_args.is_empty() {
                    name.clone()
                } else {
                    let parts: Vec<&str> = name.split('.').collect();
                    if parts.len() >= 2 {
                        let base = parts[0];
                        let rest = parts[1..].join(".");
                        let args: Vec<String> =
                            type_args.iter().map(|ta| ta.name.clone()).collect();
                        format!("{}[{}].{}", base, args.join(", "), rest)
                    } else {
                        name.clone()
                    }
                };
                let mut result = format!("{}{{", formatted_name);
                let field_strs: Vec<_> = fields
                    .iter()
                    .map(|(fname, fpat)| match fpat {
                        FieldPattern::Bind(ident) => {
                            if ident.name == *fname {
                                fname.clone()
                            } else {
                                format!("{}: {}", fname, ident.name)
                            }
                        }
                        FieldPattern::Literal(lit) => {
                            format!("{}: {}", fname, self.format_literal(lit))
                        }
                    })
                    .collect();
                result.push_str(&field_strs.join(", "));
                if *rest {
                    if !fields.is_empty() {
                        result.push_str(", ");
                    }
                    result.push_str("...");
                }
                result.push('}');
                result
            }
            PatternKind::Guard(expr) => self.format_expr(expr),
        }
    }

    fn format_literal(&self, lit: &Literal) -> String {
        match lit {
            Literal::Integer(n, fmt) => match fmt {
                IntFormat::Decimal => n.to_string(),
                IntFormat::Octal => format!("0o{:o}", n),
                IntFormat::Hex => format!("0x{:x}", n),
                IntFormat::Binary => format!("0b{:b}", n),
            },
            Literal::String(s) => format!("\"{}\"", s),
            Literal::Bool(b) => b.to_string(),
            Literal::Nil => "nil".to_string(),
        }
    }

    fn format_select_case(&mut self, case: &SelectCase) {
        self.emit_comments_before(case.span.start.line);
        self.emit_indent();
        match &case.kind {
            SelectCaseKind::Recv { channel } => {
                self.emit(&format!("case <-{}:\n", self.format_expr(channel)));
            }
            SelectCaseKind::RecvDecl { ident, channel } => {
                self.emit(&format!(
                    "case {} := <-{}:\n",
                    ident.name,
                    self.format_expr(channel)
                ));
            }
            SelectCaseKind::RecvDeclOk {
                ident,
                ok_ident,
                channel,
            } => {
                self.emit(&format!(
                    "case {}, {} := <-{}:\n",
                    ident.name,
                    ok_ident.name,
                    self.format_expr(channel)
                ));
            }
            SelectCaseKind::Send { channel, value } => {
                self.emit(&format!(
                    "case {} <- {}:\n",
                    self.format_expr(channel),
                    self.format_expr(value)
                ));
            }
            SelectCaseKind::Default => {
                self.emit("default:\n");
            }
        }
        self.indent();
        self.format_block_contents(&case.body);
        self.dedent();
    }

    /// Check if expression is an array literal that should be formatted multi-line
    fn is_multiline_array(&self, expr: &Expr) -> bool {
        if let ExprKind::ArrayLit { elements, .. } = &expr.kind {
            elements.len() > 1
                && elements
                    .iter()
                    .any(|e| matches!(&e.kind, ExprKind::StructLit { .. }))
        } else {
            false
        }
    }

    /// Format a multi-line array literal, emitting directly to output with trailing comments
    fn format_array_lit_multiline(&mut self, expr: &Expr) {
        let ExprKind::ArrayLit { ty, elements } = &expr.kind else {
            return;
        };

        if let Some(t) = ty {
            self.emit(&Self::format_type_annotation_with_indent(
                t,
                self.indent_level,
            ));
        }
        self.emit("{\n");
        self.indent();

        // First pass: format elements and find max length for comment alignment
        let formatted: Vec<_> = elements
            .iter()
            .map(|e| (self.format_expr(e), e.span.start.line))
            .collect();

        // Find max length of elements that have trailing comments (for alignment)
        let max_len = formatted
            .iter()
            .filter(|(_, line)| {
                self.comments
                    .get(self.comment_idx..)
                    .map(|cs| cs.iter().any(|c| c.span.start.line == *line && !c.is_block))
                    .unwrap_or(false)
            })
            .map(|(s, _)| s.len())
            .max()
            .unwrap_or(0);

        // Second pass: emit with alignment
        for (elem_str, elem_line) in formatted {
            self.emit_indent();
            self.emit(&elem_str);
            self.output.push(',');

            // Check if there's a trailing comment for this line
            let has_comment = self.comment_idx < self.comments.len()
                && self.comments[self.comment_idx].span.start.line == elem_line
                && !self.comments[self.comment_idx].is_block;

            if has_comment && max_len > 0 {
                // Pad to align comments (padding comes before the space in emit_trailing_comment)
                let padding = max_len.saturating_sub(elem_str.len());
                for _ in 0..padding {
                    self.output.push(' ');
                }
            }

            self.emit_trailing_comment(elem_line);
            self.output.push('\n');
        }

        self.dedent();
        self.emit_indent();
        self.output.push('}');
    }

    fn format_expr(&self, expr: &Expr) -> String {
        self.format_expr_indent(expr, self.indent_level)
    }

    fn format_expr_indent(&self, expr: &Expr, indent: usize) -> String {
        match &expr.kind {
            ExprKind::Integer(n, fmt) => match fmt {
                IntFormat::Decimal => n.to_string(),
                IntFormat::Octal => format!("0o{:o}", n),
                IntFormat::Hex => format!("0x{:x}", n),
                IntFormat::Binary => format!("0b{:b}", n),
            },
            ExprKind::Float(f) => {
                let s = f.to_string();
                if s.contains('.') {
                    s
                } else {
                    format!("{}.0", s)
                }
            }
            ExprKind::String(s) => format!("\"{}\"", s),
            ExprKind::RawString(s) => format!("`{}`", s),
            ExprKind::Rune(r) => format!("'{}'", r),
            ExprKind::StringInterpolation(parts) => {
                let mut result = String::from("\"");
                for part in parts {
                    match part {
                        StringPart::Literal(s) => result.push_str(s),
                        StringPart::Expr { expr, format } => {
                            result.push('{');
                            result.push_str(&self.format_expr(expr));
                            if let Some(fmt) = format {
                                result.push(':');
                                result.push_str(fmt);
                            }
                            result.push('}');
                        }
                    }
                }
                result.push('"');
                result
            }
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Nil => "nil".to_string(),
            ExprKind::Ident(name) => name.clone(),
            ExprKind::Binary { op, left, right } => {
                let left_str = self.format_expr(left);
                let right_str = self.format_expr(right);
                format!("{} {} {}", left_str, self.format_binop(*op), right_str)
            }
            ExprKind::Call {
                func,
                type_args,
                args,
            } => {
                let mut result = self.format_expr(func);

                // Special handling for make/new: type goes inside () not []
                let is_make_or_new =
                    matches!(&func.kind, ExprKind::Ident(name) if name == "make" || name == "new");

                if !type_args.is_empty() && !is_make_or_new {
                    result.push('[');
                    let targs: Vec<_> =
                        type_args.iter().map(Self::format_type_annotation).collect();
                    result.push_str(&targs.join(", "));
                    result.push(']');
                }

                result.push('(');

                // For make/new, type_args go first inside ()
                if is_make_or_new && !type_args.is_empty() {
                    let targs: Vec<_> =
                        type_args.iter().map(Self::format_type_annotation).collect();
                    result.push_str(&targs.join(", "));
                    if !args.is_empty() {
                        result.push_str(", ");
                    }
                }

                // Group args by line to preserve multi-line formatting
                if !args.is_empty() {
                    let mut lines: Vec<Vec<String>> = Vec::new();
                    let mut current_line_num = args[0].1.span.start.line;
                    let mut current_line_args: Vec<String> = Vec::new();

                    for (name, val, spread) in args.iter() {
                        let arg_line = val.span.start.line;
                        if arg_line != current_line_num && !current_line_args.is_empty() {
                            lines.push(current_line_args);
                            current_line_args = Vec::new();
                            current_line_num = arg_line;
                        }
                        let mut s = if let Some((n, _)) = name {
                            format!("{}: {}", n, self.format_expr(val))
                        } else {
                            self.format_expr(val)
                        };
                        if *spread {
                            s.push_str("...");
                        }
                        current_line_args.push(s);
                    }
                    if !current_line_args.is_empty() {
                        lines.push(current_line_args);
                    }

                    if lines.len() == 1 {
                        // All args on one line
                        result.push_str(&lines[0].join(", "));
                    } else {
                        // Multi-line: preserve line groupings
                        for (i, line_args) in lines.iter().enumerate() {
                            if i > 0 {
                                result.push('\n');
                                result.push_str(&"\t".repeat(indent + 1));
                            }
                            result.push_str(&line_args.join(", "));
                            if i < lines.len() - 1 {
                                result.push(',');
                            }
                        }
                    }
                }

                result.push(')');
                result
            }
            ExprKind::TypeInst { ty, type_args } => {
                // Type instantiation: Option[int]
                let mut result = self.format_expr(ty);
                if !type_args.is_empty() {
                    result.push('[');
                    let targs: Vec<_> =
                        type_args.iter().map(Self::format_type_annotation).collect();
                    result.push_str(&targs.join(", "));
                    result.push(']');
                }
                result
            }
            ExprKind::Field { expr, field, .. } => {
                format!("{}.{}", self.format_expr(expr), field)
            }
            ExprKind::Index { expr, index } => {
                format!("{}[{}]", self.format_expr(expr), self.format_expr(index))
            }
            ExprKind::Slice {
                expr,
                low,
                high,
                cap,
            } => {
                let mut result = format!("{}[", self.format_expr(expr));
                if let Some(l) = low {
                    result.push_str(&self.format_expr(l));
                }
                result.push(':');
                if let Some(h) = high {
                    result.push_str(&self.format_expr(h));
                }
                if let Some(c) = cap {
                    result.push(':');
                    result.push_str(&self.format_expr(c));
                }
                result.push(']');
                result
            }
            ExprKind::TypeAssert { expr, ty, .. } => {
                format!(
                    "{}.({})",
                    self.format_expr(expr),
                    Self::format_type_annotation(ty)
                )
            }
            ExprKind::NilAssert { expr } => {
                format!("{}.(!nil)", self.format_expr(expr))
            }
            ExprKind::ArrayLit { ty, elements } => {
                let mut result = String::new();
                if let Some(t) = ty {
                    result.push_str(&Self::format_type_annotation_with_indent(t, indent));
                }

                // Check if elements are struct literals - if so, use multi-line format
                let has_struct_elements = elements
                    .iter()
                    .any(|e| matches!(&e.kind, ExprKind::StructLit { .. }));

                if has_struct_elements && elements.len() > 1 {
                    result.push_str("{\n");
                    let elem_indent = "\t".repeat(indent + 1);
                    for elem in elements {
                        result.push_str(&elem_indent);
                        result.push_str(&self.format_expr_indent(elem, indent + 1));
                        result.push_str(",\n");
                    }
                    result.push_str(&"\t".repeat(indent));
                    result.push('}');
                } else {
                    result.push('{');
                    let elems: Vec<_> = elements
                        .iter()
                        .map(|e| self.format_expr_indent(e, indent))
                        .collect();
                    result.push_str(&elems.join(", "));
                    result.push('}');
                }
                result
            }
            ExprKind::StructLit { ty, fields, .. } => {
                let mut result = match ty {
                    Some(t) => Self::format_type_annotation(t),
                    None => String::new(),
                };

                // Check if fields span multiple lines
                let is_multiline = if fields.len() > 1 {
                    let first_line = fields.first().map(|(_, v)| v.span.start.line);
                    let last_line = fields.last().map(|(_, v)| v.span.start.line);
                    first_line != last_line
                } else {
                    false
                };

                if is_multiline && !fields.is_empty() {
                    // Multi-line: format with alignment
                    result.push_str("{\n");

                    // Calculate max field name length for alignment
                    let max_name_len = fields
                        .iter()
                        .filter_map(|(name, _)| name.as_ref().map(|n| n.len()))
                        .max()
                        .unwrap_or(0);

                    let field_indent = "\t".repeat(indent + 1);
                    for (name, val) in fields {
                        result.push_str(&field_indent);
                        if let Some(n) = name {
                            result.push_str(n);
                            result.push(':');
                            // Pad to align values
                            let padding = max_name_len - n.len() + 1;
                            for _ in 0..padding {
                                result.push(' ');
                            }
                            result.push_str(&self.format_expr_indent(val, indent + 1));
                        } else {
                            result.push_str(&self.format_expr_indent(val, indent + 1));
                        }
                        result.push_str(",\n");
                    }
                    result.push_str(&"\t".repeat(indent));
                    result.push('}');
                } else {
                    // Single-line
                    result.push('{');
                    let field_strs: Vec<_> = fields
                        .iter()
                        .map(|(name, val)| match name {
                            Some(n) => format!("{}: {}", n, self.format_expr(val)),
                            None => self.format_expr(val),
                        })
                        .collect();
                    result.push_str(&field_strs.join(", "));
                    result.push('}');
                }
                result
            }
            ExprKind::AnonStructLit { field_defs, fields } => {
                let mut result = String::from("struct {");
                for fd in field_defs {
                    result.push_str(&format!(
                        " {} {};",
                        fd.ident.name,
                        Self::format_type_annotation(&fd.ty)
                    ));
                }
                result.push_str(" }{");
                let field_strs: Vec<_> = fields
                    .iter()
                    .map(|(name, val)| match name {
                        Some(n) => format!("{}: {}", n, self.format_expr(val)),
                        None => self.format_expr(val),
                    })
                    .collect();
                result.push_str(&field_strs.join(", "));
                result.push('}');
                result
            }
            ExprKind::MapLit { ty, entries } => {
                let mut result = Self::format_type_annotation(ty);
                result.push('{');
                let entry_strs: Vec<_> = entries
                    .iter()
                    .map(|(k, v)| format!("{}: {}", self.format_expr(k), self.format_expr(v)))
                    .collect();
                result.push_str(&entry_strs.join(", "));
                result.push('}');
                result
            }
            ExprKind::Unary { op, operand } => {
                let op_str = match op {
                    UnaryOp::Neg => "-",
                    UnaryOp::Not => "!",
                    UnaryOp::Deref => "*",
                    UnaryOp::Ref => "&",
                    UnaryOp::Recv => "<-",
                };
                format!("{}{}", op_str, self.format_expr(operand))
            }
            ExprKind::FuncLit {
                params,
                returns,
                body,
            } => {
                let mut result = String::from("func(");
                result.push_str(&self.format_params(params));
                result.push(')');
                if !returns.is_empty() {
                    result.push(' ');
                    let is_named = !returns[0].ident.name.is_empty();
                    if returns.len() == 1 && !is_named {
                        result.push_str(&Self::format_type_annotation(&returns[0].ty));
                    } else {
                        result.push('(');
                        let rets: Vec<_> = returns
                            .iter()
                            .map(|p| {
                                if p.ident.name.is_empty() {
                                    Self::format_type_annotation(&p.ty)
                                } else {
                                    format!(
                                        "{} {}",
                                        p.ident.name,
                                        Self::format_type_annotation(&p.ty)
                                    )
                                }
                            })
                            .collect();
                        result.push_str(&rets.join(", "));
                        result.push(')');
                    }
                }
                // Format body using a temporary formatter
                result.push_str(" {\n");
                let mut tmp = Formatter::new();
                tmp.indent_level = indent + 1;
                tmp.format_block_contents(body);
                result.push_str(&tmp.output);
                result.push_str(&"\t".repeat(indent));
                result.push('}');
                result
            }
            ExprKind::Block(_) => {
                // Block expressions are rare and complex to format inline
                "{ ... }".to_string()
            }
            ExprKind::Paren(inner) => {
                format!("({})", self.format_expr(inner))
            }
        }
    }

    fn format_binop(&self, op: BinOp) -> &'static str {
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

    fn format_assign_op(&self, op: AssignOp) -> &'static str {
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

impl Default for Formatter {
    fn default() -> Self {
        Self::new()
    }
}

/// Format a Soppo source file
pub fn format_source(source: &str) -> Result<String, SoppoError> {
    let mut parser = Parser::new(source, FileId(0));
    let file = parser.parse_file()?;
    let mut formatter = Formatter::new();
    formatter.format_file(&file);
    Ok(formatter.output().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_preserves_pragmas() {
        let source = r#"package test

//go:generate something
//soppo:directive
func foo() {
	return
}
"#;
        let result = format_source(source).unwrap();
        assert!(result.contains("//go:generate something"));
        assert!(result.contains("//soppo:directive"));
    }

    #[test]
    fn test_format_preserves_comment_spacing() {
        let source = r#"package test

// Comment with space
//comment_without_space
func foo() {
	return
}
"#;
        let result = format_source(source).unwrap();
        assert!(result.contains("// Comment with space"));
        assert!(result.contains("//comment_without_space"));
    }

    #[test]
    fn test_format_doc_comment_not_duplicated() {
        let source = r#"package test

// This is a doc comment
func foo() {
	return
}
"#;
        let result = format_source(source).unwrap();
        let count = result.matches("// This is a doc comment").count();
        assert_eq!(count, 1, "Doc comment should appear exactly once");
    }

    #[test]
    fn test_format_preserves_blank_lines() {
        let source = r#"package main

func foo() {
	a := 1

	b := 2

	if true {
		c := 3

		d := 4
	}
}
"#;
        let result = format_source(source).unwrap();
        assert!(result.contains("a := 1\n\n\tb := 2"));
        assert!(result.contains("c := 3\n\n\t\td := 4"));
    }

    #[test]
    fn test_format_infinite_loop() {
        let source = r#"package main

func foo() {
	for {
		break
	}
}
"#;
        let result = format_source(source).unwrap();
        assert!(
            result.contains("for {"),
            "Infinite loop should be 'for {{' not 'for ; ; {{'"
        );
        assert!(!result.contains("for ;"), "Should not contain semicolons");
    }

    #[test]
    fn test_format_slice_type_not_duplicated() {
        let source = r#"package main

func foo(items []string) []string {
	return items
}
"#;
        let result = format_source(source).unwrap();
        assert!(result.contains("[]string"));
        assert!(
            !result.contains("[]string[string]"),
            "Slice type should not have duplicated type arg"
        );
    }

    #[test]
    fn test_format_escape_sequences() {
        let source = r#"package main

func foo() {
	a := "hello\nworld"
	b := "tab\there"
	c := "quote\"test"
}
"#;
        let result = format_source(source).unwrap();
        assert!(
            result.contains(r#""hello\nworld""#),
            "Newline escape should be preserved"
        );
        assert!(
            result.contains(r#""tab\there""#),
            "Tab escape should be preserved"
        );
        assert!(
            result.contains(r#""quote\"test""#),
            "Quote escape should be preserved"
        );
    }

    #[test]
    fn test_format_idempotent() {
        let source = r#"package main

import (
	"fmt"
	"os"
)

// Doc comment
type Foo struct {
	x int
}

func (f Foo) bar() int {
	if f.x > 0 {
		return f.x
	} else {
		return 0
	}
}

func main() {
	x := 1

	y := 2

	for {
		break
	}

	fmt.Println(x, y)
}
"#;
        let first = format_source(source).unwrap();
        let second = format_source(&first).unwrap();
        assert_eq!(first, second, "Formatting should be idempotent");
    }

    #[test]
    fn test_format_anon_struct_uses_tabs() {
        let source = r#"package main

func foo() {
    tests := []struct {
        name string
        age  int
    }{}
}
"#;
        let result = format_source(source).unwrap();
        // Should use tabs for indentation, not spaces
        assert!(
            result.contains("\t\tname"),
            "Anonymous struct fields should use tab indentation"
        );
        assert!(
            !result.contains("    name"),
            "Should not use space indentation"
        );
    }

    #[test]
    fn test_format_array_struct_multiline() {
        let source = r#"package main

func foo() {
    tests := []struct {
        x int
    }{
        {x: 1},
        {x: 2},
        {x: 3},
    }
}
"#;
        let result = format_source(source).unwrap();
        // Each struct literal should be on its own line
        assert!(
            result.contains("{x: 1},\n"),
            "Each struct element should be on its own line"
        );
        assert!(
            result.contains("{x: 2},\n"),
            "Each struct element should be on its own line"
        );
    }

    #[test]
    fn test_format_grouped_struct_fields() {
        let source = r#"package main

func foo() {
    tests := []struct {
        a, b, c int
        name    string
    }{}
}
"#;
        let result = format_source(source).unwrap();
        // Grouped fields should stay grouped
        assert!(
            result.contains("a, b, c int"),
            "Grouped struct fields should be preserved"
        );
    }

    #[test]
    fn test_format_else_block_with_if_preserved() {
        let source = r#"package main

func foo() {
    if true {
        println("a")
    } else {
        if false {
            println("b")
        }
    }
}
"#;
        let result = format_source(source).unwrap();
        // Should NOT collapse else { if into else if
        assert!(
            result.contains("} else {\n"),
            "else block should not be collapsed into else if"
        );
        assert!(
            result.contains("\t\tif false"),
            "Nested if should remain nested"
        );
    }

    #[test]
    fn test_format_else_if_preserved() {
        let source = r#"package main

func foo() {
    if true {
        println("a")
    } else if false {
        println("b")
    }
}
"#;
        let result = format_source(source).unwrap();
        // Should keep else if as else if
        assert!(
            result.contains("} else if false {"),
            "else if should remain as else if"
        );
    }

    #[test]
    fn test_format_trailing_comment_aligned() {
        let source = r#"package main

func foo() {
    tests := []struct {
        x int
    }{
        {x: 1},   // first
        {x: 123}, // second
    }
}
"#;
        let result = format_source(source).unwrap();
        // Trailing comments should be aligned
        assert!(
            result.contains("{x: 1},   // first"),
            "Shorter element should be padded for alignment"
        );
        assert!(
            result.contains("{x: 123}, // second"),
            "Longest element should have one space"
        );
    }

    #[test]
    fn test_format_make_not_brackets() {
        let source = r#"package main

func foo() {
    ch := make(chan int)
    m := make(map[string]int)
}
"#;
        let result = format_source(source).unwrap();
        // make should use parentheses, not brackets
        assert!(
            result.contains("make(chan int)"),
            "make should use parentheses"
        );
        assert!(!result.contains("make["), "make should not use brackets");
    }

    #[test]
    fn test_format_multiline_call_preserved() {
        let source = r#"package main

func foo() {
    t.Errorf("format %d %d",
        a, b, c)
}
"#;
        let result = format_source(source).unwrap();
        // Multi-line call should be preserved
        assert!(
            result.contains("\"format %d %d\",\n"),
            "First line should end with comma and newline"
        );
        assert!(
            result.contains("\t\ta, b, c)"),
            "Second line should have args grouped"
        );
    }

    #[test]
    fn test_format_multiline_struct_literal_preserved() {
        let source = r#"package main

func foo() {
    config := Config{
        DefaultSop: &sopVersion,
        DefaultGo:  &goVersion,
    }
}
"#;
        let result = format_source(source).unwrap();
        // Multi-line struct literal should be preserved with alignment
        assert!(
            result.contains("Config{\n"),
            "Struct literal should be multi-line"
        );
        // DefaultGo (9 chars) should have 2 spaces after : to align with DefaultSop (10 chars)
        assert!(
            result.contains("DefaultSop: &sopVersion"),
            "DefaultSop should have 1 space after colon"
        );
        assert!(
            result.contains("DefaultGo:  &goVersion"),
            "DefaultGo should have 2 spaces after colon for alignment"
        );
    }
}
