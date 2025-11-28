use super::Parser;
use crate::error::{Result, SoppoError};
use crate::parse::ast::{
    ConstDecl, Decl, EnumVariant, Field, File, FuncDecl, Generic, Import, InterfaceMethod, Param,
    TypeDecl, TypeKind,
};
use crate::parse::lexer::Token;
use crate::parse::source::Span;

impl Parser {
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
    pub(super) fn parse_type_decl(&mut self) -> Result<TypeDecl> {
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
            comments: std::mem::take(&mut self.comments),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::FileId;

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
    fn test_parse_function_with_semicolons() {
        let source = "func add(x int, y int) int { c := x + y; return c }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser.parse_func_decl().unwrap();

        assert_eq!(func.name, "add");
        assert_eq!(func.body.stmts.len(), 2);
    }
}
