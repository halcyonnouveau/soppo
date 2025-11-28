use super::Parser;
use crate::error::{Result, SoppoError};
use crate::parse::ast::{Block, Expr, ExprKind, SelectCase, SelectCaseKind, Stmt, StmtKind};
use crate::parse::lexer::Token;
use crate::parse::source::Span;

impl Parser {
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
                // Newline or semicolon after return means empty return
                let values = if matches!(
                    self.peek(),
                    Some(Token::RBrace) | Some(Token::Newline) | Some(Token::Semicolon) | None
                ) {
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

            Some(Token::Select) => {
                self.advance();
                self.parse_select_stmt(start_span)
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

    /// Parse select statement
    pub(super) fn parse_select_stmt(&mut self, start_span: Span) -> Result<Stmt> {
        self.expect(Token::LBrace)?;
        self.skip_terminators();

        let mut cases = Vec::new();

        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            let case = self.parse_select_case()?;
            cases.push(case);
            self.skip_terminators();
        }

        let end_span = self.expect(Token::RBrace)?;

        Ok(Stmt {
            kind: StmtKind::Select { cases },
            span: Span::with_bytes(
                start_span.start,
                end_span.end,
                self.file,
                start_span.byte_start,
                end_span.byte_end,
            ),
        })
    }

    /// Parse a select case: case <-ch:, case v := <-ch:, case ch <- v:, or default:
    fn parse_select_case(&mut self) -> Result<SelectCase> {
        let case_start = self.peek_span();

        // Check for default:
        if let Some(Token::Ident(s)) = self.peek()
            && s == "default"
        {
            self.advance(); // consume 'default'
            self.expect(Token::Colon)?;
            self.skip_terminators();

            // Parse body
            let body = self.parse_select_case_body()?;
            let body_span_end = body.span.end;
            let body_byte_end = body.span.byte_end;

            return Ok(SelectCase {
                kind: SelectCaseKind::Default,
                body,
                span: Span::with_bytes(
                    case_start.start,
                    body_span_end,
                    self.file,
                    case_start.byte_start,
                    body_byte_end,
                ),
            });
        }

        self.expect(Token::Case)?;

        // Now we need to determine the case kind:
        // 1. <-ch             - recv (discard)
        // 2. v := <-ch        - recv with decl
        // 3. v, ok := <-ch    - recv with ok check
        // 4. ch <- value      - send

        // Check for receive without assignment: case <-ch:
        if self.consume(&Token::Arrow) {
            let channel = self.parse_expr()?;
            self.expect(Token::Colon)?;
            self.skip_terminators();

            let body = self.parse_select_case_body()?;
            let body_span_end = body.span.end;
            let body_byte_end = body.span.byte_end;

            return Ok(SelectCase {
                kind: SelectCaseKind::Recv { channel },
                body,
                span: Span::with_bytes(
                    case_start.start,
                    body_span_end,
                    self.file,
                    case_start.byte_start,
                    body_byte_end,
                ),
            });
        }

        // Parse the first expression/identifier
        let first_expr = self.parse_expr()?;

        // Check for send: ch <- value
        if self.consume(&Token::Arrow) {
            let value = self.parse_expr()?;
            self.expect(Token::Colon)?;
            self.skip_terminators();

            let body = self.parse_select_case_body()?;
            let body_span_end = body.span.end;
            let body_byte_end = body.span.byte_end;

            return Ok(SelectCase {
                kind: SelectCaseKind::Send {
                    channel: first_expr,
                    value,
                },
                body,
                span: Span::with_bytes(
                    case_start.start,
                    body_span_end,
                    self.file,
                    case_start.byte_start,
                    body_byte_end,
                ),
            });
        }

        // Must be a receive with declaration
        // Check for second variable (v, ok := <-ch)
        let (first_name, second_name) = if let ExprKind::Ident(name) = &first_expr.kind {
            let first_name = name.clone();
            if self.consume(&Token::Comma) {
                // v, ok := <-ch
                match self.advance() {
                    Some((Token::Ident(second), _)) => (first_name, Some(second)),
                    Some((tok, span)) => {
                        return Err(SoppoError::Parse {
                            message: format!("Expected identifier after ',', found {:?}", tok),
                            span,
                        });
                    }
                    None => {
                        return Err(SoppoError::Parse {
                            message: "Expected identifier after ','".to_string(),
                            span: Span::dummy(),
                        });
                    }
                }
            } else {
                (first_name, None)
            }
        } else {
            return Err(SoppoError::Parse {
                message: "Expected identifier in select case".to_string(),
                span: first_expr.span,
            });
        };

        // Expect := <-ch
        self.expect(Token::ColonAssign)?;
        self.expect(Token::Arrow)?;
        let channel = self.parse_expr()?;
        self.expect(Token::Colon)?;
        self.skip_terminators();

        let body = self.parse_select_case_body()?;
        let body_span_end = body.span.end;
        let body_byte_end = body.span.byte_end;

        let kind = if let Some(ok_name) = second_name {
            SelectCaseKind::RecvDeclOk {
                name: first_name,
                ok_name,
                channel,
            }
        } else {
            SelectCaseKind::RecvDecl {
                name: first_name,
                channel,
            }
        };

        Ok(SelectCase {
            kind,
            body,
            span: Span::with_bytes(
                case_start.start,
                body_span_end,
                self.file,
                case_start.byte_start,
                body_byte_end,
            ),
        })
    }

    /// Parse the body of a select case (statements until next case/default/})
    fn parse_select_case_body(&mut self) -> Result<Block> {
        let mut stmts = Vec::new();
        let body_start = self.peek_span();

        while !matches!(self.peek(), Some(Token::Case) | Some(Token::RBrace) | None) {
            // Check if it's 'default'
            if let Some(Token::Ident(s)) = self.peek()
                && s == "default"
            {
                break;
            }

            stmts.push(self.parse_stmt()?);
            self.skip_terminators();
        }

        let body_end = stmts
            .last()
            .map(|s| s.span.clone())
            .unwrap_or(body_start.clone());

        Ok(Block {
            stmts,
            span: body_end,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::source::FileId;

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
    fn test_parse_select() {
        let source = r#"select {
            case v := <-ch:
                x := 1
            case ch2 <- msg:
                y := 2
            case <-done:
                return
            default:
                w := 4
        }"#;
        let mut parser = Parser::new(source, FileId(0));
        let stmt = parser.parse_stmt().unwrap();

        match stmt.kind {
            StmtKind::Select { cases } => {
                assert_eq!(cases.len(), 4);

                // First case: v := <-ch (RecvDecl)
                match &cases[0].kind {
                    SelectCaseKind::RecvDecl { name, channel } => {
                        assert_eq!(name, "v");
                        assert!(matches!(&channel.kind, ExprKind::Ident(s) if s == "ch"));
                    }
                    _ => panic!("Expected RecvDecl"),
                }

                // Second case: ch2 <- msg (Send)
                match &cases[1].kind {
                    SelectCaseKind::Send { channel, value } => {
                        assert!(matches!(&channel.kind, ExprKind::Ident(s) if s == "ch2"));
                        assert!(matches!(&value.kind, ExprKind::Ident(s) if s == "msg"));
                    }
                    _ => panic!("Expected Send"),
                }

                // Third case: <-done (Recv)
                match &cases[2].kind {
                    SelectCaseKind::Recv { channel } => {
                        assert!(matches!(&channel.kind, ExprKind::Ident(s) if s == "done"));
                    }
                    _ => panic!("Expected Recv"),
                }

                // Fourth case: default
                assert!(matches!(cases[3].kind, SelectCaseKind::Default));
            }
            _ => panic!("Expected select statement"),
        }
    }
}
