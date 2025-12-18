// rustc false positive: fields are used in #[error] and #[label] proc macro attributes
#![allow(unused_assignments)]

use miette::Diagnostic;
use thiserror::Error;

use crate::syntax::Span;
use crate::types::Type;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum SoppoError {
    #[error("Syntax error: {message}")]
    #[diagnostic(code(soppo::parse))]
    Parse {
        message: String,
        #[label("unexpected token")]
        span: Span,
    },

    #[error("Type error: {message}")]
    #[diagnostic(code(soppo::type_error))]
    Type {
        message: String,
        #[label("error occurred here")]
        span: Span,
    },

    #[error("Mismatched types")]
    #[diagnostic(
        code(soppo::type_mismatch),
        help("Change this expression to produce `{expected}`, or change the expected type")
    )]
    TypeMismatch {
        expected: Box<Type>,
        found: Box<Type>,
        #[label("expected `{expected}`, found `{found}`")]
        span: Span,
    },

    #[error("Cannot find value `{name}` in this scope")]
    #[diagnostic(
        code(soppo::undefined_variable),
        help("Use `:=` to declare a new variable: `{name} := <value>`")
    )]
    UndefinedVariable {
        name: String,
        #[label("not found in this scope")]
        span: Span,
    },

    #[error("Non-exhaustive match")]
    #[diagnostic(
        code(soppo::non_exhaustive),
        help("Ensure all enum variants are handled, or add a `default` case")
    )]
    NonExhaustive {
        missing: Vec<String>,
        #[label("missing variants: {}", missing.join(", "))]
        span: Span,
    },

    #[error("Cannot resolve sop: import without module context")]
    #[diagnostic(
        code(soppo::codegen),
        help("Run `sop build` from a directory containing go.mod, or specify files explicitly")
    )]
    MissingModuleContext {
        import_path: String,
        #[label("sop: imports require a go.mod project")]
        span: Span,
    },

    #[error("Potential nil pointer dereference")]
    #[diagnostic(
        code(soppo::nil_pointer),
        help("check for nil first, or use `.(!nil)` to assert non-nil")
    )]
    NilPointer {
        name: String,
        #[label("this pointer may be nil")]
        span: Span,
    },

    #[error("`?` requires enclosing function to return error")]
    #[diagnostic(
        code(soppo::try_no_error_return),
        help("add `error` as the last return type of this function")
    )]
    TryNoErrorReturn {
        #[label("`?` cannot be used here")]
        span: Span,
    },

    #[error("`?` requires expression to return error")]
    #[diagnostic(
        code(soppo::try_expr_no_error),
        help("the expression must return `error` or `(T, error)`")
    )]
    TryExprNoError {
        #[label("this expression doesn't return error")]
        span: Span,
    },

    #[error("Cannot assign `nil` to non-nilable type `{ty}`")]
    #[diagnostic(
        code(soppo::nil_to_non_nilable),
        help("use `?{ty}` for a nilable type")
    )]
    NilToNonNilable {
        ty: String,
        #[label("cannot be nil")]
        span: Span,
    },

    #[error("Non-nilable type `{ty}` requires initialisation")]
    #[diagnostic(
        code(soppo::non_nilable_no_init),
        help("provide an initial value, or use `?{ty}` for a nilable type")
    )]
    NonNilableNoInit {
        ty: String,
        #[label("zero value would be nil")]
        span: Span,
    },

    #[error("Cannot assign nilable `{found}` to non-nilable `{expected}`")]
    #[diagnostic(
        code(soppo::nilable_to_non_nilable),
        help("use a nil check or `.(!nil)` to assert non-nil")
    )]
    NilableToNonNilable {
        expected: String,
        found: String,
        #[label("expected non-nilable")]
        span: Span,
    },

    #[error("Circular dependency detected\n\n{}", format_cycle(cycle))]
    #[diagnostic(
        code(soppo::circular_dependency),
        help("break the cycle by extracting shared code into a separate package")
    )]
    CircularDependency {
        /// List of (source_file, import_path) pairs forming the cycle
        cycle: Vec<(String, String)>,
    },

    #[error("Generic unit variant `{enum_name}.{variant_name}` requires type arguments")]
    #[diagnostic(
        code(soppo::generic_unit_variant),
        help("use `{enum_name}.{variant_name}[T]` syntax")
    )]
    GenericUnitVariant {
        enum_name: String,
        variant_name: String,
        #[label("missing type arguments")]
        span: Span,
    },

    #[error("Type `{ty}` does not satisfy constraint `{constraint}`")]
    #[diagnostic(code(soppo::constraint_not_satisfied), help("{hint}"))]
    ConstraintNotSatisfied {
        ty: String,
        constraint: String,
        hint: String,
        #[label("type argument does not satisfy `{constraint}`")]
        span: Span,
    },

    #[error("Variable `{name}` shadows imported package `{name}`")]
    #[diagnostic(
        code(soppo::shadows_import),
        help("use a different variable name to avoid confusion")
    )]
    ShadowsImport {
        name: String,
        #[label("shadows import")]
        span: Span,
    },
}

fn format_cycle(cycle: &[(String, String)]) -> String {
    cycle
        .iter()
        .map(|(file, import)| format!("  {} imports \"{}\"\n    ↓", file, import))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end_matches("\n    ↓")
        .to_string()
}

pub type Result<T> = std::result::Result<T, SoppoError>;
