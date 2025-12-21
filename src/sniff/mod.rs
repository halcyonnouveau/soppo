// rustc false positive: fields are used in #[error] and #[label] proc macro attributes
#![allow(unused_assignments)]

//! Sniff - Soppo's linter
//!
//! Provides optional lint rules for code style and best practices.

mod ignore;
pub mod rules;

use std::collections::HashSet;

use ignore::IgnoreDirectives;
use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;

use crate::syntax::Span;
use crate::types::ast::TypedFile;

/// Configuration for the linter.
#[derive(Debug, Default)]
pub struct LintConfig {
    /// Rule codes to disable.
    pub ignored: HashSet<String>,
}

/// A lint warning.
#[allow(unused)]
#[derive(Debug, Error, Diagnostic)]
#[error("{message}")]
#[diagnostic(severity(Warning))]
pub struct LintWarning {
    pub code: &'static str,
    pub message: String,

    #[source_code]
    pub src: NamedSource<String>,

    #[label("{label}")]
    pub span: SourceSpan,
    pub label: String,

    #[help]
    pub help: Option<String>,

    /// Original span for line-based ignore checking.
    original_span: Span,
}

impl LintWarning {
    pub fn new(
        code: &'static str,
        message: impl Into<String>,
        span: Span,
        label: impl Into<String>,
        source_name: &str,
        source_code: &str,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            src: NamedSource::new(source_name, source_code.to_string()),
            span: (span.byte_start, span.byte_end - span.byte_start).into(),
            label: label.into(),
            help: None,
            original_span: span,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

/// Lint trait that all rules must implement.
pub trait Lint: Send + Sync {
    /// The unique code for this lint (e.g., "try_operator").
    fn code(&self) -> &'static str;

    /// Check the file and return any warnings.
    fn check(&self, file: &TypedFile, source_name: &str, source_code: &str) -> Vec<LintWarning>;
}

/// Lint a typed file with the given configuration.
pub fn lint_file(
    file: &TypedFile,
    source_name: &str,
    source_code: &str,
    config: &LintConfig,
) -> Vec<LintWarning> {
    let mut warnings = vec![];

    // Parse ignore directives from comments
    let ignores = IgnoreDirectives::from_comments(&file.comments);

    for rule in rules::all_rules() {
        if !config.ignored.contains(rule.code()) {
            for warning in rule.check(file, source_name, source_code) {
                // Skip warnings that are ignored by comment directives
                if !ignores.should_ignore(warning.code, &warning.original_span) {
                    warnings.push(warning);
                }
            }
        }
    }

    warnings
}
