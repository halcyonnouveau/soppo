use super::Infer;
use crate::error::{Result, SoppoError};
use crate::syntax::Span;
use crate::types::Type;

impl Infer {
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
                if occurs(*a, ty) {
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
                if a1.is_empty() && a2.is_empty() && are_compatible_numeric(&n1.name, &n2.name) =>
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
}

/// Check if two type names are compatible numeric types.
/// In Go, numeric literals are untyped and can be assigned to any compatible numeric type.
pub(super) fn are_compatible_numeric(t1: &str, t2: &str) -> bool {
    const INT_TYPES: &[&str] = &[
        "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32", "uint64",
        "uintptr", "byte", "rune",
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
pub fn occurs(var: i32, ty: &Type) -> bool {
    match ty {
        Type::Var(v) => *v == var,
        Type::Con { args, .. } => args.iter().any(|arg| occurs(var, arg)),
        Type::Fun { args, ret } => args.iter().any(|arg| occurs(var, arg)) || occurs(var, ret),
        Type::Never => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{ModuleId, Symbol};

    #[test]
    fn test_unification() {
        let mut infer = Infer::new().unwrap();

        let t1 = Type::simple("int");
        let t2 = Type::simple("int");
        assert!(infer.unify(&t1, &t2, &Span::dummy()).is_ok());

        let t3 = Type::simple("string");
        assert!(infer.unify(&t1, &t3, &Span::dummy()).is_err());
    }

    #[test]
    fn test_occurs_check() {
        // Simple occurrence
        assert!(occurs(0, &Type::var(0)));
        assert!(!occurs(0, &Type::var(1)));

        // In constructor args
        let nested = Type::Con {
            name: Symbol {
                module: ModuleId::empty(),
                name: "List".to_string(),
                span: Span::dummy(),
            },
            args: vec![Type::var(0)],
        };
        assert!(occurs(0, &nested));
        assert!(!occurs(1, &nested));

        // In function types
        let func = Type::fun(vec![Type::var(0)], Type::simple("int"));
        assert!(occurs(0, &func));
        assert!(!occurs(1, &func));
    }
}
