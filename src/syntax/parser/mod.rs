mod expr;
mod item;
mod pat;
mod stmt;
mod ty;

use super::lexer::{Comment, Lexer, Token};
use super::source::{FileId, Span};
use crate::error::{Result, SoppoError};

// Reserved keywords that cannot be used as identifiers
const RESERVED_KEYWORDS: &[&str] = &[
    // Go keywords
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
    // Soppo-specific
    "match",
    "enum",
];

pub struct Parser {
    tokens: Vec<(Token, Span)>,
    pos: usize,
    file: FileId,
    comments: Vec<Comment>,
}

impl Parser {
    pub fn new(source: &str, file: FileId) -> Self {
        let mut lexer = Lexer::new(source, file);
        let tokens = lexer.collect_all();
        let comments = lexer.take_comments();

        Self {
            tokens,
            pos: 0,
            file,
            comments,
        }
    }

    /// Create a parser with a byte offset for all spans.
    /// Used when parsing sub-expressions (e.g., string interpolation) that need
    /// their spans to point to the correct location in the original source.
    pub fn new_with_offset(source: &str, file: FileId, byte_offset: usize) -> Self {
        let mut lexer = Lexer::new_with_offset(source, file, byte_offset);
        let tokens = lexer.collect_all();
        let comments = lexer.take_comments();

        Self {
            tokens,
            pos: 0,
            file,
            comments,
        }
    }

    /// Peek at current token without consuming
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|(tok, _)| tok)
    }

    /// Peek at token at offset from current position
    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset).map(|(tok, _)| tok)
    }

    /// Peek at current span
    fn peek_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|(_, span)| *span)
            .unwrap_or_else(Span::dummy)
    }

    /// Get span of previous token (the one we just consumed)
    fn previous_span(&self) -> Span {
        if self.pos > 0 {
            self.tokens[self.pos - 1].1
        } else {
            Span::dummy()
        }
    }

    /// Check if current token matches expected (without consuming)
    fn check(&self, expected: &Token) -> bool {
        self.peek() == Some(expected)
    }

    /// Check if next non-newline token matches expected
    fn peek_next_is(&self, expected: &Token) -> bool {
        let mut offset = 1;
        while let Some(tok) = self.peek_at(offset) {
            if tok == &Token::Newline {
                offset += 1;
                continue;
            }
            return tok == expected;
        }
        false
    }

    /// Consume current token and return it with span
    fn advance(&mut self) -> Option<(Token, Span)> {
        if self.pos < self.tokens.len() {
            let result = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(result)
        } else {
            None
        }
    }

    /// Check if current token matches, consume if so
    fn consume(&mut self, expected: &Token) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Skip statement terminators (newlines and semicolons)
    fn skip_terminators(&mut self) {
        while matches!(self.peek(), Some(Token::Newline) | Some(Token::Semicolon)) {
            self.advance();
        }
    }

    /// Check if current token is a statement terminator
    fn is_terminator(&self) -> bool {
        matches!(self.peek(), Some(Token::Newline) | Some(Token::Semicolon))
    }

    /// Expect a specific token, error if not present
    fn expect(&mut self, expected: Token) -> Result<Span> {
        match self.advance() {
            Some((tok, span)) if tok == expected => Ok(span),
            Some((tok, span)) => Err(SoppoError::Parse {
                message: format!("Expected {:?}, found {:?}", expected, tok),
                span,
            }),
            None => Err(SoppoError::Parse {
                message: format!("Expected {:?}, found EOF", expected),
                span: Span::dummy(),
            }),
        }
    }

    /// Check if an identifier is a reserved keyword
    fn is_reserved_keyword(name: &str) -> bool {
        RESERVED_KEYWORDS.contains(&name)
    }

    /// Validate that an identifier is not a reserved keyword
    fn validate_identifier(&self, name: &str, span: &Span) -> Result<()> {
        if Self::is_reserved_keyword(name) {
            Err(SoppoError::Parse {
                message: format!(
                    "`{}` is a reserved keyword and cannot be used as an identifier",
                    name
                ),
                span: *span,
            })
        } else {
            Ok(())
        }
    }

    /// Find doc comments that immediately precede the given span.
    /// Doc comments are comments ending on the line immediately before the declaration,
    /// or consecutive comment lines leading up to that line.
    fn get_doc_comment(&self, decl_start_line: usize) -> Option<String> {
        if decl_start_line == 0 {
            return None;
        }

        // Collect all comments that are on consecutive lines ending right before decl
        let mut doc_lines: Vec<&str> = Vec::new();
        let mut expected_line = decl_start_line - 1;

        // Sort comments by line (descending) to process from closest to decl
        let mut relevant_comments: Vec<_> = self
            .comments
            .iter()
            .filter(|c| c.span.end.line < decl_start_line)
            .collect();
        relevant_comments.sort_by(|a, b| b.span.end.line.cmp(&a.span.end.line));

        for comment in relevant_comments {
            if comment.span.end.line == expected_line {
                // Strip comment markers and trim
                let text = comment.text.trim();
                let text = text.strip_prefix("//").unwrap_or(text).trim();
                doc_lines.push(text);
                expected_line = comment.span.start.line.saturating_sub(1);
            } else if comment.span.end.line < expected_line {
                // Gap between comments and decl - stop looking
                break;
            }
        }

        if doc_lines.is_empty() {
            None
        } else {
            // Reverse to get comments in top-to-bottom order
            doc_lines.reverse();
            Some(doc_lines.join("\n"))
        }
    }
}
