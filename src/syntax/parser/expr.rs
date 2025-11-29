use super::Parser;
use crate::error::{Result, SoppoError};
use crate::syntax::ast::{AssignOp, BinOp, Expr, ExprKind, Param, StringPart, Type, UnaryOp};
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
    pub fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_binary(0)
    }

    /// Parse binary operations with precedence
    fn parse_binary(&mut self, min_prec: u8) -> Result<Expr> {
        let mut left = self.parse_postfix()?;

        while let Some(op) = self.peek_binop() {
            let (prec, _) = op.precedence();

            if prec < min_prec {
                break;
            }

            self.advance(); // consume operator

            let right = self.parse_binary(prec + 1)?;

            left = Expr {
                span: Span::with_bytes(
                    left.span.start,
                    right.span.end,
                    self.file,
                    left.span.byte_start,
                    right.span.byte_end,
                ),
                kind: ExprKind::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
            };
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
    fn parse_postfix(&mut self) -> Result<Expr> {
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

                    // Check if ] followed by (
                    if is_type_args && matches!(self.peek(), Some(Token::RBracket)) {
                        self.advance(); // consume ]
                        if matches!(self.peek(), Some(Token::LParen)) {
                            // This is type args + call: expr[T](args)
                            self.advance(); // consume (
                            let mut args = Vec::new();

                            if !matches!(self.peek(), Some(Token::RParen)) {
                                loop {
                                    // Check for named argument: Ident followed by Colon
                                    let arg = if let Some(Token::Ident(name)) = self.peek()
                                        && matches!(self.peek_at(1), Some(Token::Colon))
                                    {
                                        let name = name.clone();
                                        let (_, name_span) = self.advance().unwrap(); // consume identifier
                                        self.advance(); // consume colon
                                        let value = self.parse_expr()?;
                                        (Some((name, name_span)), value)
                                    } else {
                                        // Positional argument
                                        // Note: We allow positional after named because variadic params
                                        // are always positional at the end. The type checker validates
                                        // that non-variadic positional args don't follow named args.
                                        (None, self.parse_expr()?)
                                    };
                                    args.push(arg);

                                    if !self.consume(&Token::Comma) {
                                        break;
                                    }
                                }
                            }

                            let end_span = self.expect(Token::RParen)?;

                            expr = Expr {
                                span: Span::with_bytes(
                                    expr.span.start,
                                    end_span.end,
                                    self.file,
                                    expr.span.byte_start,
                                    end_span.byte_end,
                                ),
                                kind: ExprKind::Call {
                                    func: Box::new(expr),
                                    type_args,
                                    args,
                                },
                            };
                            continue;
                        }
                    }

                    // Not type args - backtrack and parse as array index or slice
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

                        expr = Expr {
                            span: Span::with_bytes(
                                expr.span.start,
                                end_span.end,
                                self.file,
                                expr.span.byte_start,
                                end_span.byte_end,
                            ),
                            kind: ExprKind::Slice {
                                expr: Box::new(expr),
                                low,
                                high,
                                cap,
                            },
                        };
                    } else {
                        // Regular index expression
                        let end_span = self.expect(Token::RBracket)?;

                        expr = Expr {
                            span: Span::with_bytes(
                                expr.span.start,
                                end_span.end,
                                self.file,
                                expr.span.byte_start,
                                end_span.byte_end,
                            ),
                            kind: ExprKind::Index {
                                expr: Box::new(expr),
                                index: low.expect("index expression must have a value"),
                            },
                        };
                    }
                }

                // Function call without type args: expr(args)
                Some(Token::LParen) => {
                    self.advance();
                    let mut args = Vec::new();

                    if !matches!(self.peek(), Some(Token::RParen)) {
                        loop {
                            // Check for named argument: Ident followed by Colon
                            let arg = if let Some(Token::Ident(name)) = self.peek()
                                && matches!(self.peek_at(1), Some(Token::Colon))
                            {
                                let name = name.clone();
                                let (_, name_span) = self.advance().unwrap(); // consume identifier
                                self.advance(); // consume colon
                                let value = self.parse_expr()?;
                                (Some((name, name_span)), value)
                            } else {
                                // Positional argument
                                // Note: We allow positional after named because variadic params
                                // are always positional at the end. The type checker validates
                                // that non-variadic positional args don't follow named args.
                                (None, self.parse_expr()?)
                            };
                            args.push(arg);

                            if !self.consume(&Token::Comma) {
                                break;
                            }
                        }
                    }

                    let end_span = self.expect(Token::RParen)?;

                    expr = Expr {
                        span: Span::with_bytes(
                            expr.span.start,
                            end_span.end,
                            self.file,
                            expr.span.byte_start,
                            end_span.byte_end,
                        ),
                        kind: ExprKind::Call {
                            func: Box::new(expr),
                            type_args: vec![],
                            args,
                        },
                    };
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
                                            "Expected 'nil' in nil assertion, found {:?}",
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

                            expr = Expr {
                                span: Span::with_bytes(
                                    expr.span.start,
                                    end_span.end,
                                    self.file,
                                    expr.span.byte_start,
                                    end_span.byte_end,
                                ),
                                kind: ExprKind::NilAssert {
                                    expr: Box::new(expr),
                                },
                            };
                            continue;
                        }

                        // Regular type assertion: expr.(Type)
                        let ty = self.parse_type()?;
                        let end_span = self.expect(Token::RParen)?;

                        expr = Expr {
                            span: Span::with_bytes(
                                expr.span.start,
                                end_span.end,
                                self.file,
                                expr.span.byte_start,
                                end_span.byte_end,
                            ),
                            kind: ExprKind::TypeAssert {
                                expr: Box::new(expr),
                                ty,
                            },
                        };
                        continue;
                    }

                    let (field, field_span) = match self.advance() {
                        Some((Token::Ident(name), span)) => (name, span),
                        Some((tok, span)) => {
                            return Err(SoppoError::Parse {
                                message: format!("Expected field name, found {:?}", tok),
                                span,
                            });
                        }
                        None => {
                            return Err(SoppoError::Parse {
                                message: "Expected field name".to_string(),
                                span: Span::dummy(),
                            });
                        }
                    };

                    expr = Expr {
                        span: Span::with_bytes(
                            expr.span.start,
                            field_span.end,
                            self.file,
                            expr.span.byte_start,
                            field_span.byte_end,
                        ),
                        kind: ExprKind::Field {
                            expr: Box::new(expr),
                            field,
                            field_span,
                        },
                    };
                }

                // Struct literal: Type{field: value, ...} or Type.Variant{field: value, ...}
                Some(Token::LBrace) => {
                    // Extract type name from identifier or field access chain
                    let type_name = match &expr.kind {
                        ExprKind::Ident(name) => Some(name.clone()),
                        ExprKind::Field { .. } => {
                            // Convert field access chain to dotted type name (e.g., Shape.Circle)
                            fn extract_type_path(e: &Expr) -> Option<String> {
                                match &e.kind {
                                    ExprKind::Ident(name) => Some(name.clone()),
                                    ExprKind::Field { expr, field, .. } => extract_type_path(expr)
                                        .map(|base| format!("{}.{}", base, field)),
                                    _ => None,
                                }
                            }
                            extract_type_path(&expr)
                        }
                        _ => None,
                    };

                    if let Some(type_name) = type_name {
                        // Peek ahead to see if this looks like a struct literal
                        // Struct literals have pattern: { ident: expr, ... }
                        // Blocks have pattern: { stmt; ... }
                        // Check if next token is } (empty struct) or identifier followed by colon
                        // Need to account for newlines after {
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
                            _ => false,
                        };

                        if !is_struct_lit {
                            break;
                        }

                        self.advance(); // consume {
                        // Skip terminators after opening brace
                        self.skip_terminators();

                        let mut fields = Vec::new();

                        if !matches!(self.peek(), Some(Token::RBrace)) {
                            loop {
                                // Parse field name
                                let field_name = match self.advance() {
                                    Some((Token::Ident(name), _)) => name,
                                    Some((tok, span)) => {
                                        return Err(SoppoError::Parse {
                                            message: format!(
                                                "Expected field name, found {:?}",
                                                tok
                                            ),
                                            span,
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

                                fields.push((field_name, value));

                                if !self.consume(&Token::Comma) {
                                    break;
                                }

                                // Skip terminators after comma
                                self.skip_terminators();

                                // Allow trailing comma
                                if matches!(self.peek(), Some(Token::RBrace)) {
                                    break;
                                }
                            }
                        }

                        let end_span = self.expect(Token::RBrace)?;

                        expr = Expr {
                            span: Span::with_bytes(
                                expr.span.start,
                                end_span.end,
                                self.file,
                                expr.span.byte_start,
                                end_span.byte_end,
                            ),
                            kind: ExprKind::StructLit {
                                ty: Type {
                                    name: type_name,
                                    args: Vec::new(),
                                    span: expr.span,
                                    nullable: false,
                                },
                                fields,
                            },
                        };
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
    fn parse_primary(&mut self) -> Result<Expr> {
        let (tok, span) = self.advance().ok_or_else(|| SoppoError::Parse {
            message: "Unexpected end of input".to_string(),
            span: Span::dummy(),
        })?;

        match tok {
            // Unary operators
            Token::Ampersand => {
                // &x - address of (parse_postfix to allow &Type{...})
                let operand = self.parse_postfix()?;
                let end_span = operand.span;
                Ok(Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Ref,
                        operand: Box::new(operand),
                    },
                    span: Span::with_bytes(
                        span.start,
                        end_span.end,
                        self.file,
                        span.byte_start,
                        end_span.byte_end,
                    ),
                })
            }

            Token::Star => {
                // *p - dereference (when used as unary prefix)
                let operand = self.parse_primary()?;
                let end_span = operand.span;
                Ok(Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Deref,
                        operand: Box::new(operand),
                    },
                    span: Span::with_bytes(
                        span.start,
                        end_span.end,
                        self.file,
                        span.byte_start,
                        end_span.byte_end,
                    ),
                })
            }

            Token::Minus => {
                // -x - negation
                let operand = self.parse_primary()?;
                let end_span = operand.span;
                Ok(Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Neg,
                        operand: Box::new(operand),
                    },
                    span: Span::with_bytes(
                        span.start,
                        end_span.end,
                        self.file,
                        span.byte_start,
                        end_span.byte_end,
                    ),
                })
            }

            Token::Not => {
                // !x - logical not
                let operand = self.parse_primary()?;
                let end_span = operand.span;
                Ok(Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Not,
                        operand: Box::new(operand),
                    },
                    span: Span::with_bytes(
                        span.start,
                        end_span.end,
                        self.file,
                        span.byte_start,
                        end_span.byte_end,
                    ),
                })
            }

            Token::Arrow => {
                // <-ch - channel receive
                let operand = self.parse_primary()?;
                let end_span = operand.span;
                Ok(Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Recv,
                        operand: Box::new(operand),
                    },
                    span: Span::with_bytes(
                        span.start,
                        end_span.end,
                        self.file,
                        span.byte_start,
                        end_span.byte_end,
                    ),
                })
            }

            Token::Integer(n) => Ok(Expr {
                kind: ExprKind::Integer(n),
                span,
            }),

            Token::Float(f) => Ok(Expr {
                kind: ExprKind::Float(f),
                span,
            }),

            Token::String(s) => {
                // Check if string contains interpolation markers
                if s.contains('{') {
                    let parts = self.parse_string_interpolation(&s, span)?;
                    if parts.len() == 1 {
                        // Single literal part - just a plain string
                        if let StringPart::Literal(lit) = &parts[0] {
                            return Ok(Expr {
                                kind: ExprKind::String(lit.clone()),
                                span,
                            });
                        }
                    }
                    Ok(Expr {
                        kind: ExprKind::StringInterpolation(parts),
                        span,
                    })
                } else {
                    Ok(Expr {
                        kind: ExprKind::String(s),
                        span,
                    })
                }
            }

            Token::True => Ok(Expr {
                kind: ExprKind::Bool(true),
                span,
            }),

            Token::False => Ok(Expr {
                kind: ExprKind::Bool(false),
                span,
            }),

            Token::Nil => Ok(Expr {
                kind: ExprKind::Nil,
                span,
            }),

            Token::Ident(name) if name == "map" => {
                // Map literal: map[K]V{key: val, ...}
                self.expect(Token::LBracket)?;
                let key_ty = self.parse_type()?;
                self.expect(Token::RBracket)?;
                let val_ty = self.parse_type()?;

                let map_ty = Type {
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

                Ok(Expr {
                    kind: ExprKind::MapLit {
                        ty: map_ty,
                        entries,
                    },
                    span: Span::with_bytes(
                        span.start,
                        end_span.end,
                        self.file,
                        span.byte_start,
                        end_span.byte_end,
                    ),
                })
            }

            Token::Ident(name) if name == "make" => {
                // make(type, args...) - built-in for creating slices, maps, channels
                self.expect(Token::LParen)?;

                // First argument is a type
                let ty = self.parse_type()?;

                // Optional additional arguments (size, capacity) - always positional
                let mut args = Vec::new();
                while self.consume(&Token::Comma) {
                    args.push((None, self.parse_expr()?));
                }

                let end_span = self.expect(Token::RParen)?;

                // Generate as a call to make with type as first "argument" (special handling in codegen)
                // We'll encode the type in the call expression using a special type argument
                Ok(Expr {
                    kind: ExprKind::Call {
                        func: Box::new(Expr {
                            kind: ExprKind::Ident("make".to_string()),
                            span,
                        }),
                        type_args: vec![ty],
                        args,
                    },
                    span: Span::with_bytes(
                        span.start,
                        end_span.end,
                        self.file,
                        span.byte_start,
                        end_span.byte_end,
                    ),
                })
            }

            Token::Ident(name) if name == "new" => {
                // new(type) - built-in for creating pointer to zero value
                self.expect(Token::LParen)?;
                let ty = self.parse_type()?;
                let end_span = self.expect(Token::RParen)?;

                Ok(Expr {
                    kind: ExprKind::Call {
                        func: Box::new(Expr {
                            kind: ExprKind::Ident("new".to_string()),
                            span,
                        }),
                        type_args: vec![ty],
                        args: vec![], // new has no runtime args
                    },
                    span: Span::with_bytes(
                        span.start,
                        end_span.end,
                        self.file,
                        span.byte_start,
                        end_span.byte_end,
                    ),
                })
            }

            Token::Ident(name) => Ok(Expr {
                kind: ExprKind::Ident(name),
                span,
            }),

            Token::Underscore => Ok(Expr {
                kind: ExprKind::Ident("_".to_string()),
                span,
            }),

            Token::LParen => {
                let expr = self.parse_expr()?;
                self.expect(Token::RParen)?;
                Ok(expr)
            }

            Token::LBracket => {
                // Slice literal: []type{elements}
                // Array literal: [size]type{elements}
                if self.consume(&Token::RBracket) {
                    // []type{elements} - slice literal
                    let elem_ty = self.parse_type()?;
                    // Create a slice type with [] prefix
                    let slice_ty = Type {
                        name: format!("[]{}", elem_ty.name),
                        args: elem_ty.args.clone(),
                        span: elem_ty.span,
                        nullable: false,
                    };
                    self.expect(Token::LBrace)?;

                    let mut elements = Vec::new();
                    if !matches!(self.peek(), Some(Token::RBrace)) {
                        loop {
                            elements.push(self.parse_expr()?);
                            // Allow trailing comma
                            if !self.consume(&Token::Comma) {
                                break;
                            }
                            // Check for closing brace after trailing comma
                            if matches!(self.peek(), Some(Token::RBrace)) {
                                break;
                            }
                        }
                    }

                    let end_span = self.expect(Token::RBrace)?;

                    Ok(Expr {
                        kind: ExprKind::ArrayLit {
                            ty: Some(slice_ty),
                            elements,
                        },
                        span: Span::with_bytes(
                            span.start,
                            end_span.end,
                            self.file,
                            span.byte_start,
                            end_span.byte_end,
                        ),
                    })
                } else {
                    // [size]type{elements} - array literal
                    // Consume the size (we don't validate it)
                    while !matches!(self.peek(), Some(Token::RBracket) | None) {
                        self.advance();
                    }
                    self.expect(Token::RBracket)?;

                    let ty = self.parse_type()?;
                    self.expect(Token::LBrace)?;

                    let mut elements = Vec::new();
                    if !matches!(self.peek(), Some(Token::RBrace)) {
                        loop {
                            elements.push(self.parse_expr()?);
                            if !self.consume(&Token::Comma) {
                                break;
                            }
                            if matches!(self.peek(), Some(Token::RBrace)) {
                                break;
                            }
                        }
                    }

                    let end_span = self.expect(Token::RBrace)?;

                    Ok(Expr {
                        kind: ExprKind::ArrayLit {
                            ty: Some(ty),
                            elements,
                        },
                        span: Span::with_bytes(
                            span.start,
                            end_span.end,
                            self.file,
                            span.byte_start,
                            end_span.byte_end,
                        ),
                    })
                }
            }

            Token::Func => {
                // Anonymous function: func(params) returnTypes { body }
                self.expect(Token::LParen)?;

                // Parse parameters
                let mut params = Vec::new();
                if !matches!(self.peek(), Some(Token::RParen)) {
                    loop {
                        let (param_name, param_span) = match self.advance() {
                            Some((Token::Ident(name), span)) => (name, span),
                            Some((tok, span)) => {
                                return Err(SoppoError::Parse {
                                    message: format!("Expected parameter name, found {:?}", tok),
                                    span,
                                });
                            }
                            None => {
                                return Err(SoppoError::Parse {
                                    message: "Unexpected end of input".to_string(),
                                    span: Span::dummy(),
                                });
                            }
                        };

                        let param_ty = self.parse_type()?;
                        params.push(Param {
                            name: param_name,
                            ty: param_ty,
                            span: param_span,
                        });

                        if !self.consume(&Token::Comma) {
                            break;
                        }
                    }
                }
                self.expect(Token::RParen)?;

                // Parse return types
                let mut return_types = Vec::new();
                // Check for multi-return: (type1, type2)
                if self.consume(&Token::LParen) {
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        loop {
                            return_types.push(self.parse_type()?);
                            if !self.consume(&Token::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(Token::RParen)?;
                } else if matches!(
                    self.peek(),
                    Some(Token::Ident(_)) | Some(Token::LBracket) | Some(Token::Star)
                ) && !matches!(self.peek(), Some(Token::LBrace))
                {
                    // Single return type (not followed by {)
                    return_types.push(self.parse_type()?);
                }

                // Parse body
                let body = self.parse_block()?;

                Ok(Expr {
                    kind: ExprKind::FuncLit {
                        params,
                        return_types,
                        body: body.clone(),
                    },
                    span: Span::with_bytes(
                        span.start,
                        body.span.end,
                        self.file,
                        span.byte_start,
                        body.span.byte_end,
                    ),
                })
            }

            _ => Err(SoppoError::Parse {
                message: format!("Unexpected token: {:?}", tok),
                span,
            }),
        }
    }

    /// Parse a string with interpolation: "Hello, {name}!"
    /// Returns a vector of StringPart
    fn parse_string_interpolation(&mut self, s: &str, span: Span) -> Result<Vec<StringPart>> {
        let mut parts = Vec::new();
        let mut current_literal = String::new();
        let mut chars = s.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '{' {
                // Check for escaped brace: {{
                if chars.peek() == Some(&'{') {
                    chars.next();
                    current_literal.push('{');
                    continue;
                }

                // Save any accumulated literal
                if !current_literal.is_empty() {
                    parts.push(StringPart::Literal(current_literal.clone()));
                    current_literal.clear();
                }

                // Extract the expression inside {}
                let mut expr_str = String::new();
                let mut brace_depth = 1;

                for inner_ch in chars.by_ref() {
                    if inner_ch == '{' {
                        brace_depth += 1;
                        expr_str.push(inner_ch);
                    } else if inner_ch == '}' {
                        brace_depth -= 1;
                        if brace_depth == 0 {
                            break;
                        }
                        expr_str.push(inner_ch);
                    } else {
                        expr_str.push(inner_ch);
                    }
                }

                if brace_depth != 0 {
                    return Err(SoppoError::Parse {
                        message: "Unclosed interpolation brace in string".to_string(),
                        span,
                    });
                }

                // Parse the expression
                let mut expr_parser = Parser::new(&expr_str, self.file);
                let expr = expr_parser.parse_expr()?;
                parts.push(StringPart::Expr(Box::new(expr)));
            } else if ch == '}' {
                // Check for escaped brace: }}
                if chars.peek() == Some(&'}') {
                    chars.next();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result;
    use crate::syntax::ast::ExprKind;
    use crate::syntax::source::FileId;

    fn parse_expr_helper(source: &str) -> Result<Expr> {
        Parser::new(source, FileId(0)).parse_expr()
    }

    #[test]
    fn test_parse_integer() {
        let expr = parse_expr_helper("42").unwrap();
        assert!(matches!(expr.kind, ExprKind::Integer(42)));
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
                assert!(matches!(left.kind, ExprKind::Integer(1)));
                assert!(matches!(right.kind, ExprKind::Integer(2)));
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
                assert!(matches!(left.kind, ExprKind::Integer(1)));
                match mul_expr.kind {
                    ExprKind::Binary { op, left, right } => {
                        assert_eq!(op, BinOp::Mul);
                        assert!(matches!(left.kind, ExprKind::Integer(2)));
                        assert!(matches!(right.kind, ExprKind::Integer(3)));
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
                assert!(matches!(args[0].1.kind, ExprKind::Integer(1)));
                assert!(args[1].0.is_none());
                assert!(matches!(args[1].1.kind, ExprKind::Integer(2)));
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
                assert!(matches!(args[0].1.kind, ExprKind::Integer(1)));
                assert!(matches!(&args[1].0, Some((n, _)) if n == "b"));
                assert!(matches!(args[1].1.kind, ExprKind::Integer(2)));
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
                assert!(matches!(args[0].1.kind, ExprKind::Integer(1)));
                assert!(matches!(&args[1].0, Some((n, _)) if n == "b"));
                assert!(matches!(args[1].1.kind, ExprKind::Integer(2)));
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
}
