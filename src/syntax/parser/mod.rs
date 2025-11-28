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
}
