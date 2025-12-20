use super::Codegen;
use crate::syntax::{Expr, ExprKind, IntFormat, UnaryOp};

impl Codegen {
    /// Generate an expression
    pub(crate) fn gen_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Integer(n, fmt) => {
                // Preserve the original format in generated Go code
                match fmt {
                    IntFormat::Decimal => self.emit(n.to_string()),
                    IntFormat::Octal => self.emit(format!("0o{:o}", n)),
                    IntFormat::Hex => self.emit(format!("0x{:x}", n)),
                    IntFormat::Binary => self.emit(format!("0b{:b}", n)),
                }
            }

            ExprKind::Float(f) => {
                let s = f.to_string();
                // Ensure we have a decimal point for Go
                if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                    self.emit(format!("{}.0", s));
                } else {
                    self.emit(&s);
                }
            }

            ExprKind::String(s) => {
                self.emit(format!("\"{}\"", s));
            }

            ExprKind::RawString(s) => {
                self.emit(format!("`{}`", s));
            }

            ExprKind::Rune(r) => {
                self.emit(format!("'{}'", r));
            }

            ExprKind::StringInterpolation(parts) => {
                // Generate fmt.Sprintf("format", args...)
                self.needed_imports.insert("fmt".to_string());
                self.emit("fmt.Sprintf(\"");

                // Build the format string and collect expressions
                let mut exprs: Vec<&crate::syntax::Expr> = Vec::new();
                for part in parts {
                    match part {
                        crate::syntax::StringPart::Literal(s) => {
                            // Escape % characters for fmt.Sprintf
                            self.emit(s.replace('%', "%%"));
                        }
                        crate::syntax::StringPart::Expr { expr, format } => {
                            // Use explicit format if provided, otherwise use type-based default
                            let fmt = format
                                .as_deref()
                                .map(|f| format!("%{}", f))
                                .unwrap_or_else(|| default_format_for_expr(expr));
                            self.emit(&fmt);
                            exprs.push(expr);
                        }
                    }
                }
                self.emit("\"");

                // Add the expressions as arguments
                for expr in exprs {
                    self.emit(", ");
                    self.gen_expr(expr);
                }
                self.emit(")");
            }

            ExprKind::Bool(b) => {
                self.emit(if *b { "true" } else { "false" });
            }

            ExprKind::Nil => {
                self.emit("nil");
            }

            ExprKind::Ident(name) => {
                self.emit(name);
            }

            ExprKind::Binary { op, left, right } => {
                self.gen_expr(left);
                self.emit(format!(" {} ", self.go_binop(op)));
                self.gen_expr(right);
            }

            ExprKind::Call {
                func,
                type_args,
                args,
            } => {
                // Special handling for make and new built-ins
                if let ExprKind::Ident(name) = &func.kind
                    && (name == "make" || name == "new")
                    && !type_args.is_empty()
                {
                    // make(type, args...) or new(type)
                    self.emit(name);
                    self.emit("(");
                    // Type is first argument
                    self.emit(&type_args[0].name);
                    // Additional arguments
                    for (_, arg, spread) in args {
                        self.emit(", ");
                        self.gen_expr(arg);
                        if *spread {
                            self.emit("...");
                        }
                    }
                    self.emit(")");
                    return;
                }

                // Reorder args based on named arguments if needed
                let ordered_args = self.reorder_call_args(func, args);

                self.gen_expr(func);
                // Emit type arguments if present: func[int, string](args)
                if !type_args.is_empty() {
                    self.emit("[");
                    for (i, ty) in type_args.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.emit(self.go_type(&ty.name));
                    }
                    self.emit("]");
                }
                self.emit("(");
                for (i, (arg, spread)) in ordered_args.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(arg);
                    if *spread {
                        self.emit("...");
                    }
                }
                self.emit(")");
            }

            ExprKind::Field {
                expr: base_expr,
                field,
                ..
            } => {
                // Check if this is a generic enum variant like Option[int].None
                if let ExprKind::Call {
                    func,
                    type_args,
                    args,
                } = &base_expr.kind
                    && let ExprKind::Ident(type_name) = &func.kind
                    && !type_args.is_empty()
                    && args.is_empty()
                    && self.global_state.is_local_enum(type_name)
                {
                    // Generic enum unit variant: Option[int].None → Option_None[int]{}
                    self.emit(format!("{}_{}", type_name, field));
                    self.emit("[");
                    for (i, ty) in type_args.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.emit(self.go_type(&ty.name));
                    }
                    self.emit("]{}");
                    return;
                }

                // Check if this is an enum constructor like Colour.Red or pkg.Status.Active
                if let ExprKind::Ident(type_name) = &base_expr.kind {
                    // Check if it's a registered local enum type
                    if self.global_state.is_local_enum(type_name) {
                        // Enum values: Colour.Red → ColourRed (var or function)
                        self.emit(format!("{}{}", type_name, field));
                        // Check for inferred type args (from type inference)
                        if let Some(inferred_args) = expr.inferred_type_args.borrow().as_ref() {
                            self.emit("[");
                            for (i, arg) in inferred_args.iter().enumerate() {
                                if i > 0 {
                                    self.emit(", ");
                                }
                                self.emit(self.go_type(arg));
                            }
                            self.emit("]()");
                        }
                    } else {
                        // Regular field access (like fmt.Printf)
                        self.emit(type_name);
                        self.emit(".");
                        self.emit(field);
                    }
                } else if let ExprKind::Field {
                    expr: inner_expr,
                    field: type_name,
                    ..
                } = &base_expr.kind
                {
                    // Check for cross-package enum: pkg.Type.Variant
                    if let ExprKind::Ident(pkg_name) = &inner_expr.kind {
                        if self.global_state.is_soppo_enum(pkg_name, type_name) {
                            // Cross-package enum: types.Status.Active → types.StatusActive
                            self.emit(format!("{}.{}{}", pkg_name, type_name, field));
                        } else {
                            // Regular nested field access
                            self.gen_expr(base_expr);
                            self.emit(".");
                            self.emit(field);
                        }
                    } else {
                        // Regular nested field access
                        self.gen_expr(base_expr);
                        self.emit(".");
                        self.emit(field);
                    }
                } else {
                    // Regular field access on expression
                    self.gen_expr(base_expr);
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

            ExprKind::Slice {
                expr,
                low,
                high,
                cap,
            } => {
                self.gen_expr(expr);
                self.emit("[");
                if let Some(l) = low {
                    self.gen_expr(l);
                }
                self.emit(":");
                if let Some(h) = high {
                    self.gen_expr(h);
                }
                if let Some(c) = cap {
                    self.emit(":");
                    self.gen_expr(c);
                }
                self.emit("]");
            }

            ExprKind::TypeAssert { expr, ty, .. } => {
                // Always generate the type assertion - needed to convert
                // from interface type to concrete struct type
                let type_name = ty.name.replace('.', "_");
                self.gen_expr(expr);
                self.emit(".(");
                self.emit(&type_name);
                self.emit(")");
            }

            ExprKind::NilAssert { expr } => {
                // Nil assertion is compile-time only - just emit the inner expression
                self.gen_expr(expr);
            }

            ExprKind::ArrayLit { ty, elements } => {
                // Generate []type{elements} for slices or [size]type{elements} for arrays
                let anon_struct_fields = if let Some(ty) = ty {
                    let type_name = &ty.name;
                    if let Some(elem_name) = type_name.strip_prefix("[]") {
                        // Slice literal: []type{elements}
                        // Format anonymous struct types with proper multiline formatting
                        if elem_name.starts_with("struct {") || elem_name.starts_with("struct{") {
                            self.emit("[]");
                            self.emit_anon_struct_type(elem_name);
                        } else {
                            self.emit(self.go_type(type_name));
                        }
                        // Get field names for positional expansion
                        Self::parse_anon_struct_field_names(elem_name)
                    } else {
                        // Array literal: [size]type{elements}
                        self.emit("[");
                        self.emit(elements.len().to_string());
                        self.emit("]");
                        let elem_name = type_name;
                        if elem_name.starts_with("struct {") || elem_name.starts_with("struct{") {
                            self.emit_anon_struct_type(elem_name);
                        } else {
                            self.emit(self.go_type(type_name));
                        }
                        Self::parse_anon_struct_field_names(elem_name)
                    }
                } else {
                    // No type - infer as array with size
                    self.emit("[");
                    self.emit(elements.len().to_string());
                    self.emit("]");
                    None
                };

                // Check if elements span multiple lines (for formatting)
                let multiline = elements.len() > 1
                    && elements
                        .first()
                        .zip(elements.last())
                        .is_some_and(|(first, last)| first.span.start.line != last.span.start.line);

                self.emit("{");
                if multiline {
                    self.emit("\n");
                    self.indent();
                }
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        self.emit(",");
                        if multiline {
                            self.emit("\n");
                        } else {
                            self.emit(" ");
                        }
                    }
                    if multiline {
                        self.emit_indent();
                    }
                    // Generate element with field name context for positional expansion
                    self.gen_expr_with_struct_context(elem, anon_struct_fields.as_deref());
                }
                if multiline {
                    self.emit(",\n");
                    self.dedent();
                    self.emit_indent();
                }
                self.emit("}");
            }

            ExprKind::StructLit { ty, fields } => {
                // Generate Type{field: value, ...} or {field: value, ...} for implicit
                // For enum variants like Shape.Circle, convert to Shape_Circle
                // For cross-package types like types.User, keep as types.User
                // For cross-package enum variants like pkg.Type.Variant, convert to pkg.Type_Variant
                if let Some(ty) = ty {
                    let type_name = self.convert_struct_type_name(&ty.name);
                    self.emit(self.go_type(&type_name));
                    // For generic enum variants like Option[int].Some, emit type args
                    // Check explicit type args first, then inferred type args
                    if !ty.args.is_empty() {
                        self.emit("[");
                        for (i, arg) in ty.args.iter().enumerate() {
                            if i > 0 {
                                self.emit(", ");
                            }
                            self.emit(self.go_type(&arg.name));
                        }
                        self.emit("]");
                    } else if let Some(inferred_args) = expr.inferred_type_args.borrow().as_ref() {
                        // Use inferred type args from type inference
                        self.emit("[");
                        for (i, arg) in inferred_args.iter().enumerate() {
                            if i > 0 {
                                self.emit(", ");
                            }
                            self.emit(self.go_type(arg));
                        }
                        self.emit("]");
                    }
                }
                self.emit("{");
                for (i, (field_name, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    if let Some(name) = field_name {
                        self.emit(name);
                        self.emit(": ");
                    }
                    self.gen_expr(value);
                }
                self.emit("}");
            }

            ExprKind::AnonStructLit { field_defs, fields } => {
                // Generate struct { Name Type; ... }{Name: value, ...}
                self.emit("struct { ");
                for (i, field) in field_defs.iter().enumerate() {
                    if i > 0 {
                        self.emit("; ");
                    }
                    self.emit(&field.ident);
                    self.emit(" ");
                    self.emit(self.go_type(&field.ty.name));
                }
                self.emit(" }{");
                for (i, (field_name, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    if let Some(name) = field_name {
                        self.emit(name);
                        self.emit(": ");
                    }
                    self.gen_expr(value);
                }
                self.emit("}");
            }

            ExprKind::MapLit { ty, entries } => {
                // Generate map[K]V{key: value, ...}
                self.emit(&ty.name);
                self.emit("{");
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.gen_expr(key);
                    self.emit(": ");
                    self.gen_expr(value);
                }
                self.emit("}");
            }

            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Neg => {
                    self.emit("(-");
                    self.gen_expr(operand);
                    self.emit(")");
                }
                UnaryOp::Not => {
                    self.emit("(!");
                    self.gen_expr(operand);
                    self.emit(")");
                }
                UnaryOp::Ref => {
                    self.emit("(&");
                    self.gen_expr(operand);
                    self.emit(")");
                }
                UnaryOp::Deref => {
                    self.emit("(*");
                    self.gen_expr(operand);
                    self.emit(")");
                }
                UnaryOp::Recv => {
                    self.emit("(<-");
                    self.gen_expr(operand);
                    self.emit(")");
                }
            },

            ExprKind::FuncLit {
                params,
                returns,
                body,
            } => {
                self.emit("func(");
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.emit(&param.ident);
                    self.emit(" ");
                    self.emit(self.go_type(&param.ty.name));
                }
                self.emit(")");

                // Return types - handle named and unnamed
                if !returns.is_empty() {
                    let is_named = !returns[0].ident.name.is_empty();
                    if returns.len() == 1 && !is_named {
                        self.emit(" ");
                        self.emit(self.go_type(&returns[0].ty.name));
                    } else {
                        self.emit(" (");
                        for (i, ret) in returns.iter().enumerate() {
                            if i > 0 {
                                self.emit(", ");
                            }
                            if !ret.ident.name.is_empty() {
                                self.emit(&ret.ident.name);
                                self.emit(" ");
                            }
                            self.emit(self.go_type(&ret.ty.name));
                        }
                        self.emit(")");
                    }
                }

                self.emit(" ");
                self.gen_block(body);
            }

            ExprKind::Block(block) => {
                self.gen_block(block);
            }

            ExprKind::Paren(inner) => {
                self.emit("(");
                self.gen_expr(inner);
                self.emit(")");
            }
        }
    }

    /// Reorder function call arguments based on named arguments
    /// Returns (expr, spread) pairs in the correct order
    fn reorder_call_args<'a>(
        &self,
        func: &Expr,
        args: &'a [crate::syntax::CallArg],
    ) -> Vec<(&'a Expr, bool)> {
        // Check if any args are named
        let has_named = args.iter().any(|(name, _, _)| name.is_some());

        // If no named args, just return all in order
        if !has_named {
            return args.iter().map(|(_, arg, spread)| (arg, *spread)).collect();
        }

        // Look up parameter names (exclude variadic params)
        let param_names: Option<Vec<String>> = if let ExprKind::Ident(func_name) = &func.kind {
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
            let mut result: Vec<Option<(&Expr, bool)>> = vec![None; param_names.len()];
            let mut variadic_args: Vec<(&Expr, bool)> = Vec::new();
            let mut positional_args: Vec<(&Expr, bool)> = Vec::new();

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
            let mut ordered: Vec<(&Expr, bool)> = result.into_iter().flatten().collect();

            // Add variadic args at the end
            ordered.extend(variadic_args);

            ordered
        } else {
            // Unknown function - just use positional order (type checker would have errored)
            args.iter().map(|(_, arg, spread)| (arg, *spread)).collect()
        }
    }

    /// Generate a comma-ok expression (type assertion, map index, channel receive)
    /// Returns Some(code) if this is a comma-ok expression, None otherwise
    pub(crate) fn gen_comma_ok_expr(&mut self, expr: &Expr) -> Option<String> {
        match &expr.kind {
            // Type assertion: x.(T) -> native Go comma-ok
            ExprKind::TypeAssert {
                expr: inner, ty, ..
            } => {
                let mut code = String::new();
                let mut temp = Codegen::new();
                temp.gen_expr(inner);
                code.push_str(temp.output());
                code.push_str(".(");
                code.push_str(&ty.name.replace('.', "_"));
                code.push(')');
                Some(code)
            }

            // Map index and channel receive work natively with comma-ok
            // Just generate the expression normally
            ExprKind::Index { .. } => {
                let mut temp = Codegen::new();
                temp.gen_expr(expr);
                Some(temp.output().to_string())
            }

            ExprKind::Unary {
                op: UnaryOp::Recv, ..
            } => {
                let mut temp = Codegen::new();
                temp.gen_expr(expr);
                Some(temp.output().to_string())
            }

            _ => None,
        }
    }
}

impl Codegen {
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

    /// Parse field names from anonymous struct type like "struct { a int; b, c int }"
    fn parse_anon_struct_field_names(s: &str) -> Option<Vec<String>> {
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

    /// Parse anonymous struct fields into (name, type) pairs
    fn parse_anon_struct_fields(s: &str) -> Option<Vec<(String, String)>> {
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
                let ty = field_def[last_space + 1..].trim().to_string();
                // Split on comma for grouped names
                for name in names_part.split(',') {
                    let name = name.trim();
                    if !name.is_empty() {
                        fields.push((name.to_string(), ty.clone()));
                    }
                }
            }
        }
        Some(fields)
    }

    /// Emit anonymous struct type with proper multiline formatting
    fn emit_anon_struct_type(&mut self, s: &str) {
        let fields = match Self::parse_anon_struct_fields(s) {
            Some(f) => f,
            None => {
                // Fallback: emit as-is
                self.emit(s);
                return;
            }
        };

        if fields.is_empty() {
            self.emit("struct {}");
            return;
        }

        // Single field: emit on one line
        if fields.len() == 1 {
            self.emit("struct { ");
            self.emit(&fields[0].0);
            self.emit(" ");
            self.emit(&fields[0].1);
            self.emit(" }");
            return;
        }

        // Multiple fields: emit with proper alignment
        let max_name_len = fields.iter().map(|(n, _)| n.len()).max().unwrap_or(0);

        self.emit("struct {\n");
        self.indent();
        for (name, ty) in &fields {
            self.emit_indent();
            self.emit(name);
            // Pad to align types
            for _ in 0..(max_name_len - name.len() + 1) {
                self.emit(" ");
            }
            self.emit(ty);
            self.emit("\n");
        }
        self.dedent();
        self.emit_indent();
        self.emit("}");
    }

    /// Generate expression with struct field name context for positional expansion
    fn gen_expr_with_struct_context(&mut self, expr: &Expr, field_names: Option<&[String]>) {
        // Only apply context to implicit StructLit
        if let ExprKind::StructLit { ty: None, fields } = &expr.kind
            && let Some(names) = field_names
        {
            // Emit struct literal with positional fields expanded to named
            self.emit("{");
            for (i, (field_name, value)) in fields.iter().enumerate() {
                if i > 0 {
                    self.emit(", ");
                }
                // Use provided name or explicit name
                let name = match field_name {
                    Some(n) => n.as_str(),
                    None => names.get(i).map(|s| s.as_str()).unwrap_or(""),
                };
                if !name.is_empty() {
                    self.emit(name);
                    self.emit(": ");
                }
                self.gen_expr(value);
            }
            self.emit("}");
            return;
        }
        // Default: use normal expression generation
        self.gen_expr(expr);
    }
}

/// Determine the default format specifier based on expression kind.
/// Returns a format string like "%s", "%d", etc.
fn default_format_for_expr(expr: &crate::syntax::Expr) -> String {
    use crate::syntax::ExprKind;

    match &expr.kind {
        // Literals with known types
        ExprKind::String(_) | ExprKind::RawString(_) => "%s".to_string(),
        ExprKind::Integer(_, _) => "%d".to_string(),
        ExprKind::Bool(_) => "%t".to_string(),
        ExprKind::Rune(_) => "%c".to_string(),
        // Float keeps %v (better formatting than %f)
        ExprKind::Float(_) => "%v".to_string(),
        // Everything else uses %v
        _ => "%v".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{FileId, Parser};

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
    fn test_gen_binary_ops() {
        let source = "func test() int { return 1 + 2 * 3 }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser.parse_func_decl().unwrap();

        let mut codegen = Codegen::new();
        codegen.gen_func_decl(&func);

        let output = codegen.output();
        // No extra parentheses - Go has proper operator precedence
        assert!(output.contains("1 + 2 * 3"));
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
}
