use miette::Diagnostic;
use thiserror::Error;

use crate::syntax::Span;
use crate::types::Type;

#[derive(Error, Debug, Diagnostic)]
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
}

pub type Result<T> = std::result::Result<T, SoppoError>;
