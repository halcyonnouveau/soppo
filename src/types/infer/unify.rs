use super::Infer;
use crate::error::{SoppoError, SoppoResult};
use crate::syntax::Span;
use crate::types::Type;
use crate::types::ctx::TypeDefKind;

impl Infer {
    /// Resolve type aliases to their target types
    fn resolve_alias(&self, ty: Type) -> Type {
        match &ty {
            Type::Con {
                sym: name,
                args: _,
                nullable,
            } => {
                // Check if this type name is a type alias in the current module
                if let Some(type_def) = self.global_state.current_module().types.get(&name.name)
                    && let TypeDefKind::Alias { target } = &type_def.kind
                {
                    // Recursively resolve in case of chained aliases
                    let mut resolved = target.clone();
                    // Preserve nullability from the original type
                    if *nullable {
                        resolved = resolved.as_nullable();
                    }
                    return self.resolve_alias(resolved);
                }
                ty
            }
            _ => ty,
        }
    }

    /// Unify two types (internal version that returns Result).
    ///
    /// **Prefer `unify`** which emits errors and returns bool on failure.
    /// This version should only be used when you need to explicitly check if unification failed.
    pub fn unify_inner(&mut self, t1: &Type, t2: &Type, span: &Span) -> SoppoResult<()> {
        let t1 = self.resolve_alias(self.substitute(t1.clone()));
        let t2 = self.resolve_alias(self.substitute(t2.clone()));

        // Check if either type is `any` (Go's interface{})
        // any accepts any type, so unification always succeeds
        let is_any = |ty: &Type| -> bool {
            matches!(ty, Type::Con { sym, .. } if sym.name == "any" || sym.name == "?any" || sym.name == "interface{}")
        };

        if is_any(&t1) || is_any(&t2) {
            return Ok(());
        }

        // Helper to check if type is a nilable pointer that should trigger the
        // nilable-pointer-to-interface error. We skip types from pure Go packages
        // because they're "defensively nilable" (marked nilable because Go has no
        // annotations, not because they're actually likely to be nil).
        let is_problematic_nilable_pointer = |this: &mut Self, ty: &Type| -> bool {
            ty.is_nullable()
                && matches!(ty, Type::Con { sym, .. } if sym.name.starts_with('*'))
                && !this.is_from_go_package(ty)
        };

        // Check if expected type is "error" which is a built-in interface
        if matches!(&t1, Type::Con { sym, .. } if sym.name == "error" || sym.name == "?error") {
            if is_problematic_nilable_pointer(self, &t2) {
                return Err(SoppoError::NilablePointerToInterface {
                    found: t2.to_string(),
                    span: *span,
                });
            }
            return Ok(());
        }

        // Check if expected type is an interface from a Go package
        if self.is_go_interface_type(&t1) {
            if is_problematic_nilable_pointer(self, &t2) {
                return Err(SoppoError::NilablePointerToInterface {
                    found: t2.to_string(),
                    span: *span,
                });
            }
            return Ok(());
        }

        // Check if expected type is a user-defined interface in the current module
        // Skip if the found type is also an interface - let normal type matching handle it
        if self.is_soppo_interface_type(&t1) && !self.is_soppo_interface_type(&t2) {
            // Check if the concrete type satisfies the interface
            match self.type_satisfies_interface(&t2, &t1) {
                Ok(()) => {
                    if is_problematic_nilable_pointer(self, &t2) {
                        return Err(SoppoError::NilablePointerToInterface {
                            found: t2.to_string(),
                            span: *span,
                        });
                    }
                    return Ok(());
                }
                Err(reason) => {
                    // Get interface name for error message
                    let interface_name = match &t1 {
                        Type::Con { sym, .. } => sym.name.clone(),
                        _ => t1.to_string(),
                    };
                    return Err(SoppoError::InterfaceNotSatisfied {
                        found: t2.to_string(),
                        interface_name,
                        reason,
                        span: *span,
                    });
                }
            }
        }

        match (&t1, &t2) {
            // Error type unifies with anything (suppresses cascading errors)
            (Type::Error, _) | (_, Type::Error) => Ok(()),

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
                        span: *span,
                    });
                }
                self.substitutions.insert(*a, ty.clone());
                Ok(())
            }

            // Compatible numeric types: allow int/int8/int16/etc. to unify
            (
                Type::Con {
                    sym: n1,
                    args: a1,
                    nullable: nullable1,
                },
                Type::Con {
                    sym: n2,
                    args: a2,
                    nullable: nullable2,
                },
            ) if a1.is_empty() && a2.is_empty() && are_compatible_numeric(&n1.name, &n2.name) => {
                // Check nullability: non-nilable cannot receive nilable
                if !nullable1 && *nullable2 {
                    return Err(SoppoError::NilableToNonNilable {
                        expected: n1.name.clone(),
                        found: format!("?{}", n2.name),
                        span: *span,
                    });
                }
                Ok(())
            }

            // Go package type aliases: allow primitive literals to be assigned to type aliases
            // with compatible underlying types (e.g., int literal to fs.FileMode which is uint32,
            // string literal to a string type alias, etc.)
            (
                Type::Con {
                    sym: n1,
                    args: a1,
                    nullable: nullable1,
                },
                Type::Con {
                    sym: n2,
                    args: a2,
                    nullable: nullable2,
                },
            ) if a1.is_empty()
                && a2.is_empty()
                && is_primitive_literal_type(&n2.name)
                && !n1.module.0.is_empty() =>
            {
                // Type has a module set (e.g., from a Go package like "os" or "io/fs")
                // Get the package alias from import_path (last component)
                let pkg_alias = n1.module.0.rsplit('/').next().unwrap_or(&n1.module.0);

                // Try to resolve the underlying type
                if let Some(underlying) = self.get_underlying_type(pkg_alias, &n1.name) {
                    // Check if the underlying type is compatible with the actual type
                    let compatible = if is_numeric_type(&n2.name) {
                        are_compatible_numeric(&underlying, &n2.name)
                    } else {
                        // For string/bool: exact match
                        underlying == n2.name
                    };

                    if compatible {
                        if !nullable1 && *nullable2 {
                            return Err(SoppoError::NilableToNonNilable {
                                expected: n1.name.clone(),
                                found: format!("?{}", n2.name),
                                span: *span,
                            });
                        }
                        return Ok(());
                    }
                }
                // Fall through to mismatch
                Err(SoppoError::TypeMismatch {
                    expected: Box::new(t1.clone()),
                    found: Box::new(t2.clone()),
                    span: *span,
                })
            }

            // Pointer types: unify underlying types even if names differ (e.g., *T vs *?0)
            (
                Type::Con {
                    sym: n1,
                    args: a1,
                    nullable: nullable1,
                },
                Type::Con {
                    sym: n2,
                    args: a2,
                    nullable: nullable2,
                },
            ) if n1.name.starts_with('*') && n2.name.starts_with('*') && n1.name != n2.name => {
                // Check nullability: non-nilable cannot receive nilable
                if !nullable1 && *nullable2 {
                    return Err(SoppoError::NilableToNonNilable {
                        expected: n1.name.clone(),
                        found: format!("?{}", n2.name),
                        span: *span,
                    });
                }
                // Unify the underlying types via args if both have them
                if !a1.is_empty() && !a2.is_empty() {
                    for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                        self.unify_inner(arg1, arg2, span)?;
                    }
                }
                Ok(())
            }

            // Slice types: unify element types even if names differ (e.g., []T vs []?0)
            (
                Type::Con {
                    sym: n1,
                    args: a1,
                    nullable: nullable1,
                },
                Type::Con {
                    sym: n2,
                    args: a2,
                    nullable: nullable2,
                },
            ) if n1.name.starts_with("[]") && n2.name.starts_with("[]") && n1.name != n2.name => {
                if !nullable1 && *nullable2 {
                    return Err(SoppoError::NilableToNonNilable {
                        expected: n1.name.clone(),
                        found: format!("?{}", n2.name),
                        span: *span,
                    });
                }
                if !a1.is_empty() && !a2.is_empty() {
                    for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                        self.unify_inner(arg1, arg2, span)?;
                    }
                }
                Ok(())
            }

            // Channel types: unify element types even if names differ (e.g., chan T vs chan ?0)
            (
                Type::Con {
                    sym: n1,
                    args: a1,
                    nullable: nullable1,
                },
                Type::Con {
                    sym: n2,
                    args: a2,
                    nullable: nullable2,
                },
            ) if n1.name.starts_with("chan ")
                && n2.name.starts_with("chan ")
                && n1.name != n2.name =>
            {
                if !nullable1 && *nullable2 {
                    return Err(SoppoError::NilableToNonNilable {
                        expected: n1.name.clone(),
                        found: format!("?{}", n2.name),
                        span: *span,
                    });
                }
                if !a1.is_empty() && !a2.is_empty() {
                    for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                        self.unify_inner(arg1, arg2, span)?;
                    }
                }
                Ok(())
            }

            // Same constructor: unify arguments
            (
                Type::Con {
                    sym: n1,
                    args: a1,
                    nullable: nullable1,
                },
                Type::Con {
                    sym: n2,
                    args: a2,
                    nullable: nullable2,
                },
            ) if n1.name == n2.name => {
                // Check nullability: non-nilable cannot receive nilable
                if !nullable1 && *nullable2 {
                    return Err(SoppoError::NilableToNonNilable {
                        expected: n1.name.clone(),
                        found: format!("?{}", n2.name),
                        span: *span,
                    });
                }
                // For composite types (slices, pointers, maps, channels) where the element type
                // is encoded in the name (e.g., "[]string", "*int"), we allow mismatched args
                // since both representations are equivalent when names match
                let is_composite = n1.name.starts_with("[]")
                    || n1.name.starts_with('*')
                    || n1.name.starts_with("map[")
                    || n1.name.starts_with("chan ");

                if !is_composite && a1.len() != a2.len() {
                    return Err(SoppoError::Type {
                        message: format!(
                            "Type constructor {} has wrong number of arguments",
                            n1.name
                        ),
                        span: *span,
                    });
                }
                for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                    self.unify_inner(arg1, arg2, span)?;
                }
                Ok(())
            }

            // Functions: unify args and return (ignoring parameter names)
            (
                Type::Func {
                    args: a1,
                    ret: r1,
                    nullable: nullable1,
                },
                Type::Func {
                    args: a2,
                    ret: r2,
                    nullable: nullable2,
                },
            ) => {
                // Check nullability: non-nilable cannot receive nilable
                if !nullable1 && *nullable2 {
                    return Err(SoppoError::NilableToNonNilable {
                        expected: "func".to_string(),
                        found: "?func".to_string(),
                        span: *span,
                    });
                }
                // Check if a1 (expected) has a variadic last parameter
                let has_variadic = a1.last().is_some_and(|(_, last_ty)| {
                    matches!(last_ty, Type::Con { sym, .. } if sym.name == "variadic" || sym.name.starts_with("..."))
                });

                if has_variadic {
                    // Variadic function: check non-variadic params match, then variadic can consume 0+
                    let fixed_params = &a1[..a1.len() - 1];
                    let (_, variadic_param_ty) = a1.last().expect("checked above");

                    // Extract the variadic element type
                    let variadic_elem = if let Type::Con { args, .. } = variadic_param_ty {
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
                            span: *span,
                        });
                    }

                    // Unify fixed params (ignoring names)
                    for ((_, ty1), (_, ty2)) in fixed_params.iter().zip(a2.iter()) {
                        self.unify_inner(ty1, ty2, span)?;
                    }

                    // Unify remaining args against variadic element type
                    for (_, ty2) in a2.iter().skip(fixed_params.len()) {
                        // For "any" type (or nullable any), any argument is valid
                        let is_any = match &variadic_elem {
                            Type::Con { sym: name, .. } => name.name == "any",
                            _ => false,
                        };
                        if !is_any {
                            self.unify_inner(&variadic_elem, ty2, span)?;
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
                            span: *span,
                        });
                    }
                    // Unify types, ignoring parameter names
                    for ((_, ty1), (_, ty2)) in a1.iter().zip(a2.iter()) {
                        self.unify_inner(ty1, ty2, span)?;
                    }
                }
                self.unify_inner(r1, r2, span)?;
                Ok(())
            }

            // Mismatch
            _ => Err(SoppoError::TypeMismatch {
                expected: Box::new(t1.clone()),
                found: Box::new(t2.clone()),
                span: *span,
            }),
        }
    }

    /// Unify two types, emitting error if they don't match.
    ///
    /// This is the default unification method that collects errors. Returns true if
    /// unification succeeded, false if an error was emitted. If either type is
    /// `Type::Error`, returns true without emitting errors (to prevent cascading).
    pub fn unify(&mut self, t1: &Type, t2: &Type, span: &Span) -> bool {
        // If either type is already an error, don't emit more errors
        if t1.is_error() || t2.is_error() {
            return true;
        }
        match self.unify_inner(t1, t2, span) {
            Ok(()) => true,
            Err(e) => {
                self.emit_error(e);
                false
            }
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
            Type::Con {
                sym: name,
                args,
                nullable,
            } => Type::Con {
                sym: name,
                args: args.into_iter().map(|a| self.substitute(a)).collect(),
                nullable,
            },
            Type::Func {
                args,
                ret,
                nullable,
            } => Type::Func {
                args: args
                    .into_iter()
                    .map(|(name, ty)| (name, self.substitute(ty)))
                    .collect(),
                ret: Box::new(self.substitute(*ret)),
                nullable,
            },
            Type::Never => Type::Never,
            Type::Error => Type::Error,
        }
    }
}

/// All numeric types in Go
const INT_TYPES: &[&str] = &[
    "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32", "uint64",
    "uintptr", "byte", "rune",
];
const FLOAT_TYPES: &[&str] = &["float32", "float64"];

/// Check if a single type name is numeric (integer or float)
fn is_numeric_type(t: &str) -> bool {
    INT_TYPES.contains(&t) || FLOAT_TYPES.contains(&t)
}

/// Check if a type is a primitive literal type (can be an untyped constant in Go)
fn is_primitive_literal_type(t: &str) -> bool {
    INT_TYPES.contains(&t) || FLOAT_TYPES.contains(&t) || t == "string" || t == "bool"
}

/// Check if two type names are compatible numeric types.
/// In Go, numeric literals are untyped and can be assigned to any compatible numeric type.
pub(super) fn are_compatible_numeric(t1: &str, t2: &str) -> bool {
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
        Type::Func { args, ret, .. } => {
            args.iter().any(|(_, arg_ty)| occurs(var, arg_ty)) || occurs(var, ret)
        }
        Type::Never => false,
        Type::Error => false,
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
        assert!(infer.unify_inner(&t1, &t2, &Span::dummy()).is_ok());

        let t3 = Type::simple("string");
        assert!(infer.unify_inner(&t1, &t3, &Span::dummy()).is_err());
    }

    #[test]
    fn test_occurs_check() {
        // Simple occurrence
        assert!(occurs(0, &Type::var(0)));
        assert!(!occurs(0, &Type::var(1)));

        // In constructor args
        let nested = Type::Con {
            sym: Symbol {
                module: ModuleId::empty(),
                name: "List".to_string(),
                span: Span::dummy(),
            },
            args: vec![Type::var(0)],
            nullable: false,
        };
        assert!(occurs(0, &nested));
        assert!(!occurs(1, &nested));

        // In function types
        let func = Type::fun(vec![Type::var(0)], Type::simple("int"));
        assert!(occurs(0, &func));
        assert!(!occurs(1, &func));
    }
}
