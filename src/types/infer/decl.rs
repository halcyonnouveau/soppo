use super::Infer;
use crate::error::{SoppoError, SoppoResult};
use crate::syntax::{Block, ConstDecl, Decl, EnumVariant, File, FuncDecl, TypeDecl, TypeKind};
use crate::types::ast::{
    TypedBlock, TypedConstDecl, TypedDecl, TypedEnumVariant, TypedFile, TypedFuncDecl, TypedImport,
    TypedImportKind, TypedInterfaceMethod, TypedParam, TypedTypeDecl, TypedTypeKind, TypedVarDecl,
};
use crate::types::ctx::TypeDefKind;
use crate::types::{SymbolInfo, SymbolKind, Type};

impl Infer {
    /// Infer the type of a function body, adding named return parameters to scope.
    /// Continues checking all statements even if some have errors.
    /// Returns the TypedBlock for the function body.
    pub fn infer_func_body(
        &mut self,
        block: &Block,
        returns: &[crate::syntax::Param],
    ) -> SoppoResult<TypedBlock> {
        self.push_scope();

        // Add named return parameters to the body scope
        // Mark them as used since they're implicitly used by return
        for ret in returns {
            if !ret.ident.name.is_empty() {
                let ret_ty = self.resolve_type(&ret.ty);
                if let Err(e) =
                    self.insert_var(ret.ident.name.clone(), ret_ty, Some(ret.ident.span))
                {
                    self.emit_error(e);
                }
                // Named returns are implicitly used - mark as used
                self.mark_var_used(&ret.ident.name);
            }
        }

        // Check all statements, continuing after errors
        let typed_stmts: Vec<_> = block
            .stmts
            .iter()
            .map(|stmt| self.infer_stmt(stmt))
            .collect();

        // Check for unused variables before popping scope
        if let Err(e) = self.check_unused_vars_in_scope() {
            self.emit_error(e);
        }

        self.pop_scope();

        Ok(TypedBlock::new(typed_stmts, block.span))
    }

    /// Infer the type of a block.
    /// Continues checking all statements even if some have errors.
    /// Returns a TypedBlock containing all typed statements.
    pub fn infer_block(&mut self, block: &Block) -> TypedBlock {
        self.push_scope();

        // Check all statements, continuing after errors
        let typed_stmts: Vec<_> = block
            .stmts
            .iter()
            .map(|stmt| self.infer_stmt(stmt))
            .collect();

        self.pop_scope();

        TypedBlock::new(typed_stmts, block.span)
    }

    /// Register a function's signature without checking the body.
    /// This allows functions to be called before their bodies are checked,
    /// enabling Go-style forward references.
    pub fn register_func_signature(&mut self, func: &FuncDecl) -> SoppoResult<()> {
        // Check for methods on enum types (not variants) - this is an error
        if let Some(receiver) = &func.receiver {
            let receiver_type_name = &receiver.ty.name;

            if !receiver_type_name.contains('.') {
                let base_name = receiver_type_name
                    .strip_prefix('*')
                    .unwrap_or(receiver_type_name);

                if let Some(type_def) = self.global_state.lookup_type(base_name)
                    && matches!(type_def.kind, TypeDefKind::Enum { .. })
                {
                    return Err(SoppoError::Type {
                        message: format!(
                            "Cannot define method on enum type `{}`. \
                                 Define methods on enum variants instead: \
                                 `func (v {}.VariantName) {}(...)`",
                            base_name, base_name, func.ident
                        ),
                        span: receiver.ty.span,
                    });
                }
            }
        }

        // Build function type from signature
        // Use resolve_type to properly handle qualified types like *http.Request
        let func_ty = {
            let param_tys: Vec<(Option<String>, Type)> = func
                .params
                .iter()
                .map(|p| (Some(p.ident.name.clone()), self.resolve_type(&p.ty)))
                .collect();
            let ret_ty = if func.returns.is_empty() {
                Type::unit()
            } else if func.returns.len() == 1 {
                self.resolve_type(&func.returns[0].ty)
            } else {
                Type::generic(
                    "tuple",
                    func.returns
                        .iter()
                        .map(|r| self.resolve_type(&r.ty))
                        .collect(),
                )
            };
            Type::fun_named(param_tys, ret_ty)
        };

        // Store function type in outermost scope so it can be called
        if let Some(scope) = self.scopes.first_mut() {
            scope.insert(
                func.ident.name.clone(),
                (func_ty.clone(), Some(func.span), false),
            );
        }

        // Record function definition for LSP
        let kind = if func.receiver.is_some() {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        self.record_symbol(
            func.ident.span,
            SymbolInfo {
                name: func.ident.name.clone(),
                ty: func_ty,
                definition_span: Some(func.span),
                name_span: Some(func.ident.span),
                kind,
                doc_comment: func.doc_comment.clone(),
                go_location: None,
            },
        );

        // Register function in global state so it can be looked up for method calls
        self.global_state.register_function(func);

        Ok(())
    }

    /// Infer and check a function declaration's body, returning a TypedFuncDecl.
    /// The function signature should already be registered via `register_func_signature`.
    pub fn infer_func_decl(&mut self, func: &FuncDecl) -> SoppoResult<TypedFuncDecl> {
        self.push_scope();

        // Save old generic params and set up new ones for this function
        // For function declarations, we use Type::simple with the param name (not type variables)
        // so that codegen outputs the actual parameter name like "T"
        let old_generic_params = std::mem::take(&mut self.generic_params);
        for generic in &func.generics {
            self.generic_params.insert(
                generic.ident.name.clone(),
                Type::simple(&generic.ident.name),
            );
        }

        // Set expected return types for this function
        let old_expected_return = self.expected_return_types.clone();
        if func.returns.is_empty() {
            self.expected_return_types = Some(vec![]);
        } else {
            self.expected_return_types = Some(
                func.returns
                    .iter()
                    .map(|r| self.resolve_type(&r.ty))
                    .collect(),
            );
        }

        // Build typed receiver
        let typed_receiver = if let Some(receiver) = &func.receiver {
            let receiver_ty = self.resolve_type(&receiver.ty);
            self.insert_var(
                receiver.ident.name.clone(),
                receiver_ty.clone(),
                Some(receiver.ident.span),
            )?;

            // Record type annotation for LSP
            self.record_type_annotation(&receiver.ty);

            // Set nil state for nilable receivers based on nullability
            if Self::is_nilable_type(&receiver_ty) && !receiver_ty.is_nullable() {
                self.set_nil_state(
                    receiver.ident.name.clone(),
                    crate::types::ty::Nullability::NonNull,
                );
            }

            Some(TypedParam {
                ident: receiver.ident.clone(),
                ty: receiver_ty,
                nullable: receiver.ty.nullable,
            })
        } else {
            None
        };

        // Build typed parameters
        let mut typed_params = Vec::with_capacity(func.params.len());
        for param in &func.params {
            let param_ty = self.resolve_type(&param.ty);
            self.insert_var(
                param.ident.name.clone(),
                param_ty.clone(),
                Some(param.ident.span),
            )?;

            // Record type annotation for LSP
            self.record_type_annotation(&param.ty);

            // Set nil state for nilable parameters based on nullability
            if Self::is_nilable_type(&param_ty) && !param_ty.is_nullable() {
                self.set_nil_state(
                    param.ident.name.clone(),
                    crate::types::ty::Nullability::NonNull,
                );
            }

            typed_params.push(TypedParam {
                ident: param.ident.clone(),
                ty: param_ty,
                nullable: param.ty.nullable,
            });
        }

        // Build typed returns
        let typed_returns: Vec<_> = func
            .returns
            .iter()
            .map(|ret| {
                self.record_type_annotation(&ret.ty);
                TypedParam {
                    ident: ret.ident.clone(),
                    ty: self.resolve_type(&ret.ty),
                    nullable: ret.ty.nullable,
                }
            })
            .collect();

        // Infer the function body
        let typed_body = self.infer_func_body(&func.body, &func.returns)?;

        // Check for missing return: if function has return types, body must diverge
        if !func.returns.is_empty() && !typed_body.diverges() {
            self.emit_error(SoppoError::MissingReturn {
                span: func.body.span,
                name: func.ident.name.clone(),
            });
        }

        self.pop_scope();

        // Restore old expected return types and generic params
        self.expected_return_types = old_expected_return;
        self.generic_params = old_generic_params;

        Ok(TypedFuncDecl {
            receiver: typed_receiver,
            ident: func.ident.clone(),
            generics: func.generics.clone(),
            params: typed_params,
            returns: typed_returns,
            body: typed_body,
            span: func.span,
            doc_comment: func.doc_comment.clone(),
        })
    }

    /// Type check a const declaration and return TypedConstDecl.
    pub fn infer_const_decl(&mut self, const_decl: &ConstDecl) -> TypedConstDecl {
        // Infer the type of the value
        let typed_value = self.infer_expr(&const_decl.value);
        let has_explicit_type = const_decl.ty.is_some();

        // Determine the constant's type
        let const_ty = if typed_value.ty.is_error() {
            Type::error()
        } else if let Some(ty) = &const_decl.ty {
            // const X type = value: unify declared with inferred
            let declared_ty = Type::from_ast(ty);
            self.unify(&declared_ty, &typed_value.ty, &const_decl.value.span);
            declared_ty
        } else {
            // const X = value: infer from value
            typed_value.ty.clone()
        };

        // Add constant to the global scope
        if let Some(scope) = self.scopes.first_mut() {
            scope.insert(
                const_decl.ident.name.clone(),
                (const_ty.clone(), Some(const_decl.span), false),
            );
        }

        // Record type annotation for LSP if present
        if let Some(ty) = &const_decl.ty {
            self.record_type_annotation(ty);
        }

        // Record constant definition for LSP
        self.record_symbol(
            const_decl.ident.span,
            SymbolInfo {
                name: const_decl.ident.name.clone(),
                ty: const_ty.clone(),
                definition_span: Some(const_decl.span),
                name_span: Some(const_decl.ident.span),
                kind: SymbolKind::Constant,
                doc_comment: const_decl.doc_comment.clone(),
                go_location: None,
            },
        );

        TypedConstDecl {
            ident: const_decl.ident.clone(),
            const_ty,
            has_explicit_type,
            value: typed_value,
            span: const_decl.span,
            doc_comment: const_decl.doc_comment.clone(),
        }
    }

    /// Type check a var declaration and return TypedVarDecl.
    pub fn infer_var_decl(&mut self, var_decl: &crate::syntax::VarDecl) -> TypedVarDecl {
        let has_explicit_type = var_decl.ty.is_some();

        // Determine the variable's type and typed value
        let (var_ty, typed_value) = match (&var_decl.ty, &var_decl.value) {
            (Some(ty), Some(value)) => {
                // var X type = value: unify declared with inferred
                let declared_ty = Type::from_ast(ty);
                let typed_val = self.infer_expr(value);
                if !typed_val.ty.is_error() {
                    self.unify(&declared_ty, &typed_val.ty, &value.span);
                }
                (declared_ty, Some(typed_val))
            }
            (Some(ty), None) => {
                // var X type (zero value)
                (Type::from_ast(ty), None)
            }
            (None, Some(value)) => {
                // var X = value: infer from value
                let typed_val = self.infer_expr(value);
                let ty = typed_val.ty.clone();
                (ty, Some(typed_val))
            }
            (None, None) => {
                // This shouldn't happen - parser should reject it
                self.emit_error(SoppoError::Type {
                    message: "var declaration must have type or value".to_string(),
                    span: var_decl.span,
                });
                (Type::error(), None)
            }
        };

        // Add variable to the global scope
        if let Some(scope) = self.scopes.first_mut() {
            scope.insert(
                var_decl.ident.name.clone(),
                (var_ty.clone(), Some(var_decl.span), true), // true = mutable
            );
        }

        // Record type annotation for LSP if present
        if let Some(ty) = &var_decl.ty {
            self.record_type_annotation(ty);
        }

        // Record variable definition for LSP
        self.record_symbol(
            var_decl.ident.span,
            SymbolInfo {
                name: var_decl.ident.name.clone(),
                ty: var_ty.clone(),
                definition_span: Some(var_decl.span),
                name_span: Some(var_decl.ident.span),
                kind: SymbolKind::Variable,
                doc_comment: None,
                go_location: None,
            },
        );

        TypedVarDecl {
            ident: var_decl.ident.clone(),
            var_ty,
            has_explicit_type,
            value: typed_value,
            span: var_decl.span,
        }
    }

    /// Type check an enum/struct declaration and return TypedTypeDecl.
    pub fn infer_type_decl(&mut self, type_decl: &TypeDecl) -> SoppoResult<TypedTypeDecl> {
        // Record type definition for LSP
        self.record_symbol(
            type_decl.ident.span,
            SymbolInfo {
                name: type_decl.ident.name.clone(),
                ty: Type::simple(&type_decl.ident.name),
                definition_span: Some(type_decl.span),
                name_span: Some(type_decl.ident.span),
                kind: SymbolKind::Type,
                doc_comment: type_decl.doc_comment.clone(),
                go_location: None,
            },
        );

        // Set up generic params for this type declaration
        // For type declarations, we use Type::simple with the param name (not type variables)
        // so that codegen outputs the actual parameter name like "T"
        let old_generic_params = std::mem::take(&mut self.generic_params);
        for generic in &type_decl.generics {
            self.generic_params.insert(
                generic.ident.name.clone(),
                Type::simple(&generic.ident.name),
            );
        }

        let typed_kind = match &type_decl.kind {
            TypeKind::Alias { target } => {
                // Type aliases don't need special type checking
                self.global_state.register_type(type_decl);
                TypedTypeKind::Alias {
                    target: self.resolve_type(target),
                }
            }

            TypeKind::Definition { target } => {
                // Type definitions create new distinct types
                self.global_state.register_type(type_decl);
                TypedTypeKind::Definition {
                    target: self.resolve_type(target),
                }
            }

            TypeKind::Enum { variants } => {
                // Register the enum type in the global state
                self.global_state.register_type(type_decl);

                // Build typed variants and register constructors
                let typed_variants: Vec<_> = variants
                    .iter()
                    .map(|variant| match variant {
                        EnumVariant::Unit { ident, .. } => {
                            // Unit variants are just values of the enum type
                            let enum_ty = Type::simple(&type_decl.ident.name);
                            if let Some(scope) = self.scopes.first_mut() {
                                scope
                                    .insert(ident.name.clone(), (enum_ty, Some(ident.span), false));
                            }
                            TypedEnumVariant::Unit {
                                ident: ident.clone(),
                            }
                        }
                        EnumVariant::Single { ident, ty, .. } => {
                            // Single value variants are functions: T -> EnumType
                            let value_ty = self.resolve_type(ty);
                            let enum_ty = Type::simple(&type_decl.ident.name);
                            let constructor_ty = Type::fun(vec![value_ty.clone()], enum_ty);
                            if let Some(scope) = self.scopes.first_mut() {
                                scope.insert(
                                    ident.name.clone(),
                                    (constructor_ty, Some(ident.span), false),
                                );
                            }
                            TypedEnumVariant::Single {
                                ident: ident.clone(),
                                ty: value_ty,
                            }
                        }
                        EnumVariant::Struct { ident, fields, .. } => {
                            // Struct variants are functions: (field1, field2, ...) -> EnumType
                            let field_types: Vec<(String, Type)> = fields
                                .iter()
                                .map(|f| (f.ident.name.clone(), self.resolve_type(&f.ty)))
                                .collect();
                            let field_tys: Vec<Type> =
                                field_types.iter().map(|(_, ty)| ty.clone()).collect();
                            let enum_ty = Type::simple(&type_decl.ident.name);
                            let constructor_ty = Type::fun(field_tys, enum_ty);
                            if let Some(scope) = self.scopes.first_mut() {
                                scope.insert(
                                    ident.name.clone(),
                                    (constructor_ty, Some(ident.span), false),
                                );
                            }
                            TypedEnumVariant::Struct {
                                ident: ident.clone(),
                                fields: field_types,
                            }
                        }
                    })
                    .collect();

                TypedTypeKind::Enum {
                    variants: typed_variants,
                }
            }

            TypeKind::Struct { fields } => {
                // Register the struct type with proper field types
                self.global_state.register_type(type_decl);

                // Store field types for later field access validation
                let field_types: Vec<(String, Type, Option<String>)> = fields
                    .iter()
                    .map(|f| {
                        (
                            f.ident.name.clone(),
                            self.resolve_type(&f.ty),
                            f.tag.clone(),
                        )
                    })
                    .collect();

                // Update the registered type with actual field types
                if let Some(type_def) = self
                    .global_state
                    .current_module_mut()
                    .types
                    .get_mut(&type_decl.ident.name)
                {
                    type_def.kind = TypeDefKind::Struct {
                        fields: field_types
                            .iter()
                            .map(|(name, ty, _)| (name.clone(), ty.clone()))
                            .collect(),
                    };
                }

                TypedTypeKind::Struct {
                    fields: field_types,
                }
            }

            TypeKind::Interface { methods } => {
                // Interfaces are just type definitions that Go uses for polymorphism
                self.global_state.register_type(type_decl);

                let typed_methods: Vec<_> = methods
                    .iter()
                    .map(|m| TypedInterfaceMethod {
                        ident: m.ident.clone(),
                        params: m
                            .params
                            .iter()
                            .map(|p| TypedParam {
                                ident: p.ident.clone(),
                                ty: self.resolve_type(&p.ty),
                                nullable: p.ty.nullable,
                            })
                            .collect(),
                        returns: m.returns.iter().map(|r| self.resolve_type(r)).collect(),
                    })
                    .collect();

                TypedTypeKind::Interface {
                    methods: typed_methods,
                }
            }
        };

        // Restore old generic params
        self.generic_params = old_generic_params;

        Ok(TypedTypeDecl {
            ident: type_decl.ident.clone(),
            generics: type_decl.generics.clone(),
            kind: typed_kind,
            span: type_decl.span,
            doc_comment: type_decl.doc_comment.clone(),
        })
    }

    /// Infer a declaration and return the typed version.
    pub fn infer_decl(&mut self, decl: &Decl) -> TypedDecl {
        match decl {
            Decl::Func(func) => {
                // Signature should already be registered
                match self.infer_func_decl(func) {
                    Ok(typed_func) => TypedDecl::Func(typed_func),
                    Err(e) => {
                        self.emit_error(e);
                        // Return a minimal typed func for error recovery
                        TypedDecl::Func(TypedFuncDecl {
                            receiver: None,
                            ident: func.ident.clone(),
                            generics: func.generics.clone(),
                            params: vec![],
                            returns: vec![],
                            body: TypedBlock::new(vec![], func.body.span),
                            span: func.span,
                            doc_comment: func.doc_comment.clone(),
                        })
                    }
                }
            }
            Decl::Const(const_decl) => TypedDecl::Const(self.infer_const_decl(const_decl)),
            Decl::ConstBlock(consts) => {
                let typed_consts: Vec<_> =
                    consts.iter().map(|c| self.infer_const_decl(c)).collect();
                TypedDecl::ConstBlock(typed_consts)
            }
            Decl::Var(var_decl) => TypedDecl::Var(self.infer_var_decl(var_decl)),
            Decl::Type(type_decl) => match self.infer_type_decl(type_decl) {
                Ok(typed_type) => TypedDecl::Type(typed_type),
                Err(e) => {
                    self.emit_error(e);
                    // Return a minimal typed type for error recovery
                    TypedDecl::Type(TypedTypeDecl {
                        ident: type_decl.ident.clone(),
                        generics: type_decl.generics.clone(),
                        kind: TypedTypeKind::Alias {
                            target: Type::error(),
                        },
                        span: type_decl.span,
                        doc_comment: type_decl.doc_comment.clone(),
                    })
                }
            },
        }
    }

    /// Infer an entire file and return a TypedFile.
    /// This performs the two-pass inference: first registering signatures, then inferring bodies.
    pub fn infer_file(&mut self, file: &File) -> TypedFile {
        // Build typed imports by looking up import kinds from self.imports
        // (populated by process_imports called earlier)
        let typed_imports: Vec<_> = file
            .imports
            .iter()
            .map(|import| {
                // Look up the import kind from our processed imports
                let short_name = import.alias.clone().unwrap_or_else(|| {
                    import
                        .path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&import.path)
                        .to_string()
                });

                let kind = self
                    .imports
                    .get(&short_name)
                    .map(|(_, _, _, k)| match k {
                        super::ImportKind::Go => TypedImportKind::Go,
                        super::ImportKind::Soppo(module_id) => {
                            TypedImportKind::Soppo(module_id.clone())
                        }
                    })
                    .unwrap_or(TypedImportKind::Go);

                TypedImport {
                    alias: import.alias.clone(),
                    path: import.path.clone(),
                    span: import.span,
                    kind,
                }
            })
            .collect();

        // Pass 1: Register type definitions and function signatures
        // This allows forward references (functions calling each other, types referencing each other)
        for decl in &file.decls {
            match decl {
                Decl::Type(type_decl) => {
                    // Register type in global state (but don't build TypedTypeDecl yet)
                    self.global_state.register_type(type_decl);
                }
                Decl::Func(func) => {
                    if let Err(e) = self.register_func_signature(func) {
                        self.emit_error(e);
                    }
                }
                // Consts and vars are processed in pass 2 along with their typed versions
                Decl::Const(_) | Decl::ConstBlock(_) | Decl::Var(_) => {}
            }
        }

        // Pass 2: Infer all declarations and build typed versions
        let typed_decls: Vec<_> = file
            .decls
            .iter()
            .map(|decl| self.infer_decl(decl))
            .collect();

        TypedFile {
            package: file.package.clone(),
            imports: typed_imports,
            decls: typed_decls,
            comments: file.comments.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{Decl, FileId, Parser};

    #[test]
    fn test_infer_function() {
        let source = r#"
            func add(x int, y int) int {
                return x + y
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                assert!(infer.infer_func_decl(func).is_ok());
            }
        }
    }

    #[test]
    fn test_infer_function_type_error() {
        let source = r#"
            func bad() int {
                return "not an int"
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                // Errors are now collected rather than returned
                let _ = infer.infer_func_decl(func);
                assert!(infer.has_errors(), "Expected type error to be collected");
            }
        }
    }

    #[test]
    fn test_function_call_in_scope() {
        let source = r#"
            func add(x int, y int) int {
                return x + y
            }

            func main() int {
                return add(1, 2)
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        // Pass 1: Register function signatures
        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                assert!(infer.register_func_signature(func).is_ok());
            }
        }

        // Pass 2: Check function bodies
        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                assert!(infer.infer_func_decl(func).is_ok());
            }
        }
    }

    #[test]
    fn test_struct_field_access() {
        let source = r#"
            type Point struct {
                x int
                y int
            }

            func test() int {
                p := Point{x: 10, y: 20}
                return p.x
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        // First register the type
        for decl in &file.decls {
            if let Decl::Type(type_decl) = decl {
                infer.infer_type_decl(type_decl).unwrap();
            }
        }

        // Then check the function
        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                assert!(infer.infer_func_decl(func).is_ok());
            }
        }
    }

    #[test]
    fn test_struct_invalid_field_access() {
        let source = r#"
            type Point struct {
                x int
                y int
            }

            func test() int {
                p := Point{x: 10, y: 20}
                return p.z
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        // First register the type
        for decl in &file.decls {
            if let Decl::Type(type_decl) = decl {
                infer.infer_type_decl(type_decl).unwrap();
            }
        }

        // Then check the function - should fail because z doesn't exist
        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                // Errors are now collected rather than returned
                let _ = infer.infer_func_decl(func);
                assert!(
                    infer.has_errors(),
                    "Expected field access error to be collected"
                );
            }
        }
    }

    #[test]
    fn test_struct_field_type_checking() {
        let source = r#"
            type Point struct {
                x int
                y int
            }

            func test() string {
                p := Point{x: 10, y: 20}
                return p.x
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        // First register the type
        for decl in &file.decls {
            if let Decl::Type(type_decl) = decl {
                infer.infer_type_decl(type_decl).unwrap();
            }
        }

        // Then check the function - should fail because return type doesn't match
        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                // Errors are now collected rather than returned
                let _ = infer.infer_func_decl(func);
                assert!(
                    infer.has_errors(),
                    "Expected type mismatch error to be collected"
                );
            }
        }
    }
}
