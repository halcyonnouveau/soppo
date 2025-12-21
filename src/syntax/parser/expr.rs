use super::Parser;
use crate::error::{SoppoError, SoppoResult};
use crate::syntax::ast::{
    AssignOp, BinOp, Expr, ExprKind, Ident, Param, StringPart, TypeAnnotation, UnaryOp,
};
use crate::syntax::lexer::Token;
use crate::syntax::source::Span;

enum Assoc {
    Left,
}

impl BinOp {
    /// Returns (precedence, associativity)
    /// Higher precedence = tighter binding
    /// Go operator precedence (high to low):
    /// 5: *  /  %  <<  >>  &  &^
    /// 4: +  -  |  ^
    /// 3: ==  !=  <  <=  >  >=
    /// 2: &&
    /// 1: ||
    fn precedence(&self) -> (u8, Assoc) {
        match self {
            BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Shl | BinOp::Shr | BinOp::BitAnd => {
                (6, Assoc::Left)
            }
            BinOp::Add | BinOp::Sub | BinOp::BitOr | BinOp::BitXor => (5, Assoc::Left),
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                (4, Assoc::Left)
            }
            BinOp::And => (3, Assoc::Left),
            BinOp::Or => (2, Assoc::Left),
        }
    }
}

impl Parser {
    /// Parse an expression
    pub fn parse_expr(&mut self) -> SoppoResult<Expr> {
        self.parse_binary(0)
    }

    /// Parse a unary operator expression
    fn parse_unary(
        &mut self,
        op: UnaryOp,
        start_span: Span,
        use_postfix: bool,
    ) -> SoppoResult<Expr> {
        let operand = if use_postfix {
            self.parse_postfix()?
        } else {
            self.parse_primary()?
        };

        let end_span = operand.span;

        Ok(Expr::new(
            ExprKind::Unary {
                op,
                operand: Box::new(operand),
            },
            self.merge_spans(start_span, end_span),
        ))
    }

    /// Parse binary operations with precedence
    fn parse_binary(&mut self, min_prec: u8) -> SoppoResult<Expr> {
        let mut left = self.parse_postfix()?;

        while let Some(op) = self.peek_binop() {
            let (prec, _) = op.precedence();

            if prec < min_prec {
                break;
            }

            self.advance(); // consume operator

            let right = self.parse_binary(prec + 1)?;

            let merged_span = self.merge_spans(left.span, right.span);
            left = Expr::new(
                ExprKind::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                merged_span,
            );
        }

        Ok(left)
    }

    /// Peek at current token and convert to binary operator if applicable
    fn peek_binop(&self) -> Option<BinOp> {
        match self.peek()? {
            Token::Plus => Some(BinOp::Add),
            Token::Minus => Some(BinOp::Sub),
            Token::Star => Some(BinOp::Mul),
            Token::Slash => Some(BinOp::Div),
            Token::Percent => Some(BinOp::Mod),
            Token::Eq => Some(BinOp::Eq),
            Token::Ne => Some(BinOp::Ne),
            Token::Lt => Some(BinOp::Lt),
            Token::Le => Some(BinOp::Le),
            Token::Gt => Some(BinOp::Gt),
            Token::Ge => Some(BinOp::Ge),
            Token::And => Some(BinOp::And),
            Token::Or => Some(BinOp::Or),
            // Bitwise operators
            Token::Ampersand => Some(BinOp::BitAnd),
            Token::Pipe => Some(BinOp::BitOr),
            Token::Caret => Some(BinOp::BitXor),
            Token::Shl => Some(BinOp::Shl),
            Token::Shr => Some(BinOp::Shr),
            _ => None,
        }
    }

    /// Peek at current token and convert to compound assignment operator if applicable
    pub fn peek_assign_op(&self) -> Option<AssignOp> {
        match self.peek()? {
            Token::PlusAssign => Some(AssignOp::Add),
            Token::MinusAssign => Some(AssignOp::Sub),
            Token::StarAssign => Some(AssignOp::Mul),
            Token::SlashAssign => Some(AssignOp::Div),
            Token::PercentAssign => Some(AssignOp::Mod),
            Token::AmpersandAssign => Some(AssignOp::BitAnd),
            Token::PipeAssign => Some(AssignOp::BitOr),
            Token::CaretAssign => Some(AssignOp::BitXor),
            Token::ShlAssign => Some(AssignOp::Shl),
            Token::ShrAssign => Some(AssignOp::Shr),
            _ => None,
        }
    }

    /// Parse postfix operations (call, field access)
    fn parse_postfix(&mut self) -> SoppoResult<Expr> {
        let mut expr = self.parse_primary()?;

        loop {
            match self.peek() {
                // Either array indexing expr[index] or type args for call expr[T](args)
                // Disambiguate by checking if ( follows ]
                Some(Token::LBracket) => {
                    // Save position BEFORE consuming [ so we can backtrack fully
                    let saved_pos = self.pos;
                    self.advance();

                    // Try to parse as type args first by checking if this looks like types
                    // and is followed by (
                    // For now, use a simpler heuristic: if content is a simple identifier
                    // followed by ] and (, treat as type args. Otherwise array index.

                    // Try parsing as type args (comma-separated types)
                    let mut type_args = Vec::new();
                    let mut is_type_args = true;

                    if !matches!(self.peek(), Some(Token::RBracket)) {
                        loop {
                            // Save position before trying to parse type
                            let type_start_pos = self.pos;
                            // Try to parse a type - if this fails, it's not type args
                            match self.parse_type() {
                                Ok(ty) => type_args.push(ty),
                                Err(_) => {
                                    // Restore to before this type parse attempt
                                    self.pos = type_start_pos;
                                    is_type_args = false;
                                    break;
                                }
                            }

                            if !self.consume(&Token::Comma) {
                                break;
                            }
                        }
                    }

                    // Check if ] followed by ( or if this is a type instantiation for unit variant
                    if is_type_args && matches!(self.peek(), Some(Token::RBracket)) {
                        let bracket_end_span = self.expect(Token::RBracket)?;

                        if matches!(self.peek(), Some(Token::LParen)) {
                            // This is type args + call: expr[T](args)
                            self.advance(); // consume (
                            let args = self.parse_argument_list()?;
                            let end_span = self.expect(Token::RParen)?;

                            let expr_span = expr.span;
                            expr = Expr::new(
                                ExprKind::Call {
                                    func: Box::new(expr),
                                    type_args,
                                    args,
                                },
                                self.merge_spans(expr_span, end_span),
                            );
                            continue;
                        }

                        // No call - check if this is a type instantiation for unit enum variant
                        // Pattern: Type.Variant[TypeArgs] without ()
                        // This allows Option.None[int] syntax
                        // Don't treat as unit variant if:
                        // - Followed by `.` (e.g., `arr.Items[i].Name` - index then field access)
                        // - Type args look like index expressions (single lowercase non-type identifier)
                        let looks_like_index = type_args.len() == 1
                            && type_args[0].args.is_empty()
                            && !type_args[0].nullable
                            && type_args[0]
                                .name
                                .chars()
                                .next()
                                .is_some_and(|c| c.is_lowercase())
                            && !matches!(
                                type_args[0].name.as_str(),
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
                                    | "uintptr"
                                    | "float32"
                                    | "float64"
                                    | "complex64"
                                    | "complex128"
                                    | "bool"
                                    | "string"
                                    | "byte"
                                    | "rune"
                                    | "error"
                                    | "any"
                            );

                        // If bracket contents look like types (not an index expression),
                        // treat as type instantiation. Type system will verify later.
                        let is_type_instantiation = !type_args.is_empty() && !looks_like_index;
                        if is_type_instantiation {
                            // This is a type instantiation: Type[Args]
                            let expr_span = expr.span;
                            expr = Expr::new(
                                ExprKind::TypeInst {
                                    ty: Box::new(expr),
                                    type_args,
                                },
                                self.merge_spans(expr_span, bracket_end_span),
                            );
                            continue;
                        }

                        // Not a type instantiation - fall through to index parsing
                    }

                    // Not type args or not a type instantiation - backtrack and parse as array index or slice
                    self.pos = saved_pos;
                    self.advance(); // consume the [ we backtracked past

                    // Check for slice expression: arr[low:high] or arr[low:high:cap]
                    // Cases: arr[:], arr[low:], arr[:high], arr[low:high], arr[low:high:cap]
                    let low = if matches!(self.peek(), Some(Token::Colon)) {
                        None
                    } else {
                        Some(Box::new(self.parse_expr()?))
                    };

                    if self.consume(&Token::Colon) {
                        // This is a slice expression
                        let high =
                            if matches!(self.peek(), Some(Token::RBracket) | Some(Token::Colon)) {
                                None
                            } else {
                                Some(Box::new(self.parse_expr()?))
                            };

                        let cap = if self.consume(&Token::Colon) {
                            // 3-index slice: arr[low:high:cap]
                            Some(Box::new(self.parse_expr()?))
                        } else {
                            None
                        };

                        let end_span = self.expect(Token::RBracket)?;

                        let expr_span = expr.span;
                        expr = Expr::new(
                            ExprKind::Slice {
                                expr: Box::new(expr),
                                low,
                                high,
                                cap,
                            },
                            self.merge_spans(expr_span, end_span),
                        );
                    } else {
                        // Regular index expression
                        let end_span = self.expect(Token::RBracket)?;

                        let expr_span = expr.span;
                        expr = Expr::new(
                            ExprKind::Index {
                                expr: Box::new(expr),
                                index: low.expect("index expression must have a value"),
                            },
                            self.merge_spans(expr_span, end_span),
                        );
                    }
                }

                // Function call without type args: expr(args)
                Some(Token::LParen) => {
                    self.advance();
                    let args = self.parse_argument_list()?;
                    let end_span = self.expect(Token::RParen)?;

                    let expr_span = expr.span;
                    expr = Expr::new(
                        ExprKind::Call {
                            func: Box::new(expr),
                            type_args: vec![],
                            args,
                        },
                        self.merge_spans(expr_span, end_span),
                    );
                }

                // Field access: expr.field or type assertion: expr.(Type) or nil assertion: expr.(!nil)
                Some(Token::Dot) => {
                    self.advance();

                    // Check for type assertion: expr.(Type) or nil assertion: expr.(!nil)
                    if self.consume(&Token::LParen) {
                        // Check for nil assertion: .(!nil)
                        if self.consume(&Token::Not) {
                            // Expect "nil" keyword
                            match self.advance() {
                                Some((Token::Nil, _)) => {}
                                Some((tok, span)) => {
                                    return Err(SoppoError::Parse {
                                        message: format!(
                                            "Expected 'nil' in nil assertion, found {}",
                                            tok
                                        ),
                                        span,
                                    });
                                }
                                None => {
                                    return Err(SoppoError::Parse {
                                        message: "Expected 'nil' in nil assertion".to_string(),
                                        span: Span::dummy(),
                                    });
                                }
                            }
                            let end_span = self.expect(Token::RParen)?;

                            let expr_span = expr.span;
                            expr = Expr::new(
                                ExprKind::NilAssert {
                                    expr: Box::new(expr),
                                },
                                self.merge_spans(expr_span, end_span),
                            );
                            continue;
                        }

                        // Regular type assertion: expr.(Type)
                        let ty = self.parse_type()?;
                        let end_span = self.expect(Token::RParen)?;

                        let expr_span = expr.span;
                        expr = Expr::new(
                            ExprKind::TypeAssert {
                                expr: Box::new(expr),
                                ty,
                            },
                            self.merge_spans(expr_span, end_span),
                        );
                        continue;
                    }

                    let (field, field_span) = self.parse_identifier("field")?;

                    let expr_span = expr.span;
                    expr = Expr::new(
                        ExprKind::Field {
                            expr: Box::new(expr),
                            field,
                            span: field_span,
                        },
                        self.merge_spans(expr_span, field_span),
                    );
                }

                // Struct literal: Type{field: value, ...} or Type.Variant{field: value, ...}
                Some(Token::LBrace) => {
                    // Extract type name and type args from identifier, field access, or type instantiation
                    fn extract_type_info(e: &Expr) -> Option<(String, Vec<TypeAnnotation>)> {
                        match &e.kind {
                            ExprKind::Ident(name) => Some((name.clone(), Vec::new())),
                            ExprKind::Field { expr, field, .. } => extract_type_info(expr)
                                .map(|(base, args)| (format!("{}.{}", base, field), args)),
                            // Type instantiation: Option[int] -> ("Option", [int])
                            ExprKind::TypeInst { ty, type_args } => {
                                extract_type_info(ty).map(|(name, _)| (name, type_args.clone()))
                            }
                            _ => None,
                        }
                    }
                    let type_info = extract_type_info(&expr);

                    if let Some((type_name, type_args)) = type_info {
                        // Struct literal heuristic:
                        // - {} -> struct literal (empty)
                        // - { ident: ... } -> struct literal (named field)
                        // - { expr, ... } -> struct literal (positional with comma)
                        // - { expr } -> ambiguous, NOT a struct literal (could be block)
                        //
                        // This avoids misparsing `if min { stmt }` as struct literal.
                        // Go handles this at grammar level (conditions can't end with composite lit).
                        // Our heuristic is a pragmatic workaround.
                        let pos_after_brace = if matches!(self.peek_at(1), Some(Token::Newline)) {
                            2
                        } else {
                            1
                        };

                        let is_struct_lit = match (
                            self.peek_at(pos_after_brace),
                            self.peek_at(pos_after_brace + 1),
                        ) {
                            (Some(Token::RBrace), _) => true,                    // {}
                            (Some(Token::Ident(_)), Some(Token::Colon)) => true, // { foo: ...
                            _ => {
                                // Check for positional: expr followed by comma, ending with }
                                // But NOT if we see := or = which indicates a multi-declaration
                                // e.g., `tests { a, b := foo() }` is a block, not struct literal
                                let saved_pos = self.pos;
                                self.advance(); // consume {
                                self.skip_terminators();

                                if matches!(self.peek(), Some(Token::RBrace)) {
                                    // Empty - already handled above, but just in case
                                    self.pos = saved_pos;
                                    true
                                } else {
                                    // Try to parse an expression
                                    let expr_result = self.parse_expr();
                                    let has_comma = matches!(self.peek(), Some(Token::Comma));

                                    // If we have expr followed by comma, scan ahead to check
                                    // if there's := or = before } (which means it's a multi-decl)
                                    // Note: we DON'T check for newlines here because multi-line
                                    // struct literals like Point{\n1,\n2,\n} are valid
                                    let is_struct_lit = if expr_result.is_ok() && has_comma {
                                        let mut is_multi_decl = false;
                                        let scan_pos = self.pos;
                                        while let Some(tok) = self.peek() {
                                            match tok {
                                                Token::RBrace => break,
                                                Token::ColonAssign | Token::Assign => {
                                                    is_multi_decl = true;
                                                    break;
                                                }
                                                _ => {
                                                    self.advance();
                                                }
                                            }
                                        }
                                        self.pos = scan_pos;
                                        !is_multi_decl
                                    } else {
                                        false
                                    };

                                    self.pos = saved_pos; // restore position
                                    is_struct_lit
                                }
                            }
                        };

                        if !is_struct_lit {
                            break;
                        }

                        self.advance(); // consume {

                        // Track if this is a multiline struct literal
                        let mut multiline = matches!(self.peek(), Some(Token::Newline));
                        self.skip_terminators();

                        let mut fields = Vec::new();

                        if !matches!(self.peek(), Some(Token::RBrace)) {
                            loop {
                                // Check if this is a named field (ident:) or positional
                                let is_named = matches!(
                                    (self.peek(), self.peek_at(1)),
                                    (Some(Token::Ident(_)), Some(Token::Colon))
                                );

                                if is_named {
                                    // Parse named field: ident: expr
                                    let field_name = match self.advance() {
                                        Some((Token::Ident(name), _)) => name,
                                        _ => unreachable!(),
                                    };
                                    self.expect(Token::Colon)?;
                                    let value = self.parse_expr()?;
                                    let value_end = value.span.at_end();

                                    fields.push((Some(field_name), value));

                                    if !self.consume(&Token::Comma) {
                                        if matches!(self.peek(), Some(Token::Newline)) {
                                            return Err(SoppoError::Parse {
                                                message:
                                                    "Missing trailing comma after struct field"
                                                        .to_string(),
                                                span: value_end,
                                            });
                                        }
                                        break;
                                    }
                                } else {
                                    // Parse positional field: expr
                                    let value = self.parse_expr()?;
                                    let value_end = value.span.at_end();

                                    fields.push((None, value));

                                    if !self.consume(&Token::Comma) {
                                        if matches!(self.peek(), Some(Token::Newline)) {
                                            return Err(SoppoError::Parse {
                                                message:
                                                    "Missing trailing comma after struct field"
                                                        .to_string(),
                                                span: value_end,
                                            });
                                        }
                                        break;
                                    }
                                }

                                // Check for newlines between fields
                                if matches!(self.peek(), Some(Token::Newline)) {
                                    multiline = true;
                                }
                                self.skip_terminators();

                                // Allow trailing comma
                                if matches!(self.peek(), Some(Token::RBrace)) {
                                    break;
                                }
                            }
                        }

                        let end_span = self.expect(Token::RBrace)?;

                        expr = Expr::new(
                            ExprKind::StructLit {
                                ty: Some(TypeAnnotation {
                                    name: type_name,
                                    args: type_args,
                                    span: expr.span,
                                    nullable: false,
                                }),
                                fields,
                                multiline,
                            },
                            self.merge_spans(expr.span, end_span),
                        );
                    } else {
                        break;
                    }
                }

                _ => break,
            }
        }

        Ok(expr)
    }

    /// Parse a primary expression (literals, identifiers, parenthesized expressions)
    fn parse_primary(&mut self) -> SoppoResult<Expr> {
        let (tok, span) = self.advance().ok_or_else(|| SoppoError::Parse {
            message: "Unexpected end of input".to_string(),
            span: Span::dummy(),
        })?;

        match tok {
            // Unary operators (use parse_postfix so &Type{}, *user.Email, etc. work)
            Token::Ampersand => self.parse_unary(UnaryOp::Ref, span, true),
            Token::Star => self.parse_unary(UnaryOp::Deref, span, true),
            Token::Minus => self.parse_unary(UnaryOp::Neg, span, true),
            Token::Not => self.parse_unary(UnaryOp::Not, span, true),
            Token::Arrow => self.parse_unary(UnaryOp::Recv, span, false),

            Token::Integer(lit) => Ok(Expr::new(ExprKind::Integer(lit.value, lit.format), span)),

            Token::Float(f) => Ok(Expr::new(ExprKind::Float(f), span)),

            Token::Rune(r) => Ok(Expr::new(ExprKind::Rune(r), span)),

            Token::String(s) => {
                // Check if string contains interpolation markers
                if s.contains('{') {
                    let parts = self.parse_string_interpolation(&s, span)?;
                    if parts.len() == 1 {
                        // Single literal part - just a plain string
                        if let StringPart::Literal(lit) = &parts[0] {
                            return Ok(Expr::new(ExprKind::String(lit.clone()), span));
                        }
                    }
                    Ok(Expr::new(ExprKind::StringInterpolation(parts), span))
                } else {
                    Ok(Expr::new(ExprKind::String(s), span))
                }
            }

            // Raw string literals (backtick strings) - no interpolation support
            Token::RawString(s) => Ok(Expr::new(ExprKind::RawString(s), span)),

            Token::True => Ok(Expr::new(ExprKind::Bool(true), span)),

            Token::False => Ok(Expr::new(ExprKind::Bool(false), span)),

            Token::Nil => Ok(Expr::new(ExprKind::Nil, span)),

            Token::Ident(name) if name == "map" => {
                // Map literal: map[K]V{key: val, ...}
                self.expect(Token::LBracket)?;
                let key_ty = self.parse_type()?;
                self.expect(Token::RBracket)?;
                let val_ty = self.parse_type()?;

                let map_ty = TypeAnnotation {
                    name: format!("map[{}]{}", key_ty.name, val_ty.name),
                    args: vec![key_ty, val_ty],
                    span,
                    nullable: false,
                };

                self.expect(Token::LBrace)?;

                let mut entries = Vec::new();
                if !matches!(self.peek(), Some(Token::RBrace)) {
                    loop {
                        let key = self.parse_expr()?;
                        self.expect(Token::Colon)?;
                        let value = self.parse_expr()?;
                        entries.push((key, value));

                        if !self.consume(&Token::Comma) {
                            break;
                        }
                        // Allow trailing comma
                        if matches!(self.peek(), Some(Token::RBrace)) {
                            break;
                        }
                    }
                }

                let end_span = self.expect(Token::RBrace)?;

                Ok(Expr::new(
                    ExprKind::MapLit {
                        ty: map_ty,
                        entries,
                    },
                    self.merge_spans(span, end_span),
                ))
            }

            Token::Ident(name) if name == "make" => {
                // make(type, args...) - built-in for creating slices, maps, channels
                self.expect(Token::LParen)?;

                // First argument is a type
                let ty = self.parse_type()?;

                // Optional additional arguments (size, capacity) - always positional, no spread
                let mut args = Vec::new();
                while self.consume(&Token::Comma) {
                    args.push((None, self.parse_expr()?, false));
                }

                let end_span = self.expect(Token::RParen)?;

                // Generate as a call to make with type as first "argument" (special handling in codegen)
                // We'll encode the type in the call expression using a special type argument
                Ok(Expr::new(
                    ExprKind::Call {
                        func: Box::new(Expr::new(ExprKind::Ident("make".to_string()), span)),
                        type_args: vec![ty],
                        args,
                    },
                    self.merge_spans(span, end_span),
                ))
            }

            Token::Ident(name) if name == "new" => {
                // new(type) - built-in for creating pointer to zero value
                self.expect(Token::LParen)?;
                let ty = self.parse_type()?;
                let end_span = self.expect(Token::RParen)?;

                Ok(Expr::new(
                    ExprKind::Call {
                        func: Box::new(Expr::new(ExprKind::Ident("new".to_string()), span)),
                        type_args: vec![ty],
                        args: vec![], // new has no runtime args
                    },
                    self.merge_spans(span, end_span),
                ))
            }

            Token::Ident(name) => Ok(Expr::new(ExprKind::Ident(name), span)),

            Token::Underscore => Ok(Expr::new(ExprKind::Ident("_".to_string()), span)),

            Token::LParen => {
                let inner = self.parse_expr()?;
                let end_span = self.expect(Token::RParen)?;
                Ok(Expr::new(
                    ExprKind::Paren(Box::new(inner)),
                    self.merge_spans(span, end_span),
                ))
            }

            Token::LBracket => {
                // Slice literal: []type{elements}
                // Slice type conversion: []type(expr)
                // Array literal: [size]type{elements}
                if self.consume(&Token::RBracket) {
                    // []type{elements} - slice literal
                    // or []type(expr) - type conversion
                    let elem_ty = self.parse_type()?;
                    // Create a slice type with [] prefix
                    let slice_ty = TypeAnnotation {
                        name: format!("[]{}", elem_ty.name),
                        args: elem_ty.args.clone(),
                        span: elem_ty.span,
                        nullable: false,
                    };

                    self.skip_terminators();

                    // Check for type conversion: []type(expr)
                    if self.consume(&Token::LParen) {
                        let args = self.parse_argument_list()?;
                        let end_span = self.expect(Token::RParen)?;

                        // Create a "function" expression from the slice type name
                        let type_expr = Expr::new(ExprKind::Ident(slice_ty.name), slice_ty.span);

                        return Ok(Expr::new(
                            ExprKind::Call {
                                func: Box::new(type_expr),
                                type_args: vec![],
                                args,
                            },
                            self.merge_spans(span, end_span),
                        ));
                    }

                    self.expect(Token::LBrace)?;
                    self.skip_terminators();

                    let mut elements = Vec::new();
                    if !matches!(self.peek(), Some(Token::RBrace)) {
                        loop {
                            elements.push(self.parse_expr()?);
                            // Allow trailing comma
                            if !self.consume(&Token::Comma) {
                                break;
                            }
                            self.skip_terminators();
                            // Check for closing brace after trailing comma
                            if matches!(self.peek(), Some(Token::RBrace)) {
                                break;
                            }
                        }
                    }

                    let end_span = self.expect(Token::RBrace)?;

                    Ok(Expr::new(
                        ExprKind::ArrayLit {
                            ty: Some(slice_ty),
                            elements,
                        },
                        self.merge_spans(span, end_span),
                    ))
                } else {
                    // [size]type{elements} - array literal
                    // Consume the size (we don't validate it)
                    while !matches!(self.peek(), Some(Token::RBracket) | None) {
                        self.advance();
                    }
                    self.expect(Token::RBracket)?;

                    let ty = self.parse_type()?;

                    self.skip_terminators();
                    self.expect(Token::LBrace)?;
                    self.skip_terminators();

                    let mut elements = Vec::new();
                    if !matches!(self.peek(), Some(Token::RBrace)) {
                        loop {
                            elements.push(self.parse_expr()?);
                            if !self.consume(&Token::Comma) {
                                break;
                            }
                            self.skip_terminators();
                            if matches!(self.peek(), Some(Token::RBrace)) {
                                break;
                            }
                        }
                    }

                    let end_span = self.expect(Token::RBrace)?;

                    Ok(Expr::new(
                        ExprKind::ArrayLit {
                            ty: Some(ty),
                            elements,
                        },
                        self.merge_spans(span, end_span),
                    ))
                }
            }

            Token::Struct => {
                // Anonymous struct literal: struct { X int; Y int }{X: 1, Y: 2}
                // Also supports grouped names: struct { X, Y int }{X: 1, Y: 2}
                self.expect(Token::LBrace)?;
                self.skip_terminators();

                // Parse field definitions using parse_fields (supports grouped names)
                let mut field_defs = Vec::new();
                while !matches!(self.peek(), Some(Token::RBrace) | None) {
                    let parsed_fields = self.parse_fields()?;
                    field_defs.extend(parsed_fields);

                    // Allow semicolon or newline as separator
                    if !self.consume(&Token::Semicolon) {
                        self.skip_terminators();
                    }
                }

                self.expect(Token::RBrace)?;

                // Now parse the field values: {Name: value, ...}
                self.skip_terminators();
                self.expect(Token::LBrace)?;
                self.skip_terminators();

                let mut fields = Vec::new();
                if !matches!(self.peek(), Some(Token::RBrace)) {
                    loop {
                        // Parse field name
                        let field_name = match self.advance() {
                            Some((Token::Ident(name), _)) => name,
                            Some((tok, field_span)) => {
                                return Err(SoppoError::Parse {
                                    message: format!("Expected field name, found {}", tok),
                                    span: field_span,
                                });
                            }
                            None => {
                                return Err(SoppoError::Parse {
                                    message: "Expected field name".to_string(),
                                    span: Span::dummy(),
                                });
                            }
                        };

                        self.expect(Token::Colon)?;
                        let value = self.parse_expr()?;

                        fields.push((Some(field_name), value));

                        if !self.consume(&Token::Comma) {
                            break;
                        }

                        self.skip_terminators();

                        // Allow trailing comma
                        if matches!(self.peek(), Some(Token::RBrace)) {
                            break;
                        }
                    }
                }

                let end_span = self.expect(Token::RBrace)?;

                Ok(Expr::new(
                    ExprKind::AnonStructLit { field_defs, fields },
                    self.merge_spans(span, end_span),
                ))
            }

            Token::Func => {
                // Anonymous function: func(params) returnTypes { body }
                self.expect(Token::LParen)?;

                // Parse parameters
                let mut params = Vec::new();
                if !matches!(self.peek(), Some(Token::RParen)) {
                    loop {
                        let (param_name, param_span) = self.parse_identifier("parameter")?;

                        let param_ty = self.parse_type()?;
                        params.push(Param {
                            ident: Ident::new(param_name, param_span),
                            ty: param_ty,
                        });

                        if !self.consume(&Token::Comma) {
                            break;
                        }
                    }
                }
                self.expect(Token::RParen)?;

                // Parse return types (supports named returns)
                let returns = self.parse_return_list()?;

                // Parse body
                let body = self.parse_block()?;

                Ok(Expr::new(
                    ExprKind::FuncLit {
                        params,
                        returns,
                        body: body.clone(),
                    },
                    self.merge_spans(span, body.span),
                ))
            }

            // Implicit composite literal: {expr, expr, ...} or {Field: value, ...}
            // Used inside array/slice literals like [][]int{{1, 2}, {3, 4}}
            // or []Item{{Name: "x"}, {Name: "y"}}
            // We always parse as StructLit - positional fields have None names.
            // Type checker determines if it's a struct or array based on context.
            Token::LBrace => {
                let mut fields = Vec::new();
                let mut seen_named = false;
                let mut multiline = false;

                if !matches!(self.peek(), Some(Token::RBrace)) {
                    loop {
                        if matches!(self.peek(), Some(Token::Newline)) {
                            multiline = true;
                        }
                        self.skip_terminators();

                        if matches!(self.peek(), Some(Token::RBrace)) {
                            break;
                        }

                        // Check if this is a named field (Ident followed by Colon)
                        let is_named = matches!(self.peek(), Some(Token::Ident(_)))
                            && matches!(self.peek_at(1), Some(Token::Colon));

                        if is_named {
                            seen_named = true;
                            let field_name = match self.advance() {
                                Some((Token::Ident(name), _)) => name,
                                _ => unreachable!(),
                            };
                            self.expect(Token::Colon)?;
                            let value = self.parse_expr()?;
                            fields.push((Some(field_name), value));
                        } else {
                            // Positional field - must come before named fields
                            if seen_named {
                                return Err(SoppoError::Parse {
                                    message: "Positional fields must come before named fields"
                                        .to_string(),
                                    span,
                                });
                            }
                            let value = self.parse_expr()?;
                            fields.push((None, value));
                        }

                        if !self.consume(&Token::Comma) {
                            break;
                        }

                        if matches!(self.peek(), Some(Token::Newline)) {
                            multiline = true;
                        }
                        self.skip_terminators();

                        // Allow trailing comma
                        if matches!(self.peek(), Some(Token::RBrace)) {
                            break;
                        }
                    }
                }

                let end_span = self.expect(Token::RBrace)?;

                Ok(Expr::new(
                    ExprKind::StructLit {
                        ty: None, // Type inferred from context
                        fields,
                        multiline,
                    },
                    self.merge_spans(span, end_span),
                ))
            }

            _ => Err(SoppoError::Parse {
                message: format!("Unexpected token: {:?}", tok),
                span,
            }),
        }
    }

    /// Parse a string with interpolation: "Hello, {name}!" or "Price: {cost:.2f}"
    /// Returns a vector of StringPart
    fn parse_string_interpolation(&mut self, s: &str, span: Span) -> SoppoResult<Vec<StringPart>> {
        let mut parts = Vec::new();
        let mut current_literal = String::new();
        let mut char_indices = s.char_indices().peekable();

        // The string content starts after the opening quote
        let string_content_byte_start = span.byte_start + 1;

        while let Some((byte_pos, ch)) = char_indices.next() {
            if ch == '{' {
                // Check for escaped brace: {{
                if char_indices.peek().map(|(_, c)| *c) == Some('{') {
                    char_indices.next();
                    current_literal.push('{');
                    continue;
                }

                // Save any accumulated literal
                if !current_literal.is_empty() {
                    parts.push(StringPart::Literal(current_literal.clone()));
                    current_literal.clear();
                }

                // Track where the expression starts (after the '{')
                let expr_start_byte = string_content_byte_start + byte_pos + 1;

                // Extract the content inside {} (expression + optional format)
                let mut content = String::new();
                let mut brace_depth = 1;

                for (_, inner_ch) in char_indices.by_ref() {
                    if inner_ch == '{' {
                        brace_depth += 1;
                        content.push(inner_ch);
                    } else if inner_ch == '}' {
                        brace_depth -= 1;
                        if brace_depth == 0 {
                            break;
                        }
                        content.push(inner_ch);
                    } else {
                        content.push(inner_ch);
                    }
                }

                if brace_depth != 0 {
                    return Err(SoppoError::Parse {
                        message: "Unclosed interpolation brace in string".to_string(),
                        span,
                    });
                }

                // Split content into expression and optional format specifier
                // The format delimiter is ':' at nesting depth 0
                let (expr_str, format) = split_interpolation_content(&content);

                // Parse the expression with byte offset so errors point to correct location
                let mut expr_parser = Parser::new_with_offset(expr_str, self.file, expr_start_byte);
                let expr = expr_parser.parse_expr().map_err(|e| {
                    // Remap parse errors to parent string span as fallback
                    match e {
                        SoppoError::Parse { message, .. } => SoppoError::Parse { message, span },
                        other => other,
                    }
                })?;
                parts.push(StringPart::Expr {
                    expr: Box::new(expr),
                    format: format.map(|s| s.to_string()),
                });
            } else if ch == '}' {
                // Check for escaped brace: }}
                if char_indices.peek().map(|(_, c)| *c) == Some('}') {
                    char_indices.next();
                    current_literal.push('}');
                    continue;
                }
                // Unescaped } without matching { - just include it
                current_literal.push(ch);
            } else {
                current_literal.push(ch);
            }
        }

        // Save any remaining literal
        if !current_literal.is_empty() {
            parts.push(StringPart::Literal(current_literal));
        }

        Ok(parts)
    }
}

/// Split interpolation content into expression and optional format specifier.
/// The format delimiter is ':' at nesting depth 0 (outside brackets, braces, parens, strings).
/// Returns (expression_str, Option<format_str>).
fn split_interpolation_content(content: &str) -> (&str, Option<&str>) {
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut string_char = '"';
    let mut prev_char = '\0';

    for (i, ch) in content.char_indices() {
        // Handle string literals
        if !in_string && (ch == '"' || ch == '\'' || ch == '`') {
            in_string = true;
            string_char = ch;
        } else if in_string && ch == string_char && prev_char != '\\' {
            in_string = false;
        } else if !in_string {
            // Track nesting depth
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth = depth.saturating_sub(1),
                ':' if depth == 0 => {
                    // Found format delimiter at depth 0
                    return (&content[..i], Some(&content[i + 1..]));
                }
                _ => {}
            }
        }
        prev_char = ch;
    }

    // No format specifier found
    (content, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SoppoResult;
    use crate::syntax::ast::ExprKind;
    use crate::syntax::source::FileId;

    fn parse_expr_helper(source: &str) -> SoppoResult<Expr> {
        Parser::new(source, FileId(0)).parse_expr()
    }

    #[test]
    fn test_parse_integer() {
        let expr = parse_expr_helper("42").unwrap();
        assert!(matches!(expr.kind, ExprKind::Integer(42, _)));
    }

    #[test]
    fn test_parse_string() {
        let expr = parse_expr_helper(r#""hello""#).unwrap();
        assert!(matches!(expr.kind, ExprKind::String(s) if s == "hello"));
    }

    #[test]
    fn test_parse_bool() {
        let expr = parse_expr_helper("true").unwrap();
        assert!(matches!(expr.kind, ExprKind::Bool(true)));

        let expr = parse_expr_helper("false").unwrap();
        assert!(matches!(expr.kind, ExprKind::Bool(false)));
    }

    #[test]
    fn test_parse_ident() {
        let expr = parse_expr_helper("foo").unwrap();
        assert!(matches!(expr.kind, ExprKind::Ident(s) if s == "foo"));
    }

    #[test]
    fn test_parse_binary() {
        let expr = parse_expr_helper("1 + 2").unwrap();
        match expr.kind {
            ExprKind::Binary { op, left, right } => {
                assert_eq!(op, BinOp::Add);
                assert!(matches!(left.kind, ExprKind::Integer(1, _)));
                assert!(matches!(right.kind, ExprKind::Integer(2, _)));
            }
            _ => panic!("Expected binary expression"),
        }
    }

    #[test]
    fn test_parse_binary_precedence() {
        let expr = parse_expr_helper("1 + 2 * 3").unwrap();
        match expr.kind {
            ExprKind::Binary {
                op,
                left,
                right: mul_expr,
            } => {
                assert_eq!(op, BinOp::Add);
                assert!(matches!(left.kind, ExprKind::Integer(1, _)));
                match mul_expr.kind {
                    ExprKind::Binary { op, left, right } => {
                        assert_eq!(op, BinOp::Mul);
                        assert!(matches!(left.kind, ExprKind::Integer(2, _)));
                        assert!(matches!(right.kind, ExprKind::Integer(3, _)));
                    }
                    _ => panic!("Expected multiplication"),
                }
            }
            _ => panic!("Expected binary expression"),
        }
    }

    #[test]
    fn test_parse_function_call() {
        let expr = parse_expr_helper("foo(1, 2)").unwrap();
        match expr.kind {
            ExprKind::Call {
                func,
                args,
                type_args,
            } => {
                assert!(matches!(func.kind, ExprKind::Ident(s) if s == "foo"));
                assert_eq!(args.len(), 2);
                // Args are (Option<String>, Expr) tuples - positional args have None name
                assert!(args[0].0.is_none());
                assert!(matches!(args[0].1.kind, ExprKind::Integer(1, _)));
                assert!(args[1].0.is_none());
                assert!(matches!(args[1].1.kind, ExprKind::Integer(2, _)));
                assert!(type_args.is_empty());
            }
            _ => panic!("Expected call expression"),
        }
    }

    #[test]
    fn test_parse_named_arguments() {
        let expr = parse_expr_helper("foo(a: 1, b: 2)").unwrap();
        match expr.kind {
            ExprKind::Call { func, args, .. } => {
                assert!(matches!(func.kind, ExprKind::Ident(s) if s == "foo"));
                assert_eq!(args.len(), 2);
                // Named args have Some((name, span))
                assert!(matches!(&args[0].0, Some((n, _)) if n == "a"));
                assert!(matches!(args[0].1.kind, ExprKind::Integer(1, _)));
                assert!(matches!(&args[1].0, Some((n, _)) if n == "b"));
                assert!(matches!(args[1].1.kind, ExprKind::Integer(2, _)));
            }
            _ => panic!("Expected call expression"),
        }
    }

    #[test]
    fn test_parse_mixed_arguments() {
        let expr = parse_expr_helper("foo(1, b: 2)").unwrap();
        match expr.kind {
            ExprKind::Call { func, args, .. } => {
                assert!(matches!(func.kind, ExprKind::Ident(s) if s == "foo"));
                assert_eq!(args.len(), 2);
                // First is positional (None), second is named (Some((name, span)))
                assert!(args[0].0.is_none());
                assert!(matches!(args[0].1.kind, ExprKind::Integer(1, _)));
                assert!(matches!(&args[1].0, Some((n, _)) if n == "b"));
                assert!(matches!(args[1].1.kind, ExprKind::Integer(2, _)));
            }
            _ => panic!("Expected call expression"),
        }
    }

    #[test]
    fn test_parse_field_access() {
        let expr = parse_expr_helper("foo.bar").unwrap();
        match expr.kind {
            ExprKind::Field { expr, field, .. } => {
                assert!(matches!(expr.kind, ExprKind::Ident(s) if s == "foo"));
                assert_eq!(field, "bar");
            }
            _ => panic!("Expected field access"),
        }
    }

    #[test]
    fn test_parse_spread_argument() {
        // Spread operator for variadic arguments: slice...
        let expr = parse_expr_helper("append(a, b...)").unwrap();
        match expr.kind {
            ExprKind::Call { func, args, .. } => {
                assert!(matches!(func.kind, ExprKind::Ident(s) if s == "append"));
                assert_eq!(args.len(), 2);
                // First arg: no spread
                assert!(args[0].0.is_none());
                assert!(matches!(&args[0].1.kind, ExprKind::Ident(s) if s == "a"));
                assert!(!args[0].2); // not spread
                // Second arg: spread
                assert!(args[1].0.is_none());
                assert!(matches!(&args[1].1.kind, ExprKind::Ident(s) if s == "b"));
                assert!(args[1].2); // is spread
            }
            _ => panic!("Expected call expression"),
        }
    }
}
