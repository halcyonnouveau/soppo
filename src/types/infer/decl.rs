use super::Infer;
use crate::error::{Result, SoppoError};
use crate::syntax::{Block, ConstDecl, EnumVariant, FuncDecl, TypeDecl, TypeKind};
use crate::types::Type;
use crate::types::ctx::TypeDefKind;

impl Infer {
    /// Infer the type of a block
    /// The type of a block is the type of its last expression (if any), otherwise unit
    pub fn infer_block(&mut self, block: &Block) -> Result<Type> {
        self.push_scope();

        let mut last_ty = Type::unit();

        for stmt in &block.stmts {
            last_ty = self.infer_stmt(stmt)?;
        }

        self.pop_scope();

        Ok(last_ty)
    }

    /// Register a function's signature without checking the body.
    /// This allows functions to be called before their bodies are checked,
    /// enabling Go-style forward references.
    pub fn register_func_signature(&mut self, func: &FuncDecl) -> Result<()> {
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
            let ret_ty = if func.return_types.is_empty() {
                Type::unit()
            } else if func.return_types.len() == 1 {
                self.resolve_type(&func.return_types[0])
            } else {
                Type::generic(
                    "tuple",
                    func.return_types
                        .iter()
                        .map(|t| self.resolve_type(t))
                        .collect(),
                )
            };
            Type::fun_named(param_tys, ret_ty)
        };

        // Store function type in outermost scope so it can be called
        if let Some(scope) = self.scopes.first_mut() {
            scope.insert(func.ident.name.clone(), (func_ty, Some(func.span)));
        }

        // Register function in global state so it can be looked up for method calls
        self.global_state.register_function(func);

        Ok(())
    }

    /// Infer and check a function declaration's body.
    /// The function signature should already be registered via `register_func_signature`.
    pub fn infer_func_decl(&mut self, func: &FuncDecl) -> Result<()> {
        self.push_scope();

        // Save old generic params and set up new ones for this function
        let old_generic_params = std::mem::take(&mut self.generic_params);
        for generic in &func.generics {
            let ty_var = self.fresh_ty_var();
            self.generic_params
                .insert(generic.ident.name.clone(), ty_var);
        }

        // Set expected return types for this function
        let old_expected_return = self.expected_return_types.clone();
        if func.return_types.is_empty() {
            self.expected_return_types = Some(vec![]);
        } else {
            self.expected_return_types = Some(
                func.return_types
                    .iter()
                    .map(|ty| self.resolve_type(ty))
                    .collect(),
            );
        }

        // Add receiver parameter to scope (for methods)
        // Note: enum method validation is done in register_func_signature
        if let Some(receiver) = &func.receiver {
            let receiver_ty = self.resolve_type(&receiver.ty);
            self.insert_var(
                receiver.ident.name.clone(),
                receiver_ty.clone(),
                Some(receiver.ident.span),
            );

            // Set nil state for nilable receivers based on nullability
            // Non-nullable nilable receivers (e.g., *T not ?*T) are trusted to be non-nil
            if Self::is_nilable_type(&receiver_ty) && !receiver_ty.is_nullable() {
                self.set_nil_state(
                    receiver.ident.name.clone(),
                    crate::types::ty::Nullability::NonNull,
                );
            }
        }

        // Add parameters to scope
        for param in &func.params {
            let param_ty = self.resolve_type(&param.ty);
            self.insert_var(
                param.ident.name.clone(),
                param_ty.clone(),
                Some(param.ident.span),
            );

            // Set nil state for nilable parameters based on nullability
            // Non-nullable nilable params are trusted to be non-nil
            if Self::is_nilable_type(&param_ty) && !param_ty.is_nullable() {
                self.set_nil_state(
                    param.ident.name.clone(),
                    crate::types::ty::Nullability::NonNull,
                );
            }
        }

        // Infer body type
        let body_ty = self.infer_block(&func.body)?;

        // Check against declared return type (for single return)
        if func.return_types.len() == 1 {
            let declared_ret_ty = self.resolve_type(&func.return_types[0]);
            self.unify(&body_ty, &declared_ret_ty, &func.span)?;
        }

        self.pop_scope();

        // Restore old expected return types and generic params
        self.expected_return_types = old_expected_return;
        self.generic_params = old_generic_params;

        // Note: Function registration is done in register_func_signature

        Ok(())
    }

    /// Type check a const declaration
    pub fn infer_const_decl(&mut self, const_decl: &ConstDecl) -> Result<()> {
        // Infer the type of the value
        let value_ty = self.infer_expr(&const_decl.value)?;

        // Determine the constant's type
        let const_ty = if let Some(ty) = &const_decl.ty {
            // const X type = value: unify declared with inferred
            let declared_ty = Type::from_ast(ty);
            self.unify(&declared_ty, &value_ty, &const_decl.value.span)?;
            declared_ty
        } else {
            // const X = value: infer from value
            value_ty
        };

        // Add constant to the global scope
        if let Some(scope) = self.scopes.first_mut() {
            scope.insert(
                const_decl.ident.name.clone(),
                (const_ty, Some(const_decl.span)),
            );
        }

        Ok(())
    }

    /// Type check an enum/struct declaration
    pub fn infer_type_decl(&mut self, type_decl: &TypeDecl) -> Result<()> {
        match &type_decl.kind {
            TypeKind::Alias { .. } => {
                // Type aliases don't need special type checking
                // Just register the type in global state
                self.global_state.register_type(type_decl);
                Ok(())
            }

            TypeKind::Definition { .. } => {
                // Type definitions create new distinct types
                // Just register the type in global state
                self.global_state.register_type(type_decl);
                Ok(())
            }

            TypeKind::Enum { variants } => {
                // Register the enum type in the global state
                self.global_state.register_type(type_decl);

                // Set up generic params for this type declaration
                let old_generic_params = std::mem::take(&mut self.generic_params);
                for generic in &type_decl.generics {
                    let ty_var = self.fresh_ty_var();
                    self.generic_params
                        .insert(generic.ident.name.clone(), ty_var);
                }

                // Register each variant as a constructor function
                for variant in variants {
                    match variant {
                        EnumVariant::Unit { ident, .. } => {
                            // Unit variants are just values of the enum type
                            // They act like constructors with no arguments
                            let enum_ty = Type::simple(&type_decl.ident.name);
                            if let Some(scope) = self.scopes.first_mut() {
                                scope.insert(ident.name.clone(), (enum_ty, Some(ident.span)));
                            }
                        }
                        EnumVariant::Single { ident, ty, .. } => {
                            // Single value variants are functions: T -> EnumType
                            let value_ty = self.resolve_type(ty);
                            let enum_ty = Type::simple(&type_decl.ident.name);
                            let constructor_ty = Type::fun(vec![value_ty], enum_ty);
                            if let Some(scope) = self.scopes.first_mut() {
                                scope
                                    .insert(ident.name.clone(), (constructor_ty, Some(ident.span)));
                            }
                        }
                        EnumVariant::Struct { ident, fields, .. } => {
                            // Struct variants are functions: (field1, field2, ...) -> EnumType
                            let field_tys: Vec<Type> =
                                fields.iter().map(|f| self.resolve_type(&f.ty)).collect();
                            let enum_ty = Type::simple(&type_decl.ident.name);
                            let constructor_ty = Type::fun(field_tys, enum_ty);
                            if let Some(scope) = self.scopes.first_mut() {
                                scope
                                    .insert(ident.name.clone(), (constructor_ty, Some(ident.span)));
                            }
                        }
                    }
                }

                // Restore old generic params
                self.generic_params = old_generic_params;
                Ok(())
            }
            TypeKind::Struct { fields } => {
                // Register the struct type with proper field types
                self.global_state.register_type(type_decl);

                // Set up generic params for this type declaration
                let old_generic_params = std::mem::take(&mut self.generic_params);
                for generic in &type_decl.generics {
                    let ty_var = self.fresh_ty_var();
                    self.generic_params
                        .insert(generic.ident.name.clone(), ty_var);
                }

                // Store field types for later field access validation
                let field_types: Vec<(String, Type)> = fields
                    .iter()
                    .map(|f| (f.ident.name.clone(), self.resolve_type(&f.ty)))
                    .collect();

                // Update the registered type with actual field types
                if let Some(type_def) = self
                    .global_state
                    .current_module_mut()
                    .types
                    .get_mut(&type_decl.ident.name)
                {
                    type_def.kind = TypeDefKind::Struct {
                        fields: field_types,
                    };
                }

                // Restore old generic params
                self.generic_params = old_generic_params;
                Ok(())
            }
            TypeKind::Interface { .. } => {
                // Interfaces are just type definitions that Go uses for polymorphism
                // Just register the type, no special type checking needed
                self.global_state.register_type(type_decl);
                Ok(())
            }
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
                assert!(infer.infer_func_decl(func).is_err());
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
                assert!(infer.infer_func_decl(func).is_err());
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
                assert!(infer.infer_func_decl(func).is_err());
            }
        }
    }
}
