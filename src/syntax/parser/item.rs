use super::Parser;
use crate::error::{Result, SoppoError};
use crate::syntax::ast::{
    ConstDecl, Decl, EnumVariant, Expr, ExprKind, Field, File, FuncDecl, Generic, Ident, Import,
    InterfaceMethod, Param, TypeAnnotation, TypeDecl, TypeKind, VarDecl,
};
use crate::syntax::lexer::Token;
use crate::syntax::source::Span;

impl Parser {
    /// Parse function parameter
    fn parse_param(&mut self) -> Result<Param> {
        // Go syntax: name Type (no colon)
        let (name, name_span) = self.parse_identifier("parameter")?;
        self.validate_identifier(&name, &name_span)?;

        let ty = self.parse_type()?;

        Ok(Param {
            ident: Ident::new(name, name_span),
            ty,
        })
    }

    /// Parse function parameter list with support for grouped parameters: (a, b int, c string)
    fn parse_param_list(&mut self) -> Result<Vec<Param>> {
        let mut params = Vec::new();

        if matches!(self.peek(), Some(Token::RParen)) {
            return Ok(params);
        }

        let mut pending_names: Vec<Ident> = Vec::new();

        loop {
            // Parse name
            let (name, name_span) = self.parse_identifier("parameter")?;
            self.validate_identifier(&name, &name_span)?;
            pending_names.push(Ident::new(name, name_span));

            if self.consume(&Token::Comma) {
                // Could be more names in the group, or end of a param group
                // Continue to parse next name
                continue;
            }

            // No comma - parse the type for all pending names
            let ty = self.parse_type()?;

            // Assign type to all pending names
            for name in pending_names.drain(..) {
                params.push(Param {
                    ident: name,
                    ty: ty.clone(),
                });
            }

            // Check if there's another parameter group
            if self.consume(&Token::Comma) {
                continue;
            }

            break;
        }

        Ok(params)
    }

    /// Parse function return list: (x, y int, err error) or (int, string) or int
    /// Returns Vec<Param> where unnamed returns have empty ident name.
    pub(super) fn parse_return_list(&mut self) -> Result<Vec<Param>> {
        if matches!(self.peek(), Some(Token::LBrace)) {
            // No return type
            return Ok(vec![]);
        }

        if self.consume(&Token::LParen) {
            // Multi-value return - could be named or unnamed
            if matches!(self.peek(), Some(Token::RParen)) {
                self.advance(); // consume )
                return Ok(vec![]);
            }

            // Try to determine if named or unnamed by looking at pattern
            // Collect identifiers and look for type position
            let returns = self.parse_return_params()?;
            self.expect(Token::RParen)?;
            return Ok(returns);
        }

        // Single return type (unnamed)
        let ty = self.parse_type()?;
        Ok(vec![Param {
            ident: Ident::new("", ty.span),
            ty,
        }])
    }

    /// Parse return parameters, handling both named (x int) and unnamed (int) returns.
    /// In Go, if any return is named, all must be named.
    fn parse_return_params(&mut self) -> Result<Vec<Param>> {
        let mut params = Vec::new();
        let mut pending_names: Vec<Ident> = Vec::new();
        let mut is_named: Option<bool> = None;

        loop {
            // First, try to determine if this is named or unnamed
            // by looking at current token and what follows

            let (first_name, first_span) = match self.peek() {
                Some(Token::Ident(name)) if name == "map" || name == "chan" => {
                    // map and chan are always types, never parameter names
                    if is_named == Some(true) {
                        return Err(SoppoError::Parse {
                            message: "Mixed named and unnamed returns".to_string(),
                            span: self.peek_span(),
                        });
                    }
                    is_named = Some(false);
                    let ty = self.parse_type()?;
                    params.push(Param {
                        ident: Ident::new("", ty.span),
                        ty,
                    });
                    if !self.consume(&Token::Comma) {
                        break;
                    }
                    continue;
                }
                Some(Token::Ident(_)) => {
                    let (Token::Ident(name), span) = self.advance().unwrap() else {
                        unreachable!()
                    };
                    (name, span)
                }
                Some(
                    Token::Star
                    | Token::LBracket
                    | Token::Func
                    | Token::Question
                    | Token::Struct
                    | Token::Interface,
                ) => {
                    // Definitely a type (pointer, slice, func, nullable, struct, interface)
                    if is_named == Some(true) {
                        return Err(SoppoError::Parse {
                            message: "Mixed named and unnamed returns".to_string(),
                            span: self.peek_span(),
                        });
                    }
                    is_named = Some(false);
                    let ty = self.parse_type()?;
                    params.push(Param {
                        ident: Ident::new("", ty.span),
                        ty,
                    });
                    if !self.consume(&Token::Comma) {
                        break;
                    }
                    continue;
                }
                Some(tok) => {
                    return Err(SoppoError::Parse {
                        message: format!("Expected return type or name, found {:?}", tok),
                        span: self.peek_span(),
                    });
                }
                None => {
                    return Err(SoppoError::Parse {
                        message: "Unexpected end of return list".to_string(),
                        span: Span::dummy(),
                    });
                }
            };

            // We have an identifier. Now look at what follows to determine if it's a name or type.
            match self.peek() {
                Some(Token::Comma) => {
                    // Could be unnamed type list or start of grouped names
                    // Collect as potential name for now
                    pending_names.push(Ident::new(first_name, first_span));
                    self.advance(); // consume comma
                    continue;
                }
                Some(Token::RParen) => {
                    // End of list - identifier was a type
                    if is_named == Some(true) {
                        return Err(SoppoError::Parse {
                            message: "Mixed named and unnamed returns".to_string(),
                            span: first_span,
                        });
                    }
                    // All pending names were actually types
                    for name in pending_names.drain(..) {
                        params.push(Param {
                            ident: Ident::new("", name.span),
                            ty: TypeAnnotation {
                                name: name.name,
                                args: vec![],
                                span: name.span,
                                nullable: false,
                            },
                        });
                    }
                    // Current identifier is also a type
                    params.push(Param {
                        ident: Ident::new("", first_span),
                        ty: TypeAnnotation {
                            name: first_name,
                            args: vec![],
                            span: first_span,
                            nullable: false,
                        },
                    });
                    break;
                }
                _ => {
                    // Something else follows - this is a named return
                    // The identifier is a name, parse the type
                    if is_named == Some(false) {
                        return Err(SoppoError::Parse {
                            message: "Mixed named and unnamed returns".to_string(),
                            span: first_span,
                        });
                    }
                    is_named = Some(true);
                    pending_names.push(Ident::new(first_name, first_span));

                    // Parse the type for all pending names
                    let ty = self.parse_type()?;
                    for name in pending_names.drain(..) {
                        params.push(Param {
                            ident: name,
                            ty: ty.clone(),
                        });
                    }

                    if !self.consume(&Token::Comma) {
                        break;
                    }
                }
            }
        }

        Ok(params)
    }

    /// Parse generic parameters: [T any, E comparable]
    fn parse_generics(&mut self) -> Result<Vec<Generic>> {
        if !self.consume(&Token::LBracket) {
            return Ok(Vec::new());
        }

        let mut generics = Vec::new();

        if !matches!(self.peek(), Some(Token::RBracket)) {
            loop {
                let (name, span) = self.parse_identifier("generic parameter")?;

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
                    ident: Ident::new(name, span),
                    constraint,
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
        let doc_comment = self.get_doc_comment(start_span.start.line);

        // Check for receiver: func (r: Type) name() or func name()
        let receiver = if self.consume(&Token::LParen) {
            let param = self.parse_param()?;
            self.expect(Token::RParen)?;
            Some(param)
        } else {
            None
        };

        let (name, name_span) = self.parse_identifier("function")?;
        self.validate_identifier(&name, &name_span)?;

        // Parse optional generics [T any, U any]
        let generics = self.parse_generics()?;

        // Parse parameters (supports grouped params: a, b int)
        self.expect(Token::LParen)?;
        let params = self.parse_param_list()?;
        self.expect(Token::RParen)?;

        // Parse optional return type(s) - supports named (x int, y string) and unnamed (int, string)
        let returns = self.parse_return_list()?;

        // Parse body
        let body = self.parse_block()?;

        Ok(FuncDecl {
            receiver,
            ident: Ident::new(name, name_span),
            generics,
            params,
            returns,
            body: body.clone(),
            span: self.merge_spans(start_span, body.span),
            doc_comment,
        })
    }

    /// Parse type declaration (enum or struct)
    pub(super) fn parse_type_decl(&mut self) -> Result<TypeDecl> {
        let start_span = self.expect(Token::Type)?;
        let doc_comment = self.get_doc_comment(start_span.start.line);

        let (name, name_span) = self.parse_identifier("type")?;
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
                let parsed_fields = self.parse_fields()?;
                fields.extend(parsed_fields);

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
        } else if self.consume(&Token::Assign) {
            // Type alias: type Foo = Bar (Foo is exactly Bar)
            let target = self.parse_type()?;
            let end_span = target.span;
            (TypeKind::Alias { target }, end_span)
        } else {
            // Type definition: type Foo Bar (Foo is a new distinct type based on Bar)
            let target = self.parse_type()?;
            let end_span = target.span;
            (TypeKind::Definition { target }, end_span)
        };

        Ok(TypeDecl {
            ident: Ident::new(name, name_span),
            generics,
            kind,
            span: self.merge_spans(start_span, end_span),
            doc_comment,
        })
    }

    /// Parse enum variant
    fn parse_enum_variant(&mut self) -> Result<EnumVariant> {
        let (name, name_span) = self.parse_identifier("variant")?;

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
                let parsed_fields = self.parse_fields()?;
                fields.extend(parsed_fields);

                // Terminators as separator (like Go struct fields)
                self.skip_terminators();
            }

            let _ = self.expect(Token::RBrace)?;

            Ok(EnumVariant::Struct {
                ident: Ident::new(name, name_span),
                fields,
            })
        } else if self.is_terminator() {
            // Terminator after variant name - unit variant (like Go struct embedded field on its own line)
            Ok(EnumVariant::Unit {
                ident: Ident::new(name, name_span),
            })
        } else if matches!(
            self.peek(),
            Some(Token::Ident(_)) | Some(Token::LBracket) | Some(Token::Star)
        ) {
            // Array or pointer type follows
            let ty = self.parse_type()?;

            Ok(EnumVariant::Single {
                ident: Ident::new(name, name_span),
                ty,
            })
        } else {
            // Unit variant
            Ok(EnumVariant::Unit {
                ident: Ident::new(name, name_span),
            })
        }
    }

    /// Parse struct fields (supports grouped names like `X, Y int`)
    /// Returns a Vec because `X, Y int` produces multiple Field entries
    pub fn parse_fields(&mut self) -> Result<Vec<Field>> {
        // Collect all field names (comma-separated)
        let mut names: Vec<Ident> = Vec::new();

        // Parse first name
        let (first_name, first_span) = self.parse_identifier("field")?;
        names.push(Ident::new(first_name, first_span));

        // Parse additional comma-separated names
        while self.consume(&Token::Comma) {
            let (name, span) = self.parse_identifier("field")?;
            names.push(Ident::new(name, span));
        }

        // Parse the type (shared by all names)
        let ty = self.parse_type()?;

        // Parse optional struct tag (backtick string) - only applies to last field in Go
        // but we'll apply it to all for simplicity
        let tag = match self.peek() {
            Some(Token::RawString(_)) => {
                if let Some((Token::RawString(s), _)) = self.advance() {
                    Some(s)
                } else {
                    None
                }
            }
            _ => None,
        };

        // Create a Field for each name
        let fields = names
            .into_iter()
            .map(|name| Field {
                ident: name,
                ty: ty.clone(),
                tag: tag.clone(),
            })
            .collect();

        Ok(fields)
    }

    /// Parse interface method signature: MethodName(params) returns
    fn parse_interface_method(&mut self) -> Result<InterfaceMethod> {
        let (name, name_span) = self.parse_identifier("method")?;

        // Parse parameters (supports grouped params: a, b int)
        self.expect(Token::LParen)?;
        let params = self.parse_param_list()?;
        let end_span = self.expect(Token::RParen)?;

        // Parse optional return type(s)
        let (returns, _final_span) = if matches!(
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
            let ty_span = ty.span;
            (vec![ty], ty_span)
        };

        Ok(InterfaceMethod {
            ident: Ident::new(name, name_span),
            params,
            returns,
        })
    }

    /// Parse top-level declaration
    pub fn parse_decl(&mut self) -> Result<Decl> {
        match self.peek() {
            Some(Token::Const) => Ok(Decl::Const(self.parse_const_decl()?)),
            Some(Token::Var) => Ok(Decl::Var(self.parse_var_decl()?)),
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

    /// Parse a var declaration: var NAME = VALUE or var NAME TYPE = VALUE or var NAME TYPE
    fn parse_var_decl(&mut self) -> Result<VarDecl> {
        let start = self.expect(Token::Var)?;

        let (name, name_span) = self.parse_identifier("variable")?;
        self.validate_identifier(&name, &name_span)?;

        // Check if next token is = (type inference) or a type name
        let (ty, value) = if self.consume(&Token::Assign) {
            // var NAME = VALUE (type inference)
            let value = self.parse_expr()?;
            (None, Some(value))
        } else if matches!(
            self.peek(),
            Some(Token::Ident(_)) | Some(Token::LBracket) | Some(Token::Star)
        ) {
            // var NAME TYPE or var NAME TYPE = VALUE
            let ty = self.parse_type()?;
            if self.consume(&Token::Assign) {
                let value = self.parse_expr()?;
                (Some(ty), Some(value))
            } else {
                // var NAME TYPE (zero value)
                (Some(ty), None)
            }
        } else {
            return Err(SoppoError::Parse {
                message: "Expected type or '=' in var declaration".to_string(),
                span: name_span,
            });
        };

        let end_span = value
            .as_ref()
            .map(|v| v.span)
            .or(ty.as_ref().map(|t| t.span))
            .unwrap_or(name_span);

        Ok(VarDecl {
            ident: Ident::new(name, name_span),
            ty,
            value,
            span: self.merge_spans(start, end_span),
        })
    }

    /// Parse a const declaration: const NAME = VALUE or const NAME TYPE = VALUE
    fn parse_const_decl(&mut self) -> Result<ConstDecl> {
        let start = self.expect(Token::Const)?;
        let doc_comment = self.get_doc_comment(start.start.line);

        let (name, name_span) = self.parse_identifier("constant")?;
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
            ident: Ident::new(name, name_span),
            ty,
            span: self.merge_spans(start, value.span),
            value,
            doc_comment,
        })
    }

    /// Parse a const declaration after the 'const' keyword has been consumed
    /// doc_comment_line is the line number where 'const' appeared (for doc comment lookup)
    fn parse_const_after_keyword(&mut self, doc_comment_line: usize) -> Result<ConstDecl> {
        let start = self.peek_span();
        let doc_comment = self.get_doc_comment(doc_comment_line);

        let (name, name_span) = self.parse_identifier("constant")?;
        self.validate_identifier(&name, &name_span)?;

        // Check if next token is = (type inference) or a type name
        let ty = if self.consume(&Token::Assign) {
            None
        } else if matches!(
            self.peek(),
            Some(Token::Ident(_)) | Some(Token::LBracket) | Some(Token::Star)
        ) {
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
            ident: Ident::new(name, name_span),
            ty,
            span: self.merge_spans(start, value.span),
            value,
            doc_comment,
        })
    }

    /// Parse a const declaration inside a grouped const block
    /// Supports: NAME = VALUE, NAME TYPE = VALUE, NAME (implicit iota continuation)
    fn parse_const_in_group(&mut self) -> Result<ConstDecl> {
        let start = self.peek_span();

        let (name, name_span) = self.parse_identifier("constant")?;
        self.validate_identifier(&name, &name_span)?;

        // In grouped const, we might have:
        // 1. NAME = VALUE
        // 2. NAME TYPE = VALUE
        // 3. NAME (implicit, uses previous iota value - for Go compatibility)
        let (ty, value) = if self.consume(&Token::Assign) {
            // NAME = VALUE
            let value = self.parse_expr()?;
            (None, value)
        } else if matches!(
            self.peek(),
            Some(Token::Ident(_)) | Some(Token::LBracket) | Some(Token::Star)
        ) {
            // Could be NAME TYPE = VALUE or NAME (implicit)
            // Need to check if after type there's an =
            let ty = self.parse_type()?;
            if self.consume(&Token::Assign) {
                let value = self.parse_expr()?;
                (Some(ty), value)
            } else {
                // Implicit iota continuation - generate iota as value
                // For now, we require explicit values
                return Err(SoppoError::Parse {
                    message: "Expected '=' after type in const declaration".to_string(),
                    span: name_span,
                });
            }
        } else if matches!(
            self.peek(),
            Some(&Token::Newline) | Some(&Token::RParen) | None
        ) {
            // Implicit iota continuation (NAME on its own line)
            // Generate an iota expression - Go will handle the incrementing
            let iota_expr = Expr::new(ExprKind::Ident("iota".to_string()), name_span);
            (None, iota_expr)
        } else {
            return Err(SoppoError::Parse {
                message: "Expected type, '=', or newline in const declaration".to_string(),
                span: name_span,
            });
        };

        Ok(ConstDecl {
            ident: Ident::new(name, name_span),
            ty,
            span: self.merge_spans(start, value.span),
            value,
            doc_comment: None, // Grouped consts don't have individual doc comments
        })
    }

    /// Parse a single import: "path", alias "path", or _ "path"
    fn parse_single_import(&mut self) -> Result<Import> {
        match self.advance() {
            Some((Token::String(path), span)) => {
                // Simple import: "path"
                Ok(Import {
                    alias: None,
                    path,
                    span,
                })
            }
            Some((Token::Ident(alias), start_span)) => {
                // Aliased import: alias "path"
                match self.advance() {
                    Some((Token::String(path), _end_span)) => Ok(Import {
                        alias: Some(alias),
                        path,
                        span: start_span,
                    }),
                    Some((tok, span)) => Err(SoppoError::Parse {
                        message: format!(
                            "Expected import path string after alias, found {:?}",
                            tok
                        ),
                        span,
                    }),
                    None => Err(SoppoError::Parse {
                        message: "Expected import path string after alias".to_string(),
                        span: start_span,
                    }),
                }
            }
            Some((Token::Underscore, start_span)) => {
                // Blank import: _ "path" (for side effects only)
                match self.advance() {
                    Some((Token::String(path), _end_span)) => Ok(Import {
                        alias: Some("_".to_string()),
                        path,
                        span: start_span,
                    }),
                    Some((tok, span)) => Err(SoppoError::Parse {
                        message: format!("Expected import path string after _, found {:?}", tok),
                        span,
                    }),
                    None => Err(SoppoError::Parse {
                        message: "Expected import path string after _".to_string(),
                        span: start_span,
                    }),
                }
            }
            Some((tok, span)) => Err(SoppoError::Parse {
                message: format!("Expected import path or alias, found {:?}", tok),
                span,
            }),
            None => Err(SoppoError::Parse {
                message: "Expected import path".to_string(),
                span: Span::dummy(),
            }),
        }
    }

    /// Parse a complete file
    pub fn parse_file(&mut self) -> Result<File> {
        // Skip leading whitespace/newlines
        self.skip_terminators();

        // Parse package declaration
        let package = if self.check(&Token::Package) {
            self.advance(); // consume 'package'
            let (name, span) = self.parse_identifier("package")?;
            // Skip terminators after package declaration
            self.skip_terminators();
            Ident { name, span }
        } else {
            Ident {
                name: "main".to_string(),
                span: Span::dummy(),
            }
        };

        // Parse imports
        let mut imports = Vec::new();
        while self.consume(&Token::Import) {
            if self.consume(&Token::LParen) {
                // Grouped imports: import ( ... )
                self.skip_terminators();
                while self.peek() != Some(&Token::RParen) {
                    imports.push(self.parse_single_import()?);
                    self.skip_terminators();
                }
                self.expect(Token::RParen)?;
                self.skip_terminators();
            } else {
                // Single import: import "path" or import alias "path"
                imports.push(self.parse_single_import()?);
                self.skip_terminators();
            }
        }

        let mut decls = Vec::new();

        while self.peek().is_some() {
            // Skip terminators between declarations
            self.skip_terminators();

            if self.peek().is_none() {
                break;
            }

            // Check for grouped const: const ( ... )
            if self.peek() == Some(&Token::Const) {
                let (_, const_span) = self.advance().unwrap(); // consume 'const'
                if self.consume(&Token::LParen) {
                    // Grouped const block - use ConstBlock for iota support
                    let mut block_consts = Vec::new();
                    self.skip_terminators();
                    while self.peek() != Some(&Token::RParen) {
                        let const_decl = self.parse_const_in_group()?;
                        block_consts.push(const_decl);
                        self.skip_terminators();
                    }
                    self.expect(Token::RParen)?;
                    decls.push(Decl::ConstBlock(block_consts));
                    continue;
                } else {
                    // Single const - need to parse it, but we already consumed 'const'
                    // So call a helper that doesn't expect 'const' at the start
                    let const_decl = self.parse_const_after_keyword(const_span.start.line)?;
                    decls.push(Decl::Const(const_decl));
                    continue;
                }
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
    use crate::syntax::FileId;

    #[test]
    fn test_parse_function() {
        let source = "func add(x int, y int) int { return x + y }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser.parse_func_decl().expect("failed to parse function");

        assert_eq!(func.ident, "add");
        assert_eq!(func.params.len(), 2);
        assert_eq!(func.params[0].ident, "x");
        assert_eq!(func.params[0].ty.name, "int");
        assert_eq!(func.params[1].ident, "y");
        assert_eq!(func.returns.len(), 1);
        assert_eq!(func.returns[0].ty.name, "int");
        assert_eq!(func.returns[0].ident.name, ""); // unnamed return
        assert_eq!(func.body.stmts.len(), 1);
    }

    #[test]
    fn test_parse_function_named_returns() {
        let source =
            "func divide(a int, b int) (quotient int, remainder int) { return a / b, a % b }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser
            .parse_func_decl()
            .expect("failed to parse function with named returns");

        assert_eq!(func.ident, "divide");
        assert_eq!(func.params.len(), 2);
        assert_eq!(func.returns.len(), 2);
        assert_eq!(func.returns[0].ident.name, "quotient");
        assert_eq!(func.returns[0].ty.name, "int");
        assert_eq!(func.returns[1].ident.name, "remainder");
        assert_eq!(func.returns[1].ty.name, "int");
    }

    #[test]
    fn test_parse_function_grouped_named_returns() {
        let source = "func parseVersion(s string) (major, minor, patch int, err error) { return 0, 0, 0, nil }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser
            .parse_func_decl()
            .expect("failed to parse function with grouped named returns");

        assert_eq!(func.ident, "parseVersion");
        assert_eq!(func.returns.len(), 4);
        assert_eq!(func.returns[0].ident.name, "major");
        assert_eq!(func.returns[0].ty.name, "int");
        assert_eq!(func.returns[1].ident.name, "minor");
        assert_eq!(func.returns[1].ty.name, "int");
        assert_eq!(func.returns[2].ident.name, "patch");
        assert_eq!(func.returns[2].ty.name, "int");
        assert_eq!(func.returns[3].ident.name, "err");
        assert_eq!(func.returns[3].ty.name, "error");
    }

    #[test]
    fn test_parse_function_multiple_unnamed_returns() {
        let source = "func swap(a int, b int) (int, int) { return b, a }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser
            .parse_func_decl()
            .expect("failed to parse function with multiple unnamed returns");

        assert_eq!(func.ident, "swap");
        assert_eq!(func.returns.len(), 2);
        assert_eq!(func.returns[0].ident.name, ""); // unnamed
        assert_eq!(func.returns[0].ty.name, "int");
        assert_eq!(func.returns[1].ident.name, ""); // unnamed
        assert_eq!(func.returns[1].ty.name, "int");
    }

    #[test]
    fn test_parse_function_no_returns() {
        let source = "func log(msg string) { println(msg) }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser
            .parse_func_decl()
            .expect("failed to parse function with no returns");

        assert_eq!(func.ident, "log");
        assert_eq!(func.params.len(), 1);
        assert_eq!(func.returns.len(), 0);
    }

    #[test]
    fn test_parse_generic_function() {
        let source = "func identity[T any](x T) T { return x }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser
            .parse_func_decl()
            .expect("failed to parse generic function");

        assert_eq!(func.ident, "identity");
        assert_eq!(func.generics.len(), 1);
        assert_eq!(func.generics[0].ident, "T");
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
        let type_decl = parser.parse_type_decl().expect("failed to parse enum type");

        assert_eq!(type_decl.ident, "Result");
        assert_eq!(type_decl.generics.len(), 2);
        assert_eq!(type_decl.generics[0].ident, "T");
        assert_eq!(type_decl.generics[0].constraint, "any");
        assert_eq!(type_decl.generics[1].ident, "E");
        assert_eq!(type_decl.generics[1].constraint, "any");

        match type_decl.kind {
            TypeKind::Enum { variants } => {
                assert_eq!(variants.len(), 2);

                match &variants[0] {
                    EnumVariant::Single {
                        ident: name, ty, ..
                    } => {
                        assert_eq!(name, "Ok");
                        assert_eq!(ty.name, "T");
                    }
                    _ => panic!("Expected single variant"),
                }

                match &variants[1] {
                    EnumVariant::Single {
                        ident: name, ty, ..
                    } => {
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
            type Colour enum {
                Red
                Green
                Blue
            }

            func main() {
                colour := Colour.Red
                return colour
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().expect("failed to parse complete file");

        assert_eq!(file.decls.len(), 2);
        assert!(matches!(file.decls[0], Decl::Type(_)));
        assert!(matches!(file.decls[1], Decl::Func(_)));
    }

    #[test]
    fn test_parse_function_with_semicolons() {
        let source = "func add(x int, y int) int { c := x + y; return c }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser
            .parse_func_decl()
            .expect("failed to parse function with semicolons");

        assert_eq!(func.ident, "add");
        assert_eq!(func.body.stmts.len(), 2);
    }

    #[test]
    fn test_parse_grouped_imports() {
        let source = r#"
            import (
                "fmt"
                "net/http"
            )
            func main() {}
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser
            .parse_file()
            .expect("failed to parse grouped imports");

        assert_eq!(file.imports.len(), 2);
        assert_eq!(file.imports[0].path, "fmt");
        assert_eq!(file.imports[0].alias, None);
        assert_eq!(file.imports[1].path, "net/http");
        assert_eq!(file.imports[1].alias, None);
    }

    #[test]
    fn test_parse_aliased_imports() {
        let source = r#"
            import (
                "fmt"
                myHttp "net/http"
                "sop:util/helpers"
                h "sop:util/helpers"
            )
            func main() {}
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser
            .parse_file()
            .expect("failed to parse aliased imports");

        assert_eq!(file.imports.len(), 4);

        assert_eq!(file.imports[0].path, "fmt");
        assert_eq!(file.imports[0].alias, None);

        assert_eq!(file.imports[1].path, "net/http");
        assert_eq!(file.imports[1].alias, Some("myHttp".to_string()));

        assert_eq!(file.imports[2].path, "sop:util/helpers");
        assert_eq!(file.imports[2].alias, None);

        assert_eq!(file.imports[3].path, "sop:util/helpers");
        assert_eq!(file.imports[3].alias, Some("h".to_string()));
    }

    #[test]
    fn test_parse_single_aliased_import() {
        let source = r#"
            import myFmt "fmt"
            func main() {}
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser
            .parse_file()
            .expect("failed to parse single aliased import");

        assert_eq!(file.imports.len(), 1);
        assert_eq!(file.imports[0].path, "fmt");
        assert_eq!(file.imports[0].alias, Some("myFmt".to_string()));
    }
}
