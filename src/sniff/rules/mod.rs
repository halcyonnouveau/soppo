//! Lint rules registry.

mod shadow;
mod try_operator;
mod unreachable;

use super::Lint;

/// Returns all available lint rules.
pub fn all_rules() -> Vec<Box<dyn Lint>> {
    vec![
        Box::new(shadow::Shadow),
        Box::new(try_operator::TryOperator),
        Box::new(unreachable::Unreachable),
    ]
}
