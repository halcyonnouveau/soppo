use std::fmt;

use crate::syntax::{ModuleId, Span, Symbol, TypeAnnotation as AstType};

/// Nullability state for pointer types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nullability {
    /// May be nil - must be checked before use
    Nullable,
    /// Proven non-nil - safe to dereference
    NonNull,
}

/// Type representation after type checking
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// Concrete type constructor: int, string, Option[T]
    Con {
        sym: Symbol,
        args: Vec<Type>,
        nullable: bool, // true for ?*T, ?[]T, ?Interface
    },

    /// Function type: func(a int, b string) bool
    /// Parameter names are optional (None for anonymous params)
    Func {
        args: Vec<(Option<String>, Type)>,
        ret: Box<Type>,
        nullable: bool, // true for ?func types
    },

    /// Type variable for inference
    Var(i32),

    /// Never type (diverging, like return/break/continue)
    Never,

    /// Error type (poison) - used when type checking fails
    /// This prevents cascading errors: any operation involving Error produces Error
    Error,
}

impl Type {
    /// Dummy type for initialisation
    pub fn dummy() -> Self {
        Type::Var(-1)
    }

    /// Create a simple type from a string (for built-in types)
    pub fn simple(name: &str) -> Self {
        Type::Con {
            sym: Symbol {
                module: ModuleId::empty(),
                name: name.to_string(),
                span: Span::dummy(),
            },
            args: vec![],
            nullable: false,
        }
    }

    /// Create a generic type with string name and type arguments
    pub fn generic(name: &str, args: Vec<Type>) -> Self {
        Type::Con {
            sym: Symbol {
                module: ModuleId::empty(),
                name: name.to_string(),
                span: Span::dummy(),
            },
            args,
            nullable: false,
        }
    }

    /// Built-in type: unit/void
    pub fn unit() -> Self {
        Type::simple("()")
    }

    /// Never type (diverging)
    pub fn never() -> Self {
        Type::Never
    }

    /// Error/poison type
    pub fn error() -> Self {
        Type::Error
    }

    /// Check if this type is the error/poison type
    pub fn is_error(&self) -> bool {
        matches!(self, Type::Error)
    }

    /// Create a type variable
    pub fn var(id: i32) -> Self {
        Type::Var(id)
    }

    /// Create a function type with unnamed parameters
    pub fn fun(args: Vec<Type>, ret: Type) -> Self {
        Type::Func {
            args: args.into_iter().map(|ty| (None, ty)).collect(),
            ret: Box::new(ret),
            nullable: false,
        }
    }

    /// Create a function type with named parameters
    pub fn fun_named(args: Vec<(Option<String>, Type)>, ret: Type) -> Self {
        Type::Func {
            args,
            ret: Box::new(ret),
            nullable: false,
        }
    }

    /// Create a generic type with arguments
    pub fn con_with_args(name: Symbol, args: Vec<Type>) -> Self {
        Type::Con {
            sym: name,
            args,
            nullable: false,
        }
    }

    /// Create an array type with element type
    pub fn array(element_type: Type) -> Self {
        Type::Con {
            sym: Symbol {
                module: ModuleId::empty(),
                name: "array".to_string(),
                span: Span::dummy(),
            },
            args: vec![element_type],
            nullable: false,
        }
    }

    /// Create a slice type with element type: []T
    pub fn slice(element_type: Type) -> Self {
        Type::Con {
            sym: Symbol {
                module: ModuleId::empty(),
                name: format!("[]{}", element_type),
                span: Span::dummy(),
            },
            args: vec![element_type],
            nullable: false,
        }
    }

    /// Create an anonymous struct type
    /// The name encodes the struct definition for codegen: "struct { Name1 Type1; Name2 Type2; ... }"
    pub fn anon_struct(fields: Vec<(String, Type)>) -> Self {
        let fields_str = fields
            .iter()
            .map(|(name, ty)| format!("{} {}", name, ty))
            .collect::<Vec<_>>()
            .join("; ");
        let name = format!("struct {{ {} }}", fields_str);
        Type::Con {
            sym: Symbol {
                module: ModuleId::empty(),
                name,
                span: Span::dummy(),
            },
            args: vec![],
            nullable: false,
        }
    }

    /// Create a pointer type (non-nullable by default)
    pub fn ptr(inner: Type) -> Self {
        Self::ptr_nullable(inner, false)
    }

    /// Create a pointer type with explicit nullability
    pub fn ptr_nullable(inner: Type, nullable: bool) -> Self {
        let inner_name = match &inner {
            Type::Con { sym: name, .. } => name.name.clone(),
            _ => "?".to_string(),
        };
        Type::Con {
            sym: Symbol {
                module: ModuleId::empty(),
                name: format!("*{}", inner_name),
                span: Span::dummy(),
            },
            args: vec![inner],
            nullable,
        }
    }

    /// Convert an AST type to a runtime type
    pub fn from_ast(ast_ty: &AstType) -> Self {
        Self::from_ast_in_module(ast_ty, &ModuleId::empty())
    }

    /// Convert an AST type to a runtime type, with module context for non-builtin types
    pub fn from_ast_in_module(ast_ty: &AstType, module: &ModuleId) -> Self {
        // Handle variadic types: ...T -> variadic[T]
        if ast_ty.name.starts_with("...") {
            let inner_name = &ast_ty.name[3..];
            return Type::generic("variadic", vec![Type::simple(inner_name)]);
        }

        // Determine the module for this type
        // - Built-in types get empty module
        // - Pointer/slice prefixes: extract base name for module check
        // - Qualified names (pkg.Type): use the package name as module
        // - Otherwise: use the provided module context
        let base_name = ast_ty
            .name
            .strip_prefix('*')
            .or_else(|| ast_ty.name.strip_prefix("[]"))
            .unwrap_or(&ast_ty.name);

        let type_module = if Self::is_builtin(base_name) {
            ModuleId::empty()
        } else if let Some(dot_idx) = base_name.find('.') {
            // Qualified name like "config.Config" - extract package
            ModuleId::new(&base_name[..dot_idx])
        } else {
            // Unqualified user type - use context module
            module.clone()
        };

        Type::Con {
            sym: Symbol {
                module: type_module,
                name: ast_ty.name.clone(),
                span: ast_ty.span,
            },
            args: ast_ty
                .args
                .iter()
                .map(|arg| Type::from_ast_in_module(arg, module))
                .collect(),
            nullable: ast_ty.nullable,
        }
    }

    /// Check if a name is a Go/Soppo built-in type (not function)
    pub fn is_builtin_type(name: &str) -> bool {
        matches!(
            name,
            "string"
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
                | "bool"
                | "complex64"
                | "complex128"
                | "error"
                | "any"
        )
    }

    /// Check if a name is a Go/Soppo built-in function
    pub fn is_builtin_func(name: &str) -> bool {
        matches!(
            name,
            "make"
                | "new"
                | "len"
                | "cap"
                | "append"
                | "copy"
                | "delete"
                | "panic"
                | "recover"
                | "print"
                | "println"
                | "complex"
                | "real"
                | "imag"
                | "close"
        )
    }

    /// Check if a name is a Go/Soppo built-in (type or function)
    pub fn is_builtin(name: &str) -> bool {
        Self::is_builtin_type(name) || Self::is_builtin_func(name)
    }

    /// Check if this type is nullable
    pub fn is_nullable(&self) -> bool {
        match self {
            Type::Con { nullable, .. } => *nullable,
            Type::Func { nullable, .. } => *nullable,
            Type::Var(_) => false, // Type variables are not inherently nullable
            Type::Never => false,
            Type::Error => false,
        }
    }

    /// Return a nullable version of this type
    pub fn as_nullable(self) -> Self {
        match self {
            Type::Con {
                sym: name,
                args,
                nullable: _,
            } => Type::Con {
                sym: name,
                args,
                nullable: true,
            },
            Type::Func {
                args,
                ret,
                nullable: _,
            } => Type::Func {
                args,
                ret,
                nullable: true,
            },
            other => other,
        }
    }

    /// Return a non-nullable version of this type
    pub fn as_non_nullable(self) -> Self {
        match self {
            Type::Con {
                sym: name,
                args,
                nullable: _,
            } => Type::Con {
                sym: name,
                args,
                nullable: false,
            },
            Type::Func {
                args,
                ret,
                nullable: _,
            } => Type::Func {
                args,
                ret,
                nullable: false,
            },
            other => other,
        }
    }

    /// Check if this type is a "nilable kind" (can be nil in Go)
    /// This includes: pointers, slices, maps, channels, interfaces, funcs
    pub fn is_nilable_kind(&self) -> bool {
        match self {
            Type::Con { sym: name, .. } => {
                let n = &name.name;
                n.starts_with('*')           // pointer
                    || n.starts_with("[]")   // slice
                    || n.starts_with("map[") // map
                    || n.starts_with("chan ") // channel
                    || n == "error"          // error interface
                    || n == "any"            // any/interface{}
                    || n == "interface{}" // explicit interface
            }
            Type::Func { .. } => true, // function types can be nil
            Type::Var(_) => false,
            Type::Never => false,
            Type::Error => false,
        }
    }

    /// Check if this is a Go interface type (error, any, interface{})
    /// These are always implicitly nilable in Go
    pub fn is_go_interface(&self) -> bool {
        match self {
            Type::Con { sym: name, .. } => {
                let n = &name.name;
                n == "error" || n == "any" || n == "interface{}"
            }
            _ => false,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Con {
                sym: name,
                args,
                nullable,
            } => {
                let prefix = if *nullable { "?" } else { "" };
                let n = &name.name;
                // tuples are multivalue returns, Soppo does not have tuples
                // TODO: i dunno why they're named tuple
                if n == "tuple" {
                    let args_str = args
                        .iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return write!(f, "{}({})", prefix, args_str);
                }
                // For types where the inner type is encoded in the name, don't show args
                // e.g., *int, []string, map[K]V, chan T
                let hide_args = n.starts_with('*')
                    || n.starts_with("[]")
                    || n.starts_with("map[")
                    || n.starts_with("chan ")
                    || n.starts_with("...");
                if args.is_empty() || hide_args {
                    write!(f, "{}{}", prefix, name.name)
                } else {
                    let args_str = args
                        .iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    write!(f, "{}{}[{}]", prefix, name.name, args_str)
                }
            }
            Type::Func {
                args,
                ret,
                nullable,
            } => {
                let prefix = if *nullable { "?" } else { "" };
                let args_str = args
                    .iter()
                    .map(|(name, ty)| {
                        if let Some(n) = name {
                            format!("{} {}", n, ty)
                        } else {
                            ty.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                // Format return type: nothing for unit, parens for multiple
                let ret_str = if ret.as_ref() == &Type::unit() {
                    String::new()
                } else {
                    format!(" {}", ret)
                };
                write!(f, "{}func({}){}", prefix, args_str, ret_str)
            }
            Type::Var(v) => write!(f, "?{}", v),
            Type::Never => write!(f, "!"),
            Type::Error => write!(f, "<error>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_types() {
        let int_ty = Type::simple("int");
        assert_eq!(int_ty.to_string(), "int");

        let string_ty = Type::simple("string");
        assert_eq!(string_ty.to_string(), "string");

        let bool_ty = Type::simple("bool");
        assert_eq!(bool_ty.to_string(), "bool");
    }

    #[test]
    fn test_type_variable() {
        let var = Type::var(0);
        assert_eq!(var.to_string(), "?0");

        let var2 = Type::var(42);
        assert_eq!(var2.to_string(), "?42");
    }

    #[test]
    fn test_function_type() {
        // Unnamed parameters
        let func = Type::fun(
            vec![Type::simple("int"), Type::simple("string")],
            Type::simple("bool"),
        );
        assert_eq!(func.to_string(), "func(int, string) bool");

        // Named parameters
        let func_named = Type::fun_named(
            vec![
                (Some("a".to_string()), Type::simple("int")),
                (Some("b".to_string()), Type::simple("string")),
            ],
            Type::simple("bool"),
        );
        assert_eq!(func_named.to_string(), "func(a int, b string) bool");

        // No return type
        let func_void = Type::fun(vec![Type::simple("int")], Type::unit());
        assert_eq!(func_void.to_string(), "func(int)");
    }

    #[test]
    fn test_generic_type() {
        let option_int = Type::con_with_args(
            Symbol {
                module: ModuleId::empty(),
                name: "Option".to_string(),
                span: Span::dummy(),
            },
            vec![Type::simple("int")],
        );
        assert_eq!(option_int.to_string(), "Option[int]");
    }

    #[test]
    fn test_nested_generic() {
        let result = Type::con_with_args(
            Symbol {
                module: ModuleId::empty(),
                name: "Result".to_string(),
                span: Span::dummy(),
            },
            vec![
                Type::con_with_args(
                    Symbol {
                        module: ModuleId::empty(),
                        name: "Option".to_string(),
                        span: Span::dummy(),
                    },
                    vec![Type::simple("int")],
                ),
                Type::simple("string"),
            ],
        );
        assert_eq!(result.to_string(), "Result[Option[int], string]");
    }

    #[test]
    fn test_is_builtin_type() {
        // Primitive types
        assert!(Type::is_builtin_type("int"));
        assert!(Type::is_builtin_type("int8"));
        assert!(Type::is_builtin_type("int16"));
        assert!(Type::is_builtin_type("int32"));
        assert!(Type::is_builtin_type("int64"));
        assert!(Type::is_builtin_type("uint"));
        assert!(Type::is_builtin_type("uint8"));
        assert!(Type::is_builtin_type("uint16"));
        assert!(Type::is_builtin_type("uint32"));
        assert!(Type::is_builtin_type("uint64"));
        assert!(Type::is_builtin_type("uintptr"));
        assert!(Type::is_builtin_type("float32"));
        assert!(Type::is_builtin_type("float64"));
        assert!(Type::is_builtin_type("complex64"));
        assert!(Type::is_builtin_type("complex128"));
        assert!(Type::is_builtin_type("bool"));
        assert!(Type::is_builtin_type("string"));
        assert!(Type::is_builtin_type("byte"));
        assert!(Type::is_builtin_type("rune"));
        assert!(Type::is_builtin_type("error"));
        assert!(Type::is_builtin_type("any"));

        // User-defined types
        assert!(!Type::is_builtin_type("Config"));
        assert!(!Type::is_builtin_type("MyStruct"));
        assert!(!Type::is_builtin_type("Result"));
    }

    #[test]
    fn test_from_ast_in_module() {
        let module = ModuleId::new("mypackage");

        // Built-in type should have empty module
        let int_ast = AstType {
            name: "int".to_string(),
            args: vec![],
            span: Span::dummy(),
            nullable: false,
        };
        let int_ty = Type::from_ast_in_module(&int_ast, &module);
        if let Type::Con { sym, .. } = int_ty {
            assert!(sym.module.0.is_empty());
        } else {
            panic!("Expected Con type");
        }

        // User type should have the provided module
        let config_ast = AstType {
            name: "Config".to_string(),
            args: vec![],
            span: Span::dummy(),
            nullable: false,
        };
        let config_ty = Type::from_ast_in_module(&config_ast, &module);
        if let Type::Con { sym, .. } = config_ty {
            assert_eq!(sym.module.0, "mypackage");
        } else {
            panic!("Expected Con type");
        }

        // Qualified type should use the package from the name
        let qualified_ast = AstType {
            name: "http.Request".to_string(),
            args: vec![],
            span: Span::dummy(),
            nullable: false,
        };
        let qualified_ty = Type::from_ast_in_module(&qualified_ast, &module);
        if let Type::Con { sym, .. } = qualified_ty {
            assert_eq!(sym.module.0, "http");
        } else {
            panic!("Expected Con type");
        }

        // Pointer to user type should have module on inner type
        let ptr_ast = AstType {
            name: "*Config".to_string(),
            args: vec![AstType {
                name: "Config".to_string(),
                args: vec![],
                span: Span::dummy(),
                nullable: false,
            }],
            span: Span::dummy(),
            nullable: false,
        };
        let ptr_ty = Type::from_ast_in_module(&ptr_ast, &module);
        if let Type::Con { sym, args, .. } = ptr_ty {
            // The outer type (*Config) should also have the module
            assert_eq!(sym.module.0, "mypackage");
            // And the inner type should have it too
            if let Type::Con { sym: inner_sym, .. } = &args[0] {
                assert_eq!(inner_sym.module.0, "mypackage");
            }
        } else {
            panic!("Expected Con type");
        }
    }
}
