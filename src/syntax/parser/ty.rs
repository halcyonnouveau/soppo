use super::Parser;
use crate::error::{Result, SoppoError};
use crate::syntax::ast::Type;
use crate::syntax::lexer::Token;
use crate::syntax::source::Span;

impl Parser {
    /// Parse a type annotation
    /// Supports: T, []T, [N]T, *T, map[K]V, chan T, T[A, B], ...T (variadic)
    /// Also supports nullable prefix: ?*T, ?[]T, ?Interface
    pub(super) fn parse_type(&mut self) -> Result<Type> {
        let start_span = self.peek_span();

        // Check for nullable prefix: ?
        let nullable = self.consume(&Token::Question);

        // Variadic type: ...T (cannot be nullable)
        if self.consume(&Token::DotDotDot) {
            if nullable {
                return Err(SoppoError::Parse {
                    message: "Variadic types cannot be nullable".to_string(),
                    span: start_span,
                });
            }
            let elem_ty = self.parse_type()?;
            return Ok(Type {
                name: format!("...{}", elem_ty.name),
                args: vec![elem_ty],
                span: start_span,
                nullable: false,
            });
        }

        // Slice type: []T or ?[]T
        if self.consume(&Token::LBracket) {
            if self.consume(&Token::RBracket) {
                // []T - slice
                let elem_ty = self.parse_type()?;
                return Ok(Type {
                    name: format!("[]{}", elem_ty.name),
                    args: vec![elem_ty],
                    span: start_span,
                    nullable,
                });
            } else {
                // [N]T - array (consume the size, we don't validate it)
                // Arrays cannot be nullable (only slices can)
                if nullable {
                    return Err(SoppoError::Parse {
                        message: "Array types cannot be nullable, only slices can".to_string(),
                        span: start_span,
                    });
                }
                while !matches!(self.peek(), Some(Token::RBracket) | None) {
                    self.advance();
                }
                self.expect(Token::RBracket)?;
                let elem_ty = self.parse_type()?;
                return Ok(Type {
                    name: format!("[]{}", elem_ty.name), // Treat arrays as slices for simplicity
                    args: vec![elem_ty],
                    span: start_span,
                    nullable: false,
                });
            }
        }

        // Pointer type: *T or ?*T
        if self.consume(&Token::Star) {
            let pointee_ty = self.parse_type()?;
            return Ok(Type {
                name: format!("*{}", pointee_ty.name),
                args: vec![pointee_ty],
                span: start_span,
                nullable,
            });
        }

        // Anonymous struct type: struct { fields } - cannot be nullable on its own
        if self.consume(&Token::Struct) {
            if nullable {
                return Err(SoppoError::Parse {
                    message: "Struct types cannot be nullable directly, use a pointer (*struct{...}) with ? prefix".to_string(),
                    span: start_span,
                });
            }
            self.expect(Token::LBrace)?;
            self.skip_terminators();

            let mut field_strs = Vec::new();
            while !matches!(self.peek(), Some(Token::RBrace) | None) {
                // Parse field: name type or grouped names: name1, name2 type
                let parsed_fields = self.parse_fields()?;
                for field in parsed_fields {
                    field_strs.push(format!("{} {}", field.name, field.ty.name));
                }

                // Skip terminators between fields
                self.skip_terminators();
            }
            self.expect(Token::RBrace)?;

            let struct_name = format!("struct {{ {} }}", field_strs.join("; "));
            return Ok(Type {
                name: struct_name,
                args: vec![],
                span: start_span,
                nullable: false,
            });
        }

        // Function type: func(params) returns - can be nullable
        if self.consume(&Token::Func) {
            // Parse parameter types
            self.expect(Token::LParen)?;
            let mut param_types = Vec::new();
            if !matches!(self.peek(), Some(Token::RParen)) {
                loop {
                    let param_ty = self.parse_type()?;
                    param_types.push(param_ty);
                    if !self.consume(&Token::Comma) {
                        break;
                    }
                }
            }
            self.expect(Token::RParen)?;

            // Parse return type(s)
            let mut return_types = Vec::new();
            if self.consume(&Token::LParen) {
                // Multiple return types: func(A) (B, C)
                if !matches!(self.peek(), Some(Token::RParen)) {
                    loop {
                        let ret_ty = self.parse_type()?;
                        return_types.push(ret_ty);
                        if !self.consume(&Token::Comma) {
                            break;
                        }
                    }
                }
                self.expect(Token::RParen)?;
            } else if !matches!(
                self.peek(),
                Some(Token::Comma)
                    | Some(Token::RParen)
                    | Some(Token::RBrace)
                    | Some(Token::Semicolon)
                    | None
            ) {
                // Single return type: func(A) B
                let ret_ty = self.parse_type()?;
                return_types.push(ret_ty);
            }

            // Build function type name: func(A, B) C
            let params_str = param_types
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let returns_str = if return_types.is_empty() {
                String::new()
            } else if return_types.len() == 1 {
                format!(" {}", return_types[0].name)
            } else {
                let rets = return_types
                    .iter()
                    .map(|t| t.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(" ({})", rets)
            };

            let mut all_types = param_types;
            all_types.extend(return_types);

            return Ok(Type {
                name: format!("func({}){}", params_str, returns_str),
                args: all_types,
                span: start_span,
                nullable,
            });
        }

        // Now we need an identifier
        let (mut name, mut span) = match self.advance() {
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

        // Handle qualified types like Option.Some (for enum variants in type assertions)
        while self.consume(&Token::Dot) {
            let (field_name, field_span) = match self.advance() {
                Some((Token::Ident(field), field_span)) => (field, field_span),
                Some((tok, span)) => {
                    return Err(SoppoError::Parse {
                        message: format!("Expected identifier after '.', found {:?}", tok),
                        span,
                    });
                }
                None => {
                    return Err(SoppoError::Parse {
                        message: "Expected identifier after '.'".to_string(),
                        span: Span::dummy(),
                    });
                }
            };
            name = format!("{}.{}", name, field_name);
            span = Span::with_bytes(
                span.start,
                field_span.end,
                self.file,
                span.byte_start,
                field_span.byte_end,
            );
        }

        // Map type: map[K]V - can be nullable
        if name == "map" {
            self.expect(Token::LBracket)?;
            let key_ty = self.parse_type()?;
            self.expect(Token::RBracket)?;
            let val_ty = self.parse_type()?;
            return Ok(Type {
                name: format!("map[{}]{}", key_ty.name, val_ty.name),
                args: vec![key_ty, val_ty],
                span,
                nullable,
            });
        }

        // Channel type: chan T - can be nullable
        if name == "chan" {
            let elem_ty = self.parse_type()?;
            return Ok(Type {
                name: format!("chan {}", elem_ty.name),
                args: vec![elem_ty],
                span,
                nullable,
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

        // For simple types (not pointers, slices, maps, channels), validate nullable usage
        // Only interface types and func types can be nullable as simple names
        // We'll validate this in the type checker since we don't know if a name is an interface here
        // For now, allow ? on any named type (type checker will validate)

        Ok(Type {
            name,
            args,
            span,
            nullable,
        })
    }
}
