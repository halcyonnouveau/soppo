use std::fmt;

use crate::syntax::{ModuleId, Span, Symbol, Type as AstType};

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
        name: Symbol,
        args: Vec<Type>,
        nullable: bool, // true for ?*T, ?[]T, ?Interface
    },

    /// Function type: fn(int, string) -> bool
    Fun {
        args: Vec<Type>,
        ret: Box<Type>,
        nullable: bool, // true for ?func types
    },

    /// Type variable for inference
    Var(i32),

    /// Never type (diverging, like return/break/continue)
    Never,
}

impl Type {
    /// Dummy type for initialisation
    pub fn dummy() -> Self {
        Type::Var(-1)
    }

    /// Create a simple type from a string (for built-in types)
    pub fn simple(name: &str) -> Self {
        Type::Con {
            name: Symbol {
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
            name: Symbol {
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

    /// Create a type variable
    pub fn var(id: i32) -> Self {
        Type::Var(id)
    }

    /// Create a function type
    pub fn fun(args: Vec<Type>, ret: Type) -> Self {
        Type::Fun {
            args,
            ret: Box::new(ret),
            nullable: false,
        }
    }

    /// Create a generic type with arguments
    pub fn con_with_args(name: Symbol, args: Vec<Type>) -> Self {
        Type::Con {
            name,
            args,
            nullable: false,
        }
    }

    /// Create an array type with element type
    pub fn array(element_type: Type) -> Self {
        Type::Con {
            name: Symbol {
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
            name: Symbol {
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
            name: Symbol {
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
            Type::Con { name, .. } => name.name.clone(),
            _ => "?".to_string(),
        };
        Type::Con {
            name: Symbol {
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
        // Handle variadic types: ...T -> variadic[T]
        if ast_ty.name.starts_with("...") {
            let inner_name = &ast_ty.name[3..];
            return Type::generic("variadic", vec![Type::simple(inner_name)]);
        }

        Type::Con {
            name: Symbol {
                module: ModuleId::empty(),
                name: ast_ty.name.clone(),
                span: ast_ty.span,
            },
            args: ast_ty.args.iter().map(Type::from_ast).collect(),
            nullable: ast_ty.nullable,
        }
    }

    /// Check if this type is nullable
    pub fn is_nullable(&self) -> bool {
        match self {
            Type::Con { nullable, .. } => *nullable,
            Type::Fun { nullable, .. } => *nullable,
            Type::Var(_) => false, // Type variables are not inherently nullable
            Type::Never => false,
        }
    }

    /// Return a nullable version of this type
    pub fn as_nullable(self) -> Self {
        match self {
            Type::Con {
                name,
                args,
                nullable: _,
            } => Type::Con {
                name,
                args,
                nullable: true,
            },
            Type::Fun {
                args,
                ret,
                nullable: _,
            } => Type::Fun {
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
                name,
                args,
                nullable: _,
            } => Type::Con {
                name,
                args,
                nullable: false,
            },
            Type::Fun {
                args,
                ret,
                nullable: _,
            } => Type::Fun {
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
            Type::Con { name, .. } => {
                let n = &name.name;
                n.starts_with('*')           // pointer
                    || n.starts_with("[]")   // slice
                    || n.starts_with("map[") // map
                    || n.starts_with("chan ") // channel
                    || n == "error"          // error interface
                    || n == "any"            // any/interface{}
                    || n == "interface{}" // explicit interface
            }
            Type::Fun { .. } => true, // function types can be nil
            Type::Var(_) => false,
            Type::Never => false,
        }
    }

    /// Check if this is a Go interface type (error, any, interface{})
    /// These are always implicitly nilable in Go
    pub fn is_go_interface(&self) -> bool {
        match self {
            Type::Con { name, .. } => {
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
                name,
                args,
                nullable,
            } => {
                let prefix = if *nullable { "?" } else { "" };
                let n = &name.name;
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
            Type::Fun {
                args,
                ret,
                nullable,
            } => {
                let prefix = if *nullable { "?" } else { "" };
                let args_str = args
                    .iter()
                    .map(|a| a.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{}fn({}) -> {}", prefix, args_str, ret)
            }
            Type::Var(v) => write!(f, "?{}", v),
            Type::Never => write!(f, "!"),
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
        let func = Type::fun(
            vec![Type::simple("int"), Type::simple("string")],
            Type::simple("bool"),
        );
        assert_eq!(func.to_string(), "fn(int, string) -> bool");
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
}
