//! Ignore comment parsing for sniff.
//!
//! Supports `//sniff:ignore <rule>` comments to suppress specific warnings.

use std::collections::HashMap;

use crate::syntax::{Comment, Span};

/// Collection of ignore directives for a file.
#[derive(Debug, Default)]
pub struct IgnoreDirectives {
    /// Map from line number to list of rules to ignore on that line.
    /// None in the vec means ignore all rules.
    by_line: HashMap<usize, Vec<Option<String>>>,
}

impl IgnoreDirectives {
    /// Parse ignore directives from a list of comments.
    pub fn from_comments(comments: &[Comment]) -> Self {
        let mut directives = Self::default();

        for comment in comments {
            if comment.is_block {
                continue; // Only support line comments for now
            }

            // Strip // prefix and whitespace
            let text = comment.text.trim();
            let text = text.strip_prefix("//").unwrap_or(text).trim();

            // Check for sniff:ignore pattern
            if let Some(rest) = text.strip_prefix("sniff:ignore") {
                let rest = rest.trim();
                let rule = if rest.is_empty() {
                    None // Ignore all rules
                } else {
                    Some(rest.to_string())
                };

                // The directive applies to the line after the comment
                let target_line = comment.span.end.line + 1;

                directives
                    .by_line
                    .entry(target_line)
                    .or_default()
                    .push(rule);
            }
        }

        directives
    }

    /// Check if a rule should be ignored at the given span.
    pub fn should_ignore(&self, rule: &str, span: &Span) -> bool {
        let line = span.start.line;

        if let Some(rules) = self.by_line.get(&line) {
            for r in rules {
                match r {
                    None => return true, // Ignore all rules
                    Some(name) if name == rule => return true,
                    _ => {}
                }
            }
        }

        false
    }
}
