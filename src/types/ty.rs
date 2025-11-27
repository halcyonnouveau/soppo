use std::fmt;

use crate::parse::{ModuleId, Span, Symbol, Type as AstType};

/// Type representation after type checking
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// Concrete type constructor: int, string, Option[T]
    Con { name: Symbol, args: Vec<Type> },

    /// Function type: fn(int, string) -> bool
    Fun { args: Vec<Type>, ret: Box<Type> },

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
        }
    }

    /// Create a generic type with arguments
    pub fn con_with_args(name: Symbol, args: Vec<Type>) -> Self {
        Type::Con { name, args }
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
        }
    }

    /// Convert an AST type to a runtime type
    pub fn from_ast(ast_ty: &AstType) -> Self {
        Type::Con {
            name: Symbol {
                module: ModuleId::empty(),
                name: ast_ty.name.clone(),
                span: ast_ty.span.clone(),
            },
            args: ast_ty.args.iter().map(Type::from_ast).collect(),
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Con { name, args } => {
                if args.is_empty() {
                    write!(f, "{}", name.name)
                } else {
                    let args_str = args
                        .iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    write!(f, "{}[{}]", name.name, args_str)
                }
            }
            Type::Fun { args, ret } => {
                let args_str = args
                    .iter()
                    .map(|a| a.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "fn({}) -> {}", args_str, ret)
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
        let func = Type::fun(vec![Type::simple("int"), Type::simple("string")], Type::simple("bool"));
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
