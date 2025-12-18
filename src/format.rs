use crate::error::SoppoError;
use crate::syntax::{
    Arm, AssignOp, BinOp, Block, Comment, ConstDecl, Decl, EnumVariant, Expr, ExprKind, Field,
    FieldPattern, File, FileId, FuncDecl, Generic, Import, InterfaceMethod, Literal, Param, Parser,
    Pattern, PatternKind, SelectCase, SelectCaseKind, Stmt, StmtKind, StringPart, TypeAnnotation,
    TypeDecl, TypeKind, UnaryOp,
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
                for import in &file.imports {
                    self.emit_comments_before(import.span.start.line);
                    self.emit_indent();
                    if let Some(alias) = &import.alias {
                        self.emit(&format!("{} \"{}\"", alias, import.path));
                    } else {
                        self.emit(&format!("\"{}\"", import.path));
                    }
                    self.emit_trailing_comment(import.span.start.line);
                    self.output.push('\n');
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
            self.emit(&self.format_type_annotation(ty));
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
                self.emit(&self.format_type_annotation(ty));
            }
            self.emit(" = ");
            self.emit(&self.format_expr(&c.value));
            self.emit_trailing_comment(c.span.start.line);
            self.output.push('\n');
        }
        self.dedent();
        self.emit_line(")");
    }

    fn format_type_decl(&mut self, t: &TypeDecl) {
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
                self.emit(&self.format_type_annotation(target));
                self.output.push('\n');
            }
            TypeKind::Definition { target } => {
                self.emit(" ");
                self.emit(&self.format_type_annotation(target));
                self.output.push('\n');
            }
            TypeKind::Enum { variants } => {
                self.emit(" enum {\n");
                self.indent();
                for variant in variants {
                    self.format_enum_variant(variant);
                }
                self.dedent();
                self.emit_line("}");
            }
            TypeKind::Struct { fields } => {
                self.emit(" struct {\n");
                self.indent();
                for field in fields {
                    self.format_field(field);
                }
                self.dedent();
                self.emit_line("}");
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

    fn format_enum_variant(&mut self, variant: &EnumVariant) {
        match variant {
            EnumVariant::Unit { ident } => {
                self.emit_line(&ident.name);
            }
            EnumVariant::Single { ident, ty } => {
                self.emit_indent();
                self.emit(&ident.name);
                self.emit(" ");
                self.emit(&self.format_type_annotation(ty));
                self.output.push('\n');
            }
            EnumVariant::Struct { ident, fields } => {
                self.emit_indent();
                self.emit(&format!("{} struct {{\n", ident.name));
                self.indent();
                for field in fields {
                    self.format_field(field);
                }
                self.dedent();
                self.emit_line("}");
            }
        }
    }

    fn format_field(&mut self, field: &Field) {
        self.emit_indent();
        self.emit(&field.ident.name);
        self.emit(" ");
        self.emit(&self.format_type_annotation(&field.ty));
        if let Some(tag) = &field.tag {
            self.emit(&format!(" `{}`", tag));
        }
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
                self.emit(&self.format_type_annotation(&method.returns[0]));
            } else {
                self.emit("(");
                let returns: Vec<_> = method
                    .returns
                    .iter()
                    .map(|t| self.format_type_annotation(t))
                    .collect();
                self.emit(&returns.join(", "));
                self.emit(")");
            }
        }
        self.output.push('\n');
    }

    fn format_func_decl(&mut self, f: &FuncDecl) {
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
                self.format_type_annotation(&recv.ty)
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
                self.emit(&self.format_type_annotation(&f.returns[0].ty));
            } else {
                self.emit("(");
                let returns: Vec<_> = f
                    .returns
                    .iter()
                    .map(|p| {
                        if p.ident.name.is_empty() {
                            self.format_type_annotation(&p.ty)
                        } else {
                            format!("{} {}", p.ident.name, self.format_type_annotation(&p.ty))
                        }
                    })
                    .collect();
                self.emit(&returns.join(", "));
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
        params
            .iter()
            .map(|p| format!("{} {}", p.ident.name, self.format_type_annotation(&p.ty)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn format_type_annotation(&self, ty: &TypeAnnotation) -> String {
        let mut result = String::new();

        if ty.nullable {
            result.push('?');
        }

        result.push_str(&ty.name);

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
                .map(|a| self.format_type_annotation(a))
                .collect();
            result.push_str(&args.join(", "));
            result.push(']');
        }

        result
    }

    fn format_block_contents(&mut self, block: &Block) {
        let mut prev_end_line = 0;
        for stmt in &block.stmts {
            // Preserve blank lines between statements
            if prev_end_line > 0 && stmt.span.start.line > prev_end_line + 1 {
                self.output.push('\n');
            }
            self.emit_comments_before(stmt.span.start.line);
            self.format_stmt(stmt);
            prev_end_line = stmt.span.end.line;
        }
    }

    fn format_stmt(&mut self, stmt: &Stmt) {
        let line = stmt.span.start.line;
        match &stmt.kind {
            StmtKind::Decl { ident, value } => {
                self.emit_indent();
                self.emit(&format!("{} := ", ident.name));
                self.emit(&self.format_expr(value));
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
                    self.emit(&self.format_type_annotation(t));
                }
                if let Some(v) = value {
                    self.emit(" = ");
                    self.emit(&self.format_expr(v));
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
                    self.emit(&self.format_type_annotation(t));
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
                    self.emit(&self.format_type_annotation(t));
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
                    self.emit(&self.format_type_annotation(t));
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
                self.emit(&self.format_expr(value));
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
                    self.emit_line("} else {");
                    self.indent();
                    self.format_block_contents(else_b);
                    self.dedent();
                }
                self.emit_line("}");
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

    /// Format a statement without indentation or newline (for inline use)
    fn format_stmt_inline(&self, stmt: &Stmt) -> String {
        match &stmt.kind {
            StmtKind::Decl { ident, value } => {
                format!("{} := {}", ident.name, self.format_expr(value))
            }
            StmtKind::Assign { target, value } => {
                format!("{} = {}", self.format_expr(target), self.format_expr(value))
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
            PatternKind::Variant(name) => name.clone(),
            PatternKind::Literal(lit) => self.format_literal(lit),
            PatternKind::Destructor { name, binding } => {
                format!("{}({})", name, binding.name)
            }
            PatternKind::StructDestructor { name, fields, rest } => {
                let mut result = format!("{}{{", name);
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
            Literal::Integer(n) => n.to_string(),
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

    fn format_expr(&self, expr: &Expr) -> String {
        match &expr.kind {
            ExprKind::Integer(n) => n.to_string(),
            ExprKind::Float(f) => {
                let s = f.to_string();
                if s.contains('.') {
                    s
                } else {
                    format!("{}.0", s)
                }
            }
            ExprKind::String(s) => format!("\"{}\"", s),
            ExprKind::Rune(r) => format!("'{}'", r),
            ExprKind::StringInterpolation(parts) => {
                let mut result = String::from("\"");
                for part in parts {
                    match part {
                        StringPart::Literal(s) => result.push_str(s),
                        StringPart::Expr(e) => {
                            result.push('{');
                            result.push_str(&self.format_expr(e));
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
                if !type_args.is_empty() {
                    result.push('[');
                    let targs: Vec<_> = type_args
                        .iter()
                        .map(|t| self.format_type_annotation(t))
                        .collect();
                    result.push_str(&targs.join(", "));
                    result.push(']');
                }
                result.push('(');
                let arg_strs: Vec<_> = args
                    .iter()
                    .map(|(name, val)| {
                        if let Some((n, _)) = name {
                            format!("{}: {}", n, self.format_expr(val))
                        } else {
                            self.format_expr(val)
                        }
                    })
                    .collect();
                result.push_str(&arg_strs.join(", "));
                result.push(')');
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
            ExprKind::TypeAssert { expr, ty } => {
                format!(
                    "{}.({})",
                    self.format_expr(expr),
                    self.format_type_annotation(ty)
                )
            }
            ExprKind::NilAssert { expr } => {
                format!("{}.(!nil)", self.format_expr(expr))
            }
            ExprKind::ArrayLit { ty, elements } => {
                let mut result = String::new();
                if let Some(t) = ty {
                    result.push_str(&self.format_type_annotation(t));
                }
                result.push('{');
                let elems: Vec<_> = elements.iter().map(|e| self.format_expr(e)).collect();
                result.push_str(&elems.join(", "));
                result.push('}');
                result
            }
            ExprKind::StructLit { ty, fields } => {
                let mut result = self.format_type_annotation(ty);
                result.push('{');
                let field_strs: Vec<_> = fields
                    .iter()
                    .map(|(name, val)| format!("{}: {}", name, self.format_expr(val)))
                    .collect();
                result.push_str(&field_strs.join(", "));
                result.push('}');
                result
            }
            ExprKind::AnonStructLit { field_defs, fields } => {
                let mut result = String::from("struct {");
                for fd in field_defs {
                    result.push_str(&format!(
                        " {} {};",
                        fd.ident.name,
                        self.format_type_annotation(&fd.ty)
                    ));
                }
                result.push_str(" }{");
                let field_strs: Vec<_> = fields
                    .iter()
                    .map(|(name, val)| format!("{}: {}", name, self.format_expr(val)))
                    .collect();
                result.push_str(&field_strs.join(", "));
                result.push('}');
                result
            }
            ExprKind::MapLit { ty, entries } => {
                let mut result = self.format_type_annotation(ty);
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
                body: _,
            } => {
                let mut result = String::from("func(");
                result.push_str(&self.format_params(params));
                result.push(')');
                if !returns.is_empty() {
                    result.push(' ');
                    let is_named = !returns[0].ident.name.is_empty();
                    if returns.len() == 1 && !is_named {
                        result.push_str(&self.format_type_annotation(&returns[0].ty));
                    } else {
                        result.push('(');
                        let rets: Vec<_> = returns
                            .iter()
                            .map(|p| {
                                if p.ident.name.is_empty() {
                                    self.format_type_annotation(&p.ty)
                                } else {
                                    format!(
                                        "{} {}",
                                        p.ident.name,
                                        self.format_type_annotation(&p.ty)
                                    )
                                }
                            })
                            .collect();
                        result.push_str(&rets.join(", "));
                        result.push(')');
                    }
                }
                result.push_str(" { ");
                // For simple bodies, inline; otherwise would need multi-line
                // This is a simplification - proper formatting would be more complex
                result.push_str("... }");
                result
            }
            ExprKind::Block(_) => {
                // Block expressions are rare and complex to format inline
                "{ ... }".to_string()
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
}
