use super::Parser;
use crate::error::{Result, SoppoError};
use crate::syntax::ast::Type;
use crate::syntax::lexer::Token;
use crate::syntax::source::Span;

impl Parser {
    /// Parse a type annotation
    /// Supports: T, []T, [N]T, *T, map[K]V, chan T, T[A, B]
    pub(super) fn parse_type(&mut self) -> Result<Type> {
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
}
