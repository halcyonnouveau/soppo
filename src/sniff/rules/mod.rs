//! Lint rules registry.

mod shadow;
mod try_operator;

use super::Lint;

/// Returns all available lint rules.
pub fn all_rules() -> Vec<Box<dyn Lint>> {
    vec![
        Box::new(shadow::Shadow),
        Box::new(try_operator::TryOperator),
    ]
}
