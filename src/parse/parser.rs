use super::ast::*;
use super::lexer::{Lexer, Token};
use super::source::{FileId, Span};
use crate::error::{Result, SoppoError};

// Reserved keywords that cannot be used as identifiers
const RESERVED_KEYWORDS: &[&str] = &[
    // Go keywords
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
    // Soppo-specific
    "match",
    "enum",
];

pub struct Parser {
    tokens: Vec<(Token, Span)>,
    pos: usize,
    file: FileId,
}

impl Parser {
    pub fn new(source: &str, file: FileId) -> Self {
        let mut lexer = Lexer::new(source, file);
        let tokens = lexer.collect_all();

        Self {
            tokens,
            pos: 0,
            file,
        }
    }

    /// Peek at current token without consuming
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|(tok, _)| tok)
    }

    /// Peek at token at offset from current position
    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset).map(|(tok, _)| tok)
    }

    /// Peek at current span
    fn peek_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|(_, span)| span.clone())
            .unwrap_or_else(Span::dummy)
    }

    /// Consume current token and return it with span
    fn advance(&mut self) -> Option<(Token, Span)> {
        if self.pos < self.tokens.len() {
            let result = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(result)
        } else {
            None
        }
    }

    /// Check if current token matches, consume if so
    fn consume(&mut self, expected: &Token) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Skip statement terminators (newlines and semicolons)
    fn skip_terminators(&mut self) {
        while matches!(self.peek(), Some(Token::Newline) | Some(Token::Semicolon)) {
            self.advance();
        }
    }

    /// Check if current token is a statement terminator
    fn is_terminator(&self) -> bool {
        matches!(self.peek(), Some(Token::Newline) | Some(Token::Semicolon))
    }

    /// Expect a specific token, error if not present
    fn expect(&mut self, expected: Token) -> Result<Span> {
        match self.advance() {
            Some((tok, span)) if tok == expected => Ok(span),
            Some((tok, span)) => Err(SoppoError::Parse {
                message: format!("Expected {:?}, found {:?}", expected, tok),
                span,
            }),
            None => Err(SoppoError::Parse {
                message: format!("Expected {:?}, found EOF", expected),
                span: Span::dummy(),
            }),
        }
    }

    /// Check if an identifier is a reserved keyword
    fn is_reserved_keyword(name: &str) -> bool {
        RESERVED_KEYWORDS.contains(&name)
    }

    /// Validate that an identifier is not a reserved keyword
    fn validate_identifier(&self, name: &str, span: &Span) -> Result<()> {
        if Self::is_reserved_keyword(name) {
            Err(SoppoError::Parse {
                message: format!(
                    "`{}` is a reserved keyword and cannot be used as an identifier",
                    name
                ),
                span: span.clone(),
            })
        } else {
            Ok(())
        }
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
                // &x - address of
                let operand = self.parse_primary()?;
                let end_span = operand.span.clone();
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
                let end_span = operand.span.clone();
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
                let end_span = operand.span.clone();
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
                let end_span = operand.span.clone();
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
                let end_span = operand.span.clone();
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

            Token::String(s) => Ok(Expr {
                kind: ExprKind::String(s),
                span,
            }),

            Token::True => Ok(Expr {
                kind: ExprKind::Bool(true),
                span,
            }),

            Token::False => Ok(Expr {
                kind: ExprKind::Bool(false),
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
                    span: span.clone(),
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

                // Optional additional arguments (size, capacity)
                let mut args = Vec::new();
                while self.consume(&Token::Comma) {
                    args.push(self.parse_expr()?);
                }

                let end_span = self.expect(Token::RParen)?;

                // Generate as a call to make with type as first "argument" (special handling in codegen)
                // We'll encode the type in the call expression using a special type argument
                Ok(Expr {
                    kind: ExprKind::Call {
                        func: Box::new(Expr {
                            kind: ExprKind::Ident("make".to_string()),
                            span: span.clone(),
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
                            span: span.clone(),
                        }),
                        type_args: vec![ty],
                        args: vec![],
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
                        span: elem_ty.span.clone(),
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

    /// Parse match statement
    fn parse_match_stmt(&mut self, start_span: Span) -> Result<Stmt> {
        // Check for expression-less match: `match {`
        let (scrutinee, is_expression_less) = if matches!(self.peek(), Some(Token::LBrace)) {
            (None, true)
        } else {
            (Some(self.parse_expr()?), false)
        };

        self.expect(Token::LBrace)?;
        // Skip terminators after opening brace
        self.skip_terminators();

        let mut arms = Vec::new();

        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let arm = self.parse_match_arm(is_expression_less)?;
            arms.push(arm);

            // Skip terminators between arms
            self.skip_terminators();
        }

        let end_span = self.expect(Token::RBrace)?;

        Ok(Stmt {
            kind: StmtKind::Match { scrutinee, arms },
            span: Span::with_bytes(
                start_span.start,
                end_span.end,
                self.file,
                start_span.byte_start,
                end_span.byte_end,
            ),
        })
    }

    /// Parse a match arm: case pattern, pattern: statements (until next case/default/})
    fn parse_match_arm(&mut self, is_expression_less: bool) -> Result<Arm> {
        // Handle both 'case Pattern:' and 'default:'
        let patterns = if let Some(Token::Ident(s)) = self.peek() {
            if s == "default" {
                let span = self.peek_span();
                self.advance(); // consume 'default'
                vec![Pattern {
                    kind: PatternKind::Default,
                    span,
                }]
            } else {
                self.expect(Token::Case)?;
                self.parse_patterns(is_expression_less)?
            }
        } else {
            self.expect(Token::Case)?;
            self.parse_patterns(is_expression_less)?
        };

        self.expect(Token::Colon)?;
        // Skip terminators after colon
        self.skip_terminators();

        // Parse statements until we hit the next case, default, or closing brace
        let mut stmts = Vec::new();
        let body_start = self.peek_span();

        while !matches!(self.peek(), Some(Token::Case) | Some(Token::RBrace) | None) {
            // Check if it's 'default' - this is a bit hacky but works for now
            if let Some(Token::Ident(s)) = self.peek()
                && s == "default"
            {
                break;
            }

            stmts.push(self.parse_stmt()?);
            // Skip terminators between statements in arm
            self.skip_terminators();
        }

        let body_end = stmts
            .last()
            .map(|s| s.span.clone())
            .unwrap_or(body_start.clone());

        let body = Block {
            stmts,
            span: body_end.clone(),
        };

        let first_span = &patterns[0].span;
        let span = Span::with_bytes(
            first_span.start,
            body.span.end,
            self.file,
            first_span.byte_start,
            body.span.byte_end,
        );

        Ok(Arm {
            patterns,
            body,
            span,
        })
    }

    /// Parse comma-separated patterns for a match arm
    fn parse_patterns(&mut self, is_expression_less: bool) -> Result<Vec<Pattern>> {
        let mut patterns = vec![self.parse_pattern_or_guard(is_expression_less)?];

        // Parse additional comma-separated patterns
        while matches!(self.peek(), Some(Token::Comma)) {
            self.advance(); // consume comma
            patterns.push(self.parse_pattern_or_guard(is_expression_less)?);
        }

        Ok(patterns)
    }

    /// Parse a pattern or guard expression (for expression-less match)
    fn parse_pattern_or_guard(&mut self, is_expression_less: bool) -> Result<Pattern> {
        if is_expression_less {
            // For expression-less match, parse an expression as the guard
            let expr = self.parse_expr()?;
            let span = expr.span.clone();
            Ok(Pattern {
                kind: PatternKind::Guard(Box::new(expr)),
                span,
            })
        } else {
            self.parse_pattern()
        }
    }

    /// Parse a pattern
    fn parse_pattern(&mut self) -> Result<Pattern> {
        let (tok, span) = self.advance().ok_or_else(|| SoppoError::Parse {
            message: "Expected pattern".to_string(),
            span: Span::dummy(),
        })?;

        match tok {
            Token::Underscore => Ok(Pattern {
                kind: PatternKind::Default,
                span,
            }),

            // Literal patterns
            Token::Integer(n) => Ok(Pattern {
                kind: PatternKind::Literal(super::ast::Literal::Integer(n)),
                span,
            }),

            Token::String(s) => Ok(Pattern {
                kind: PatternKind::Literal(super::ast::Literal::String(s)),
                span,
            }),

            Token::True => Ok(Pattern {
                kind: PatternKind::Literal(super::ast::Literal::Bool(true)),
                span,
            }),

            Token::False => Ok(Pattern {
                kind: PatternKind::Literal(super::ast::Literal::Bool(false)),
                span,
            }),

            Token::Ident(mut name) => {
                let mut current_span = span;

                // Check for field access: Type.Variant
                while self.consume(&Token::Dot) {
                    let field_name = match self.advance() {
                        Some((Token::Ident(field), field_span)) => {
                            current_span = Span::with_bytes(
                                current_span.start,
                                field_span.end,
                                self.file,
                                current_span.byte_start,
                                field_span.byte_end,
                            );
                            field
                        }
                        Some((tok, span)) => {
                            return Err(SoppoError::Parse {
                                message: format!("Expected field name after '.', found {:?}", tok),
                                span,
                            });
                        }
                        None => {
                            return Err(SoppoError::Parse {
                                message: "Expected field name after '.'".to_string(),
                                span: Span::dummy(),
                            });
                        }
                    };
                    name = format!("{}.{}", name, field_name);
                }

                // Check if it's a destructor pattern: Result.Ok(value)
                if self.consume(&Token::LParen) {
                    // Parse the single binding variable
                    let binding = match self.advance() {
                        Some((Token::Ident(binding), _)) => binding,
                        Some((tok, span)) => {
                            return Err(SoppoError::Parse {
                                message: format!(
                                    "Expected binding variable in pattern, found {:?}",
                                    tok
                                ),
                                span,
                            });
                        }
                        None => {
                            return Err(SoppoError::Parse {
                                message: "Expected binding variable in pattern".to_string(),
                                span: Span::dummy(),
                            });
                        }
                    };

                    let end_span = self.expect(Token::RParen)?;

                    Ok(Pattern {
                        kind: PatternKind::Destructor { name, binding },
                        span: Span::with_bytes(
                            current_span.start,
                            end_span.end,
                            self.file,
                            current_span.byte_start,
                            end_span.byte_end,
                        ),
                    })
                }
                // Check if it's a struct destructor pattern: Shape.Circle{radius: r, ...}
                else if self.consume(&Token::LBrace) {
                    let mut fields = Vec::new();
                    let mut rest = false;

                    // Parse field bindings
                    if !matches!(self.peek(), Some(Token::RBrace)) {
                        loop {
                            // Check for ... (rest pattern)
                            if self.consume(&Token::DotDotDot) {
                                rest = true;
                                break;
                            }

                            // Parse field_name: binding_name
                            let field_name = match self.advance() {
                                Some((Token::Ident(name), _)) => name,
                                Some((tok, span)) => {
                                    return Err(SoppoError::Parse {
                                        message: format!(
                                            "Expected field name in struct pattern, found {:?}",
                                            tok
                                        ),
                                        span,
                                    });
                                }
                                None => {
                                    return Err(SoppoError::Parse {
                                        message: "Expected field name in struct pattern"
                                            .to_string(),
                                        span: Span::dummy(),
                                    });
                                }
                            };

                            self.expect(Token::Colon)?;

                            let binding_name = match self.advance() {
                                Some((Token::Ident(name), _)) => name,
                                Some((tok, span)) => {
                                    return Err(SoppoError::Parse {
                                        message: format!(
                                            "Expected binding name in struct pattern, found {:?}",
                                            tok
                                        ),
                                        span,
                                    });
                                }
                                None => {
                                    return Err(SoppoError::Parse {
                                        message: "Expected binding name in struct pattern"
                                            .to_string(),
                                        span: Span::dummy(),
                                    });
                                }
                            };

                            fields.push((field_name, binding_name));

                            if !self.consume(&Token::Comma) {
                                break;
                            }
                        }
                    }

                    let end_span = self.expect(Token::RBrace)?;

                    Ok(Pattern {
                        kind: PatternKind::StructDestructor { name, fields, rest },
                        span: Span::with_bytes(
                            current_span.start,
                            end_span.end,
                            self.file,
                            current_span.byte_start,
                            end_span.byte_end,
                        ),
                    })
                }
                // Just a variant name
                else {
                    Ok(Pattern {
                        kind: PatternKind::Variant(name),
                        span: current_span,
                    })
                }
            }

            _ => Err(SoppoError::Parse {
                message: format!("Expected pattern, found {:?}", tok),
                span,
            }),
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
                                    args.push(self.parse_expr()?);

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

                    // Not type args - backtrack and parse as array index
                    self.pos = saved_pos;
                    self.advance(); // consume the [ we backtracked past
                    let index = self.parse_expr()?;
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
                            index: Box::new(index),
                        },
                    };
                }

                // Function call without type args: expr(args)
                Some(Token::LParen) => {
                    self.advance();
                    let mut args = Vec::new();

                    if !matches!(self.peek(), Some(Token::RParen)) {
                        loop {
                            args.push(self.parse_expr()?);

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

                // Field access: expr.field
                Some(Token::Dot) => {
                    self.advance();
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
                                    span: expr.span.clone(),
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
            _ => None,
        }
    }

    /// Parse an expression
    pub fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_binary(0)
    }

    /// Parse a statement
    pub fn parse_stmt(&mut self) -> Result<Stmt> {
        let start_span = self.peek_span();

        match self.peek() {
            Some(Token::Ident(_)) => {
                // Parse as expression first, then check for assignment operators
                let first_target = self.parse_expr()?;

                // Check for multi-value declaration/assignment: a, b := ... or a, b = ...
                if let ExprKind::Ident(first_name) = &first_target.kind
                    && self.consume(&Token::Comma)
                {
                    // Multi-value: collect more identifiers
                    let mut names = vec![first_name.clone()];
                    let mut targets = vec![first_target.clone()];

                    loop {
                        let target = self.parse_expr()?;
                        if let ExprKind::Ident(name) = &target.kind {
                            names.push(name.clone());
                            targets.push(target);
                        } else {
                            return Err(SoppoError::Parse {
                                message: "Multi-value assignment targets must be identifiers"
                                    .to_string(),
                                span: target.span,
                            });
                        }

                        if !self.consume(&Token::Comma) {
                            break;
                        }
                    }

                    if self.consume(&Token::ColonAssign) {
                        // Multi-value declaration: a, b := f() or a, b := expr1, expr2
                        for name in &names {
                            self.validate_identifier(name, &first_target.span)?;
                        }
                        let mut values = vec![self.parse_expr()?];
                        while self.consume(&Token::Comma) {
                            values.push(self.parse_expr()?);
                        }
                        // Allow 1 value (multi-return) or N values (one per name)
                        if values.len() != 1 && values.len() != names.len() {
                            return Err(SoppoError::Parse {
                                message: format!(
                                    "Expected 1 or {} values but got {}",
                                    names.len(),
                                    values.len()
                                ),
                                span: first_target.span.clone(),
                            });
                        }
                        let end_span = values.last().unwrap().span.clone();
                        return Ok(Stmt {
                            span: Span::with_bytes(
                                first_target.span.start,
                                end_span.end,
                                self.file,
                                first_target.span.byte_start,
                                end_span.byte_end,
                            ),
                            kind: StmtKind::MultiDecl { names, values },
                        });
                    } else if self.consume(&Token::Assign) {
                        // Multi-value assignment: a, b = f() or a, b = expr1, expr2
                        let mut values = vec![self.parse_expr()?];
                        while self.consume(&Token::Comma) {
                            values.push(self.parse_expr()?);
                        }
                        // Allow 1 value (multi-return) or N values (one per target)
                        if values.len() != 1 && values.len() != targets.len() {
                            return Err(SoppoError::Parse {
                                message: format!(
                                    "Expected 1 or {} values but got {}",
                                    targets.len(),
                                    values.len()
                                ),
                                span: first_target.span.clone(),
                            });
                        }
                        let end_span = values.last().unwrap().span.clone();
                        return Ok(Stmt {
                            span: Span::with_bytes(
                                first_target.span.start,
                                end_span.end,
                                self.file,
                                first_target.span.byte_start,
                                end_span.byte_end,
                            ),
                            kind: StmtKind::MultiAssign { targets, values },
                        });
                    } else {
                        return Err(SoppoError::Parse {
                            message: "Expected := or = after multi-value target".to_string(),
                            span: self.peek_span(),
                        });
                    }
                }

                // Single target
                if self.consume(&Token::ColonAssign) {
                    // Short variable declaration: x := value
                    // target must be a simple identifier
                    if let ExprKind::Ident(name) = first_target.kind {
                        self.validate_identifier(&name, &first_target.span)?;
                        let value = self.parse_expr()?;
                        Ok(Stmt {
                            span: Span::with_bytes(
                                first_target.span.start,
                                value.span.end,
                                self.file,
                                first_target.span.byte_start,
                                value.span.byte_end,
                            ),
                            kind: StmtKind::Decl { name, value },
                        })
                    } else {
                        Err(SoppoError::Parse {
                            message: "Left side of := must be a simple identifier".to_string(),
                            span: first_target.span,
                        })
                    }
                } else if self.consume(&Token::Assign) {
                    // Assignment: x = value or x.y = value
                    let value = self.parse_expr()?;
                    Ok(Stmt {
                        span: Span::with_bytes(
                            first_target.span.start,
                            value.span.end,
                            self.file,
                            first_target.span.byte_start,
                            value.span.byte_end,
                        ),
                        kind: StmtKind::Assign {
                            target: first_target,
                            value,
                        },
                    })
                } else if self.consume(&Token::Arrow) {
                    // Channel send: ch <- value
                    let value = self.parse_expr()?;
                    Ok(Stmt {
                        span: Span::with_bytes(
                            first_target.span.start,
                            value.span.end,
                            self.file,
                            first_target.span.byte_start,
                            value.span.byte_end,
                        ),
                        kind: StmtKind::Send {
                            channel: first_target,
                            value,
                        },
                    })
                } else {
                    // Just an expression statement
                    Ok(Stmt {
                        span: first_target.span.clone(),
                        kind: StmtKind::Expr(first_target),
                    })
                }
            }

            Some(Token::Var) => {
                // var name = value, var name type, var name type = value
                // var a, b, c type, var a, b = 1, 2, var a, b type = 1, 2
                self.advance(); // consume 'var'

                // Parse the first variable name
                let (first_name, first_name_span) = match self.advance() {
                    Some((Token::Ident(name), span)) => (name, span),
                    Some((tok, span)) => {
                        return Err(SoppoError::Parse {
                            message: format!("Expected variable name, found {:?}", tok),
                            span,
                        });
                    }
                    None => {
                        return Err(SoppoError::Parse {
                            message: "Expected variable name".to_string(),
                            span: Span::dummy(),
                        });
                    }
                };

                self.validate_identifier(&first_name, &first_name_span)?;

                // Check for multi-var declaration (comma after first name)
                if self.consume(&Token::Comma) {
                    // Multi-var: var a, b, c type or var a, b = 1, 2
                    let mut names = vec![first_name];

                    // Parse remaining names
                    loop {
                        let (name, name_span) = match self.advance() {
                            Some((Token::Ident(name), span)) => (name, span),
                            Some((tok, span)) => {
                                return Err(SoppoError::Parse {
                                    message: format!("Expected variable name, found {:?}", tok),
                                    span,
                                });
                            }
                            None => {
                                return Err(SoppoError::Parse {
                                    message: "Expected variable name".to_string(),
                                    span: Span::dummy(),
                                });
                            }
                        };
                        self.validate_identifier(&name, &name_span)?;
                        names.push(name);

                        if !self.consume(&Token::Comma) {
                            break;
                        }
                    }

                    // Now parse type and/or values
                    // Allow either:
                    // - var a, b = f() (single multi-return expression)
                    // - var a, b = expr1, expr2 (one expression per name)
                    let (ty, values, end_span) = if self.consume(&Token::Assign) {
                        // var a, b = expr1, expr2 or var a, b = f()
                        let mut vals = vec![self.parse_expr()?];
                        while self.consume(&Token::Comma) {
                            vals.push(self.parse_expr()?);
                        }
                        // Allow 1 value (multi-return) or N values (one per name)
                        if vals.len() != 1 && vals.len() != names.len() {
                            return Err(SoppoError::Parse {
                                message: format!(
                                    "Expected 1 or {} values but got {}",
                                    names.len(),
                                    vals.len()
                                ),
                                span: start_span.clone(),
                            });
                        }
                        let end = vals.last().unwrap().span.clone();
                        (None, vals, end)
                    } else if matches!(
                        self.peek(),
                        Some(Token::Ident(_)) | Some(Token::LBracket) | Some(Token::Star)
                    ) {
                        // var a, b, c type or var a, b type = 1, 2
                        let ty = self.parse_type()?;
                        let ty_span = ty.span.clone();

                        if self.consume(&Token::Assign) {
                            // var a, b type = expr1, expr2 or var a, b type = f()
                            let mut vals = vec![self.parse_expr()?];
                            while self.consume(&Token::Comma) {
                                vals.push(self.parse_expr()?);
                            }
                            // Allow 1 value (multi-return) or N values (one per name)
                            if vals.len() != 1 && vals.len() != names.len() {
                                return Err(SoppoError::Parse {
                                    message: format!(
                                        "Expected 1 or {} values but got {}",
                                        names.len(),
                                        vals.len()
                                    ),
                                    span: start_span.clone(),
                                });
                            }
                            let end = vals.last().unwrap().span.clone();
                            (Some(ty), vals, end)
                        } else {
                            // var a, b, c type (zero values)
                            (Some(ty), vec![], ty_span)
                        }
                    } else {
                        return Err(SoppoError::Parse {
                            message: "Multi-variable declaration requires a type or initializers"
                                .to_string(),
                            span: start_span.clone(),
                        });
                    };

                    Ok(Stmt {
                        span: Span::with_bytes(
                            start_span.start,
                            end_span.end,
                            self.file,
                            start_span.byte_start,
                            end_span.byte_end,
                        ),
                        kind: StmtKind::MultiVarDecl { names, ty, values },
                    })
                } else {
                    // Single var declaration
                    let (ty, value, end_span) = if self.consume(&Token::Assign) {
                        // var name = value (type inference)
                        let expr = self.parse_expr()?;
                        let span = expr.span.clone();
                        (None, Some(expr), span)
                    } else if matches!(
                        self.peek(),
                        Some(Token::Ident(_)) | Some(Token::LBracket) | Some(Token::Star)
                    ) {
                        // var name type ... (explicit type)
                        let ty = self.parse_type()?;
                        let ty_span = ty.span.clone();

                        if self.consume(&Token::Assign) {
                            // var name type = value
                            let expr = self.parse_expr()?;
                            let span = expr.span.clone();
                            (Some(ty), Some(expr), span)
                        } else {
                            // var name type (zero value)
                            (Some(ty), None, ty_span)
                        }
                    } else {
                        // var name (no type, no value - error)
                        return Err(SoppoError::Parse {
                            message:
                                "Variable declaration requires either a type or an initializer"
                                    .to_string(),
                            span: first_name_span,
                        });
                    };

                    Ok(Stmt {
                        span: Span::with_bytes(
                            start_span.start,
                            end_span.end,
                            self.file,
                            start_span.byte_start,
                            end_span.byte_end,
                        ),
                        kind: StmtKind::VarDecl {
                            name: first_name,
                            ty,
                            value,
                        },
                    })
                }
            }

            Some(Token::Const) => {
                // const name = value, const name type = value
                // const a, b = 1, 2, const a, b type = 1, 2
                self.advance(); // consume 'const'

                // Parse the first constant name
                let (first_name, first_name_span) = match self.advance() {
                    Some((Token::Ident(name), span)) => (name, span),
                    Some((tok, span)) => {
                        return Err(SoppoError::Parse {
                            message: format!("Expected constant name, found {:?}", tok),
                            span,
                        });
                    }
                    None => {
                        return Err(SoppoError::Parse {
                            message: "Expected constant name".to_string(),
                            span: Span::dummy(),
                        });
                    }
                };

                self.validate_identifier(&first_name, &first_name_span)?;

                // Check for multi-const declaration (comma after first name)
                if self.consume(&Token::Comma) {
                    // Multi-const: const a, b = 1, 2 or const a, b type = 1, 2
                    let mut names = vec![first_name];

                    // Parse remaining names
                    loop {
                        let (name, name_span) = match self.advance() {
                            Some((Token::Ident(name), span)) => (name, span),
                            Some((tok, span)) => {
                                return Err(SoppoError::Parse {
                                    message: format!("Expected constant name, found {:?}", tok),
                                    span,
                                });
                            }
                            None => {
                                return Err(SoppoError::Parse {
                                    message: "Expected constant name".to_string(),
                                    span: Span::dummy(),
                                });
                            }
                        };
                        self.validate_identifier(&name, &name_span)?;
                        names.push(name);

                        if !self.consume(&Token::Comma) {
                            break;
                        }
                    }

                    // Now parse type and/or values (consts must have values)
                    let (ty, values, end_span) = if self.consume(&Token::Assign) {
                        // const a, b = expr1, expr2 (type inference)
                        let mut vals = vec![self.parse_expr()?];
                        while self.consume(&Token::Comma) {
                            vals.push(self.parse_expr()?);
                        }
                        if vals.len() != names.len() {
                            return Err(SoppoError::Parse {
                                message: format!(
                                    "Expected {} values but got {}",
                                    names.len(),
                                    vals.len()
                                ),
                                span: start_span.clone(),
                            });
                        }
                        let end = vals.last().unwrap().span.clone();
                        (None, vals, end)
                    } else if matches!(
                        self.peek(),
                        Some(Token::Ident(_)) | Some(Token::LBracket) | Some(Token::Star)
                    ) {
                        // const a, b type = 1, 2
                        let ty = self.parse_type()?;
                        let ty_span = ty.span.clone();

                        if !self.consume(&Token::Assign) {
                            return Err(SoppoError::Parse {
                                message: "Multi-constant declaration requires initializers"
                                    .to_string(),
                                span: ty_span,
                            });
                        }

                        let mut vals = vec![self.parse_expr()?];
                        while self.consume(&Token::Comma) {
                            vals.push(self.parse_expr()?);
                        }
                        if vals.len() != names.len() {
                            return Err(SoppoError::Parse {
                                message: format!(
                                    "Expected {} values but got {}",
                                    names.len(),
                                    vals.len()
                                ),
                                span: start_span.clone(),
                            });
                        }
                        let end = vals.last().unwrap().span.clone();
                        (Some(ty), vals, end)
                    } else {
                        return Err(SoppoError::Parse {
                            message: "Expected type or '=' in multi-const declaration".to_string(),
                            span: start_span.clone(),
                        });
                    };

                    Ok(Stmt {
                        span: Span::with_bytes(
                            start_span.start,
                            end_span.end,
                            self.file,
                            start_span.byte_start,
                            end_span.byte_end,
                        ),
                        kind: StmtKind::MultiConstDecl { names, ty, values },
                    })
                } else {
                    // Single const declaration
                    let ty = if self.consume(&Token::Assign) {
                        // const name = value (type inference)
                        None
                    } else if matches!(
                        self.peek(),
                        Some(Token::Ident(_)) | Some(Token::LBracket) | Some(Token::Star)
                    ) {
                        // const name type = value (explicit type)
                        let ty = self.parse_type()?;
                        let ty_span = ty.span.clone();
                        if !self.consume(&Token::Assign) {
                            return Err(SoppoError::Parse {
                                message: format!(
                                    "Constant '{}' requires an initializer (use `const {} {} = <value>`)",
                                    first_name, first_name, ty.name
                                ),
                                span: ty_span,
                            });
                        }
                        Some(ty)
                    } else {
                        return Err(SoppoError::Parse {
                            message: "Expected type or '=' in const declaration".to_string(),
                            span: first_name_span,
                        });
                    };

                    let value = self.parse_expr()?;
                    let end_span = value.span.clone();

                    Ok(Stmt {
                        span: Span::with_bytes(
                            start_span.start,
                            end_span.end,
                            self.file,
                            start_span.byte_start,
                            end_span.byte_end,
                        ),
                        kind: StmtKind::ConstDecl {
                            name: first_name,
                            ty,
                            value,
                        },
                    })
                }
            }

            Some(Token::LBrace) => {
                // Block as statement (creates new scope)
                let block = self.parse_block()?;
                // A block statement evaluates to its last expression
                Ok(Stmt {
                    span: block.span.clone(),
                    kind: StmtKind::Expr(Expr {
                        kind: ExprKind::Block(block.clone()),
                        span: block.span.clone(),
                    }),
                })
            }

            Some(Token::For) => {
                self.advance();

                // Check if this is a range loop: for x := range ... or for x, y := range ...
                // We need to look ahead to see if we have: ident [, ident] := range
                let saved_pos = self.pos;

                // Try to parse range loop
                if let Some(Token::Ident(first_name)) = self.peek().cloned() {
                    self.advance();
                    let first_name = first_name.clone();

                    // Check for second variable: for x, y := range
                    let second_name = if self.consume(&Token::Comma) {
                        if let Some(Token::Ident(second)) = self.peek().cloned() {
                            self.advance();
                            Some(second)
                        } else {
                            // Not a valid range pattern, backtrack
                            self.pos = saved_pos;
                            None
                        }
                    } else {
                        None
                    };

                    // Check for := range
                    if (second_name.is_some() || matches!(self.peek(), Some(Token::ColonAssign)))
                        && self.consume(&Token::ColonAssign)
                        && self.consume(&Token::Range)
                    {
                        // This is a range loop
                        let collection = self.parse_expr()?;
                        let body = self.parse_block()?;

                        return Ok(Stmt {
                            span: Span::with_bytes(
                                start_span.start,
                                body.span.end,
                                self.file,
                                start_span.byte_start,
                                body.span.byte_end,
                            ),
                            kind: StmtKind::ForRange {
                                key: first_name,
                                value: second_name,
                                collection,
                                body,
                            },
                        });
                    }

                    // Not a range loop, backtrack
                    self.pos = saved_pos;
                }

                // Regular for loop with condition
                let condition = self.parse_expr()?;
                let body = self.parse_block()?;

                Ok(Stmt {
                    span: Span::with_bytes(
                        start_span.start,
                        body.span.end,
                        self.file,
                        start_span.byte_start,
                        body.span.byte_end,
                    ),
                    kind: StmtKind::For { condition, body },
                })
            }

            Some(Token::If) => {
                self.advance();
                let condition = self.parse_expr()?;
                let then_block = self.parse_block()?;

                let else_block = if self.consume(&Token::Else) {
                    // Check for else if
                    if matches!(self.peek(), Some(Token::If)) {
                        // else if is treated as else { if ... }
                        let if_stmt = self.parse_stmt()?;
                        let span = if_stmt.span.clone();
                        Some(Block {
                            stmts: vec![if_stmt],
                            span,
                        })
                    } else {
                        Some(self.parse_block()?)
                    }
                } else {
                    None
                };

                let end_span = else_block
                    .as_ref()
                    .map(|b| b.span.clone())
                    .unwrap_or(then_block.span.clone());

                Ok(Stmt {
                    span: Span::with_bytes(
                        start_span.start,
                        end_span.end,
                        self.file,
                        start_span.byte_start,
                        end_span.byte_end,
                    ),
                    kind: StmtKind::If {
                        condition,
                        then_block,
                        else_block,
                    },
                })
            }

            Some(Token::Return) => {
                self.advance();
                let values = if matches!(self.peek(), Some(Token::RBrace) | None) {
                    vec![]
                } else {
                    // Parse comma-separated return values
                    let mut values = vec![self.parse_expr()?];
                    while self.consume(&Token::Comma) {
                        values.push(self.parse_expr()?);
                    }
                    values
                };

                let end_span = values
                    .last()
                    .map(|v| v.span.clone())
                    .unwrap_or(start_span.clone());

                Ok(Stmt {
                    span: Span::with_bytes(
                        start_span.start,
                        end_span.end,
                        self.file,
                        start_span.byte_start,
                        end_span.byte_end,
                    ),
                    kind: StmtKind::Return { values },
                })
            }

            Some(Token::Match) => {
                self.advance();
                self.parse_match_stmt(start_span)
            }

            Some(Token::Go) => {
                self.advance();
                let expr = self.parse_expr()?;
                let end_span = expr.span.clone();
                Ok(Stmt {
                    span: Span::with_bytes(
                        start_span.start,
                        end_span.end,
                        self.file,
                        start_span.byte_start,
                        end_span.byte_end,
                    ),
                    kind: StmtKind::Go(expr),
                })
            }

            Some(Token::Defer) => {
                self.advance();
                let expr = self.parse_expr()?;
                let end_span = expr.span.clone();
                Ok(Stmt {
                    span: Span::with_bytes(
                        start_span.start,
                        end_span.end,
                        self.file,
                        start_span.byte_start,
                        end_span.byte_end,
                    ),
                    kind: StmtKind::DeferStmt(expr),
                })
            }

            Some(Token::Break) => {
                self.advance();
                Ok(Stmt {
                    span: start_span,
                    kind: StmtKind::Break,
                })
            }

            Some(Token::Continue) => {
                self.advance();
                Ok(Stmt {
                    span: start_span,
                    kind: StmtKind::Continue,
                })
            }

            _ => {
                // Parse as expression, then check for assignment
                let expr = self.parse_expr()?;

                if self.consume(&Token::Assign) {
                    // Assignment to a dereference or other expression: *p = value
                    let value = self.parse_expr()?;
                    Ok(Stmt {
                        span: Span::with_bytes(
                            expr.span.start,
                            value.span.end,
                            self.file,
                            expr.span.byte_start,
                            value.span.byte_end,
                        ),
                        kind: StmtKind::Assign {
                            target: expr,
                            value,
                        },
                    })
                } else if self.consume(&Token::Arrow) {
                    // Channel send: ch <- value
                    let value = self.parse_expr()?;
                    Ok(Stmt {
                        span: Span::with_bytes(
                            expr.span.start,
                            value.span.end,
                            self.file,
                            expr.span.byte_start,
                            value.span.byte_end,
                        ),
                        kind: StmtKind::Send {
                            channel: expr,
                            value,
                        },
                    })
                } else {
                    // Just an expression statement
                    Ok(Stmt {
                        span: expr.span.clone(),
                        kind: StmtKind::Expr(expr),
                    })
                }
            }
        }
    }

    /// Parse a block of statements
    pub fn parse_block(&mut self) -> Result<Block> {
        let start_span = self.expect(Token::LBrace)?;
        // Skip terminators after opening brace
        self.skip_terminators();

        let mut stmts = Vec::new();

        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            stmts.push(self.parse_stmt()?);
            // Skip terminators between statements
            self.skip_terminators();
        }

        let end_span = self.expect(Token::RBrace)?;

        Ok(Block {
            stmts,
            span: Span::with_bytes(
                start_span.start,
                end_span.end,
                self.file,
                start_span.byte_start,
                end_span.byte_end,
            ),
        })
    }

    /// Parse a type annotation
    /// Supports: T, []T, [N]T, *T, map[K]V, chan T, T[A, B]
    fn parse_type(&mut self) -> Result<Type> {
        let start_span = self.peek_span();

        // Slice type: []T
        if self.consume(&Token::LBracket) {
            if self.consume(&Token::RBracket) {
                // []T - slice
                let elem_ty = self.parse_type()?;
                return Ok(Type {
                    name: format!("[]{}", elem_ty.name),
                    args: vec![elem_ty],
                    span: start_span,
                });
            } else {
                // [N]T - array (consume the size, we don't validate it)
                // Could be a number or ...
                while !matches!(self.peek(), Some(Token::RBracket) | None) {
                    self.advance();
                }
                self.expect(Token::RBracket)?;
                let elem_ty = self.parse_type()?;
                return Ok(Type {
                    name: format!("[]{}", elem_ty.name), // Treat arrays as slices for simplicity
                    args: vec![elem_ty],
                    span: start_span,
                });
            }
        }

        // Pointer type: *T
        if self.consume(&Token::Star) {
            let pointee_ty = self.parse_type()?;
            return Ok(Type {
                name: format!("*{}", pointee_ty.name),
                args: vec![pointee_ty],
                span: start_span,
            });
        }

        // Now we need an identifier
        let (name, span) = match self.advance() {
            Some((Token::Ident(name), span)) => (name, span),
            Some((tok, span)) => {
                return Err(SoppoError::Parse {
                    message: format!("Expected type name, found {:?}", tok),
                    span,
                });
            }
            None => {
                return Err(SoppoError::Parse {
                    message: "Expected type name".to_string(),
                    span: Span::dummy(),
                });
            }
        };

        // Map type: map[K]V
        if name == "map" {
            self.expect(Token::LBracket)?;
            let key_ty = self.parse_type()?;
            self.expect(Token::RBracket)?;
            let val_ty = self.parse_type()?;
            return Ok(Type {
                name: format!("map[{}]{}", key_ty.name, val_ty.name),
                args: vec![key_ty, val_ty],
                span,
            });
        }

        // Channel type: chan T, <-chan T, chan<- T
        if name == "chan" {
            let elem_ty = self.parse_type()?;
            return Ok(Type {
                name: format!("chan {}", elem_ty.name),
                args: vec![elem_ty],
                span,
            });
        }

        // Check for type arguments: Type[T, U]
        let args = if self.consume(&Token::LBracket) {
            let mut args = Vec::new();

            if !matches!(self.peek(), Some(Token::RBracket)) {
                loop {
                    args.push(self.parse_type()?);

                    if !self.consume(&Token::Comma) {
                        break;
                    }
                }
            }

            self.expect(Token::RBracket)?;
            args
        } else {
            Vec::new()
        };

        Ok(Type { name, args, span })
    }

    /// Parse function parameter
    fn parse_param(&mut self) -> Result<Param> {
        // Go syntax: name Type (no colon)
        let (name, name_span) = match self.advance() {
            Some((Token::Ident(name), span)) => (name, span),
            Some((tok, span)) => {
                return Err(SoppoError::Parse {
                    message: format!("Expected parameter name, found {:?}", tok),
                    span,
                });
            }
            None => {
                return Err(SoppoError::Parse {
                    message: "Expected parameter name".to_string(),
                    span: Span::dummy(),
                });
            }
        };

        self.validate_identifier(&name, &name_span)?;

        let ty = self.parse_type()?;

        Ok(Param {
            name,
            ty,
            span: name_span,
        })
    }

    /// Parse generic parameters: [T any, E comparable]
    fn parse_generics(&mut self) -> Result<Vec<Generic>> {
        if !self.consume(&Token::LBracket) {
            return Ok(Vec::new());
        }

        let mut generics = Vec::new();

        if !matches!(self.peek(), Some(Token::RBracket)) {
            loop {
                let (name, span) = match self.advance() {
                    Some((Token::Ident(name), span)) => (name, span),
                    Some((tok, span)) => {
                        return Err(SoppoError::Parse {
                            message: format!("Expected generic parameter name, found {:?}", tok),
                            span,
                        });
                    }
                    None => {
                        return Err(SoppoError::Parse {
                            message: "Expected generic parameter name".to_string(),
                            span: Span::dummy(),
                        });
                    }
                };

                // Parse constraint (required in Go): T any, E comparable, etc.
                let constraint = match self.advance() {
                    Some((Token::Ident(constraint), _)) => constraint,
                    Some((tok, span)) => {
                        return Err(SoppoError::Parse {
                            message: format!(
                                "Expected type constraint (e.g., 'any', 'comparable'), found {:?}",
                                tok
                            ),
                            span,
                        });
                    }
                    None => {
                        return Err(SoppoError::Parse {
                            message: "Expected type constraint".to_string(),
                            span: Span::dummy(),
                        });
                    }
                };

                generics.push(Generic {
                    name,
                    constraint,
                    span,
                });

                if !self.consume(&Token::Comma) {
                    break;
                }
            }
        }

        self.expect(Token::RBracket)?;
        Ok(generics)
    }

    /// Parse function declaration
    pub fn parse_func_decl(&mut self) -> Result<FuncDecl> {
        let start_span = self.expect(Token::Func)?;

        // Check for receiver: func (r: Type) name() or func name()
        let receiver = if self.consume(&Token::LParen) {
            let param = self.parse_param()?;
            self.expect(Token::RParen)?;
            Some(param)
        } else {
            None
        };

        let (name, name_span) = match self.advance() {
            Some((Token::Ident(name), span)) => (name, span),
            Some((tok, span)) => {
                return Err(SoppoError::Parse {
                    message: format!("Expected function name, found {:?}", tok),
                    span,
                });
            }
            None => {
                return Err(SoppoError::Parse {
                    message: "Expected function name".to_string(),
                    span: Span::dummy(),
                });
            }
        };

        self.validate_identifier(&name, &name_span)?;

        // Parse optional generics [T any, U any]
        let generics = self.parse_generics()?;

        // Parse parameters
        self.expect(Token::LParen)?;
        let mut params = Vec::new();

        if !matches!(self.peek(), Some(Token::RParen)) {
            loop {
                params.push(self.parse_param()?);

                if !self.consume(&Token::Comma) {
                    break;
                }
            }
        }

        self.expect(Token::RParen)?;

        // Parse optional return type(s)
        // Go-style: single type or (type1, type2, ...)
        let return_types = if matches!(self.peek(), Some(Token::LBrace)) {
            // No return type
            vec![]
        } else if self.consume(&Token::LParen) {
            // Multi-value return: (int, string, error)
            let mut types = vec![];
            if !matches!(self.peek(), Some(Token::RParen)) {
                loop {
                    types.push(self.parse_type()?);
                    if !self.consume(&Token::Comma) {
                        break;
                    }
                }
            }
            self.expect(Token::RParen)?;
            types
        } else {
            // Single return type
            vec![self.parse_type()?]
        };

        // Parse body
        let body = self.parse_block()?;

        Ok(FuncDecl {
            receiver,
            name,
            generics,
            params,
            return_types,
            body: body.clone(),
            span: Span::with_bytes(
                start_span.start,
                body.span.end,
                self.file,
                start_span.byte_start,
                body.span.byte_end,
            ),
        })
    }

    /// Parse type declaration (enum or struct)
    fn parse_type_decl(&mut self) -> Result<TypeDecl> {
        let start_span = self.expect(Token::Type)?;

        let (name, name_span) = match self.advance() {
            Some((Token::Ident(name), span)) => (name, span),
            Some((tok, span)) => {
                return Err(SoppoError::Parse {
                    message: format!("Expected type name, found {:?}", tok),
                    span,
                });
            }
            None => {
                return Err(SoppoError::Parse {
                    message: "Expected type name".to_string(),
                    span: Span::dummy(),
                });
            }
        };

        self.validate_identifier(&name, &name_span)?;

        // Parse optional generics [T any, U any]
        let generics = self.parse_generics()?;

        // Check for 'enum', 'struct', or type alias
        let (kind, end_span) = if self.consume(&Token::Enum) {
            // Parse enum
            self.expect(Token::LBrace)?;
            // Skip terminators after opening brace
            self.skip_terminators();

            let mut variants = Vec::new();

            while !matches!(self.peek(), Some(Token::RBrace) | None) {
                let variant = self.parse_enum_variant()?;
                variants.push(variant);

                // Terminators as separator (like Go struct fields)
                self.skip_terminators();
            }

            let end_span = self.expect(Token::RBrace)?;
            (TypeKind::Enum { variants }, end_span)
        } else if self.consume(&Token::Struct) {
            // Parse struct
            self.expect(Token::LBrace)?;
            // Skip terminators after opening brace
            self.skip_terminators();

            let mut fields = Vec::new();

            while !matches!(self.peek(), Some(Token::RBrace) | None) {
                let field = self.parse_field()?;
                fields.push(field);

                // Terminators as separator (like Go struct fields)
                self.skip_terminators();
            }

            let end_span = self.expect(Token::RBrace)?;
            (TypeKind::Struct { fields }, end_span)
        } else if self.consume(&Token::Interface) {
            // Parse interface
            self.expect(Token::LBrace)?;
            // Skip terminators after opening brace
            self.skip_terminators();

            let mut methods = Vec::new();

            while !matches!(self.peek(), Some(Token::RBrace) | None) {
                let method = self.parse_interface_method()?;
                methods.push(method);

                // Terminators as separator
                self.skip_terminators();
            }

            let end_span = self.expect(Token::RBrace)?;
            (TypeKind::Interface { methods }, end_span)
        } else {
            // Type alias: type Foo = Bar or type Foo int
            let target = self.parse_type()?;
            let end_span = target.span.clone();
            (TypeKind::Alias { target }, end_span)
        };

        Ok(TypeDecl {
            name,
            generics,
            kind,
            span: Span::with_bytes(
                start_span.start,
                end_span.end,
                self.file,
                start_span.byte_start,
                end_span.byte_end,
            ),
        })
    }

    /// Parse enum variant
    fn parse_enum_variant(&mut self) -> Result<EnumVariant> {
        let (name, name_span) = match self.advance() {
            Some((Token::Ident(name), span)) => (name, span),
            Some((tok, span)) => {
                return Err(SoppoError::Parse {
                    message: format!("Expected variant name, found {:?}", tok),
                    span,
                });
            }
            None => {
                return Err(SoppoError::Parse {
                    message: "Expected variant name".to_string(),
                    span: Span::dummy(),
                });
            }
        };

        // Check for data: Single Type or Struct { fields }
        // Optional `struct` keyword before `{`
        let has_struct_keyword = self.consume(&Token::Struct);
        if has_struct_keyword || self.consume(&Token::LBrace) {
            // If we consumed `struct`, now consume the `{`
            if has_struct_keyword {
                self.expect(Token::LBrace)?;
            }
            // Skip terminators after opening brace
            self.skip_terminators();

            // Struct variant
            let mut fields = Vec::new();

            while !matches!(self.peek(), Some(Token::RBrace) | None) {
                let field = self.parse_field()?;
                fields.push(field);

                // Terminators as separator (like Go struct fields)
                self.skip_terminators();
            }

            let end_span = self.expect(Token::RBrace)?;

            Ok(EnumVariant::Struct {
                name,
                fields,
                span: Span::with_bytes(
                    name_span.start,
                    end_span.end,
                    self.file,
                    name_span.byte_start,
                    end_span.byte_end,
                ),
            })
        } else if self.is_terminator() {
            // Terminator after variant name - unit variant (like Go struct embedded field on its own line)
            Ok(EnumVariant::Unit {
                name,
                span: name_span,
            })
        } else if matches!(
            self.peek(),
            Some(Token::Ident(_)) | Some(Token::LBracket) | Some(Token::Star)
        ) {
            // Array or pointer type follows
            let ty = self.parse_type()?;

            Ok(EnumVariant::Single {
                name,
                ty,
                span: name_span,
            })
        } else {
            // Unit variant
            Ok(EnumVariant::Unit {
                name,
                span: name_span,
            })
        }
    }

    /// Parse struct field
    fn parse_field(&mut self) -> Result<Field> {
        let (name, name_span) = match self.advance() {
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

        let ty = self.parse_type()?;

        Ok(Field {
            name,
            ty,
            span: name_span,
        })
    }

    /// Parse interface method signature: MethodName(params) returns
    fn parse_interface_method(&mut self) -> Result<InterfaceMethod> {
        let (name, name_span) = match self.advance() {
            Some((Token::Ident(name), span)) => (name, span),
            Some((tok, span)) => {
                return Err(SoppoError::Parse {
                    message: format!("Expected method name, found {:?}", tok),
                    span,
                });
            }
            None => {
                return Err(SoppoError::Parse {
                    message: "Expected method name".to_string(),
                    span: Span::dummy(),
                });
            }
        };

        // Parse parameters
        self.expect(Token::LParen)?;
        let mut params = Vec::new();

        if !matches!(self.peek(), Some(Token::RParen)) {
            loop {
                params.push(self.parse_param()?);

                if !self.consume(&Token::Comma) {
                    break;
                }
            }
        }

        let end_span = self.expect(Token::RParen)?;

        // Parse optional return type(s)
        let (returns, final_span) = if matches!(
            self.peek(),
            Some(Token::Newline) | Some(Token::RBrace) | None
        ) {
            // No return type
            (vec![], end_span)
        } else if self.consume(&Token::LParen) {
            // Multi-value return: (int, string, error)
            let mut types = vec![];
            if !matches!(self.peek(), Some(Token::RParen)) {
                loop {
                    types.push(self.parse_type()?);
                    if !self.consume(&Token::Comma) {
                        break;
                    }
                }
            }
            let rparen_span = self.expect(Token::RParen)?;
            (types, rparen_span)
        } else {
            // Single return type
            let ty = self.parse_type()?;
            let ty_span = ty.span.clone();
            (vec![ty], ty_span)
        };

        Ok(InterfaceMethod {
            name,
            params,
            returns,
            span: Span::with_bytes(
                name_span.start,
                final_span.end,
                self.file,
                name_span.byte_start,
                final_span.byte_end,
            ),
        })
    }

    /// Parse top-level declaration
    pub fn parse_decl(&mut self) -> Result<Decl> {
        match self.peek() {
            Some(Token::Const) => Ok(Decl::Const(self.parse_const_decl()?)),
            Some(Token::Func) => Ok(Decl::Func(self.parse_func_decl()?)),
            Some(Token::Type) => Ok(Decl::Type(self.parse_type_decl()?)),
            Some(tok) => Err(SoppoError::Parse {
                message: format!("Expected declaration, found {:?}", tok),
                span: self.peek_span(),
            }),
            None => Err(SoppoError::Parse {
                message: "Expected declaration".to_string(),
                span: Span::dummy(),
            }),
        }
    }

    /// Parse a const declaration: const NAME = VALUE or const NAME TYPE = VALUE
    fn parse_const_decl(&mut self) -> Result<ConstDecl> {
        let start = self.expect(Token::Const)?;

        let (name, name_span) = match self.advance() {
            Some((Token::Ident(name), span)) => (name, span),
            Some((tok, span)) => {
                return Err(SoppoError::Parse {
                    message: format!("Expected identifier, found {:?}", tok),
                    span,
                });
            }
            None => {
                return Err(SoppoError::Parse {
                    message: "Expected identifier".to_string(),
                    span: self.peek_span(),
                });
            }
        };

        self.validate_identifier(&name, &name_span)?;

        // Check if next token is = (type inference) or a type name
        let ty = if self.consume(&Token::Assign) {
            // const NAME = VALUE (type inference)
            None
        } else if matches!(
            self.peek(),
            Some(Token::Ident(_)) | Some(Token::LBracket) | Some(Token::Star)
        ) {
            // const NAME TYPE = VALUE (explicit type)
            let ty = self.parse_type()?;
            self.expect(Token::Assign)?;
            Some(ty)
        } else {
            return Err(SoppoError::Parse {
                message: "Expected type or '=' in const declaration".to_string(),
                span: name_span,
            });
        };

        let value = self.parse_expr()?;

        Ok(ConstDecl {
            name,
            ty,
            value,
            span: start,
        })
    }

    /// Parse a complete file
    pub fn parse_file(&mut self) -> Result<File> {
        // Skip leading whitespace/newlines
        self.skip_terminators();

        // Parse package declaration
        let package = if self.consume(&Token::Package) {
            let name = match self.advance() {
                Some((Token::Ident(name), _)) => name,
                Some((tok, span)) => {
                    return Err(SoppoError::Parse {
                        message: format!("Expected package name, found {:?}", tok),
                        span,
                    });
                }
                None => {
                    return Err(SoppoError::Parse {
                        message: "Expected package name".to_string(),
                        span: Span::dummy(),
                    });
                }
            };
            // Skip terminators after package declaration
            self.skip_terminators();
            name
        } else {
            "main".to_string()
        };

        // Parse imports
        let mut imports = Vec::new();
        while self.consume(&Token::Import) {
            match self.advance() {
                Some((Token::String(path), span)) => {
                    imports.push(Import { path, span });
                    // Skip terminators after import
                    self.skip_terminators();
                }
                Some((tok, span)) => {
                    return Err(SoppoError::Parse {
                        message: format!("Expected import path string, found {:?}", tok),
                        span,
                    });
                }
                None => {
                    return Err(SoppoError::Parse {
                        message: "Expected import path".to_string(),
                        span: Span::dummy(),
                    });
                }
            }
        }

        let mut decls = Vec::new();

        while self.peek().is_some() {
            // Skip terminators between declarations
            self.skip_terminators();

            if self.peek().is_none() {
                break;
            }

            decls.push(self.parse_decl()?);
        }

        Ok(File {
            package,
            imports,
            decls,
        })
    }
}

impl BinOp {
    /// Returns (precedence, associativity)
    /// Higher precedence = tighter binding
    fn precedence(&self) -> (u8, Assoc) {
        match self {
            BinOp::Mul | BinOp::Div | BinOp::Mod => (6, Assoc::Left),
            BinOp::Add | BinOp::Sub => (5, Assoc::Left),
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                (4, Assoc::Left)
            }
            BinOp::And => (3, Assoc::Left),
            BinOp::Or => (2, Assoc::Left),
        }
    }
}

enum Assoc {
    Left,
}

#[cfg(test)]
mod tests {
    use super::*;

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
                assert!(matches!(args[0].kind, ExprKind::Integer(1)));
                assert!(matches!(args[1].kind, ExprKind::Integer(2)));
                assert!(type_args.is_empty());
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
    fn test_parse_match() {
        let source = r#"match x {
            case Ok(value):
                result = value
            case Err(msg):
                result = 0
        }"#;
        let mut parser = Parser::new(source, FileId(0));
        let stmt = parser.parse_stmt().unwrap();

        match stmt.kind {
            StmtKind::Match { scrutinee, arms } => {
                let scrutinee = scrutinee.unwrap();
                assert!(matches!(scrutinee.kind, ExprKind::Ident(s) if s == "x"));
                assert_eq!(arms.len(), 2);

                // First arm: Ok(value) -> result = value
                assert_eq!(arms[0].patterns.len(), 1);
                match &arms[0].patterns[0].kind {
                    PatternKind::Destructor { name, binding } => {
                        assert_eq!(name, "Ok");
                        assert_eq!(binding, "value");
                    }
                    _ => panic!("Expected destructor pattern"),
                }
                // Arm body is now a Block
                assert_eq!(arms[0].body.stmts.len(), 1);

                // Second arm: Err(msg) -> result = 0
                assert_eq!(arms[1].patterns.len(), 1);
                match &arms[1].patterns[0].kind {
                    PatternKind::Destructor { name, binding } => {
                        assert_eq!(name, "Err");
                        assert_eq!(binding, "msg");
                    }
                    _ => panic!("Expected destructor pattern"),
                }
            }
            _ => panic!("Expected match statement"),
        }
    }

    #[test]
    fn test_parse_let_stmt() {
        let source = "x := 42";
        let mut parser = Parser::new(source, FileId(0));
        let stmt = parser.parse_stmt().unwrap();

        match stmt.kind {
            StmtKind::Decl { name, value } => {
                assert_eq!(name, "x");
                assert!(matches!(value.kind, ExprKind::Integer(42)));
            }
            _ => panic!("Expected let statement"),
        }
    }

    #[test]
    fn test_parse_return_stmt() {
        let source = "return 42";
        let mut parser = Parser::new(source, FileId(0));
        let stmt = parser.parse_stmt().unwrap();

        match stmt.kind {
            StmtKind::Return { values } if values.len() == 1 => {
                assert!(matches!(values[0].kind, ExprKind::Integer(42)));
            }
            _ => panic!("Expected return statement with one value"),
        }
    }

    #[test]
    fn test_parse_block() {
        let source = "{ x := 1\ny := 2\nreturn x }";
        let mut parser = Parser::new(source, FileId(0));
        let block = parser.parse_block().unwrap();

        assert_eq!(block.stmts.len(), 3);
        assert!(matches!(block.stmts[0].kind, StmtKind::Decl { .. }));
        assert!(matches!(block.stmts[1].kind, StmtKind::Decl { .. }));
        assert!(matches!(block.stmts[2].kind, StmtKind::Return { .. }));
    }

    #[test]
    fn test_parse_function() {
        let source = "func add(x int, y int) int { return x + y }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser.parse_func_decl().unwrap();

        assert_eq!(func.name, "add");
        assert_eq!(func.params.len(), 2);
        assert_eq!(func.params[0].name, "x");
        assert_eq!(func.params[0].ty.name, "int");
        assert_eq!(func.params[1].name, "y");
        assert_eq!(func.return_types.len(), 1);
        assert_eq!(func.return_types[0].name, "int");
        assert_eq!(func.body.stmts.len(), 1);
    }

    #[test]
    fn test_parse_generic_function() {
        let source = "func identity[T any](x T) T { return x }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser.parse_func_decl().unwrap();

        assert_eq!(func.name, "identity");
        assert_eq!(func.generics.len(), 1);
        assert_eq!(func.generics[0].name, "T");
        assert_eq!(func.generics[0].constraint, "any");
        assert_eq!(func.params[0].ty.name, "T");
    }

    #[test]
    fn test_parse_enum() {
        let source = r#"type Result[T any, E any] enum {
            Ok T
            Err E
        }"#;
        let mut parser = Parser::new(source, FileId(0));
        let type_decl = parser.parse_type_decl().unwrap();

        assert_eq!(type_decl.name, "Result");
        assert_eq!(type_decl.generics.len(), 2);
        assert_eq!(type_decl.generics[0].name, "T");
        assert_eq!(type_decl.generics[0].constraint, "any");
        assert_eq!(type_decl.generics[1].name, "E");
        assert_eq!(type_decl.generics[1].constraint, "any");

        match type_decl.kind {
            TypeKind::Enum { variants } => {
                assert_eq!(variants.len(), 2);

                match &variants[0] {
                    EnumVariant::Single { name, ty, .. } => {
                        assert_eq!(name, "Ok");
                        assert_eq!(ty.name, "T");
                    }
                    _ => panic!("Expected single variant"),
                }

                match &variants[1] {
                    EnumVariant::Single { name, ty, .. } => {
                        assert_eq!(name, "Err");
                        assert_eq!(ty.name, "E");
                    }
                    _ => panic!("Expected single variant"),
                }
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    fn test_parse_complete_file() {
        let source = r#"
            type Color enum {
                Red
                Green
                Blue
            }

            func main() {
                color := Color.Red
                return color
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();

        assert_eq!(file.decls.len(), 2);
        assert!(matches!(file.decls[0], Decl::Type(_)));
        assert!(matches!(file.decls[1], Decl::Func(_)));
    }

    #[test]
    fn test_parse_semicolons_as_statement_separators() {
        // Semicolons should work the same as newlines
        let source = "{ x := 1; y := 2; return x }";
        let mut parser = Parser::new(source, FileId(0));
        let block = parser.parse_block().unwrap();

        assert_eq!(block.stmts.len(), 3);
        assert!(matches!(block.stmts[0].kind, StmtKind::Decl { .. }));
        assert!(matches!(block.stmts[1].kind, StmtKind::Decl { .. }));
        assert!(matches!(block.stmts[2].kind, StmtKind::Return { .. }));
    }

    #[test]
    fn test_parse_mixed_semicolons_and_newlines() {
        let source = "{ x := 1;\ny := 2\nz := 3; return x }";
        let mut parser = Parser::new(source, FileId(0));
        let block = parser.parse_block().unwrap();

        assert_eq!(block.stmts.len(), 4);
        assert!(matches!(block.stmts[0].kind, StmtKind::Decl { .. }));
        assert!(matches!(block.stmts[1].kind, StmtKind::Decl { .. }));
        assert!(matches!(block.stmts[2].kind, StmtKind::Decl { .. }));
        assert!(matches!(block.stmts[3].kind, StmtKind::Return { .. }));
    }

    #[test]
    fn test_parse_function_with_semicolons() {
        let source = "func add(x int, y int) int { c := x + y; return c }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser.parse_func_decl().unwrap();

        assert_eq!(func.name, "add");
        assert_eq!(func.body.stmts.len(), 2);
    }
}
