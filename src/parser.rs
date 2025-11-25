use crate::ast::*;
use crate::error::{Result, SoppoError};
use crate::lexer::{Lexer, Token};
use crate::source::{FileId, Span};

// Reserved keywords that cannot be used as identifiers
const RESERVED_KEYWORDS: &[&str] = &[
    "break",
    "default",
    "func",
    "interface",
    "select",
    "case",
    "defer",
    "go",
    "map",
    "struct",
    "chan",
    "else",
    "goto",
    "package",
    "switch",
    "const",
    "fallthrough",
    "if",
    "range",
    "type",
    "continue",
    "for",
    "import",
    "return",
    "var",
    // Soppo
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
                // Array literal: [size]type{elements}
                // Parse the size (which is an expression)
                let _size = self.parse_expr()?;
                self.expect(Token::RBracket)?;

                // Parse the element type
                let ty = self.parse_type()?;

                // Expect opening brace for the composite literal
                self.expect(Token::LBrace)?;

                // Parse array elements
                let mut elements = Vec::new();
                if !matches!(self.peek(), Some(Token::RBrace)) {
                    loop {
                        elements.push(self.parse_expr()?);

                        if !self.consume(&Token::Comma) {
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

            _ => Err(SoppoError::Parse {
                message: format!("Unexpected token: {:?}", tok),
                span,
            }),
        }
    }

    /// Parse match statement
    fn parse_match_stmt(&mut self, start_span: Span) -> Result<Stmt> {
        let scrutinee = self.parse_expr()?;
        self.expect(Token::LBrace)?;
        // Skip terminators after opening brace
        self.skip_terminators();

        let mut arms = Vec::new();

        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let arm = self.parse_match_arm()?;
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

    /// Parse a match arm: case pattern: statements (until next case/default/})
    fn parse_match_arm(&mut self) -> Result<Arm> {
        // Handle both 'case Pattern:' and 'default:'
        let pattern = if let Some(Token::Ident(s)) = self.peek() {
            if s == "default" {
                let span = self.peek_span();
                self.advance(); // consume 'default'
                Pattern {
                    kind: PatternKind::Default,
                    span,
                }
            } else {
                self.expect(Token::Case)?;
                self.parse_pattern()?
            }
        } else {
            self.expect(Token::Case)?;
            self.parse_pattern()?
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

        let span = Span::with_bytes(
            pattern.span.start,
            body.span.end,
            self.file,
            pattern.span.byte_start,
            body.span.byte_end,
        );

        Ok(Arm {
            pattern,
            body,
            span,
        })
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
                kind: PatternKind::Literal(crate::ast::Literal::Integer(n)),
                span,
            }),

            Token::String(s) => Ok(Pattern {
                kind: PatternKind::Literal(crate::ast::Literal::String(s)),
                span,
            }),

            Token::True => Ok(Pattern {
                kind: PatternKind::Literal(crate::ast::Literal::Bool(true)),
                span,
            }),

            Token::False => Ok(Pattern {
                kind: PatternKind::Literal(crate::ast::Literal::Bool(false)),
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
                // Function call: expr(args)
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
                        },
                    };
                }

                // Array indexing: expr[index]
                Some(Token::LBracket) => {
                    self.advance();
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

                // Struct literal: Type{field: value, ...}
                Some(Token::LBrace) => {
                    // Only parse as struct literal if expr is an identifier (type name)
                    // AND the content looks like field initialization (not a block)
                    if let ExprKind::Ident(type_name) = &expr.kind {
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
                                    name: type_name.clone(),
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
                let target = self.parse_expr()?;

                if self.consume(&Token::ColonAssign) {
                    // Short variable declaration: x := value
                    // target must be a simple identifier
                    if let ExprKind::Ident(name) = target.kind {
                        self.validate_identifier(&name, &target.span)?;
                        let value = self.parse_expr()?;
                        Ok(Stmt {
                            span: Span::with_bytes(
                                target.span.start,
                                value.span.end,
                                self.file,
                                target.span.byte_start,
                                value.span.byte_end,
                            ),
                            kind: StmtKind::Decl { name, value },
                        })
                    } else {
                        Err(SoppoError::Parse {
                            message: "Left side of := must be a simple identifier".to_string(),
                            span: target.span,
                        })
                    }
                } else if self.consume(&Token::Assign) {
                    // Assignment: x = value or x.y = value
                    let value = self.parse_expr()?;
                    Ok(Stmt {
                        span: Span::with_bytes(
                            target.span.start,
                            value.span.end,
                            self.file,
                            target.span.byte_start,
                            value.span.byte_end,
                        ),
                        kind: StmtKind::Assign { target, value },
                    })
                } else {
                    // Just an expression statement
                    Ok(Stmt {
                        span: target.span.clone(),
                        kind: StmtKind::Expr(target),
                    })
                }
            }

            Some(Token::Var) => {
                // var name type or var name type = value
                self.advance(); // consume 'var'

                // Parse the variable name
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

                // Parse the type
                let ty = self.parse_type()?;

                // Check for optional initializer
                let (value, end_span) = if self.consume(&Token::Assign) {
                    let expr = self.parse_expr()?;
                    let span = expr.span.clone();
                    (Some(expr), span)
                } else {
                    (None, ty.span.clone())
                };

                Ok(Stmt {
                    span: Span::with_bytes(
                        start_span.start,
                        end_span.end,
                        self.file,
                        start_span.byte_start,
                        end_span.byte_end,
                    ),
                    kind: StmtKind::VarDecl { name, ty, value },
                })
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
                let value = if matches!(self.peek(), Some(Token::RBrace) | None) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };

                let end_span = value
                    .as_ref()
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
                    kind: StmtKind::Return { value },
                })
            }

            Some(Token::Match) => {
                self.advance();
                self.parse_match_stmt(start_span)
            }

            _ => {
                let expr = self.parse_expr()?;
                Ok(Stmt {
                    span: expr.span.clone(),
                    kind: StmtKind::Expr(expr),
                })
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
    fn parse_type(&mut self) -> Result<Type> {
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

        // Parse optional return type (Go-style: comes after params without arrow)
        let return_type = if !matches!(self.peek(), Some(Token::LBrace)) {
            Some(self.parse_type()?)
        } else {
            None
        };

        // Parse body
        let body = self.parse_block()?;

        Ok(FuncDecl {
            receiver,
            name,
            generics,
            params,
            return_type,
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
        if self.consume(&Token::LBrace) {
            // Struct variant
            let mut fields = Vec::new();

            if !matches!(self.peek(), Some(Token::RBrace)) {
                loop {
                    let field = self.parse_field()?;
                    fields.push(field);

                    if !self.consume(&Token::Comma) {
                        break;
                    }
                }
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

    /// Parse a const declaration: const NAME TYPE = VALUE
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

        let ty = self.parse_type()?;

        self.expect(Token::Assign)?;

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
            ExprKind::Call { func, args } => {
                assert!(matches!(func.kind, ExprKind::Ident(s) if s == "foo"));
                assert_eq!(args.len(), 2);
                assert!(matches!(args[0].kind, ExprKind::Integer(1)));
                assert!(matches!(args[1].kind, ExprKind::Integer(2)));
            }
            _ => panic!("Expected call expression"),
        }
    }

    #[test]
    fn test_parse_field_access() {
        let expr = parse_expr_helper("foo.bar").unwrap();
        match expr.kind {
            ExprKind::Field { expr, field } => {
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
                assert!(matches!(scrutinee.kind, ExprKind::Ident(s) if s == "x"));
                assert_eq!(arms.len(), 2);

                // First arm: Ok(value) -> result = value
                match &arms[0].pattern.kind {
                    PatternKind::Destructor { name, binding } => {
                        assert_eq!(name, "Ok");
                        assert_eq!(binding, "value");
                    }
                    _ => panic!("Expected destructor pattern"),
                }
                // Arm body is now a Block
                assert_eq!(arms[0].body.stmts.len(), 1);

                // Second arm: Err(msg) -> result = 0
                match &arms[1].pattern.kind {
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
            StmtKind::Return { value: Some(value) } => {
                assert!(matches!(value.kind, ExprKind::Integer(42)));
            }
            _ => panic!("Expected return statement"),
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
        assert!(func.return_type.is_some());
        assert_eq!(func.return_type.unwrap().name, "int");
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
