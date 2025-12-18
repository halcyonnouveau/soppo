use super::Parser;
use crate::error::{Result, SoppoError};
use crate::syntax::ast::{
    Arm, Block, FieldPattern, Ident, Literal, Pattern, PatternKind, Stmt, StmtKind,
};
use crate::syntax::lexer::Token;
use crate::syntax::source::Span;

impl Parser {
    /// Parse match statement
    pub(super) fn parse_match_stmt(&mut self, start_span: Span) -> Result<Stmt> {
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

        let body_end = stmts.last().map(|s| s.span).unwrap_or(body_start);

        let body = Block {
            stmts,
            span: body_end,
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
            let span = expr.span;
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
            Token::Integer(lit) => Ok(Pattern {
                kind: PatternKind::Literal(super::super::ast::Literal::Integer(
                    lit.value, lit.format,
                )),
                span,
            }),

            Token::String(s) => Ok(Pattern {
                kind: PatternKind::Literal(super::super::ast::Literal::String(s)),
                span,
            }),

            Token::True => Ok(Pattern {
                kind: PatternKind::Literal(super::super::ast::Literal::Bool(true)),
                span,
            }),

            Token::False => Ok(Pattern {
                kind: PatternKind::Literal(super::super::ast::Literal::Bool(false)),
                span,
            }),

            Token::Nil => Ok(Pattern {
                kind: PatternKind::Literal(super::super::ast::Literal::Nil),
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
                    let (binding_name, binding_span) = match self.advance() {
                        Some((Token::Ident(binding), span)) => (binding, span),
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
                        kind: PatternKind::Destructor {
                            name,
                            binding: Ident::new(binding_name, binding_span),
                        },
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

                            // Parse field_name: pattern or just field_name (shorthand)
                            let (field_name, field_span) = match self.advance() {
                                Some((Token::Ident(name), span)) => (name, span),
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

                            // Check for colon - if not present, use shorthand (field binds to same name)
                            let field_pattern = if self.consume(&Token::Colon) {
                                // After colon: either a literal or a binding name
                                match self.peek() {
                                    Some(Token::Integer(lit)) => {
                                        let lit = *lit;
                                        self.advance();
                                        FieldPattern::Literal(Literal::Integer(
                                            lit.value, lit.format,
                                        ))
                                    }
                                    Some(Token::String(s)) => {
                                        let s = s.clone();
                                        self.advance();
                                        FieldPattern::Literal(Literal::String(s))
                                    }
                                    Some(Token::True) => {
                                        self.advance();
                                        FieldPattern::Literal(Literal::Bool(true))
                                    }
                                    Some(Token::False) => {
                                        self.advance();
                                        FieldPattern::Literal(Literal::Bool(false))
                                    }
                                    Some(Token::Ident(name)) => {
                                        let name = name.clone();
                                        let span = self.peek_span();
                                        self.advance();
                                        FieldPattern::Bind(Ident::new(name, span))
                                    }
                                    Some(tok) => {
                                        let tok = tok.clone();
                                        return Err(SoppoError::Parse {
                                            message: format!(
                                                "Expected binding name or literal in struct pattern, found {:?}",
                                                tok
                                            ),
                                            span: self.peek_span(),
                                        });
                                    }
                                    None => {
                                        return Err(SoppoError::Parse {
                                            message:
                                                "Expected binding name or literal in struct pattern"
                                                    .to_string(),
                                            span: Span::dummy(),
                                        });
                                    }
                                }
                            } else {
                                // Shorthand: `field` is same as `field: field`
                                FieldPattern::Bind(Ident::new(field_name.clone(), field_span))
                            };

                            fields.push((field_name, field_pattern));

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
                        kind: PatternKind::Variant(name, std::cell::Cell::new(true)),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::ast::ExprKind;
    use crate::syntax::source::FileId;

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
    fn test_parse_match_nil_pattern() {
        let source = r#"match ptr {
            case nil:
                result = "nil"
            default:
                result = "not nil"
        }"#;
        let mut parser = Parser::new(source, FileId(0));
        let stmt = parser.parse_stmt().unwrap();

        match stmt.kind {
            StmtKind::Match { scrutinee, arms } => {
                let scrutinee = scrutinee.unwrap();
                assert!(matches!(scrutinee.kind, ExprKind::Ident(s) if s == "ptr"));
                assert_eq!(arms.len(), 2);

                // First arm: nil
                assert_eq!(arms[0].patterns.len(), 1);
                match &arms[0].patterns[0].kind {
                    PatternKind::Literal(crate::syntax::ast::Literal::Nil) => {}
                    _ => panic!("Expected nil pattern"),
                }
            }
            _ => panic!("Expected match statement"),
        }
    }
}
