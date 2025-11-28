mod decl;
mod expr;
mod pat;
mod stmt;
mod unify;

use std::collections::HashMap;

use super::ctx::GlobalCtxt;
use super::ty::{Nullability, Type};
use crate::go::{GoCache, Project, parse_go_type};
use crate::syntax::{Expr, ExprKind, Import, ModuleId, Span, Symbol, Type as AstType, UnaryOp};

/// Check if a type name is a Go primitive/built-in type
pub(crate) fn is_primitive_type(ty: &str) -> bool {
    matches!(
        ty,
        "bool"
            | "string"
            | "int"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uintptr"
            | "byte"
            | "rune"
            | "float32"
            | "float64"
            | "complex64"
            | "complex128"
            | "error"
    )
}

/// Check if a type name is a numeric primitive type
pub(crate) fn is_numeric_primitive(ty: &str) -> bool {
    matches!(
        ty,
        "int"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uintptr"
            | "byte"
            | "rune"
            | "float32"
            | "float64"
    )
}

/// Type inference engine
pub struct Infer {
    /// Global state tracking all modules
    pub(super) global_state: GlobalCtxt,

    /// Current scope: variable name -> type
    pub(super) scopes: Vec<HashMap<String, Type>>,

    /// Type variable substitutions (solutions)
    pub(super) substitutions: HashMap<i32, Type>,

    /// Next fresh type variable ID
    pub(super) next_var: i32,

    /// Expected return types for the current function (None if not in a function)
    pub(super) expected_return_types: Option<Vec<Type>>,

    /// Generic type parameters in scope: param name -> type variable
    pub(super) generic_params: HashMap<String, Type>,

    /// Cache for Go package information (always enabled)
    pub(super) go_cache: GoCache,

    /// Current project (for external module resolution, optional)
    pub(super) project: Option<Project>,

    /// Imported Go packages: short name -> import path
    /// e.g., "fmt" -> "fmt", "strings" -> "strings"
    pub(super) imported_packages: HashMap<String, String>,

    /// Imported Soppo modules: short name -> ModuleId
    /// e.g., "helpers" -> ModuleId("util/helpers")
    pub(super) soppo_imports: HashMap<String, ModuleId>,

    /// Nil state tracking for pointer variables (scoped like variable bindings)
    /// Each scope maps variable names to their current nullability state
    pub(super) nil_state: Vec<HashMap<String, Nullability>>,
}

impl Infer {
    /// Create a new type inference engine
    ///
    /// Go stdlib resolution is always enabled.
    /// For external module resolution, use `with_project`.
    pub fn new() -> miette::Result<Self> {
        Ok(Self {
            global_state: GlobalCtxt::new(),
            scopes: vec![HashMap::new()],
            substitutions: HashMap::new(),
            next_var: 0,
            expected_return_types: None,
            generic_params: HashMap::new(),
            go_cache: GoCache::new()?,
            project: None,
            imported_packages: HashMap::new(),
            soppo_imports: HashMap::new(),
            nil_state: vec![HashMap::new()],
        })
    }

    /// Create an Infer with project context for external module resolution
    pub fn with_project(project: Project) -> miette::Result<Self> {
        Ok(Self {
            global_state: GlobalCtxt::new(),
            scopes: vec![HashMap::new()],
            substitutions: HashMap::new(),
            next_var: 0,
            expected_return_types: None,
            generic_params: HashMap::new(),
            go_cache: GoCache::new()?,
            project: Some(project),
            imported_packages: HashMap::new(),
            soppo_imports: HashMap::new(),
            nil_state: vec![HashMap::new()],
        })
    }

    /// Create an Infer with existing GlobalCtxt for multi-file compilation
    pub fn with_global_state(global_state: GlobalCtxt) -> miette::Result<Self> {
        Ok(Self {
            global_state,
            scopes: vec![HashMap::new()],
            substitutions: HashMap::new(),
            next_var: 0,
            expected_return_types: None,
            generic_params: HashMap::new(),
            go_cache: GoCache::new()?,
            project: None,
            imported_packages: HashMap::new(),
            soppo_imports: HashMap::new(),
            nil_state: vec![HashMap::new()],
        })
    }

    /// Create an Infer with existing GlobalCtxt and project context
    pub fn with_global_state_and_project(
        global_state: GlobalCtxt,
        project: Project,
    ) -> miette::Result<Self> {
        Ok(Self {
            global_state,
            scopes: vec![HashMap::new()],
            substitutions: HashMap::new(),
            next_var: 0,
            expected_return_types: None,
            generic_params: HashMap::new(),
            go_cache: GoCache::new()?,
            project: Some(project),
            imported_packages: HashMap::new(),
            soppo_imports: HashMap::new(),
            nil_state: vec![HashMap::new()],
        })
    }

    pub fn global_state(self) -> GlobalCtxt {
        self.global_state
    }

    /// Process imports and add package names to scope
    ///
    /// Imports are classified as Soppo or Go based on whether the import path:
    /// 1. Starts with the project's module path
    /// 2. Corresponds to a local directory with .sop files
    pub fn process_imports(&mut self, imports: &[Import]) {
        for import in imports {
            let import_path = import.path.trim_matches('"');

            // Check if this is a local Soppo package
            let is_soppo = self.project.as_ref().is_some_and(|project| {
                crate::deps::is_soppo_import(import_path, &project.module_path, &project.root)
            });

            if is_soppo {
                let project = self.project.as_ref().unwrap();
                // Get the local path portion (e.g., "helpers" from "github.com/user/project/helpers")
                let local_path =
                    crate::deps::get_local_package_path(import_path, &project.module_path).unwrap();

                // Use alias if provided, otherwise derive from path
                let package_name = import
                    .alias
                    .as_deref()
                    .unwrap_or_else(|| local_path.rsplit('/').next().unwrap_or(local_path));

                // Track the Soppo import with its ModuleId for cross-package lookups
                // The local_path is the module ID (e.g., "helpers" or "util/helpers")
                self.soppo_imports
                    .insert(package_name.to_string(), ModuleId::new(local_path));

                // Also track in imported_packages for `is_imported_package` checks
                self.imported_packages
                    .insert(package_name.to_string(), import_path.to_string());

                // Add package name to scope with a special "soppo_package" type
                self.insert_var(package_name.to_string(), Type::simple("soppo_package"));
            } else {
                // Go package import
                // Use alias if provided, otherwise derive from path
                let package_name = import
                    .alias
                    .as_deref()
                    .unwrap_or_else(|| import_path.rsplit('/').next().unwrap_or(import_path));

                // Track the import for later lookup
                self.imported_packages
                    .insert(package_name.to_string(), import_path.to_string());

                // Add package name to scope with a special "package" type
                // This allows field access like fmt.Printf to work
                self.insert_var(package_name.to_string(), Type::simple("package"));
            }
        }
    }

    /// Check if a package name refers to a Soppo module import
    pub fn is_soppo_import(&self, package_name: &str) -> bool {
        self.soppo_imports.contains_key(package_name)
    }

    /// Get the ModuleId for a Soppo import
    pub fn get_soppo_module(&self, package_name: &str) -> Option<&ModuleId> {
        self.soppo_imports.get(package_name)
    }

    /// Look up a function in an imported Soppo module
    pub(super) fn lookup_soppo_function(
        &self,
        package_name: &str,
        func_name: &str,
    ) -> Option<Type> {
        // Get the ModuleId for this package
        let module_id = self.soppo_imports.get(package_name)?;

        // Look up the function in GlobalCtxt
        let func_def = self.global_state.lookup_function_in(module_id, func_name)?;

        // Convert FuncDef to Type::Fun
        let param_types: Vec<Type> = func_def.params.iter().map(|(_, ty)| ty.clone()).collect();

        let return_type = if func_def.return_types.is_empty() {
            Type::unit()
        } else if func_def.return_types.len() == 1 {
            func_def.return_types[0].clone()
        } else {
            // Multiple return types - use a tuple type
            Type::generic("tuple", func_def.return_types.clone())
        };

        Some(Type::fun(param_types, return_type))
    }

    /// Look up a type in an imported Soppo module
    pub(super) fn lookup_soppo_type(&self, package_name: &str, type_name: &str) -> Option<Type> {
        // Get the ModuleId for this package
        let module_id = self.soppo_imports.get(package_name)?;

        // Look up the type in GlobalCtxt
        let type_def = self.global_state.lookup_type_in(module_id, type_name)?;

        // Return the type as a simple type constructor
        Some(Type::simple(&type_def.name))
    }

    /// Look up a constant in an imported Soppo module
    pub(super) fn lookup_soppo_constant(
        &self,
        package_name: &str,
        const_name: &str,
    ) -> Option<Type> {
        // Get the ModuleId for this package
        let module_id = self.soppo_imports.get(package_name)?;

        // Look up the constant in GlobalCtxt
        let const_def = self
            .global_state
            .lookup_constant_in(module_id, const_name)?;

        Some(const_def.ty.clone())
    }

    /// Look up a function in an imported Go package
    pub(super) fn lookup_go_function(
        &mut self,
        package_name: &str,
        func_name: &str,
    ) -> Option<Type> {
        // Get the import path for this package
        let import_path = self.imported_packages.get(package_name)?.clone();

        // Try to get the package info (project is optional - stdlib works without it)
        let pkg = self
            .go_cache
            .get_or_parse(&import_path, self.project.as_ref())
            .ok()?;

        // Look up the function
        let func_def = pkg.functions.get(func_name)?;

        // Convert Go signature to Soppo Type
        let param_types: Vec<Type> = func_def
            .params
            .iter()
            .map(|p| parse_go_type(&p.ty))
            .collect();

        let return_type = if func_def.return_type.is_empty() {
            Type::unit()
        } else {
            parse_go_type(&func_def.return_type)
        };

        Some(Type::fun(param_types, return_type))
    }

    /// Look up a type in an imported Go package
    pub(super) fn lookup_go_type(&mut self, package_name: &str, type_name: &str) -> Option<Type> {
        // Get the import path for this package
        let import_path = self.imported_packages.get(package_name)?.clone();

        // Try to get the package info (project is optional - stdlib works without it)
        let pkg = self
            .go_cache
            .get_or_parse(&import_path, self.project.as_ref())
            .ok()?;

        // Check if it's a type
        if pkg.types.contains_key(type_name) {
            return Some(Type::Con {
                name: Symbol {
                    module: ModuleId::new(package_name),
                    name: type_name.to_string(),
                    span: Span::dummy(),
                },
                args: vec![],
            });
        }

        // Check if it's a constant
        if let Some(const_def) = pkg.constants.get(type_name) {
            let const_ty = &const_def.ty;
            // If the constant's type is a type defined in this package, return it with module info
            if pkg.types.contains_key(const_ty) {
                return Some(Type::Con {
                    name: Symbol {
                        module: ModuleId::new(package_name),
                        name: const_ty.to_string(),
                        span: Span::dummy(),
                    },
                    args: vec![],
                });
            }
            // Otherwise parse it as a Go type (for primitive types, etc.)
            return Some(parse_go_type(const_ty));
        }

        None
    }

    /// Check if a name refers to an imported Go package
    pub(super) fn is_imported_package(&self, name: &str) -> bool {
        self.imported_packages.contains_key(name)
    }

    /// Get the ultimate underlying type for a type alias chain.
    /// For example, if we have:
    ///   type Duration int64
    ///   type MyDuration Duration
    /// Then get_underlying_type("time", "MyDuration") returns Some("int64")
    ///
    /// Returns None if the type is not found or is not an alias (e.g., struct, interface).
    pub(super) fn get_underlying_type(
        &mut self,
        package_name: &str,
        type_name: &str,
    ) -> Option<String> {
        let import_path = self.imported_packages.get(package_name)?.clone();
        self.resolve_underlying_type_recursive(&import_path, type_name, 0)
    }

    /// Recursively resolve through type alias chains to find the ultimate underlying type.
    /// The depth parameter prevents infinite loops in case of circular type definitions.
    fn resolve_underlying_type_recursive(
        &mut self,
        import_path: &str,
        type_name: &str,
        depth: usize,
    ) -> Option<String> {
        // Prevent infinite recursion (shouldn't happen with valid Go code, but be safe)
        if depth > 20 {
            return None;
        }

        // Extract all needed info from the package in one go to avoid borrow issues
        let (underlying, is_in_same_pkg) = {
            let pkg = self
                .go_cache
                .get_or_parse(import_path, self.project.as_ref())
                .ok()?;

            let type_def = pkg.types.get(type_name)?;

            // If it's not an alias, it has no underlying type in the relevant sense
            if type_def.kind != "alias" {
                return None;
            }

            let underlying = type_def.underlying.as_ref()?.clone();
            let is_in_same_pkg = pkg.types.contains_key(&underlying);
            (underlying, is_in_same_pkg)
        };

        // Check if the underlying type is a primitive/built-in type
        if is_primitive_type(&underlying) {
            return Some(underlying);
        }

        // Check if the underlying type is qualified (pkg.Type)
        if let Some(dot_idx) = underlying.find('.') {
            let pkg_name = underlying[..dot_idx].to_string();
            let inner_type_name = underlying[dot_idx + 1..].to_string();

            // Look up the package's import path
            if let Some(inner_import_path) = self.imported_packages.get(&pkg_name).cloned() {
                return self.resolve_underlying_type_recursive(
                    &inner_import_path,
                    &inner_type_name,
                    depth + 1,
                );
            }
        }

        // The underlying type might be in the same package
        if is_in_same_pkg {
            return self.resolve_underlying_type_recursive(import_path, &underlying, depth + 1);
        }

        // Otherwise, it's a type we can't resolve further - return it as-is
        Some(underlying)
    }

    /// Check if two types are compatible for arithmetic via underlying type resolution.
    /// Returns Some(result_type) if compatible, None if not.
    ///
    /// This handles cases like `time.Duration * int` where Duration has underlying type int64,
    /// which is compatible with int literals.
    pub(super) fn check_numeric_underlying_compatibility(
        &mut self,
        left: &Type,
        right: &Type,
    ) -> Option<Type> {
        let (left_name, left_module) = match left {
            Type::Con { name, .. } => {
                let module = if name.module.0.is_empty() {
                    None
                } else {
                    Some(name.module.0.clone())
                };
                (name.name.clone(), module)
            }
            _ => return None,
        };
        let (right_name, right_module) = match right {
            Type::Con { name, .. } => {
                let module = if name.module.0.is_empty() {
                    None
                } else {
                    Some(name.module.0.clone())
                };
                (name.name.clone(), module)
            }
            _ => return None,
        };

        // Check if left is a defined type with numeric underlying, and right is numeric primitive
        if let Some(ref left_pkg) = left_module
            && let Some(left_underlying) = self.get_underlying_type(left_pkg, &left_name)
            && is_numeric_primitive(&left_underlying)
            && self.is_compatible_with_underlying(&right_name, &left_underlying)
        {
            return Some(left.clone());
        }

        // Check if right is a defined type with numeric underlying, and left is numeric primitive
        if let Some(ref right_pkg) = right_module
            && let Some(right_underlying) = self.get_underlying_type(right_pkg, &right_name)
            && is_numeric_primitive(&right_underlying)
            && self.is_compatible_with_underlying(&left_name, &right_underlying)
        {
            return Some(right.clone());
        }

        None
    }

    /// Check if a type name is compatible with an underlying type.
    /// This handles Go's untyped constant promotion - an `int` literal can be used where
    /// `int64` is expected, etc.
    fn is_compatible_with_underlying(&self, ty_name: &str, underlying: &str) -> bool {
        // Exact match
        if ty_name == underlying {
            return true;
        }

        // Int literals (represented as "int") are compatible with any integer type
        if ty_name == "int" {
            return matches!(
                underlying,
                "int"
                    | "int8"
                    | "int16"
                    | "int32"
                    | "int64"
                    | "uint"
                    | "uint8"
                    | "uint16"
                    | "uint32"
                    | "uint64"
                    | "uintptr"
                    | "byte"
                    | "rune"
            );
        }

        // Float literals are compatible with float types
        if ty_name == "float64" {
            return matches!(underlying, "float32" | "float64");
        }

        false
    }

    /// Generate a fresh type variable
    pub fn fresh_ty_var(&mut self) -> Type {
        let var = Type::var(self.next_var);
        self.next_var += 1;
        var
    }

    /// Resolve an AST type to a runtime Type, checking for generic params
    pub(super) fn resolve_type(&mut self, ast_ty: &AstType) -> Type {
        // Check if the type name is a generic parameter
        if let Some(ty_var) = self.generic_params.get(&ast_ty.name) {
            return ty_var.clone();
        }

        // Not a generic param - create a concrete type
        // Recursively resolve type arguments
        let args: Vec<Type> = ast_ty
            .args
            .iter()
            .map(|arg| self.resolve_type(arg))
            .collect();

        Type::Con {
            name: Symbol {
                module: ModuleId::empty(),
                name: ast_ty.name.clone(),
                span: ast_ty.span,
            },
            args,
        }
    }

    /// Instantiate a type name using a substitution map
    /// If the name is in the subst map, return the substituted type variable
    /// Otherwise return the type as-is
    pub(super) fn instantiate_type(&self, type_name: &str, subst: &HashMap<String, Type>) -> Type {
        if let Some(ty_var) = subst.get(type_name) {
            ty_var.clone()
        } else {
            Type::simple(type_name)
        }
    }

    /// Push a new scope (for both variables and nil state)
    pub(super) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.nil_state.push(HashMap::new());
    }

    /// Pop the current scope (for both variables and nil state)
    pub(super) fn pop_scope(&mut self) {
        self.scopes.pop();
        self.nil_state.pop();
    }

    /// Push only a nil state scope (for narrowing in if branches without new variable scope)
    pub(super) fn push_nil_scope(&mut self) {
        self.nil_state.push(HashMap::new());
    }

    /// Pop only a nil state scope
    pub(super) fn pop_nil_scope(&mut self) {
        self.nil_state.pop();
    }

    /// Insert a variable into the current scope
    pub(super) fn insert_var(&mut self, name: String, ty: Type) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, ty);
        }
    }

    /// Lookup a variable in scopes (from innermost to outermost)
    pub(super) fn lookup_var(&self, name: &str) -> Option<Type> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        None
    }

    /// Get the nil state for a variable (from innermost to outermost scope)
    /// Returns Nullable if not explicitly tracked (conservative default)
    pub(super) fn get_nil_state(&self, name: &str) -> Nullability {
        for scope in self.nil_state.iter().rev() {
            if let Some(state) = scope.get(name) {
                return *state;
            }
        }
        // Default: assume nullable if not tracked
        Nullability::Nullable
    }

    /// Set the nil state for a variable in the current scope
    pub(super) fn set_nil_state(&mut self, name: String, state: Nullability) {
        if let Some(scope) = self.nil_state.last_mut() {
            scope.insert(name, state);
        }
    }

    /// Check if a type is a pointer type
    pub(super) fn is_pointer_type(ty: &Type) -> bool {
        match ty {
            Type::Con { name, args } => {
                // Check for ptr[T] or *T patterns
                let ty_name = &name.name;
                (ty_name == "ptr" || ty_name.starts_with('*')) && args.len() == 1
            }
            _ => false,
        }
    }

    /// Determine the nullability of an expression's result
    /// Returns NonNull for expressions that are guaranteed non-nil:
    /// - &expr (address-of)
    /// - new(T)
    /// - Struct literals (when taken by address)
    ///
    /// Returns Nullable for everything else that could be nil
    pub(super) fn get_expr_nullability(&self, expr: &Expr, ty: &Type) -> Nullability {
        // Only pointer types can be nullable
        if !Self::is_pointer_type(ty) {
            return Nullability::NonNull;
        }

        match &expr.kind {
            // &expr is always non-nil
            ExprKind::Unary {
                op: UnaryOp::Ref, ..
            } => Nullability::NonNull,

            // new(T) is always non-nil
            ExprKind::Call {
                func, type_args, ..
            } => {
                if let ExprKind::Ident(name) = &func.kind
                    && name == "new"
                    && !type_args.is_empty()
                {
                    return Nullability::NonNull;
                }
                // Other function calls returning pointers are nullable
                Nullability::Nullable
            }

            // .(!nil) assertion explicitly marks as non-nil
            ExprKind::NilAssert { .. } => Nullability::NonNull,

            // Variable reference: look up its tracked nil state
            ExprKind::Ident(name) => self.get_nil_state(name),

            // All other expressions producing pointers are conservatively nullable
            _ => Nullability::Nullable,
        }
    }

    /// Update nil state for a variable after assignment
    pub(super) fn update_nil_state_for_assignment(
        &mut self,
        name: &str,
        value: &Expr,
        value_ty: &Type,
    ) {
        if Self::is_pointer_type(value_ty) {
            let nullability = self.get_expr_nullability(value, value_ty);
            self.set_nil_state(name.to_string(), nullability);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{Decl, FileId, Parser};

    #[test]
    fn test_import_tracking() {
        let source = r#"
            import "fmt"
            import "strings"

            func test() {
                fmt.Println("hello")
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        // Process imports
        infer.process_imports(&file.imports);

        // Check that packages are tracked
        assert!(infer.is_imported_package("fmt"));
        assert!(infer.is_imported_package("strings"));
        assert!(!infer.is_imported_package("os")); // not imported
    }

    #[test]
    fn test_go_package_field_access_stdlib() {
        let source = r#"
            import "fmt"

            func test() {
                fmt.Println("hello")
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();
        let mut infer = Infer::new().unwrap();

        // Process imports
        infer.process_imports(&file.imports);

        // Infer the function - should work with stdlib
        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                // This should succeed - fmt.Println is a valid Go function
                assert!(infer.infer_func_decl(func).is_ok());
            }
        }
    }
}
