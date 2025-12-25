mod decl;
mod expr;
mod stmt;

use std::collections::HashSet;

use crate::syntax::{AssignOp, BinOp, Comment, Generic, Stmt};
use crate::types::{GlobalCtxt, Infer, Type};

/// Current version of generated Soppo markers.
/// Bump this when making breaking changes to the marker format.
pub const MARKER_VERSION: &str = "v1";

/// Code generator for emitting Go code
pub struct Codegen {
    pub(crate) output: String,
    indent_level: usize,
    pub(crate) global_state: GlobalCtxt,
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
            current_return_types: Vec::new(),
            error_var_counter: 0,
            comments: Vec::new(),
            comment_idx: 0,
            module_path: None,
            output_dir: None,
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
    ) -> Self {
        Self {
            module_path: Some(module_path),
            output_dir,
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

    /// Check if a typed expression is a Go-compatible constant expression.
    /// Go const only allows compile-time constant expressions (literals, arithmetic on literals, etc.)
    /// For non-const expressions, we need to emit `var` instead.
    pub(crate) fn is_go_const_expr(expr: &crate::types::ast::TypedExpr) -> bool {
        use crate::types::ast::TypedExprKind;

        match &expr.kind {
            // Literals are always const
            TypedExprKind::Integer(_, _)
            | TypedExprKind::Float(_)
            | TypedExprKind::String(_)
            | TypedExprKind::RawString(_)
            | TypedExprKind::Rune(_)
            | TypedExprKind::Bool(_) => true,

            // Binary operations are const if both operands are const
            TypedExprKind::Binary { left, right, .. } => {
                Self::is_go_const_expr(left) && Self::is_go_const_expr(right)
            }

            // Unary operations are const if the operand is const
            TypedExprKind::Unary { operand, .. } => Self::is_go_const_expr(operand),

            // Parenthesised expressions are const if the inner is const
            TypedExprKind::Paren(expr) => Self::is_go_const_expr(expr),

            // Type conversions to basic types are const if the value is const
            TypedExprKind::TypeConversion { target_ty, value } => {
                let ty_name = match target_ty {
                    Type::Con { sym, .. } => &sym.name,
                    _ => return false,
                };
                // Only basic type conversions are allowed in const
                matches!(
                    ty_name.as_str(),
                    "int"
                        | "int8"
                        | "int16"
                        | "int32"
                        | "int64"
                        | "uint"
                        | "uint8"
                        | "uint16"
                        | "uint32"
                        | "uint64"
                        | "float32"
                        | "float64"
                        | "string"
                        | "rune"
                        | "byte"
                ) && Self::is_go_const_expr(value)
            }

            // Identifiers could be const if they reference other constants,
            // but we can't easily determine this, so we conservatively return false
            // (the Go compiler will error if we emit `var` for an iota-based const)
            TypedExprKind::Ident(_) => false,

            // Everything else (function calls, struct literals, etc.) is not const
            _ => false,
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

    /// Generate Go code for a list of statements (used for doctests).
    /// This creates a minimal type inference context, infers types for the statements,
    /// and uses the typed codegen.
    pub fn gen_statements(&mut self, stmts: &[Stmt]) {
        // Create a minimal type inference context with existing global state
        let mut infer = match Infer::with_global_state(self.global_state.clone()) {
            Ok(infer) => infer,
            Err(_) => return, // Can't create infer context
        };

        for stmt in stmts {
            // Infer types and get the typed statement directly
            let typed_stmt = infer.infer_stmt(stmt);
            self.gen_stmt(&typed_stmt);
        }
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
            self.output.push('\t');
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

    /// Convert a struct type name for codegen:
    /// - `Type` → `Type` (simple type)
    /// - `pkg.Type` → `pkg.Type` (cross-package type)
    /// - `Type.Variant` → `Type_Variant` (enum variant)
    /// - `pkg.Type.Variant` → `pkg.Type_Variant` (cross-package enum variant)
    pub(crate) fn convert_struct_type_name(&self, name: &str) -> String {
        let parts: Vec<&str> = name.split('.').collect();
        match parts.len() {
            0 | 1 => name.to_string(), // Simple type
            2 => {
                // Could be pkg.Type or Type.Variant
                // Use the type system to check if it's an enum
                if self.global_state.is_enum(parts[0]) {
                    // Enum variant - convert to Type_Variant
                    format!("{}_{}", parts[0], parts[1])
                } else {
                    // pkg.Type - keep as is
                    name.to_string()
                }
            }
            _ => {
                // pkg.Type.Variant or deeper
                // Check if the second-to-last part is an enum in the package context
                let prefix = parts[..parts.len() - 2].join(".");
                let type_name = parts[parts.len() - 2];
                let variant = parts[parts.len() - 1];

                // For cross-package enums, check using soppo_enum detection
                if parts.len() == 3 {
                    let pkg = parts[0];
                    if self.global_state.is_soppo_enum(pkg, type_name) {
                        return format!("{}.{}_{}", pkg, type_name, variant);
                    }
                }

                // Default: assume enum variant pattern
                if prefix.is_empty() {
                    format!("{}_{}", type_name, variant)
                } else {
                    format!("{}.{}_{}", prefix, type_name, variant)
                }
            }
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

        // Parse and emit each field, handling inner nilable types and grouped fields
        for field_def in struct_body.split("; ") {
            if !field_def.is_empty() {
                // Handle grouped fields like "a, b, c int" - split from end to get type
                if let Some(last_space) = field_def.rfind(' ') {
                    let names_part = &field_def[..last_space];
                    let ty = &field_def[last_space + 1..];
                    // Check if field type has ? prefix (nilable)
                    if let Some(inner_ty) = ty.strip_prefix('?') {
                        // Nilable inner field
                        self.emit_line(&format!("{} {} //soppo:nilable", names_part, inner_ty));
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

/// Convert a Type to a Go type string
pub fn type_to_go_string(ty: &Type) -> String {
    match ty {
        Type::Con { sym, args, .. } => {
            let name = sym.name.as_str();
            let module = &sym.module.0;

            // Build the full qualified name if module is present
            let full_name = if module.is_empty() {
                name.to_string()
            } else {
                format!("{}.{}", module, name)
            };

            // Handle pointer types - they store inner type in args but don't emit as generics
            if let Some(stripped) = name.strip_prefix('*') {
                // For qualified pointer types like *types.Config, the inner type is in args
                // Check if there's an inner type with a module
                if let Some(inner_ty) = args.first() {
                    let inner_str = type_to_go_string(inner_ty);
                    // If the inner type has a module prefix, use it
                    if inner_str.contains('.') || !module.is_empty() {
                        return format!("*{}", inner_str);
                    }
                }
                // Fallback: qualified pointer type like *types.Config with module in sym
                if !module.is_empty() {
                    return format!("*{}.{}", module, stripped);
                }
                return name.to_string();
            }

            // Handle slice types - they store element type in args but don't emit as generics
            if let Some(stripped) = name.strip_prefix("[]") {
                // For qualified slice types like []types.Config, the inner type is in args
                if let Some(inner_ty) = args.first() {
                    let inner_str = type_to_go_string(inner_ty);
                    if inner_str.contains('.') || !module.is_empty() {
                        return format!("[]{}", inner_str);
                    }
                }
                // Fallback: qualified slice type with module in sym
                if !module.is_empty() {
                    return format!("[]{}.{}", module, stripped);
                }
                return name.to_string();
            }

            // Handle channel types
            if name.starts_with("chan ")
                || name.starts_with("<-chan ")
                || name.starts_with("chan<-")
            {
                return full_name;
            }

            // Handle map types - format is map[K]V
            if name.starts_with("map[") {
                return full_name;
            }

            // Handle variadic types: variadic[T] -> ...T
            if name == "variadic" && args.len() == 1 {
                return format!("...{}", type_to_go_string(&args[0]));
            }

            if args.is_empty() {
                // Handle special type names
                match name {
                    "()" => String::new(),
                    _ => full_name,
                }
            } else {
                // Generic type with arguments
                let arg_strs: Vec<String> = args.iter().map(type_to_go_string).collect();
                format!("{}[{}]", full_name, arg_strs.join(", "))
            }
        }
        Type::Func { args, ret, .. } => {
            // Function type: func(a int, b string) bool
            let param_strs: Vec<String> = args
                .iter()
                .map(|(name, ty)| {
                    if let Some(n) = name {
                        format!("{} {}", n, type_to_go_string(ty))
                    } else {
                        type_to_go_string(ty)
                    }
                })
                .collect();
            let ret_str = type_to_go_string(ret);
            if ret_str.is_empty() {
                format!("func({})", param_strs.join(", "))
            } else {
                format!("func({}) {}", param_strs.join(", "), ret_str)
            }
        }
        Type::Var(id) => {
            // Type variable - shouldn't appear in codegen, but handle gracefully
            format!("T{}", id)
        }
        Type::Never => {
            // Never type - shouldn't appear in codegen
            "/* never */".to_string()
        }
        Type::Error => "/* error type */".to_string(),
    }
}

/// Get the default format specifier for a Type
pub(crate) fn default_format_for_type(ty: &Type) -> String {
    match ty {
        Type::Con { sym, .. } => {
            match sym.name.as_str() {
                // String types
                "string" => "%s".to_string(),

                // Integer types
                "int" | "int8" | "int16" | "int32" | "int64" | "uint" | "uint8" | "uint16"
                | "uint32" | "uint64" | "uintptr" | "byte" | "rune" => "%d".to_string(),

                // Float types
                "float32" | "float64" => "%g".to_string(),

                // Boolean
                "bool" => "%t".to_string(),

                // Default to %v for other types
                _ => "%v".to_string(),
            }
        }
        // Functions, type variables, never, and errors all use %v
        Type::Func { .. } | Type::Var(_) | Type::Never | Type::Error => "%v".to_string(),
    }
}

/// Parse field definitions from an anonymous struct type name.
/// Input: "struct { input string; major, minor, patch int; wantErr bool }"
/// Output: [("input", "string"), ("major", "int"), ("minor", "int"), ("patch", "int"), ("wantErr", "bool")]
pub(crate) fn parse_anon_struct_fields(ty: &Type) -> Option<Vec<(String, String)>> {
    let s = match ty {
        Type::Con { sym, .. } => &sym.name,
        _ => return None,
    };

    // Handle both "struct{...}" and "struct { ... }" formats
    let inner = s
        .strip_prefix("struct {")
        .or_else(|| s.strip_prefix("struct{"))?
        .strip_suffix('}')?
        .trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }

    let mut fields = Vec::new();
    for field_def in inner.split(';') {
        let field_def = field_def.trim();
        if field_def.is_empty() {
            continue;
        }
        // Handle grouped fields like "a, b, c int" - split from end to get type
        if let Some(last_space) = field_def.rfind(' ') {
            let names_part = &field_def[..last_space];
            let type_part = field_def[last_space + 1..].trim();
            // Split on comma for grouped names
            for name in names_part.split(',') {
                let name = name.trim();
                if !name.is_empty() {
                    fields.push((name.to_string(), type_part.to_string()));
                }
            }
        }
    }
    Some(fields)
}

/// Parse field names from an anonymous struct type name.
/// Input: "struct { input string; major, minor, patch int; wantErr bool }"
/// Output: ["input", "major", "minor", "patch", "wantErr"]
pub(crate) fn parse_anon_struct_field_names(ty: &Type) -> Option<Vec<String>> {
    let s = match ty {
        Type::Con { sym, .. } => &sym.name,
        _ => return None,
    };

    // Handle both "struct{...}" and "struct { ... }" formats
    let inner = s
        .strip_prefix("struct {")
        .or_else(|| s.strip_prefix("struct{"))?
        .strip_suffix('}')?
        .trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }

    let mut names = Vec::new();
    for field_def in inner.split(';') {
        let field_def = field_def.trim();
        if field_def.is_empty() {
            continue;
        }
        // Handle grouped fields like "a, b, c int" - split from end to get type
        if let Some(last_space) = field_def.rfind(' ') {
            let names_part = &field_def[..last_space];
            // Split on comma for grouped names
            for name in names_part.split(',') {
                let name = name.trim();
                if !name.is_empty() {
                    names.push(name.to_string());
                }
            }
        }
    }
    Some(names)
}
