use std::collections::{HashMap, HashSet};

use super::module::GlobalState;
use super::ty::Type;
use crate::error::{Result, SoppoError};
use crate::go::{GoCache, Project, parse_go_type};
use crate::parse::{
    BinOp, Block, ConstDecl, EnumVariant, Expr, ExprKind, FuncDecl, Import, ModuleId, Pattern,
    PatternKind, Span, Stmt, StmtKind, Symbol, Type as AstType, TypeDecl, TypeKind,
};

/// Check if a type name is a Go primitive/built-in type
fn is_primitive_type(ty: &str) -> bool {
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
fn is_numeric_primitive(ty: &str) -> bool {
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
    global_state: GlobalState,

    /// Current scope: variable name -> type
    scopes: Vec<HashMap<String, Type>>,

    /// Type variable substitutions (solutions)
    substitutions: HashMap<i32, Type>,

    /// Next fresh type variable ID
    next_var: i32,

    /// Expected return types for the current function (None if not in a function)
    expected_return_types: Option<Vec<Type>>,

    /// Generic type parameters in scope: param name -> type variable
    generic_params: HashMap<String, Type>,

    /// Cache for Go package information (always enabled)
    go_cache: GoCache,

    /// Current project (for external module resolution, optional)
    project: Option<Project>,

    /// Imported Go packages: short name -> import path
    /// e.g., "fmt" -> "fmt", "strings" -> "strings"
    imported_packages: HashMap<String, String>,
}

impl Infer {
    /// Create a new type inference engine
    ///
    /// Go stdlib resolution is always enabled.
    /// For external module resolution, use `with_project`.
    pub fn new() -> miette::Result<Self> {
        Ok(Self {
            global_state: GlobalState::new(),
            scopes: vec![HashMap::new()],
            substitutions: HashMap::new(),
            next_var: 0,
            expected_return_types: None,
            generic_params: HashMap::new(),
            go_cache: GoCache::new()?,
            project: None,
            imported_packages: HashMap::new(),
        })
    }

    /// Create an Infer with project context for external module resolution
    pub fn with_project(project: Project) -> miette::Result<Self> {
        Ok(Self {
            global_state: GlobalState::new(),
            scopes: vec![HashMap::new()],
            substitutions: HashMap::new(),
            next_var: 0,
            expected_return_types: None,
            generic_params: HashMap::new(),
            go_cache: GoCache::new()?,
            project: Some(project),
            imported_packages: HashMap::new(),
        })
    }

    pub fn global_state(self) -> GlobalState {
        self.global_state
    }

    /// Process imports and add package names to scope
    pub fn process_imports(&mut self, imports: &[Import]) {
        for import in imports {
            // Extract package name from import path
            // e.g., "fmt" from "fmt" or "http" from "net/http"
            let import_path = import.path.trim_matches('"');
            let package_name = import_path.rsplit('/').next().unwrap_or(import_path);

            // Track the import for later lookup
            self.imported_packages
                .insert(package_name.to_string(), import_path.to_string());

            // Add package name to scope with a special "package" type
            // This allows field access like fmt.Printf to work
            self.insert_var(package_name.to_string(), Type::simple("_package"));
        }
    }

    /// Look up a function in an imported Go package
    fn lookup_go_function(&mut self, package_name: &str, func_name: &str) -> Option<Type> {
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
    fn lookup_go_type(&mut self, package_name: &str, type_name: &str) -> Option<Type> {
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
    fn is_imported_package(&self, name: &str) -> bool {
        self.imported_packages.contains_key(name)
    }

    /// Get the ultimate underlying type for a type alias chain.
    /// For example, if we have:
    ///   type Duration int64
    ///   type MyDuration Duration
    /// Then get_underlying_type("time", "MyDuration") returns Some("int64")
    ///
    /// Returns None if the type is not found or is not an alias (e.g., struct, interface).
    fn get_underlying_type(&mut self, package_name: &str, type_name: &str) -> Option<String> {
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
    fn check_numeric_underlying_compatibility(
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
    fn resolve_type(&mut self, ast_ty: &AstType) -> Type {
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
                span: ast_ty.span.clone(),
            },
            args,
        }
    }

    /// Instantiate a type name using a substitution map
    /// If the name is in the subst map, return the substituted type variable
    /// Otherwise return the type as-is
    fn instantiate_type(&self, type_name: &str, subst: &HashMap<String, Type>) -> Type {
        if let Some(ty_var) = subst.get(type_name) {
            ty_var.clone()
        } else {
            Type::simple(type_name)
        }
    }

    /// Push a new scope
    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Pop the current scope
    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Insert a variable into the current scope
    fn insert_var(&mut self, name: String, ty: Type) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, ty);
        }
    }

    /// Lookup a variable in scopes (from innermost to outermost)
    fn lookup_var(&self, name: &str) -> Option<Type> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        None
    }

    /// Add pattern bindings to the current scope
    fn add_pattern_bindings(&mut self, pattern: &Pattern, scrutinee_ty: &Type) -> Result<()> {
        use PatternKind;

        match &pattern.kind {
            PatternKind::Default => {
                // Default doesn't bind anything
                Ok(())
            }
            PatternKind::Variant(name) => {
                // In the context of a tuple/struct pattern, this is a binding variable
                // (e.g., Ok(value) where "value" is parsed as Variant)
                // Add it to scope with the scrutinee type
                self.insert_var(name.clone(), scrutinee_ty.clone());
                Ok(())
            }
            PatternKind::Literal(_) => {
                // Literal pattern doesn't bind anything
                Ok(())
            }
            PatternKind::Destructor { name, binding } => {
                // For destructor patterns like Ok(value), extract the inner type from variant
                let variant_name = name.rsplit('.').next().unwrap_or(name);

                // Try to find the actual type from the enum variant
                if let Type::Con {
                    name: type_name, ..
                } = scrutinee_ty
                    && let Some(type_def) = self.global_state.lookup_type(&type_name.name)
                    && let super::module::TypeDefKind::Enum { variants } = &type_def.kind
                {
                    for variant in variants {
                        if let EnumVariant::Single {
                            name: vname, ty, ..
                        } = variant
                            && vname == variant_name
                        {
                            let binding_ty = Type::simple(&ty.name);
                            self.insert_var(binding.clone(), binding_ty);
                            return Ok(());
                        }
                    }
                }
                // Fallback to fresh type variable if we can't determine the type
                let binding_ty = self.fresh_ty_var();
                self.insert_var(binding.clone(), binding_ty);
                Ok(())
            }
            PatternKind::StructDestructor {
                name,
                fields,
                rest: _,
            } => {
                // For struct destructor patterns like Circle{radius: r, ...}
                let variant_name = name.rsplit('.').next().unwrap_or(name);

                // Collect field types first to avoid borrow conflicts
                let mut bindings: Vec<(String, Type)> = Vec::new();
                let mut found_variant = false;

                // Look up the struct variant to get field types
                if let Type::Con {
                    name: type_name, ..
                } = scrutinee_ty
                    && let Some(type_def) = self.global_state.lookup_type(&type_name.name)
                    && let super::module::TypeDefKind::Enum { variants } = &type_def.kind
                {
                    for variant in variants {
                        if let EnumVariant::Struct {
                            name: vname,
                            fields: variant_fields,
                            ..
                        } = variant
                            && vname == variant_name
                        {
                            found_variant = true;
                            // Collect field types
                            for (field_name, binding_name) in fields {
                                if let Some(field) =
                                    variant_fields.iter().find(|f| &f.name == field_name)
                                {
                                    let field_ty = Type::simple(&field.ty.name);
                                    bindings.push((binding_name.clone(), field_ty));
                                }
                            }
                            break;
                        }
                    }
                }

                // Insert bindings after borrows are released
                if found_variant {
                    for (binding_name, field_ty) in bindings {
                        self.insert_var(binding_name, field_ty);
                    }
                } else {
                    // Fallback: add bindings with fresh type variables
                    for (_field_name, binding_name) in fields {
                        let binding_ty = self.fresh_ty_var();
                        self.insert_var(binding_name.clone(), binding_ty);
                    }
                }
                Ok(())
            }
        }
    }

    /// Unify two types (solve constraint)
    pub fn unify(&mut self, t1: &Type, t2: &Type, span: &Span) -> Result<()> {
        let t1 = self.substitute(t1.clone());
        let t2 = self.substitute(t2.clone());

        match (&t1, &t2) {
            // Never type unifies with anything (it's bottom type)
            (Type::Never, _) | (_, Type::Never) => Ok(()),

            // Same type variable
            (Type::Var(a), Type::Var(b)) if a == b => Ok(()),

            // One is a type variable: create substitution
            (Type::Var(a), ty) | (ty, Type::Var(a)) => {
                // Occurs check: prevent infinite types like T = List[T]
                if Self::occurs(*a, ty) {
                    return Err(SoppoError::Type {
                        message: format!("Infinite type: ?{} = {}", a, ty),
                        span: span.clone(),
                    });
                }
                self.substitutions.insert(*a, ty.clone());
                Ok(())
            }

            // Compatible numeric types: allow int/int8/int16/etc. to unify
            (Type::Con { name: n1, args: a1 }, Type::Con { name: n2, args: a2 })
                if a1.is_empty()
                    && a2.is_empty()
                    && Self::are_compatible_numeric(&n1.name, &n2.name) =>
            {
                Ok(())
            }

            // Same constructor: unify arguments
            (Type::Con { name: n1, args: a1 }, Type::Con { name: n2, args: a2 })
                if n1.name == n2.name =>
            {
                if a1.len() != a2.len() {
                    return Err(SoppoError::Type {
                        message: format!(
                            "Type constructor {} has wrong number of arguments",
                            n1.name
                        ),
                        span: span.clone(),
                    });
                }
                for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                    self.unify(arg1, arg2, span)?;
                }
                Ok(())
            }

            // Functions: unify args and return
            (Type::Fun { args: a1, ret: r1 }, Type::Fun { args: a2, ret: r2 }) => {
                // Check if a1 (expected) has a variadic last parameter
                let has_variadic = a1.last().is_some_and(
                    |last| matches!(last, Type::Con { name, .. } if name.name == "variadic"),
                );

                if has_variadic {
                    // Variadic function: check non-variadic params match, then variadic can consume 0+
                    let fixed_params = &a1[..a1.len() - 1];
                    let variadic_param = a1.last().expect("checked above");

                    // Extract the variadic element type
                    let variadic_elem = if let Type::Con { args, .. } = variadic_param {
                        args.first().cloned().unwrap_or(Type::simple("any"))
                    } else {
                        Type::simple("any")
                    };

                    // Check we have at least the fixed params
                    if a2.len() < fixed_params.len() {
                        return Err(SoppoError::Type {
                            message: format!(
                                "Function has {} arguments, but expected at least {}",
                                a2.len(),
                                fixed_params.len()
                            ),
                            span: span.clone(),
                        });
                    }

                    // Unify fixed params
                    for (arg1, arg2) in fixed_params.iter().zip(a2.iter()) {
                        self.unify(arg1, arg2, span)?;
                    }

                    // Unify remaining args against variadic element type
                    for arg2 in a2.iter().skip(fixed_params.len()) {
                        // For "any" type, any argument is valid
                        if variadic_elem != Type::simple("any") {
                            self.unify(&variadic_elem, arg2, span)?;
                        }
                    }
                } else {
                    // Non-variadic function: exact arg count required
                    if a1.len() != a2.len() {
                        return Err(SoppoError::Type {
                            message: format!(
                                "Function has {} arguments, but expected {}",
                                a2.len(),
                                a1.len()
                            ),
                            span: span.clone(),
                        });
                    }
                    for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                        self.unify(arg1, arg2, span)?;
                    }
                }
                self.unify(r1, r2, span)?;
                Ok(())
            }

            // Mismatch
            _ => Err(SoppoError::TypeMismatch {
                expected: Box::new(t1.clone()),
                found: Box::new(t2.clone()),
                span: span.clone(),
            }),
        }
    }

    /// Check if two type names are compatible numeric types.
    /// In Go, numeric literals are untyped and can be assigned to any compatible numeric type.
    fn are_compatible_numeric(t1: &str, t2: &str) -> bool {
        const INT_TYPES: &[&str] = &[
            "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32",
            "uint64", "uintptr", "byte", "rune",
        ];
        const FLOAT_TYPES: &[&str] = &["float32", "float64"];

        // Both are integer types
        if INT_TYPES.contains(&t1) && INT_TYPES.contains(&t2) {
            return true;
        }

        // Both are float types
        if FLOAT_TYPES.contains(&t1) && FLOAT_TYPES.contains(&t2) {
            return true;
        }

        false
    }

    /// Check if type variable occurs in type (for occurs check)
    fn occurs(var: i32, ty: &Type) -> bool {
        match ty {
            Type::Var(v) => *v == var,
            Type::Con { args, .. } => args.iter().any(|arg| Self::occurs(var, arg)),
            Type::Fun { args, ret } => {
                args.iter().any(|arg| Self::occurs(var, arg)) || Self::occurs(var, ret)
            }
            Type::Never => false,
        }
    }

    /// Apply substitutions to a type
    pub fn substitute(&self, ty: Type) -> Type {
        match ty {
            Type::Var(v) => {
                if let Some(subst) = self.substitutions.get(&v) {
                    // Recursively substitute in case substitution contains more variables
                    self.substitute(subst.clone())
                } else {
                    Type::Var(v)
                }
            }
            Type::Con { name, args } => Type::Con {
                name,
                args: args.into_iter().map(|a| self.substitute(a)).collect(),
            },
            Type::Fun { args, ret } => Type::Fun {
                args: args.into_iter().map(|a| self.substitute(a)).collect(),
                ret: Box::new(self.substitute(*ret)),
            },
            Type::Never => Type::Never,
        }
    }

    /// Infer the type of an expression
    pub fn infer_expr(&mut self, expr: &Expr) -> Result<Type> {
        match &expr.kind {
            ExprKind::Integer(_) => Ok(Type::simple("int")),

            ExprKind::Float(_) => Ok(Type::simple("float64")),

            ExprKind::String(_) => Ok(Type::simple("string")),

            ExprKind::Bool(_) => Ok(Type::simple("bool")),

            ExprKind::Ident(name) => {
                self.lookup_var(name)
                    .ok_or_else(|| SoppoError::UndefinedVariable {
                        name: name.clone(),
                        span: expr.span.clone(),
                    })
            }

            ExprKind::Binary { op, left, right } => {
                let left_ty = self.infer_expr(left)?;
                let right_ty = self.infer_expr(right)?;

                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                        // Arithmetic: try normal unification first
                        // Point error at right operand since left is typically the "expected" type
                        if self.unify(&left_ty, &right_ty, &right.span).is_ok() {
                            return Ok(self.substitute(left_ty));
                        }

                        // If unification failed, check if we have a defined type with numeric
                        // underlying type on one side and a compatible numeric on the other.
                        // In Go: `time.Duration * int` is allowed because Duration's underlying
                        // type is int64.
                        let left_ty_sub = self.substitute(left_ty.clone());
                        let right_ty_sub = self.substitute(right_ty.clone());

                        // Try to check if types are compatible via underlying type
                        if let Some(result_ty) =
                            self.check_numeric_underlying_compatibility(&left_ty_sub, &right_ty_sub)
                        {
                            return Ok(result_ty);
                        }

                        // Neither worked - return the original unification error
                        // Point to right operand as the "found" type
                        self.unify(&left_ty, &right_ty, &right.span)?;
                        Ok(self.substitute(left_ty))
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        // Comparison: both must be same type, result is bool
                        self.unify(&left_ty, &right_ty, &expr.span)?;
                        Ok(Type::simple("bool"))
                    }
                    BinOp::And | BinOp::Or => {
                        // Logical: both must be bool, result is bool
                        self.unify(&left_ty, &Type::simple("bool"), &left.span)?;
                        self.unify(&right_ty, &Type::simple("bool"), &right.span)?;
                        Ok(Type::simple("bool"))
                    }
                }
            }

            ExprKind::Call {
                func,
                type_args,
                args,
            } => {
                // Handle built-in make(type, ...) and new(type)
                if let ExprKind::Ident(name) = &func.kind {
                    if name == "make" && !type_args.is_empty() {
                        // make(type, ...) - returns the type
                        // Validate additional arguments are integers (size, capacity)
                        for arg in args {
                            let arg_ty = self.infer_expr(arg)?;
                            self.unify(&arg_ty, &Type::simple("int"), &arg.span)?;
                        }
                        // Return the type being made (properly resolving type args)
                        let ty = &type_args[0];
                        return Ok(self.resolve_type(ty));
                    }

                    if name == "new" && !type_args.is_empty() {
                        // new(type) - returns *type
                        // Return a pointer to the type
                        let ty = &type_args[0];
                        let inner_ty = self.resolve_type(ty);
                        // Use *{type} naming pattern consistent with UnaryOp::Ref
                        let ptr_name = format!("*{}", inner_ty);
                        return Ok(Type::generic(&ptr_name, vec![inner_ty]));
                    }
                }

                // Check if this is a type conversion: TypeName(value) or pkg.TypeName(value)
                if let ExprKind::Ident(type_name) = &func.kind
                    && self.global_state.has_type(type_name)
                {
                    // This is a type conversion, not a function call
                    // Type conversions take exactly one argument
                    if args.len() != 1 {
                        return Err(SoppoError::Type {
                            message: format!(
                                "Type conversion requires exactly 1 argument, but got {}",
                                args.len()
                            ),
                            span: expr.span.clone(),
                        });
                    }

                    // Infer the argument type (we don't need to use it, just check it's valid)
                    self.infer_expr(&args[0])?;

                    // Return the target type
                    return Ok(Type::simple(type_name));
                }

                // Check if this is a type conversion from an imported package: pkg.TypeName(value)
                if let ExprKind::Field {
                    expr: pkg_expr,
                    field: type_name,
                    ..
                } = &func.kind
                    && let ExprKind::Ident(pkg_name) = &pkg_expr.kind
                    && self.is_imported_package(pkg_name)
                {
                    // Look up the type from the package
                    if let Some(ty) = self.lookup_go_type(pkg_name, type_name) {
                        // This is a type conversion
                        if args.len() != 1 {
                            return Err(SoppoError::Type {
                                message: format!(
                                    "Type conversion requires exactly 1 argument, but got {}",
                                    args.len()
                                ),
                                span: expr.span.clone(),
                            });
                        }

                        // Infer the argument type (we don't need to use it, just check it's valid)
                        self.infer_expr(&args[0])?;

                        // Return the target type
                        return Ok(ty);
                    }
                }

                // Regular function call
                let func_ty = self.infer_expr(func)?;
                let func_ty = self.substitute(func_ty);

                // Infer argument types with their spans
                let mut arg_tys = Vec::new();
                for arg in args {
                    arg_tys.push((self.infer_expr(arg)?, arg.span.clone()));
                }

                // Check function call with detailed error spans
                match &func_ty {
                    Type::Fun {
                        args: param_tys,
                        ret,
                    } => {
                        // Check if last param is variadic
                        let has_variadic = param_tys.last().is_some_and(|last| {
                            matches!(last, Type::Con { name, .. } if name.name == "variadic")
                        });

                        if has_variadic {
                            let fixed_params = &param_tys[..param_tys.len() - 1];
                            let variadic_param = param_tys.last().expect("checked above");
                            let variadic_elem = if let Type::Con { args, .. } = variadic_param {
                                args.first().cloned().unwrap_or(Type::simple("any"))
                            } else {
                                Type::simple("any")
                            };

                            // Check we have at least the fixed params
                            if arg_tys.len() < fixed_params.len() {
                                return Err(SoppoError::Type {
                                    message: format!(
                                        "Function has {} arguments, but expected at least {}",
                                        arg_tys.len(),
                                        fixed_params.len()
                                    ),
                                    span: func.span.clone(),
                                });
                            }

                            // Check fixed params
                            for (param_ty, (arg_ty, arg_span)) in
                                fixed_params.iter().zip(arg_tys.iter())
                            {
                                self.unify(param_ty, arg_ty, arg_span)?;
                            }

                            // Check variadic args
                            for (arg_ty, arg_span) in arg_tys.iter().skip(fixed_params.len()) {
                                if variadic_elem != Type::simple("any") {
                                    self.unify(&variadic_elem, arg_ty, arg_span)?;
                                }
                            }
                        } else {
                            // Non-variadic: exact arg count required
                            if arg_tys.len() != param_tys.len() {
                                return Err(SoppoError::Type {
                                    message: format!(
                                        "Function has {} arguments, but expected {}",
                                        arg_tys.len(),
                                        param_tys.len()
                                    ),
                                    span: func.span.clone(),
                                });
                            }

                            // Check each argument type
                            for (param_ty, (arg_ty, arg_span)) in
                                param_tys.iter().zip(arg_tys.iter())
                            {
                                self.unify(param_ty, arg_ty, arg_span)?;
                            }
                        }

                        Ok(self.substitute(ret.as_ref().clone()))
                    }
                    Type::Var(_) => {
                        // Function type is unknown, use standard unification
                        let result_ty = self.fresh_ty_var();
                        let arg_types: Vec<Type> = arg_tys.into_iter().map(|(ty, _)| ty).collect();
                        let expected_func_ty = Type::fun(arg_types, result_ty.clone());
                        self.unify(&func_ty, &expected_func_ty, &expr.span)?;
                        Ok(self.substitute(result_ty))
                    }
                    _ => Err(SoppoError::Type {
                        message: format!("Cannot call non-function type `{}`", func_ty),
                        span: func.span.clone(),
                    }),
                }
            }

            ExprKind::Field {
                expr: field_expr,
                field,
                field_span,
            } => {
                // Check if this is accessing something from an imported Go package
                // e.g., fmt.Println, strings.HasPrefix
                if let ExprKind::Ident(name) = &field_expr.kind
                    && self.is_imported_package(name)
                {
                    // Try to look up as a function first
                    if let Some(func_ty) = self.lookup_go_function(name, field) {
                        return Ok(func_ty);
                    }
                    // Try to look up as a type or constant
                    if let Some(ty) = self.lookup_go_type(name, field) {
                        return Ok(ty);
                    }
                    // Couldn't find it - error
                    return Err(SoppoError::Type {
                        message: format!("`{}` not found in package `{}`", field, name),
                        span: field_span.clone(),
                    });
                }

                // Check if this is an enum constructor like Color.Red or Result.Ok
                if let ExprKind::Ident(type_name) = &field_expr.kind {
                    // Check if type_name is a registered type
                    if let Some(type_def) = self.global_state.lookup_type(type_name).cloned() {
                        // Check if this is an enum variant
                        if let super::module::TypeDefKind::Enum { variants } = &type_def.kind {
                            // Create fresh type variables for generic params
                            let generic_subst: HashMap<String, Type> = type_def
                                .generics
                                .iter()
                                .map(|g| (g.clone(), self.fresh_ty_var()))
                                .collect();

                            // Find the variant
                            for variant in variants {
                                let variant_name = match variant {
                                    EnumVariant::Unit { name, .. } => name,
                                    EnumVariant::Single { name, .. } => name,
                                    EnumVariant::Struct { name, .. } => name,
                                };

                                if variant_name == field {
                                    // Found the variant
                                    return match variant {
                                        EnumVariant::Unit { .. } => {
                                            // Unit variant: just returns the enum type
                                            Ok(Type::simple(type_name))
                                        }
                                        EnumVariant::Single { ty, .. } => {
                                            // Single variant: returns a constructor function
                                            // Ok(T) -> fn(T) -> Result[T, E]
                                            // Instantiate generic params with fresh type vars
                                            let param_ty =
                                                self.instantiate_type(&ty.name, &generic_subst);
                                            let return_ty = Type::simple(type_name);
                                            Ok(Type::fun(vec![param_ty], return_ty))
                                        }
                                        EnumVariant::Struct { fields, .. } => {
                                            // Struct variant: returns a constructor function
                                            // taking all fields as parameters
                                            let param_tys: Vec<Type> = fields
                                                .iter()
                                                .map(|f| {
                                                    self.instantiate_type(
                                                        &f.ty.name,
                                                        &generic_subst,
                                                    )
                                                })
                                                .collect();
                                            let return_ty = Type::simple(type_name);
                                            Ok(Type::fun(param_tys, return_ty))
                                        }
                                    };
                                }
                            }
                        }
                        // Not an enum, but still a type - might be for other purposes
                        return Ok(Type::simple(type_name));
                    }
                }

                // Otherwise it's a regular field access
                let expr_ty = self.infer_expr(field_expr)?;
                let expr_ty = self.substitute(expr_ty);

                // Handle built-in error type's Error() method
                if let Type::Con { name, .. } = &expr_ty
                    && name.name == "error"
                    && field == "Error"
                {
                    // error.Error() returns string
                    return Ok(Type::fun(vec![], Type::simple("string")));
                }

                // Look up the struct type to validate field access
                if let Type::Con { name, .. } = &expr_ty
                    && let Some(type_def) = self.global_state.lookup_type(&name.name)
                    && let super::module::TypeDefKind::Struct { fields } = &type_def.kind
                {
                    // Check if the field exists
                    if let Some((_, field_ty)) = fields.iter().find(|(f, _)| f == field) {
                        return Ok(field_ty.clone());
                    } else {
                        // Field not found - check if it might be a method
                        // If we can find a function with this name, return a type variable
                        // and let the Call handler deal with it
                        if self.global_state.lookup_function(field).is_some() {
                            return Ok(self.fresh_ty_var());
                        }

                        return Err(SoppoError::Type {
                            message: format!(
                                "Struct `{}` has no field named `{}`",
                                name.name, field
                            ),
                            span: field_expr.span.clone(),
                        });
                    }
                }

                // If we can't determine the struct type, return a type variable
                // (this allows field access on generic/unknown types)
                Ok(self.fresh_ty_var())
            }

            ExprKind::Index { expr, index } => {
                let container_ty = self.infer_expr(expr)?;
                let container_ty = self.substitute(container_ty);
                let index_ty = self.infer_expr(index)?;

                if let Type::Con { name, args } = &container_ty {
                    // Map indexing: map[K]V - index is K, result is V
                    if (name.name == "map" || name.name.starts_with("map[")) && args.len() == 2 {
                        self.unify(&index_ty, &args[0], &index.span)?;
                        return Ok(args[1].clone());
                    }

                    // Slice indexing: []T - index is int, result is T
                    if name.name.starts_with("[]") {
                        self.unify(&index_ty, &Type::simple("int"), &index.span)?;
                        if args.len() == 1 {
                            return Ok(args[0].clone());
                        }
                        let elem_name = &name.name[2..];
                        return Ok(Type::simple(elem_name));
                    }

                    // Array indexing: array or [N]T - index is int
                    if name.name == "array" && args.len() == 1 {
                        self.unify(&index_ty, &Type::simple("int"), &index.span)?;
                        return Ok(args[0].clone());
                    }

                    // String indexing - index is int, result is byte
                    if name.name == "string" {
                        self.unify(&index_ty, &Type::simple("int"), &index.span)?;
                        return Ok(Type::simple("byte"));
                    }
                }

                // Default: assume int index
                self.unify(&index_ty, &Type::simple("int"), &index.span)?;
                Ok(self.fresh_ty_var())
            }

            ExprKind::ArrayLit { ty, elements } => {
                // Infer element type from the declared type or first element
                let (elem_ty, is_slice) = if let Some(ty) = ty {
                    // Extract element type from []T or T
                    if ty.name.starts_with("[]") {
                        let elem_name = &ty.name[2..];
                        (Type::simple(elem_name), true)
                    } else {
                        (Type::simple(&ty.name), false)
                    }
                } else if !elements.is_empty() {
                    (self.infer_expr(&elements[0])?, false)
                } else {
                    (self.fresh_ty_var(), false)
                };

                // All elements must have the same type
                for elem in elements {
                    let elem_ty_actual = self.infer_expr(elem)?;
                    self.unify(&elem_ty, &elem_ty_actual, &elem.span)?;
                }

                // Return proper slice/array type with element type
                if is_slice {
                    Ok(Type::generic("slice", vec![elem_ty]))
                } else {
                    Ok(Type::array(elem_ty))
                }
            }

            ExprKind::StructLit { ty, fields } => {
                // Type check each field
                for (_field_name, value) in fields {
                    self.infer_expr(value)?;
                }

                // Check if this is an enum variant (e.g., Shape.Circle)
                // If so, return the enum type, not the variant
                if ty.name.contains('.') {
                    let parts: Vec<&str> = ty.name.split('.').collect();
                    if parts.len() == 2 {
                        let enum_name = parts[0];
                        return Ok(Type::simple(enum_name));
                    }
                }

                // Return the struct type
                Ok(Type::simple(&ty.name))
            }

            ExprKind::MapLit { ty, entries } => {
                // Extract key and value types from map[K]V
                let (key_ty, val_ty) = if ty.args.len() == 2 {
                    (
                        Type::simple(&ty.args[0].name),
                        Type::simple(&ty.args[1].name),
                    )
                } else {
                    // Fallback: infer from first entry
                    if let Some((k, v)) = entries.first() {
                        (self.infer_expr(k)?, self.infer_expr(v)?)
                    } else {
                        (self.fresh_ty_var(), self.fresh_ty_var())
                    }
                };

                // Type check all entries
                for (key, value) in entries {
                    let k_ty = self.infer_expr(key)?;
                    let v_ty = self.infer_expr(value)?;
                    self.unify(&key_ty, &k_ty, &key.span)?;
                    self.unify(&val_ty, &v_ty, &value.span)?;
                }

                // Return map[K]V type
                Ok(Type::generic("map", vec![key_ty, val_ty]))
            }

            ExprKind::Unary { op, operand } => {
                use crate::parse::UnaryOp;
                let operand_ty = self.infer_expr(operand)?;

                match op {
                    UnaryOp::Neg => {
                        // -x: operand must be numeric, result is same type
                        // We allow any numeric type here
                        Ok(operand_ty)
                    }
                    UnaryOp::Not => {
                        // !x: operand must be bool, result is bool
                        self.unify(&operand_ty, &Type::simple("bool"), &operand.span)?;
                        Ok(Type::simple("bool"))
                    }
                    UnaryOp::Ref => {
                        // &x: result is *T where T is the operand type
                        let operand_ty = self.substitute(operand_ty);
                        let ptr_name = format!("*{}", operand_ty);
                        Ok(Type::generic(&ptr_name, vec![operand_ty]))
                    }
                    UnaryOp::Deref => {
                        // *p: operand must be *T, result is T
                        let operand_ty = self.substitute(operand_ty);
                        // Extract the pointee type from *T
                        if let Type::Con { name, args } = &operand_ty {
                            if name.name.starts_with('*') && args.len() == 1 {
                                return Ok(args[0].clone());
                            }
                            // Also handle case where type name encodes the pointee
                            if name.name.starts_with('*') {
                                let pointee_name = &name.name[1..];
                                return Ok(Type::simple(pointee_name));
                            }
                        }
                        // If we can't determine the pointer type, return a type variable
                        Ok(self.fresh_ty_var())
                    }
                    UnaryOp::Recv => {
                        // <-ch: operand must be chan T, result is T
                        let operand_ty = self.substitute(operand_ty);
                        // Extract the element type from chan T
                        if let Type::Con { name, args } = &operand_ty {
                            // Handle "chan T" type with args
                            if name.name.starts_with("chan ") && args.len() == 1 {
                                return Ok(args[0].clone());
                            }
                            // Also handle case where type name encodes the element type
                            if name.name.starts_with("chan ") {
                                let elem_name = &name.name[5..]; // skip "chan "
                                return Ok(Type::simple(elem_name));
                            }
                        }
                        // If we can't determine the channel type, return a type variable
                        Ok(self.fresh_ty_var())
                    }
                }
            }

            ExprKind::FuncLit {
                params,
                return_types,
                body,
            } => {
                // Save the current expected return types
                let prev_expected = self.expected_return_types.take();

                // Create a new scope for the function body
                self.push_scope();

                // Add parameters to scope
                for param in params {
                    let param_ty = Type::simple(&param.ty.name);
                    self.insert_var(param.name.clone(), param_ty);
                }

                // Set expected return types for this function
                let expected_ret_types: Vec<Type> =
                    return_types.iter().map(|t| Type::simple(&t.name)).collect();
                if !expected_ret_types.is_empty() {
                    self.expected_return_types = Some(expected_ret_types.clone());
                }

                // Infer body
                self.infer_block(body)?;

                self.pop_scope();

                // Restore previous expected return types
                self.expected_return_types = prev_expected;

                // Build function type
                let param_types: Vec<Type> =
                    params.iter().map(|p| Type::simple(&p.ty.name)).collect();
                let ret_ty = if return_types.is_empty() {
                    Type::unit()
                } else if return_types.len() == 1 {
                    Type::simple(&return_types[0].name)
                } else {
                    // Multiple return types - use a tuple type
                    let ret_types: Vec<Type> =
                        return_types.iter().map(|t| Type::simple(&t.name)).collect();
                    Type::generic("tuple", ret_types)
                };

                Ok(Type::fun(param_types, ret_ty))
            }

            ExprKind::Block(block) => self.infer_block(block),
        }
    }

    /// Infer the type of a statement
    /// Returns the type of the statement (unit for most, or the type of the expression)
    pub fn infer_stmt(&mut self, stmt: &Stmt) -> Result<Type> {
        match &stmt.kind {
            StmtKind::Decl { name, value } => {
                let value_ty = self.infer_expr(value)?;
                self.insert_var(name.clone(), value_ty.clone());
                Ok(Type::unit())
            }

            StmtKind::MultiDecl { names, values } => {
                if values.len() == 1 && names.len() > 1 {
                    // a, b := f() (multi-return unpacking)
                    let value = &values[0];
                    let value_ty = self.infer_expr(value)?;
                    let value_ty = self.substitute(value_ty);

                    // The value should be a tuple type with matching arity
                    if let Type::Con {
                        name: type_name,
                        args,
                    } = &value_ty
                        && type_name.name == "_tuple"
                        && args.len() == names.len()
                    {
                        for (name, ty) in names.iter().zip(args.iter()) {
                            self.insert_var(name.clone(), ty.clone());
                        }
                        return Ok(Type::unit());
                    }

                    // Not a tuple type or wrong arity
                    Err(SoppoError::Type {
                        message: format!(
                            "Cannot unpack {} values from type `{}`",
                            names.len(),
                            value_ty
                        ),
                        span: value.span.clone(),
                    })
                } else {
                    // a, b := expr1, expr2 (one value per name)
                    for (name, value) in names.iter().zip(values.iter()) {
                        let value_ty = self.infer_expr(value)?;
                        self.insert_var(name.clone(), value_ty);
                    }
                    Ok(Type::unit())
                }
            }

            StmtKind::VarDecl { name, ty, value } => {
                let var_ty = match (ty, value) {
                    (Some(t), Some(expr)) => {
                        // var x type = value: unify declared with inferred
                        let declared_ty = Type::from_ast(t);
                        let value_ty = self.infer_expr(expr)?;
                        self.unify(&declared_ty, &value_ty, &expr.span)?;
                        declared_ty
                    }
                    (Some(t), None) => {
                        // var x type: use declared type (zero value)
                        Type::from_ast(t)
                    }
                    (None, Some(expr)) => {
                        // var x = value: infer from value
                        self.infer_expr(expr)?
                    }
                    (None, None) => {
                        // var x: error (should be caught by parser)
                        return Err(SoppoError::Type {
                            message:
                                "Variable declaration requires either a type or an initializer"
                                    .to_string(),
                            span: stmt.span.clone(),
                        });
                    }
                };
                self.insert_var(name.clone(), var_ty);
                Ok(Type::unit())
            }

            StmtKind::MultiVarDecl { names, ty, values } => {
                if values.is_empty() {
                    // var a, b, c type (zero values)
                    let declared_ty =
                        ty.as_ref()
                            .map(Type::from_ast)
                            .ok_or_else(|| SoppoError::Type {
                                message:
                                    "Multi-variable declaration without values requires a type"
                                        .to_string(),
                                span: stmt.span.clone(),
                            })?;
                    for name in names {
                        self.insert_var(name.clone(), declared_ty.clone());
                    }
                } else if values.len() == 1 && names.len() > 1 {
                    // var a, b = f() (multi-return unpacking)
                    let value = &values[0];
                    let value_ty = self.infer_expr(value)?;
                    let value_ty = self.substitute(value_ty);

                    // The value should be a tuple type with matching arity
                    if let Type::Con {
                        name: type_name,
                        args,
                    } = &value_ty
                        && type_name.name == "_tuple"
                        && args.len() == names.len()
                    {
                        for (name, arg_ty) in names.iter().zip(args.iter()) {
                            let var_ty = if let Some(t) = ty {
                                let declared_ty = Type::from_ast(t);
                                self.unify(&declared_ty, arg_ty, &value.span)?;
                                declared_ty
                            } else {
                                arg_ty.clone()
                            };
                            self.insert_var(name.clone(), var_ty);
                        }
                        return Ok(Type::unit());
                    }

                    return Err(SoppoError::Type {
                        message: format!(
                            "Cannot unpack {} values from type `{}`",
                            names.len(),
                            value_ty
                        ),
                        span: value.span.clone(),
                    });
                } else {
                    // var a, b = expr1, expr2 or var a, b type = expr1, expr2
                    for (name, value) in names.iter().zip(values.iter()) {
                        let value_ty = self.infer_expr(value)?;
                        let var_ty = if let Some(t) = ty {
                            let declared_ty = Type::from_ast(t);
                            self.unify(&declared_ty, &value_ty, &value.span)?;
                            declared_ty
                        } else {
                            value_ty
                        };
                        self.insert_var(name.clone(), var_ty);
                    }
                }
                Ok(Type::unit())
            }

            StmtKind::ConstDecl { name, ty, value } => {
                // Infer the type of the value
                let value_ty = self.infer_expr(value)?;

                // Determine the constant's type
                let const_ty = if let Some(t) = ty {
                    // const x type = value: unify declared with inferred
                    let declared_ty = Type::from_ast(t);
                    self.unify(&declared_ty, &value_ty, &value.span)?;
                    declared_ty
                } else {
                    // const x = value: infer from value
                    value_ty
                };

                self.insert_var(name.clone(), const_ty);
                Ok(Type::unit())
            }

            StmtKind::MultiConstDecl { names, ty, values } => {
                // const a, b = expr1, expr2 or const a, b type = expr1, expr2
                for (name, value) in names.iter().zip(values.iter()) {
                    let value_ty = self.infer_expr(value)?;
                    let const_ty = if let Some(t) = ty {
                        let declared_ty = Type::from_ast(t);
                        self.unify(&declared_ty, &value_ty, &value.span)?;
                        declared_ty
                    } else {
                        value_ty
                    };
                    self.insert_var(name.clone(), const_ty);
                }
                Ok(Type::unit())
            }

            StmtKind::Assign { target, value } => {
                let target_ty = self.infer_expr(target)?;
                let value_ty = self.infer_expr(value)?;
                self.unify(&target_ty, &value_ty, &stmt.span)?;
                Ok(Type::unit())
            }

            StmtKind::MultiAssign { targets, values } => {
                if values.len() == 1 && targets.len() > 1 {
                    // a, b = f() (multi-return unpacking)
                    let value = &values[0];
                    let value_ty = self.infer_expr(value)?;
                    let value_ty = self.substitute(value_ty);

                    // The value should be a tuple type with matching arity
                    if let Type::Con {
                        name: type_name,
                        args,
                    } = &value_ty
                        && type_name.name == "_tuple"
                        && args.len() == targets.len()
                    {
                        for (target, expected_ty) in targets.iter().zip(args.iter()) {
                            let target_ty = self.infer_expr(target)?;
                            self.unify(&target_ty, expected_ty, &target.span)?;
                        }
                        return Ok(Type::unit());
                    }

                    // Not a tuple type or wrong arity
                    Err(SoppoError::Type {
                        message: format!(
                            "Cannot unpack {} values from type `{}`",
                            targets.len(),
                            value_ty
                        ),
                        span: value.span.clone(),
                    })
                } else {
                    // a, b = expr1, expr2 (one value per target)
                    for (target, value) in targets.iter().zip(values.iter()) {
                        let target_ty = self.infer_expr(target)?;
                        let value_ty = self.infer_expr(value)?;
                        self.unify(&target_ty, &value_ty, &target.span)?;
                    }
                    Ok(Type::unit())
                }
            }

            StmtKind::For { condition, body } => {
                // Check condition is bool
                let cond_ty = self.infer_expr(condition)?;
                self.unify(&Type::simple("bool"), &cond_ty, &condition.span)?;

                // Type check body
                self.infer_block(body)?;

                Ok(Type::unit())
            }

            StmtKind::ForRange {
                key,
                value,
                collection,
                body,
            } => {
                // Infer collection type
                let coll_ty = self.infer_expr(collection)?;
                let coll_ty = self.substitute(coll_ty);

                // Determine key and value types based on collection type
                let (key_ty, value_ty) = if let Type::Con { name, args } = &coll_ty {
                    if name.name.starts_with("[]") {
                        // Slice: key is int, value is element type
                        let elem_ty = if args.len() == 1 {
                            args[0].clone()
                        } else {
                            let elem_name = &name.name[2..];
                            Type::simple(elem_name)
                        };
                        (Type::simple("int"), elem_ty)
                    } else if name.name.starts_with("map[") {
                        // Map: key is key type, value is value type
                        if args.len() == 2 {
                            (args[0].clone(), args[1].clone())
                        } else {
                            (self.fresh_ty_var(), self.fresh_ty_var())
                        }
                    } else if name.name.starts_with("chan ") {
                        // Channel: only one variable (value type)
                        let elem_ty = if args.len() == 1 {
                            args[0].clone()
                        } else {
                            let elem_name = &name.name[5..];
                            Type::simple(elem_name)
                        };
                        (elem_ty.clone(), elem_ty)
                    } else if name.name == "string" {
                        // String: key is int (index), value is rune
                        (Type::simple("int"), Type::simple("rune"))
                    } else {
                        (self.fresh_ty_var(), self.fresh_ty_var())
                    }
                } else {
                    (self.fresh_ty_var(), self.fresh_ty_var())
                };

                // Bind the key variable
                self.insert_var(key.clone(), key_ty);

                // Bind the value variable if present
                if let Some(val_name) = value {
                    self.insert_var(val_name.clone(), value_ty);
                }

                // Type check body
                self.infer_block(body)?;

                Ok(Type::unit())
            }

            StmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                // Check condition is bool
                let cond_ty = self.infer_expr(condition)?;
                self.unify(&Type::simple("bool"), &cond_ty, &condition.span)?;

                // Type check then block
                let then_ty = self.infer_block(then_block)?;

                // Type check else block if present
                let else_ty = if let Some(else_block) = else_block {
                    self.infer_block(else_block)?
                } else {
                    Type::unit()
                };

                // If both branches diverge (return never), the if statement also diverges
                if matches!(then_ty, Type::Never) && matches!(else_ty, Type::Never) {
                    Ok(Type::never())
                } else {
                    Ok(Type::unit())
                }
            }

            StmtKind::Return { values } => {
                // Check return values against expected return types
                if let Some(expected_types) = self.expected_return_types.clone() {
                    if values.len() != expected_types.len() {
                        return Err(SoppoError::Type {
                            message: format!(
                                "Expected {} return value(s), got {}",
                                expected_types.len(),
                                values.len()
                            ),
                            span: stmt.span.clone(),
                        });
                    }
                    for (expr, expected) in values.iter().zip(expected_types.iter()) {
                        let value_ty = self.infer_expr(expr)?;
                        self.unify(expected, &value_ty, &expr.span)?;
                    }
                } else if !values.is_empty() {
                    // Infer types but no expected types to check against
                    for expr in values {
                        self.infer_expr(expr)?;
                    }
                }
                // Return statements are diverging - they never produce a value
                Ok(Type::never())
            }

            StmtKind::Match { scrutinee, arms } => {
                // Infer the type of the scrutinee
                let scrutinee_ty = self.infer_expr(scrutinee)?;
                let scrutinee_ty = self.substitute(scrutinee_ty);

                for arm in arms {
                    // Create a new scope for pattern bindings
                    self.push_scope();

                    // Add pattern bindings to scope
                    self.add_pattern_bindings(&arm.pattern, &scrutinee_ty)?;

                    // Type check the arm body
                    self.infer_block(&arm.body)?;

                    // Pop the scope after processing the arm
                    self.pop_scope();
                }

                // Exhaustiveness check for enum types
                if let Type::Con { name, .. } = &scrutinee_ty
                    && let Some(type_def) = self.global_state.lookup_type(&name.name)
                    && let super::module::TypeDefKind::Enum { variants } = &type_def.kind
                {
                    // Check if any arm is Default (catch-all)
                    let has_default = arms
                        .iter()
                        .any(|arm| matches!(&arm.pattern.kind, PatternKind::Default));

                    if !has_default {
                        // Collect covered variants from patterns
                        let covered: HashSet<String> = arms
                            .iter()
                            .filter_map(|arm| match &arm.pattern.kind {
                                PatternKind::Variant(v) => {
                                    // Extract variant name from qualified name like "Color.Red"
                                    Some(v.rsplit('.').next().unwrap_or(v).to_string())
                                }
                                PatternKind::Destructor { name, .. } => {
                                    Some(name.rsplit('.').next().unwrap_or(name).to_string())
                                }
                                PatternKind::StructDestructor { name, .. } => {
                                    Some(name.rsplit('.').next().unwrap_or(name).to_string())
                                }
                                _ => None,
                            })
                            .collect();

                        // Find missing variants
                        let missing: Vec<String> = variants
                            .iter()
                            .filter_map(|v| {
                                let vname = match v {
                                    EnumVariant::Unit { name, .. } => name,
                                    EnumVariant::Single { name, .. } => name,
                                    EnumVariant::Struct { name, .. } => name,
                                };
                                if !covered.contains(vname) {
                                    Some(vname.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();

                        if !missing.is_empty() {
                            return Err(SoppoError::NonExhaustive {
                                missing,
                                span: stmt.span.clone(),
                            });
                        }
                    }
                }

                Ok(Type::unit())
            }

            StmtKind::Send { channel, value } => {
                // ch <- value: channel must be chan T, value must be T
                let channel_ty = self.infer_expr(channel)?;
                let channel_ty = self.substitute(channel_ty);
                let value_ty = self.infer_expr(value)?;

                // Extract element type from channel
                if let Type::Con { name, args } = &channel_ty {
                    if name.name.starts_with("chan ") && args.len() == 1 {
                        self.unify(&args[0], &value_ty, &value.span)?;
                    } else if name.name.starts_with("chan ") {
                        let elem_name = &name.name[5..]; // skip "chan "
                        let elem_ty = Type::simple(elem_name);
                        self.unify(&elem_ty, &value_ty, &value.span)?;
                    }
                }

                Ok(Type::unit())
            }

            StmtKind::Go(expr) => {
                // go expr: expr should be a function call
                self.infer_expr(expr)?;
                Ok(Type::unit())
            }

            StmtKind::DeferStmt(expr) => {
                // defer expr: expr should be a function call
                self.infer_expr(expr)?;
                Ok(Type::unit())
            }

            StmtKind::Break | StmtKind::Continue => {
                // break/continue don't have types, just return unit
                Ok(Type::unit())
            }

            StmtKind::Expr(expr) => self.infer_expr(expr),
        }
    }

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

    /// Infer and check a function declaration
    pub fn infer_func_decl(&mut self, func: &FuncDecl) -> Result<()> {
        self.push_scope();

        // Save old generic params and set up new ones for this function
        let old_generic_params = std::mem::take(&mut self.generic_params);
        for generic in &func.generics {
            let ty_var = self.fresh_ty_var();
            self.generic_params.insert(generic.name.clone(), ty_var);
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
        if let Some(receiver) = &func.receiver {
            let receiver_ty = self.resolve_type(&receiver.ty);
            self.insert_var(receiver.name.clone(), receiver_ty);
        }

        // Add parameters to scope
        for param in &func.params {
            let param_ty = self.resolve_type(&param.ty);
            self.insert_var(param.name.clone(), param_ty);
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

        // Register function in global state
        // For multi-value returns, we use a tuple type representation
        let func_ty = {
            let param_tys: Vec<Type> = func.params.iter().map(|p| Type::from_ast(&p.ty)).collect();
            let ret_ty = if func.return_types.is_empty() {
                Type::unit()
            } else if func.return_types.len() == 1 {
                Type::from_ast(&func.return_types[0])
            } else {
                // Multi-value return: create a tuple type
                Type::generic(
                    "_tuple",
                    func.return_types.iter().map(Type::from_ast).collect(),
                )
            };
            Type::fun(param_tys, ret_ty)
        };

        // Store function type in outermost scope so it can be called
        if let Some(scope) = self.scopes.first_mut() {
            scope.insert(func.name.clone(), func_ty);
        }

        // Register function in global state so it can be looked up for method calls
        self.global_state.register_function(func);

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
            scope.insert(const_decl.name.clone(), const_ty);
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

            TypeKind::Enum { variants } => {
                // Register the enum type in the global state
                self.global_state.register_type(type_decl);

                // Set up generic params for this type declaration
                let old_generic_params = std::mem::take(&mut self.generic_params);
                for generic in &type_decl.generics {
                    let ty_var = self.fresh_ty_var();
                    self.generic_params.insert(generic.name.clone(), ty_var);
                }

                // Register each variant as a constructor function
                for variant in variants {
                    match variant {
                        EnumVariant::Unit { name, .. } => {
                            // Unit variants are just values of the enum type
                            // They act like constructors with no arguments
                            let enum_ty = Type::simple(&type_decl.name);
                            if let Some(scope) = self.scopes.first_mut() {
                                scope.insert(name.clone(), enum_ty);
                            }
                        }
                        EnumVariant::Single { name, ty, .. } => {
                            // Single value variants are functions: T -> EnumType
                            let value_ty = self.resolve_type(ty);
                            let enum_ty = Type::simple(&type_decl.name);
                            let constructor_ty = Type::fun(vec![value_ty], enum_ty);
                            if let Some(scope) = self.scopes.first_mut() {
                                scope.insert(name.clone(), constructor_ty);
                            }
                        }
                        EnumVariant::Struct { name, fields, .. } => {
                            // Struct variants are functions: (field1, field2, ...) -> EnumType
                            let field_tys: Vec<Type> =
                                fields.iter().map(|f| self.resolve_type(&f.ty)).collect();
                            let enum_ty = Type::simple(&type_decl.name);
                            let constructor_ty = Type::fun(field_tys, enum_ty);
                            if let Some(scope) = self.scopes.first_mut() {
                                scope.insert(name.clone(), constructor_ty);
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
                    self.generic_params.insert(generic.name.clone(), ty_var);
                }

                // Store field types for later field access validation
                let field_types: Vec<(String, Type)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), self.resolve_type(&f.ty)))
                    .collect();

                // Update the registered type with actual field types
                if let Some(type_def) = self
                    .global_state
                    .current_module_mut()
                    .types
                    .get_mut(&type_decl.name)
                {
                    type_def.kind = super::module::TypeDefKind::Struct {
                        fields: field_types,
                    };
                }

                // Restore old generic params
                self.generic_params = old_generic_params;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{Decl, FileId, Parser};

    fn infer_expr_helper(source: &str) -> Result<Type> {
        let mut parser = Parser::new(source, FileId(0));
        let expr = parser.parse_expr()?;
        let mut infer = Infer::new().expect("Failed to create Infer");
        let ty = infer.infer_expr(&expr)?;
        Ok(infer.substitute(ty))
    }

    #[test]
    fn test_infer_integer() {
        let ty = infer_expr_helper("42").unwrap();
        assert_eq!(ty, Type::simple("int"));
    }

    #[test]
    fn test_infer_string() {
        let ty = infer_expr_helper(r#""hello""#).unwrap();
        assert_eq!(ty, Type::simple("string"));
    }

    #[test]
    fn test_infer_bool() {
        let ty = infer_expr_helper("true").unwrap();
        assert_eq!(ty, Type::simple("bool"));
    }

    #[test]
    fn test_infer_arithmetic() {
        let ty = infer_expr_helper("1 + 2").unwrap();
        assert_eq!(ty, Type::simple("int"));

        let ty = infer_expr_helper("10 * 5").unwrap();
        assert_eq!(ty, Type::simple("int"));
    }

    #[test]
    fn test_infer_comparison() {
        let ty = infer_expr_helper("1 < 2").unwrap();
        assert_eq!(ty, Type::simple("bool"));

        let ty = infer_expr_helper("5 == 5").unwrap();
        assert_eq!(ty, Type::simple("bool"));
    }

    #[test]
    fn test_infer_complex_expr() {
        let ty = infer_expr_helper("(1 + 2) * 3").unwrap();
        assert_eq!(ty, Type::simple("int"));

        let ty = infer_expr_helper("(1 + 2) < (3 * 4)").unwrap();
        assert_eq!(ty, Type::simple("bool"));
    }

    #[test]
    fn test_type_error_arithmetic() {
        // Can't add string to int
        let result = infer_expr_helper(r#"1 + "hello""#);
        assert!(result.is_err());
    }

    #[test]
    fn test_unification() {
        let mut infer = Infer::new().expect("Failed to create Infer");

        // Unify two type variables
        let t1 = infer.fresh_ty_var();
        let t2 = infer.fresh_ty_var();
        infer.unify(&t1, &Type::simple("int"), &Span::dummy()).unwrap();
        infer.unify(&t2, &t1, &Span::dummy()).unwrap();

        let t2_subst = infer.substitute(t2);
        assert_eq!(t2_subst, Type::simple("int"));
    }

    #[test]
    fn test_occurs_check() {
        let mut infer = Infer::new().expect("Failed to create Infer");

        // Create a type variable
        let t = infer.fresh_ty_var();

        // Try to unify with a type containing itself: T = Option[T]
        let option_t = Type::con_with_args(
            Symbol {
                module: ModuleId::empty(),
                name: "Option".to_string(),
                span: Span::dummy(),
            },
            vec![t.clone()],
        );

        let result = infer.unify(&t, &option_t, &Span::dummy());
        assert!(result.is_err());
    }

    #[test]
    fn test_infer_let_stmt() {
        let source = "{ x := 42\nx }";
        let mut parser = Parser::new(source, FileId(0));
        let block = parser.parse_block().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");
        let ty = infer.infer_block(&block).unwrap();

        assert_eq!(ty, Type::simple("int"));
    }

    #[test]
    fn test_infer_multiple_lets() {
        let source = "{ x := 42\ny := x\ny }";
        let mut parser = Parser::new(source, FileId(0));
        let block = parser.parse_block().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");
        let ty = infer.infer_block(&block).unwrap();

        assert_eq!(ty, Type::simple("int"));
    }

    #[test]
    fn test_infer_return_stmt() {
        let source = "{ return 42 }";
        let mut parser = Parser::new(source, FileId(0));
        let block = parser.parse_block().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");
        let ty = infer.infer_block(&block).unwrap();

        // Return statements are diverging, so the block returns Never
        assert_eq!(ty, Type::never());
    }

    #[test]
    fn test_infer_function() {
        let source = "func add(x int, y int) int { return x + y }";
        let mut parser = Parser::new(source, FileId(0));
        let func = parser.parse_func_decl().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");
        let result = infer.infer_func_decl(&func);

        assert!(result.is_ok());
    }

    #[test]
    fn test_infer_function_type_error() {
        // Function returns string but declares int
        let source = r#"func bad() int { return "hello" }"#;
        let mut parser = Parser::new(source, FileId(0));
        let func = parser.parse_func_decl().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");
        let result = infer.infer_func_decl(&func);

        assert!(result.is_err());
    }

    #[test]
    fn test_function_call_in_scope() {
        let source = r#"
            func add(x int, y int) int { return x + y }
            func main() int { return add(1, 2) }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");

        // Infer both functions
        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                infer.infer_func_decl(func).unwrap();
            }
        }
    }

    #[test]
    fn test_variable_shadowing() {
        let source = "{ x := 42\n{ x := true\nx } }";
        let mut parser = Parser::new(source, FileId(0));
        let block = parser.parse_block().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");
        let ty = infer.infer_block(&block).unwrap();

        // Inner block shadows x with bool, so result is bool
        assert_eq!(ty, Type::simple("bool"));
    }

    #[test]
    fn test_struct_field_access() {
        // Test that field access on struct returns correct type
        let source = r#"
            type User struct {
                name string
                age int
            }
            func test(u User) string {
                return u.name
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");

        // Register struct type
        for decl in &file.decls {
            if let Decl::Type(type_decl) = decl {
                infer.infer_type_decl(type_decl).unwrap();
            }
        }

        // Type check function
        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                let result = infer.infer_func_decl(func);
                assert!(result.is_ok(), "Function should type check: {:?}", result);
            }
        }
    }

    #[test]
    fn test_struct_invalid_field_access() {
        // Test that accessing non-existent field produces error
        let source = r#"
            type User struct {
                name string
                age int
            }
            func test(u User) string {
                return u.email
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");

        // Register struct type
        for decl in &file.decls {
            if let Decl::Type(type_decl) = decl {
                infer.infer_type_decl(type_decl).unwrap();
            }
        }

        // Type check function - should fail
        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                let result = infer.infer_func_decl(func);
                assert!(result.is_err(), "Should error on invalid field access");
            }
        }
    }

    #[test]
    fn test_struct_field_type_checking() {
        // Test that field types are properly enforced
        let source = r#"
            type Point struct {
                x int
                y int
            }
            func test(p Point) int {
                return p.x + p.y
            }
        "#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");

        // Register struct type
        for decl in &file.decls {
            if let Decl::Type(type_decl) = decl {
                infer.infer_type_decl(type_decl).unwrap();
            }
        }

        // Type check function
        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                let result = infer.infer_func_decl(func);
                assert!(
                    result.is_ok(),
                    "Addition of int fields should work: {:?}",
                    result
                );
            }
        }
    }

    #[test]
    fn test_array_literal_type() {
        // Test that array literals have proper array type
        let source = "[5]int{1, 2, 3, 4, 5}";
        let mut parser = Parser::new(source, FileId(0));
        let expr = parser.parse_expr().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");
        let ty = infer.infer_expr(&expr).unwrap();

        // Should be array[int]
        if let Type::Con { name, args } = ty {
            assert_eq!(name.name, "array");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0], Type::simple("int"));
        } else {
            panic!("Expected array type, got: {:?}", ty);
        }
    }

    #[test]
    fn test_array_index_type() {
        // Test that indexing an array returns the element type
        let source = r#"{
                arr := [3]int{1, 2, 3}
                arr[0]
            }"#;
        let mut parser = Parser::new(source, FileId(0));
        let block = parser.parse_block().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");
        let ty = infer.infer_block(&block).unwrap();

        // Should be int (the element type)
        assert_eq!(ty, Type::simple("int"));
    }

    #[test]
    fn test_array_element_type_checking() {
        // Test that all array elements must have the same type
        let source = r#"{
                arr := [3]int{1, 2, 3}
                x := arr[0]
                y := arr[1]
                x + y
            }"#;
        let mut parser = Parser::new(source, FileId(0));
        let block = parser.parse_block().unwrap();

        let mut infer = Infer::new().expect("Failed to create Infer");
        let result = infer.infer_block(&block);

        // Should succeed - adding two ints from array
        assert!(
            result.is_ok(),
            "Array element arithmetic should work: {:?}",
            result
        );
    }

    #[test]
    fn test_import_tracking() {
        let mut infer = Infer::new().expect("Failed to create Infer");

        // Process some imports
        let imports = vec![
            Import {
                path: "\"fmt\"".to_string(),
                span: Span::dummy(),
            },
            Import {
                path: "\"net/http\"".to_string(),
                span: Span::dummy(),
            },
        ];

        infer.process_imports(&imports);

        // Check imports are tracked correctly
        assert!(infer.is_imported_package("fmt"));
        assert!(infer.is_imported_package("http")); // short name from net/http
        assert!(!infer.is_imported_package("net"));
        assert!(!infer.is_imported_package("strings"));

        // Check import paths are stored
        assert_eq!(infer.imported_packages.get("fmt"), Some(&"fmt".to_string()));
        assert_eq!(
            infer.imported_packages.get("http"),
            Some(&"net/http".to_string())
        );
    }

    #[test]
    fn test_go_package_field_access_stdlib() {
        // Go stdlib resolution should work and find fmt.Println
        let source = r#"package main
import "fmt"
func main() {
    fmt.Println("hello")
}"#;
        let mut parser = Parser::new(source, FileId(0));
        let file = parser.parse_file().expect("Failed to parse");

        let mut infer = Infer::new().expect("Failed to create Infer");
        infer.process_imports(&file.imports);

        // Type check the function - should succeed because fmt.Println is resolved
        for decl in &file.decls {
            if let Decl::Func(func) = decl {
                let result = infer.infer_func_decl(func);
                assert!(
                    result.is_ok(),
                    "Should type check Go package access: {:?}",
                    result
                );
            }
        }
    }
}
