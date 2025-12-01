mod decl;
mod expr;
mod pat;
mod stmt;
mod unify;

use std::collections::HashMap;

use super::ctx::GlobalCtxt;
use super::symbols::{SymbolInfo, SymbolKind, SymbolTable};
use super::ty::{Nullability, Type};
use crate::error::{Result, SoppoError};
use crate::go::{GoCache, Project, parse_go_type};
use crate::syntax::{
    Expr, ExprKind, Import, ModuleId, Span, Symbol, TypeAnnotation as AstType, UnaryOp,
};

/// Result of looking up a symbol in a Soppo module.
/// Contains: (type, definition_span, name_span, doc_comment)
type LookupResult = Option<(Type, Option<Span>, Option<Span>, Option<String>)>;

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

    /// Current scope: variable name -> (type, definition span)
    pub(super) scopes: Vec<HashMap<String, (Type, Option<Span>)>>,

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

    /// Symbol table for LSP features (hover, go-to-definition)
    /// Maps spans to symbol information
    pub(super) symbols: SymbolTable,
}

impl Infer {
    /// Create base Infer with given global state, project, and cache
    fn base(global_state: GlobalCtxt, project: Option<Project>, go_cache: GoCache) -> Self {
        Self {
            global_state,
            scopes: vec![HashMap::new()],
            substitutions: HashMap::new(),
            next_var: 0,
            expected_return_types: None,
            generic_params: HashMap::new(),
            go_cache,
            project,
            imported_packages: HashMap::new(),
            soppo_imports: HashMap::new(),
            nil_state: vec![HashMap::new()],
            symbols: SymbolTable::new(),
        }
    }

    /// Create a new type inference engine
    ///
    /// Go stdlib resolution is always enabled.
    /// For external module resolution, use `with_project`.
    pub fn new() -> miette::Result<Self> {
        Ok(Self::base(GlobalCtxt::new(), None, GoCache::new()?))
    }

    /// Create an Infer with project context for external module resolution
    pub fn with_project(project: Project) -> miette::Result<Self> {
        Ok(Self::base(
            GlobalCtxt::new(),
            Some(project),
            GoCache::new()?,
        ))
    }

    /// Create an Infer with existing GlobalCtxt for multi-file compilation
    pub fn with_global_state(global_state: GlobalCtxt) -> miette::Result<Self> {
        Ok(Self::base(global_state, None, GoCache::new()?))
    }

    /// Create an Infer with existing GlobalCtxt and project context
    pub fn with_global_state_and_project(
        global_state: GlobalCtxt,
        project: Project,
    ) -> miette::Result<Self> {
        Ok(Self::base(global_state, Some(project), GoCache::new()?))
    }

    /// Consume the Infer and return the global context
    pub fn into_global_state(self) -> GlobalCtxt {
        self.global_state
    }

    /// Consume the Infer and return the symbol table
    pub fn into_symbols(mut self) -> SymbolTable {
        // Copy soppo imports to symbol table for cross-file completion
        for (alias, module_id) in self.soppo_imports {
            self.symbols.add_import(alias, module_id);
        }
        self.symbols
    }

    /// Get a reference to the symbol table
    pub fn symbols(&self) -> &SymbolTable {
        &self.symbols
    }

    /// Record a symbol at the given span
    ///
    /// - `span`: The span of the symbol reference (where it's used)
    /// - `name`: The symbol's name
    /// - `ty`: The symbol's type
    /// - `definition_span`: The full declaration span (e.g., entire `func foo(x int) int`)
    /// - `name_span`: Just the identifier span (e.g., just `foo`) for goto-definition highlighting
    /// - `kind`: The kind of symbol
    /// - `doc_comment`: Optional documentation comment
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_symbol(
        &mut self,
        span: Span,
        name: String,
        ty: Type,
        definition_span: Option<Span>,
        name_span: Option<Span>,
        kind: SymbolKind,
        doc_comment: Option<String>,
    ) {
        self.symbols.record(
            span,
            SymbolInfo {
                name,
                ty,
                definition_span,
                name_span,
                kind,
                doc_comment,
            },
        );
    }

    /// Record a type annotation as a symbol for LSP hover/goto-definition
    /// This handles type references in declarations like `var x int`, `func(a string)`, etc.
    /// Recursively processes composite types like `*MyType`, `[]MyType`, `map[K]V`, etc.
    pub(super) fn record_type_annotation(&mut self, ty: &crate::syntax::TypeAnnotation) {
        let type_name = &ty.name;

        // For composite types (pointer, slice, map, channel, variadic, func),
        // the inner types are stored in args - process them recursively
        // The main type name (e.g., "*MyType") doesn't need recording since it's not a definition
        let is_composite = type_name.starts_with('*')
            || type_name.starts_with("[]")
            || type_name.starts_with("map[")
            || type_name.starts_with("chan ")
            || type_name.starts_with("...")
            || type_name.starts_with("func(")
            || type_name.starts_with("struct {");

        if is_composite {
            // Recurse into inner types
            for arg in &ty.args {
                self.record_type_annotation(arg);
            }
            return;
        }

        // Skip built-in/primitive types (no definition to go to)
        if is_primitive_type(type_name) {
            return;
        }

        // Handle qualified types like pkg.Type - extract the base type name
        let base_name = if let Some(dot_idx) = type_name.find('.') {
            &type_name[dot_idx + 1..]
        } else {
            type_name.as_str()
        };

        // Look up user-defined type
        if let Some(type_def) = self.global_state.lookup_type(base_name).cloned() {
            self.record_symbol(
                ty.span,
                type_name.clone(),
                Type::simple(base_name),
                type_def.span,
                type_def.name_span,
                SymbolKind::Type,
                type_def.doc_comment,
            );
        }

        // Also recurse into generic type arguments (e.g., Option[MyType])
        for arg in &ty.args {
            self.record_type_annotation(arg);
        }
    }

    /// Process imports and add package names to scope
    ///
    /// Imports are classified as Soppo or Go based on whether the import path:
    /// 1. Starts with the project's module path
    /// 2. Corresponds to a local directory with .sop files
    pub fn process_imports(&mut self, imports: &[Import]) {
        for import in imports {
            let import_path = import.path.trim_matches('"');

            // Check if this is a local Soppo package and get the local path if so
            let soppo_local_path = self.project.as_ref().and_then(|project| {
                if crate::deps::is_soppo_import(import_path, &project.module_path, &project.root) {
                    crate::deps::get_local_package_path(import_path, &project.module_path)
                } else {
                    None
                }
            });

            if let Some(local_path) = soppo_local_path {
                // Use alias if provided, otherwise derive from path
                let package_name = import
                    .alias
                    .as_deref()
                    .unwrap_or_else(|| local_path.rsplit('/').next().unwrap_or(local_path));

                // Track the Soppo import with its ModuleId for cross-package lookups
                // The local_path is the module ID (e.g., "helpers" or "util/helpers")
                let module_id = ModuleId::new(local_path);
                self.soppo_imports
                    .insert(package_name.to_string(), module_id.clone());

                // Also register in GlobalCtxt for codegen to access
                self.global_state
                    .register_soppo_import(package_name.to_string(), module_id);

                // Also track in imported_packages for `is_imported_package` checks
                self.imported_packages
                    .insert(package_name.to_string(), import_path.to_string());

                // Add package name to scope with a special "soppo_package" type
                self.insert_var(
                    package_name.to_string(),
                    Type::simple("soppo_package"),
                    None,
                );
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

                // Check if Go package has soppo type markers and register them
                if let Ok(pkg) = self
                    .go_cache
                    .get_or_parse(import_path, self.project.as_ref())
                {
                    for (type_name, soppo_type) in &pkg.soppo_types {
                        let kind = match soppo_type.kind.as_str() {
                            "enum" => crate::types::ctx::GoSoppoKind::Enum,
                            "nilable" => crate::types::ctx::GoSoppoKind::Nilable,
                            _ => continue,
                        };
                        self.global_state
                            .register_go_soppo_type(package_name, type_name, kind);
                    }
                }

                // Add package name to scope with a special "package" type
                // This allows field access like fmt.Printf to work
                self.insert_var(package_name.to_string(), Type::simple("package"), None);
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

    /// Look up a function in an imported Soppo module.
    /// Returns the function type, definition span, name span (for go-to-definition), and doc comment.
    pub(super) fn lookup_soppo_function(
        &self,
        package_name: &str,
        func_name: &str,
    ) -> LookupResult {
        // Get the ModuleId for this package
        let module_id = self.soppo_imports.get(package_name)?;

        // Look up the function in GlobalCtxt
        let func_def = self.global_state.lookup_function_in(module_id, func_name)?;

        // Convert FuncDef to Type::Fun
        let param_types: Vec<(Option<String>, Type)> = func_def
            .params
            .iter()
            .map(|(name, ty)| (Some(name.clone()), ty.clone()))
            .collect();

        let return_type = if func_def.return_types.is_empty() {
            Type::unit()
        } else if func_def.return_types.len() == 1 {
            func_def.return_types[0].clone()
        } else {
            // Multiple return types - use a tuple type
            Type::generic("tuple", func_def.return_types.clone())
        };

        Some((
            Type::fun_named(param_types, return_type),
            func_def.span,
            func_def.name_span,
            func_def.doc_comment.clone(),
        ))
    }

    /// Look up a type in an imported Soppo module.
    /// Returns the type, definition span, name span (for go-to-definition), and doc comment.
    pub(super) fn lookup_soppo_type(&self, package_name: &str, type_name: &str) -> LookupResult {
        // Get the ModuleId for this package
        let module_id = self.soppo_imports.get(package_name)?;

        // Look up the type in GlobalCtxt
        let type_def = self.global_state.lookup_type_in(module_id, type_name)?;

        // Return the type as a simple type constructor
        Some((
            Type::simple(&type_def.name),
            type_def.span,
            type_def.name_span,
            type_def.doc_comment.clone(),
        ))
    }

    /// Look up a constant in an imported Soppo module.
    /// Returns the constant type, definition span, name span (for go-to-definition), and doc comment.
    pub(super) fn lookup_soppo_constant(
        &self,
        package_name: &str,
        const_name: &str,
    ) -> LookupResult {
        // Get the ModuleId for this package
        let module_id = self.soppo_imports.get(package_name)?;

        // Look up the constant in GlobalCtxt
        let const_def = self
            .global_state
            .lookup_constant_in(module_id, const_name)?;

        Some((
            const_def.ty.clone(),
            const_def.span,
            const_def.name_span,
            const_def.doc_comment.clone(),
        ))
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
        // Apply module to parameter types as well for proper unification
        let param_types: Vec<Type> = func_def
            .params
            .iter()
            .map(|p| Self::parse_go_type_with_module(&p.ty, package_name))
            .collect();

        let return_type = if func_def.return_type.is_empty() {
            Type::unit()
        } else {
            // Parse the return type and set module for types from this package
            Self::parse_go_type_with_module(&func_def.return_type, package_name)
        };

        Some(Type::fun(param_types, return_type))
    }

    /// Parse a Go type string and set the module for types that are from the given package
    fn parse_go_type_with_module(type_str: &str, package_name: &str) -> Type {
        let ty = parse_go_type(type_str);
        Self::set_module_recursive(ty, package_name)
    }

    /// Recursively set the module on types that don't already have one
    fn set_module_recursive(ty: Type, package_name: &str) -> Type {
        match ty {
            Type::Con {
                sym: name,
                args,
                nullable,
            } => {
                // Set module if empty and this looks like a type from the package
                // (not a built-in like int, string, etc.)
                let is_builtin = matches!(
                    name.name.as_str(),
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
                        | "float32"
                        | "float64"
                        | "complex64"
                        | "complex128"
                        | "bool"
                        | "string"
                        | "byte"
                        | "rune"
                        | "error"
                        | "any"
                );

                // Don't set module on:
                // - Builtin types (int, string, etc.)
                // - Slice types ([]T)
                // - Map types (map[K]V)
                // - Channel types (chan T)
                // - Pointer types (*T) - module goes on inner type via args recursion
                let new_module = if name.module.0.is_empty()
                    && !is_builtin
                    && !name.name.starts_with("[]")
                    && !name.name.starts_with("map[")
                    && !name.name.starts_with("chan ")
                    && !name.name.starts_with('*')
                {
                    ModuleId::new(package_name)
                } else {
                    name.module
                };

                Type::Con {
                    sym: Symbol {
                        module: new_module,
                        name: name.name,
                        span: name.span,
                    },
                    args: args
                        .into_iter()
                        .map(|a| Self::set_module_recursive(a, package_name))
                        .collect(),
                    nullable,
                }
            }
            Type::Func {
                args,
                ret,
                nullable,
            } => Type::Func {
                args: args
                    .into_iter()
                    .map(|(name, ty)| (name, Self::set_module_recursive(ty, package_name)))
                    .collect(),
                ret: Box::new(Self::set_module_recursive(*ret, package_name)),
                nullable,
            },
            other => other,
        }
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

        // Check if it's a regular Go type
        if pkg.types.contains_key(type_name) {
            return Some(Type::Con {
                sym: Symbol {
                    module: ModuleId::new(package_name),
                    name: type_name.to_string(),
                    span: Span::dummy(),
                },
                args: vec![],
                nullable: false,
            });
        }

        // Check if it's a Soppo type (from //soppo:enum markers)
        if pkg.soppo_types.contains_key(type_name) {
            return Some(Type::Con {
                sym: Symbol {
                    module: ModuleId::new(package_name),
                    name: type_name.to_string(),
                    span: Span::dummy(),
                },
                args: vec![],
                nullable: false,
            });
        }

        // Check if it's a constant
        if let Some(const_def) = pkg.constants.get(type_name) {
            let const_ty = &const_def.ty;
            // If the constant's type is a type defined in this package, return it with module info
            if pkg.types.contains_key(const_ty) {
                return Some(Type::Con {
                    sym: Symbol {
                        module: ModuleId::new(package_name),
                        name: const_ty.to_string(),
                        span: Span::dummy(),
                    },
                    args: vec![],
                    nullable: false,
                });
            }
            // Otherwise parse it as a Go type (for primitive types, etc.)
            return Some(parse_go_type(const_ty));
        }

        // Check if it's a variable (e.g., os.Stdin, os.Stdout)
        if let Some(var_def) = pkg.variables.get(type_name) {
            return Some(parse_go_type(&var_def.ty));
        }

        None
    }

    /// Look up a field in a Go struct type
    /// Returns the field type with nullable set based on //soppo:nilable marker
    pub(super) fn lookup_go_struct_field(
        &mut self,
        package_name: &str,
        type_name: &str,
        field_name: &str,
    ) -> Option<Type> {
        let import_path = self.imported_packages.get(package_name)?.clone();

        let pkg = self
            .go_cache
            .get_or_parse(&import_path, self.project.as_ref())
            .ok()?;

        // Get the type definition
        let type_def = pkg.types.get(type_name)?;

        // Only works for structs
        if type_def.kind != "struct" {
            return None;
        }

        // Find the field
        let field = type_def.fields.iter().find(|f| f.name == field_name)?;

        // Parse the Go type
        let mut field_ty = parse_go_type(&field.ty);

        // If this is a soppo-generated package, apply nullable info
        // In soppo-generated code:
        // - Fields WITH //soppo:nilable are nullable
        // - Fields WITHOUT the marker are NOT nullable (even if pointer type)
        // In regular Go code:
        // - All pointer types are considered nullable
        if pkg.soppo_generated {
            // Only mark as nullable if the field has the marker
            if let Type::Con { nullable, .. } = &mut field_ty {
                *nullable = field.nullable;
            }
        }
        // For non-soppo-generated Go code, parse_go_type already handles
        // pointer types as nullable

        Some(field_ty)
    }

    /// Look up a method on a Go type
    /// Returns the method type as a function type
    pub(super) fn lookup_go_method(
        &mut self,
        package_name: &str,
        type_name: &str,
        method_name: &str,
    ) -> Option<Type> {
        let import_path = self.imported_packages.get(package_name)?.clone();

        let pkg = self
            .go_cache
            .get_or_parse(&import_path, self.project.as_ref())
            .ok()?;

        // Look up methods for this type (methods are stored by base type name without *)
        let base_type = type_name.strip_prefix('*').unwrap_or(type_name);
        let methods = pkg.methods.get(base_type)?;

        // Find the method
        let method = methods.iter().find(|m| m.name == method_name)?;

        // Build the function type
        let param_tys: Vec<Type> = method.params.iter().map(|p| parse_go_type(&p.ty)).collect();
        let return_ty = if method.return_type.is_empty() {
            Type::unit()
        } else {
            parse_go_type(&method.return_type)
        };

        Some(Type::fun(param_tys, return_ty))
    }

    /// Check if a name refers to an imported Go package
    pub(super) fn is_imported_package(&self, name: &str) -> bool {
        self.imported_packages.contains_key(name)
    }

    /// Check if a type is an interface from a Go package
    /// Returns true if the type is defined as an interface in its source package
    pub(super) fn is_go_interface_type(&mut self, ty: &Type) -> bool {
        let (type_name, package_name) = match ty {
            Type::Con { sym: name, .. } => {
                // Skip built-in types that aren't from a package
                let pkg = if name.module.0.is_empty() {
                    return false;
                } else {
                    name.module.0.clone()
                };
                // Strip nullable prefix if present
                let ty_name = name
                    .name
                    .strip_prefix('?')
                    .unwrap_or(&name.name)
                    .to_string();
                (ty_name, pkg)
            }
            _ => return false,
        };

        // Get the import path for this package
        // First check if it's explicitly imported, otherwise try the package name directly
        // (for stdlib packages that are dependencies of imported packages)
        let import_path = self
            .imported_packages
            .get(&package_name)
            .cloned()
            .unwrap_or_else(|| package_name.clone());

        // Try to get the package info
        let pkg = match self
            .go_cache
            .get_or_parse(&import_path, self.project.as_ref())
        {
            Ok(pkg) => pkg,
            Err(_) => return false,
        };

        // Look up the type and check if it's an interface
        if let Some(type_def) = pkg.types.get(&type_name) {
            return type_def.kind == "interface";
        }

        false
    }

    /// Check if a type is a user-defined interface in the current module
    pub(super) fn is_soppo_interface_type(&self, ty: &Type) -> bool {
        let type_name = match ty {
            Type::Con { sym: name, .. } => {
                // Strip nullable prefix if present
                name.name
                    .strip_prefix('?')
                    .unwrap_or(&name.name)
                    .to_string()
            }
            _ => return false,
        };

        // Look up in current module's types
        if let Some(type_def) = self.global_state.current_module().types.get(&type_name) {
            return matches!(
                type_def.kind,
                crate::types::ctx::TypeDefKind::Interface { .. }
            );
        }

        false
    }

    /// Get the interface methods for a type if it's an interface
    pub(super) fn get_interface_methods(
        &self,
        ty: &Type,
    ) -> Option<Vec<crate::types::ctx::MethodSig>> {
        let type_name = match ty {
            Type::Con { sym: name, .. } => {
                // Strip nullable prefix if present
                name.name
                    .strip_prefix('?')
                    .unwrap_or(&name.name)
                    .to_string()
            }
            _ => return None,
        };

        // Look up in current module's types
        if let Some(type_def) = self.global_state.current_module().types.get(&type_name)
            && let crate::types::ctx::TypeDefKind::Interface { methods } = &type_def.kind
        {
            return Some(methods.clone());
        }

        None
    }

    /// Check if a concrete type satisfies an interface (has all required methods)
    pub(super) fn type_satisfies_interface(&self, concrete_ty: &Type, interface_ty: &Type) -> bool {
        // Get the interface methods
        let interface_methods = match self.get_interface_methods(interface_ty) {
            Some(methods) => methods,
            None => return false,
        };

        // Get the concrete type name
        let concrete_name = match concrete_ty {
            Type::Con { sym: name, .. } => {
                // Strip nullable and pointer prefix if present
                let type_name = name.name.strip_prefix('?').unwrap_or(&name.name);
                type_name.strip_prefix('*').unwrap_or(type_name).to_string()
            }
            _ => return false,
        };

        // Get methods defined on the concrete type
        let module = self.global_state.current_module();
        let type_methods = match module.methods.get(&concrete_name) {
            Some(methods) => methods,
            None => return interface_methods.is_empty(), // No methods, only satisfies empty interface
        };

        // Check each interface method is implemented
        for required_method in &interface_methods {
            match type_methods.get(&required_method.name) {
                Some(impl_method) => {
                    // Check parameter count matches (excluding receiver)
                    if impl_method.params.len() != required_method.params.len() {
                        return false;
                    }

                    // Check return type count matches
                    if impl_method.return_types.len() != required_method.returns.len() {
                        return false;
                    }

                    // Note: We do a basic check here. A full implementation would
                    // unify parameter and return types for exact matching.
                }
                None => return false, // Required method not found
            }
        }

        true
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
            Type::Con { sym: name, .. } => {
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
            Type::Con { sym: name, .. } => {
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

        // Handle variadic types: ...T -> variadic[T]
        if let Some(inner_name) = ast_ty.name.strip_prefix("...") {
            let inner_ty = if let Some(dot_idx) = inner_name.find('.') {
                // Qualified variadic type: ...pkg.Type
                let pkg = &inner_name[..dot_idx];
                let type_name = &inner_name[dot_idx + 1..];
                Type::Con {
                    sym: Symbol {
                        module: ModuleId::new(pkg),
                        name: type_name.to_string(),
                        span: ast_ty.span,
                    },
                    args: vec![],
                    nullable: false,
                }
            } else {
                Type::simple(inner_name)
            };
            return Type::generic("variadic", vec![inner_ty]);
        }

        // Handle function types: func(A, B) C -> Type::Fun
        if ast_ty.name.starts_with("func(") {
            // The args contain param types followed by return types
            // Parse the name to figure out how many are params vs returns
            // Format: "func(A, B) C" or "func(A, B) (C, D)" or "func(A, B)"
            let after_func = &ast_ty.name[4..]; // Skip "func"

            // Find matching paren for params
            let mut paren_depth = 0;
            let mut params_end = 0;
            for (i, c) in after_func.chars().enumerate() {
                match c {
                    '(' => paren_depth += 1,
                    ')' => {
                        paren_depth -= 1;
                        if paren_depth == 0 {
                            params_end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            // Count params by counting commas + 1 (if non-empty)
            let params_str = &after_func[1..params_end];
            let num_params = if params_str.trim().is_empty() {
                0
            } else {
                params_str.matches(',').count() + 1
            };

            // Split args into params and returns
            let resolved_args: Vec<Type> =
                ast_ty.args.iter().map(|a| self.resolve_type(a)).collect();
            let (param_types, return_types): (Vec<_>, Vec<_>) = resolved_args
                .into_iter()
                .enumerate()
                .partition(|(i, _)| *i < num_params);
            let param_types: Vec<Type> = param_types.into_iter().map(|(_, t)| t).collect();
            let return_types: Vec<Type> = return_types.into_iter().map(|(_, t)| t).collect();

            let ret_ty = if return_types.is_empty() {
                Type::unit()
            } else if return_types.len() == 1 {
                return_types.into_iter().next().unwrap()
            } else {
                Type::generic("tuple", return_types)
            };

            // Convert to named params (with no names, from AST)
            let named_params: Vec<(Option<String>, Type)> =
                param_types.into_iter().map(|ty| (None, ty)).collect();

            return Type::Func {
                args: named_params,
                ret: Box::new(ret_ty),
                nullable: ast_ty.nullable,
            };
        }

        // Handle pointer types: *T, *pkg.Type
        if let Some(inner_name) = ast_ty.name.strip_prefix('*') {
            // Check if inner type is a generic parameter
            if let Some(ty_var) = self.generic_params.get(inner_name) {
                // Generic pointer type: *T where T is a type parameter
                return Type::Con {
                    sym: Symbol {
                        module: ModuleId::empty(),
                        name: format!("*{}", inner_name),
                        span: ast_ty.span,
                    },
                    args: vec![ty_var.clone()],
                    nullable: ast_ty.nullable,
                };
            }

            // Parse the inner type
            if let Some(dot_idx) = inner_name.find('.') {
                // Qualified pointer type: *pkg.Type
                let pkg = &inner_name[..dot_idx];
                let type_name = &inner_name[dot_idx + 1..];

                // Create inner type with module
                let inner_ty = Type::Con {
                    sym: Symbol {
                        module: ModuleId::new(pkg),
                        name: type_name.to_string(),
                        span: ast_ty.span,
                    },
                    args: vec![],
                    nullable: false,
                };

                // Create pointer type
                return Type::Con {
                    sym: Symbol {
                        module: ModuleId::empty(),
                        name: format!("*{}", type_name),
                        span: ast_ty.span,
                    },
                    args: vec![inner_ty],
                    nullable: ast_ty.nullable,
                };
            }
        }

        // Handle slice types: []T
        if let Some(inner_name) = ast_ty.name.strip_prefix("[]") {
            // Check if inner type is a generic parameter
            if let Some(ty_var) = self.generic_params.get(inner_name) {
                return Type::Con {
                    sym: Symbol {
                        module: ModuleId::empty(),
                        name: format!("[]{}", inner_name),
                        span: ast_ty.span,
                    },
                    args: vec![ty_var.clone()],
                    nullable: ast_ty.nullable,
                };
            }
        }

        // Handle channel types: chan T
        if let Some(inner_name) = ast_ty.name.strip_prefix("chan ") {
            // Check if inner type is a generic parameter
            if let Some(ty_var) = self.generic_params.get(inner_name) {
                return Type::Con {
                    sym: Symbol {
                        module: ModuleId::empty(),
                        name: format!("chan {}", inner_name),
                        span: ast_ty.span,
                    },
                    args: vec![ty_var.clone()],
                    nullable: ast_ty.nullable,
                };
            }
        }

        // Handle non-pointer qualified types: pkg.Type
        if !ast_ty.name.starts_with('*')
            && !ast_ty.name.starts_with("[]")
            && !ast_ty.name.starts_with("map[")
            && let Some(dot_idx) = ast_ty.name.find('.')
        {
            let pkg = &ast_ty.name[..dot_idx];
            let type_name = &ast_ty.name[dot_idx + 1..];

            return Type::Con {
                sym: Symbol {
                    module: ModuleId::new(pkg),
                    name: type_name.to_string(),
                    span: ast_ty.span,
                },
                args: ast_ty
                    .args
                    .iter()
                    .map(|arg| self.resolve_type(arg))
                    .collect(),
                nullable: ast_ty.nullable,
            };
        }

        // Not a generic param - create a concrete type
        // Recursively resolve type arguments
        let args: Vec<Type> = ast_ty
            .args
            .iter()
            .map(|arg| self.resolve_type(arg))
            .collect();

        Type::Con {
            sym: Symbol {
                module: ModuleId::empty(),
                name: ast_ty.name.clone(),
                span: ast_ty.span,
            },
            args,
            nullable: ast_ty.nullable,
        }
    }

    /// Validate that a type annotation refers to a real type.
    /// This catches errors like `Option.None[String]` where `String` isn't a valid Go type.
    pub(super) fn validate_type_arg(&self, ast_ty: &AstType) -> Result<()> {
        let name = &ast_ty.name;

        // Generic parameters are always valid (they're checked elsewhere)
        if self.generic_params.contains_key(name) {
            return Ok(());
        }

        // Primitive types are valid
        if is_primitive_type(name) {
            return Ok(());
        }

        // Qualified types (pkg.Type) are assumed valid (external types)
        if name.contains('.') {
            // Recursively validate type arguments
            for arg in &ast_ty.args {
                self.validate_type_arg(arg)?;
            }
            return Ok(());
        }

        // Constructed types - validate inner types recursively
        if name.starts_with('*')
            || name.starts_with("[]")
            || name.starts_with("map[")
            || name.starts_with("chan ")
            || name.starts_with("func(")
            || name.starts_with("...")
        {
            for arg in &ast_ty.args {
                self.validate_type_arg(arg)?;
            }
            return Ok(());
        }

        // Check if it's a known type in the current module
        if self.global_state.lookup_type(name).is_some() {
            // Recursively validate type arguments
            for arg in &ast_ty.args {
                self.validate_type_arg(arg)?;
            }
            return Ok(());
        }

        // Check if it's a known type in any imported Soppo module
        for module_id in self.soppo_imports.values() {
            if self.global_state.lookup_type_in(module_id, name).is_some() {
                for arg in &ast_ty.args {
                    self.validate_type_arg(arg)?;
                }
                return Ok(());
            }
        }

        // Not a valid type
        Err(SoppoError::Type {
            message: format!("cannot find type `{}` in this scope", name),
            span: ast_ty.span,
        })
    }

    /// Instantiate a Type by substituting generic parameters with concrete types
    /// This handles composite types like *T, []T, map[K]V, chan T, and func types
    pub(super) fn instantiate_generic_type(ty: &Type, subst: &HashMap<String, Type>) -> Type {
        match ty {
            Type::Con {
                sym: name,
                args,
                nullable,
            } => {
                // Check if the whole name is a generic param (e.g., T)
                if let Some(concrete) = subst.get(&name.name) {
                    let mut result = concrete.clone();
                    // Preserve nullability
                    if *nullable {
                        result = result.as_nullable();
                    }
                    return result;
                }

                // Handle pointer types: *T
                if let Some(inner) = name.name.strip_prefix('*')
                    && let Some(concrete) = subst.get(inner)
                {
                    let ptr_name = format!("*{}", concrete);
                    return Type::Con {
                        sym: Symbol {
                            module: ModuleId::empty(),
                            name: ptr_name,
                            span: name.span,
                        },
                        args: vec![concrete.clone()],
                        nullable: *nullable,
                    };
                }

                // Handle slice types: []T
                if let Some(inner) = name.name.strip_prefix("[]")
                    && let Some(concrete) = subst.get(inner)
                {
                    let slice_name = format!("[]{}", concrete);
                    return Type::Con {
                        sym: Symbol {
                            module: ModuleId::empty(),
                            name: slice_name,
                            span: name.span,
                        },
                        args: vec![concrete.clone()],
                        nullable: *nullable,
                    };
                }

                // Handle channel types: chan T
                if let Some(inner) = name.name.strip_prefix("chan ")
                    && let Some(concrete) = subst.get(inner)
                {
                    let chan_name = format!("chan {}", concrete);
                    return Type::Con {
                        sym: Symbol {
                            module: ModuleId::empty(),
                            name: chan_name,
                            span: name.span,
                        },
                        args: vec![concrete.clone()],
                        nullable: *nullable,
                    };
                }

                // Recursively instantiate type arguments
                let new_args: Vec<Type> = args
                    .iter()
                    .map(|arg| Self::instantiate_generic_type(arg, subst))
                    .collect();

                Type::Con {
                    sym: name.clone(),
                    args: new_args,
                    nullable: *nullable,
                }
            }
            Type::Func {
                args,
                ret,
                nullable,
            } => {
                let new_args: Vec<(Option<String>, Type)> = args
                    .iter()
                    .map(|(name, ty)| (name.clone(), Self::instantiate_generic_type(ty, subst)))
                    .collect();
                let new_ret = Self::instantiate_generic_type(ret, subst);
                Type::Func {
                    args: new_args,
                    ret: Box::new(new_ret),
                    nullable: *nullable,
                }
            }
            Type::Var(_) | Type::Never => ty.clone(),
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
    pub(super) fn insert_var(&mut self, name: String, ty: Type, def_span: Option<Span>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, (ty, def_span));
        }
    }

    /// Lookup a variable in scopes (from innermost to outermost)
    /// Returns the type and optionally the definition span
    pub(super) fn lookup_var(&self, name: &str) -> Option<(Type, Option<Span>)> {
        for scope in self.scopes.iter().rev() {
            if let Some((ty, span)) = scope.get(name) {
                return Some((ty.clone(), *span));
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

    /// Check if a type is nilable (can hold nil in Go)
    /// This includes: pointers, slices, maps, channels, functions, and interfaces
    pub(super) fn is_nilable_type(ty: &Type) -> bool {
        match ty {
            Type::Con { sym: name, .. } => {
                let ty_name = &name.name;
                // Pointers: *T or ptr[T]
                ty_name.starts_with('*')
                    || ty_name == "ptr"
                    // Slices: []T
                    || ty_name.starts_with("[]")
                    // Maps: map[K]V or map
                    || ty_name.starts_with("map")
                    // Channels: chan T
                    || ty_name.starts_with("chan ")
                    // Functions: func(...) or fn(...)
                    || ty_name.starts_with("func")
                    || ty_name.starts_with("fn")
                    // Interfaces: interface{} or common interface types
                    || ty_name == "interface"
                    || ty_name == "any"
                    || ty_name == "error"
            }
            Type::Func { .. } => true, // Function types are nilable
            _ => false,
        }
    }

    /// Check if a type is a pointer type (*T or ptr)
    pub(super) fn is_pointer_type(ty: &Type) -> bool {
        matches!(ty, Type::Con { sym, .. } if sym.name.starts_with('*') || sym.name == "ptr")
    }

    /// Check if a type is a slice type ([]T)
    pub(super) fn is_slice_type(ty: &Type) -> bool {
        matches!(ty, Type::Con { sym, .. } if sym.name.starts_with("[]"))
    }

    /// Check if a type is a map type (map[K]V)
    pub(super) fn is_map_type(ty: &Type) -> bool {
        matches!(ty, Type::Con { sym, .. } if sym.name.starts_with("map["))
    }

    /// Check if a type is a channel type (chan T)
    pub(super) fn is_channel_type(ty: &Type) -> bool {
        matches!(ty, Type::Con { sym, .. } if sym.name.starts_with("chan "))
    }

    /// Extract element type from a channel type (chan T -> T)
    /// Returns None if not a channel type
    pub(super) fn extract_channel_element(ty: &Type) -> Option<Type> {
        match ty {
            Type::Con {
                sym: name, args, ..
            } if name.name.starts_with("chan ") => {
                if !args.is_empty() {
                    Some(args[0].clone())
                } else {
                    // Fallback: parse from name "chan T" -> "T"
                    Some(Type::simple(&name.name[5..]))
                }
            }
            _ => None,
        }
    }

    /// Extract element type from a slice type ([]T -> T)
    /// Returns None if not a slice type
    pub(super) fn extract_slice_element(ty: &Type) -> Option<Type> {
        match ty {
            Type::Con {
                sym: name, args, ..
            } if name.name.starts_with("[]") => {
                if !args.is_empty() {
                    Some(args[0].clone())
                } else {
                    // Fallback: parse from name "[]T" -> "T"
                    Some(Type::simple(&name.name[2..]))
                }
            }
            _ => None,
        }
    }

    /// Extract key and value types from a map type (map[K]V -> (K, V))
    /// Returns None if not a map type
    pub(super) fn extract_map_elements(ty: &Type) -> Option<(Type, Type)> {
        match ty {
            Type::Con {
                sym: name, args, ..
            } if name.name.starts_with("map[") => {
                if args.len() >= 2 {
                    Some((args[0].clone(), args[1].clone()))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Extract element type from pointer type (*T -> T)
    /// Returns None if not a pointer type
    pub(super) fn extract_pointer_element(ty: &Type) -> Option<Type> {
        match ty {
            Type::Con {
                sym: name, args, ..
            } if name.name.starts_with('*') => {
                if !args.is_empty() {
                    Some(args[0].clone())
                } else {
                    // Fallback: parse from name "*T" -> "T"
                    Some(Type::simple(&name.name[1..]))
                }
            }
            _ => None,
        }
    }

    /// Check if assigning nil to a target type would be an error
    /// Returns Some(error) if nil cannot be assigned, None if it's OK
    pub(super) fn check_nil_to_non_nilable(target_ty: &Type, span: Span) -> Option<SoppoError> {
        if target_ty.is_nilable_kind() && !target_ty.is_nullable() && !target_ty.is_go_interface() {
            Some(SoppoError::NilToNonNilable {
                ty: target_ty.to_string(),
                span,
            })
        } else {
            None
        }
    }

    /// Get a helpful hint message for a constraint violation
    pub(super) fn constraint_hint(constraint: &str) -> String {
        match constraint {
            "comparable" => "slices, maps, and functions are not comparable".to_string(),
            _ => format!("type must satisfy `{}`", constraint),
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
        // Only nilable types can be nullable
        if !Self::is_nilable_type(ty) {
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

            // Field access: look up the full path in nil state
            ExprKind::Field { .. } => {
                if let Some(key) = stmt::expr_to_key(expr) {
                    self.get_nil_state(&key)
                } else {
                    Nullability::Nullable
                }
            }

            // All other expressions producing pointers are conservatively nullable
            _ => Nullability::Nullable,
        }
    }

    /// Infer an expression's type with nil-state narrowing applied.
    /// If the expression is known to be non-nil (from flow analysis), a nullable type
    /// will be converted to non-nullable.
    pub(super) fn infer_expr_narrowed(&mut self, expr: &Expr) -> crate::error::Result<Type> {
        let ty = self.infer_expr(expr)?;

        // Check if this expression is known to be non-nil
        let nullability = self.get_expr_nullability(expr, &ty);
        if nullability == Nullability::NonNull && ty.is_nullable() {
            Ok(ty.as_non_nullable())
        } else {
            Ok(ty)
        }
    }

    /// Update nil state for a variable after assignment
    pub(super) fn update_nil_state_for_assignment(
        &mut self,
        name: &str,
        value: &Expr,
        value_ty: &Type,
    ) {
        if Self::is_nilable_type(value_ty) {
            let nullability = self.get_expr_nullability(value, value_ty);
            self.set_nil_state(name.to_string(), nullability);
        }
    }

    /// Check if a type is the `error` type
    pub(super) fn is_error_type(&self, ty: &Type) -> bool {
        match ty {
            Type::Con { sym: name, .. } => name.name == "error",
            _ => false,
        }
    }

    /// Check if a type returns error (either `error` or `(T, error)`)
    /// Error must be the last element for multi-return types
    pub(super) fn returns_error(&self, ty: &Type) -> bool {
        // Simple error type
        if self.is_error_type(ty) {
            return true;
        }

        // Tuple type with error as last element
        if let Type::Con {
            sym: name, args, ..
        } = ty
            && name.name == "tuple"
            && !args.is_empty()
            && let Some(last) = args.last()
        {
            return self.is_error_type(last);
        }

        false
    }

    /// Strip the error type from a tuple, returning the non-error portion
    /// For `tuple[T, error]` returns `T`
    /// For `tuple[T, U, error]` returns `tuple[T, U]`
    /// For `error` returns `()`
    /// For non-tuple types, returns the type unchanged
    pub(super) fn strip_error_from_tuple(&self, ty: &Type) -> Type {
        if self.is_error_type(ty) {
            return Type::unit();
        }

        if let Type::Con {
            sym: name, args, ..
        } = ty
            && name.name == "tuple"
            && !args.is_empty()
        {
            // Check if last element is error
            if let Some(last) = args.last()
                && self.is_error_type(last)
            {
                let non_error_args: Vec<_> = args.iter().take(args.len() - 1).cloned().collect();
                return match non_error_args.len() {
                    0 => Type::unit(),
                    1 => non_error_args.into_iter().next().unwrap(),
                    _ => Type::generic("tuple", non_error_args),
                };
            }
        }

        ty.clone()
    }

    /// Extract the variable name from a declaration or assignment statement
    pub(super) fn get_assigned_var_name(&self, stmt: &crate::syntax::Stmt) -> Option<String> {
        use crate::syntax::StmtKind;

        match &stmt.kind {
            StmtKind::Decl { ident, .. } => Some(ident.name.clone()),
            StmtKind::Assign { target, .. } => {
                if let ExprKind::Ident(name) = &target.kind {
                    Some(name.clone())
                } else {
                    None
                }
            }
            StmtKind::MultiDecl { ident: names, .. } if names.len() == 1 => {
                Some(names[0].name.clone())
            }
            _ => None,
        }
    }

    /// Check if an expression supports the comma-ok idiom and return the types
    /// Returns None if the expression doesn't support comma-ok
    /// Returns Some((value_type, bool_type)) for:
    /// - Type assertions: x.(T) -> (T, bool)
    /// - Map index: m[k] -> (V, bool) when m is a map
    /// - Channel receive: <-ch -> (T, bool)
    pub(super) fn infer_comma_ok_expr(
        &mut self,
        expr: &crate::syntax::Expr,
    ) -> crate::error::Result<Option<(Type, Type)>> {
        use crate::syntax::UnaryOp;

        match &expr.kind {
            // Type assertion: x.(T) -> (T, bool)
            ExprKind::TypeAssert { expr: _, ty } => {
                let asserted_ty = self.resolve_type(ty);
                Ok(Some((asserted_ty, Type::simple("bool"))))
            }

            // Map index: m[k] -> (V, bool) when m is a map type
            ExprKind::Index {
                expr: map_expr,
                index: _,
            } => {
                let map_ty = self.infer_expr(map_expr)?;
                let map_ty = self.substitute(map_ty);

                // Check if this is a map type
                if let Type::Con {
                    sym: name, args, ..
                } = &map_ty
                    && name.name.starts_with("map[")
                    && args.len() == 2
                {
                    // args[0] is key type, args[1] is value type
                    let value_ty = args[1].clone();
                    return Ok(Some((value_ty, Type::simple("bool"))));
                }
                // Not a map, could be slice/array index - no comma-ok
                Ok(None)
            }

            // Channel receive: <-ch -> (T, bool)
            ExprKind::Unary {
                op: UnaryOp::Recv,
                operand,
            } => {
                let chan_ty = self.infer_expr(operand)?;
                let chan_ty = self.substitute(chan_ty);

                // Extract element type from channel
                if let Type::Con {
                    sym: name, args, ..
                } = &chan_ty
                    && name.name.starts_with("chan ")
                    && args.len() == 1
                {
                    let elem_ty = args[0].clone();
                    return Ok(Some((elem_ty, Type::simple("bool"))));
                }
                // Fallback: try to extract from name
                if let Type::Con { sym: name, .. } = &chan_ty
                    && let Some(elem) = name.name.strip_prefix("chan ")
                {
                    let elem_ty = Type::simple(elem);
                    return Ok(Some((elem_ty, Type::simple("bool"))));
                }
                Ok(None)
            }

            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{Decl, FileId, Parser};

    #[test]
    fn test_is_go_interface_type() {
        let mut infer = Infer::new().unwrap();

        // Import io package to look up Reader interface
        infer
            .imported_packages
            .insert("io".to_string(), "io".to_string());

        // io.Reader is an interface
        let reader_type = Type::Con {
            sym: Symbol {
                module: ModuleId::new("io"),
                name: "Reader".to_string(),
                span: Span::dummy(),
            },
            args: vec![],
            nullable: false,
        };
        assert!(infer.is_go_interface_type(&reader_type));

        // io.Writer is an interface
        let writer_type = Type::Con {
            sym: Symbol {
                module: ModuleId::new("io"),
                name: "Writer".to_string(),
                span: Span::dummy(),
            },
            args: vec![],
            nullable: false,
        };
        assert!(infer.is_go_interface_type(&writer_type));

        // Primitive types are not interfaces
        let int_type = Type::simple("int");
        assert!(!infer.is_go_interface_type(&int_type));

        // Types without module are not checked
        let no_module = Type::simple("Reader");
        assert!(!infer.is_go_interface_type(&no_module));
    }

    #[test]
    fn test_is_go_interface_type_indirect_import() {
        let mut infer = Infer::new().unwrap();

        // Only import bufio, not io
        infer
            .imported_packages
            .insert("bufio".to_string(), "bufio".to_string());

        // io.Reader should still be detected as interface (stdlib fallback)
        let reader_type = Type::Con {
            sym: Symbol {
                module: ModuleId::new("io"),
                name: "Reader".to_string(),
                span: Span::dummy(),
            },
            args: vec![],
            nullable: false,
        };
        assert!(infer.is_go_interface_type(&reader_type));
    }

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
