use crate::source::Span;
use crate::ty::Type;
use miette::Diagnostic;
use thiserror::Error;

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
}

pub type Result<T> = std::result::Result<T, SoppoError>;
