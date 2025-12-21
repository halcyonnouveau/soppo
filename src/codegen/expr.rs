use super::{Codegen, type_to_go_string};
use super::{default_format_for_type, parse_anon_struct_field_names};
use crate::syntax::{IntFormat, UnaryOp};
use crate::types::Type;
use crate::types::ast::{TypedExpr, TypedExprKind, TypedStringPart};

impl Codegen {
    /// Generate an expression from TypedExpr
    pub(crate) fn gen_expr(&mut self, expr: &TypedExpr) {
        match &expr.kind {
            TypedExprKind::Integer(n, fmt) => match fmt {
                IntFormat::Decimal => self.emit(n.to_string()),
                IntFormat::Octal => self.emit(format!("0o{:o}", n)),
                IntFormat::Hex => self.emit(format!("0x{:x}", n)),
                IntFormat::Binary => self.emit(format!("0b{:b}", n)),
            },

            TypedExprKind::Float(f) => {
                let s = f.to_string();
                if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                    self.emit(format!("{}.0", s));
                } else {
                    self.emit(&s);
                }
            }

            TypedExprKind::String(s) => {
                self.emit(format!("\"{}\"", s));
            }

            TypedExprKind::RawString(s) => {
                self.emit(format!("`{}`", s));
            }

            TypedExprKind::Rune(r) => {
                self.emit(format!("'{}'", r));
            }

            TypedExprKind::Bool(b) => {
                self.emit(if *b { "true" } else { "false" });
            }

            TypedExprKind::Nil => {
                self.emit("nil");
            }

            TypedExprKind::Ident(name) => {
                self.emit(name);
            }

            TypedExprKind::StringInterpolation(parts) => {
                self.needed_imports.insert("fmt".to_string());
                self.emit("fmt.Sprintf(\"");

                let mut exprs: Vec<&TypedExpr> = Vec::new();
                for part in parts {
                    match part {
                        TypedStringPart::Literal(s) => {
                            self.emit(s.replace('%', "%%"));
                        }
                        TypedStringPart::Expr { expr, format } => {
                            // Use explicit format if provided, otherwise use type-based default
                            let fmt = format
                                .as_deref()
                                .map(|f| format!("%{}", f))
                                .unwrap_or_else(|| default_format_for_type(&expr.ty));
                            self.emit(&fmt);
                            exprs.push(expr);
                        }
                    }
                }
                self.emit("\"");

                for expr in exprs {
                    self.emit(", ");
                    self.gen_expr(expr);
                }
                self.emit(")");
            }

            TypedExprKind::Binary { op, left, right } => {
                self.gen_expr(left);
                self.emit(format!(" {} ", self.go_binop(op)));
                self.gen_expr(right);
            }

            TypedExprKind::Call {
                func,
                type_args,
                args,
            } => {
                // Special handling for make and new built-ins
                if let TypedExprKind::Ident(name) = &func.kind
                    && (name == "make" || name == "new")
                    && !type_args.is_empty()
                {
                    self.emit(name);
                    self.emit("(");
                    // Type is first argument
                    self.emit(type_to_go_string(&type_args[0]));
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
                // Emit type arguments if present
                if !type_args.is_empty() {
                    self.emit("[");
                    for (i, ty) in type_args.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.emit(type_to_go_string(ty));
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

            TypedExprKind::TypeConversion { target_ty, value } => {
                // Type conversion: int(x), []byte(s), pkg.Type(value)
                self.emit(type_to_go_string(target_ty));
                self.emit("(");
                self.gen_expr(value);
                self.emit(")");
            }

            TypedExprKind::TypeInst { ty } => {
                // Type instantiation: Option[int] for accessing generic type members
                // This is used as a base for field access, e.g., Option[int].None
                self.emit(type_to_go_string(ty));
            }

            TypedExprKind::PackageMember { pkg, member } => {
                // Package member access: fmt.Println → fmt.Println
                self.emit(format!("{}.{}", pkg, member));
            }

            TypedExprKind::EnumVariant { enum_ty, variant } => {
                // Enum variant access
                // Note: If this EnumVariant is the func of a Call expression,
                // the Call will add type args and arguments. We only add type args
                // here if they're fully resolved (not type variables), which indicates
                // this is a standalone variant usage like `Option[int].None`.
                if let Type::Con {
                    sym,
                    args: type_args,
                    ..
                } = enum_ty
                {
                    let type_name = &sym.name;
                    let is_local = self.global_state.is_local_enum(type_name);
                    let pkg_name = if sym.module.0.is_empty() {
                        None
                    } else {
                        Some(&sym.module.0)
                    };

                    // Check if all type args are resolved (no type variables)
                    let type_args_resolved = type_args.iter().all(|t| !t.contains_var());

                    if is_local {
                        // Local enum variant: Colour.Red → ColourRed
                        self.emit(format!("{}{}", type_name, variant));

                        // Add type args for generic enums only if fully resolved
                        // If unresolved, this EnumVariant is part of a Call which handles type args
                        if !type_args.is_empty() && type_args_resolved {
                            self.emit("[");
                            for (i, ty) in type_args.iter().enumerate() {
                                if i > 0 {
                                    self.emit(", ");
                                }
                                self.emit(type_to_go_string(ty));
                            }
                            self.emit("]()");
                        }
                    } else if let Some(pkg) = pkg_name {
                        // Cross-package enum: types.Status.Active → types.StatusActive
                        self.emit(format!("{}.{}{}", pkg, type_name, variant));
                    } else {
                        // Fallback
                        self.emit(format!("{}{}", type_name, variant));
                    }
                } else {
                    // Fallback for non-Con types
                    self.emit(variant);
                }
            }

            TypedExprKind::Field {
                expr: base_expr,
                field,
                ..
            } => {
                // Check if this is a generic enum variant with explicit type args
                // (base is TypeInst like Option[int])
                if let TypedExprKind::TypeInst { ty } = &base_expr.kind
                    && let Type::Con {
                        sym,
                        args: type_args,
                        ..
                    } = ty
                    && !type_args.is_empty()
                    && self.global_state.is_local_enum(&sym.name)
                {
                    // Generic enum unit variant: Option[int].None → OptionNone[int]()
                    // Use constructor function which returns the interface type
                    self.emit(format!("{}{}", sym.name, field));
                    self.emit("[");
                    for (i, ty_arg) in type_args.iter().enumerate() {
                        if i > 0 {
                            self.emit(", ");
                        }
                        self.emit(type_to_go_string(ty_arg));
                    }
                    self.emit("]()");
                    return;
                }

                // Check for cross-package enum via PackageMember: pkg.Type.Variant
                if let TypedExprKind::PackageMember { pkg, member } = &base_expr.kind
                    && self.global_state.is_soppo_enum(pkg, member)
                {
                    // Cross-package enum: types.Status.Active → types.StatusActive
                    self.emit(format!("{}.{}{}", pkg, member, field));
                    return;
                }

                // Regular field access
                self.gen_expr(base_expr);
                self.emit(".");
                self.emit(field);
            }

            TypedExprKind::Index { expr, index } => {
                self.gen_expr(expr);
                self.emit("[");
                self.gen_expr(index);
                self.emit("]");
            }

            TypedExprKind::Slice {
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

            TypedExprKind::TypeAssert {
                expr, target_ty, ..
            } => {
                let type_name = type_to_go_string(target_ty).replace('.', "_");
                self.gen_expr(expr);
                self.emit(".(");
                self.emit(&type_name);
                self.emit(")");
            }

            TypedExprKind::NilAssert { expr } => {
                // Nil assertion is compile-time only
                self.gen_expr(expr);
            }

            TypedExprKind::ArrayLit { elem_ty, elements } => {
                // Check if this is a slice or array based on the expression's type
                // Also get field names for anonymous struct elements
                let anon_field_names = parse_anon_struct_field_names(elem_ty);

                // Check if element type is an anonymous struct
                let is_anon_struct = matches!(elem_ty, Type::Con { sym, .. } if sym.name.starts_with("struct {") || sym.name.starts_with("struct{"));

                let full_type = type_to_go_string(&expr.ty);
                if full_type.starts_with("[]") {
                    // Slice literal: []type{elements}
                    self.emit("[]");
                    if is_anon_struct {
                        self.emit_anon_struct_type(elem_ty);
                    } else {
                        self.emit(type_to_go_string(elem_ty));
                    }
                } else {
                    // Array literal: [size]type{elements}
                    self.emit("[");
                    self.emit(elements.len().to_string());
                    self.emit("]");
                    if is_anon_struct {
                        self.emit_anon_struct_type(elem_ty);
                    } else {
                        self.emit(type_to_go_string(elem_ty));
                    }
                }

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
                    self.gen_expr_with_struct_context(elem, anon_field_names.as_deref());
                }
                if multiline {
                    self.emit(",\n");
                    self.dedent();
                    self.emit_indent();
                }
                self.emit("}");
            }

            TypedExprKind::StructLit {
                struct_ty,
                fields,
                implicit,
                multiline,
            } => {
                // Generate Type{field: value, ...}
                // For implicit struct literals (inside slice/array), skip the type name
                if !implicit {
                    // For enum variants, convert Type.Variant to Type_Variant
                    // Handle type args separately to avoid issues with convert_struct_type_name
                    if let Type::Con { sym, args, .. } = struct_ty {
                        let base_name = if sym.module.0.is_empty() {
                            sym.name.clone()
                        } else {
                            format!("{}.{}", sym.module.0, sym.name)
                        };
                        // Convert enum variant notation (Type.Variant -> Type_Variant)
                        let converted = self.convert_struct_type_name(&base_name);
                        self.emit(&converted);
                        // Emit type args if present in struct_ty
                        if !args.is_empty() {
                            self.emit("[");
                            for (i, ty) in args.iter().enumerate() {
                                if i > 0 {
                                    self.emit(", ");
                                }
                                self.emit(type_to_go_string(ty));
                            }
                            self.emit("]");
                        } else if let Type::Con {
                            args: inferred_args,
                            ..
                        } = &expr.ty
                        {
                            // Check for inferred type args from expression's type
                            if !inferred_args.is_empty() {
                                self.emit("[");
                                for (i, ty) in inferred_args.iter().enumerate() {
                                    if i > 0 {
                                        self.emit(", ");
                                    }
                                    self.emit(type_to_go_string(ty));
                                }
                                self.emit("]");
                            }
                        }
                    } else {
                        // Fallback for non-Con types
                        let type_str = type_to_go_string(struct_ty);
                        let converted = self.convert_struct_type_name(&type_str);
                        self.emit(&converted);
                    }
                }
                // For implicit struct literals, parse field names from struct_ty
                // to expand positional fields to named (Go requires all named or all positional)
                let anon_field_names = if *implicit {
                    parse_anon_struct_field_names(struct_ty)
                } else {
                    None
                };

                self.emit("{");
                if *multiline {
                    self.emit("\n");
                    self.indent();
                }
                for (i, (field_name, value)) in fields.iter().enumerate() {
                    if *multiline {
                        self.emit_indent();
                    } else if i > 0 {
                        self.emit(", ");
                    }
                    // Use explicit name, or fall back to parsed name for positional fields
                    let name = match field_name {
                        Some(n) => Some(n.as_str()),
                        None => anon_field_names
                            .as_ref()
                            .and_then(|names| names.get(i).map(|s| s.as_str())),
                    };
                    if let Some(name) = name {
                        self.emit(name);
                        self.emit(": ");
                    }
                    self.gen_expr(value);
                    if *multiline {
                        self.emit(",\n");
                    }
                }
                if *multiline {
                    self.dedent();
                    self.emit_indent();
                }
                self.emit("}");
            }

            TypedExprKind::AnonStructLit { struct_ty, fields } => {
                // Extract field types from struct_ty and emit the struct type definition
                self.emit(type_to_go_string(struct_ty));
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

            TypedExprKind::MapLit { map_ty, entries } => {
                self.emit(type_to_go_string(map_ty));
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

            TypedExprKind::Unary { op, operand } => match op {
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

            TypedExprKind::FuncLit {
                params,
                returns,
                body,
            } => {
                self.emit("func(");
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        self.emit(", ");
                    }
                    self.emit(&param.ident.name);
                    self.emit(" ");
                    self.emit(type_to_go_string(&param.ty));
                }
                self.emit(")");

                if !returns.is_empty() {
                    let is_named = !returns[0].ident.name.is_empty();
                    if returns.len() == 1 && !is_named {
                        self.emit(" ");
                        self.emit(type_to_go_string(&returns[0].ty));
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
                            self.emit(type_to_go_string(&ret.ty));
                        }
                        self.emit(")");
                    }
                }

                self.emit(" ");
                self.gen_block(body);
            }

            TypedExprKind::Block(block) => {
                self.gen_block(block);
            }

            TypedExprKind::Paren(inner) => {
                self.emit("(");
                self.gen_expr(inner);
                self.emit(")");
            }

            TypedExprKind::Error => {
                // Error placeholder - emit something safe
                self.emit("nil /* error */");
            }
        }
    }

    /// Emit an anonymous struct type with proper multiline formatting
    pub(crate) fn emit_anon_struct_type(&mut self, ty: &Type) {
        let fields = match super::parse_anon_struct_fields(ty) {
            Some(f) => f,
            None => {
                // Fallback: emit as-is
                self.emit(type_to_go_string(ty));
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

    /// Generate a typed expression with struct field name context for positional expansion
    pub(crate) fn gen_expr_with_struct_context(
        &mut self,
        expr: &TypedExpr,
        field_names: Option<&[String]>,
    ) {
        // Only apply context to implicit StructLit
        if let TypedExprKind::StructLit {
            fields, implicit, ..
        } = &expr.kind
            && *implicit
            && let Some(names) = field_names
        {
            // Emit struct literal with positional fields expanded to named
            self.emit("{");
            for (i, (field_name, value)) in fields.iter().enumerate() {
                if i > 0 {
                    self.emit(", ");
                }
                // Use explicit name or fall back to context name
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
