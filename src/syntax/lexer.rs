use logos::Logos;

use super::source::{FileId, LineColumn, Span};

#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\f]+")]
pub enum Token {
    // Single-line comment: // ...
    #[regex(r"//[^\n]*", allow_greedy = true, callback = |lex| lex.slice().to_string())]
    LineComment(String),

    // Multi-line comment: /* ... */
    #[regex(r"/\*([^*]|\*[^/])*\*/", allow_greedy = true, callback = |lex| lex.slice().to_string())]
    BlockComment(String),

    #[regex(r"\n+")]
    Newline,

    // Keywords
    #[token("package")]
    Package,
    #[token("func")]
    Func,
    #[token("const")]
    Const,
    #[token("type")]
    Type,
    #[token("match")]
    Match,
    #[token("case")]
    Case,
    #[token("for")]
    For,
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("return")]
    Return,
    #[token("import")]
    Import,
    #[token("true")]
    True,
    #[token("false")]
    False,
    #[token("nil")]
    Nil,
    #[token("enum")]
    Enum,
    #[token("struct")]
    Struct,
    #[token("interface")]
    Interface,
    #[token("var")]
    Var,
    #[token("go")]
    Go,
    #[token("defer")]
    Defer,
    #[token("break")]
    Break,
    #[token("continue")]
    Continue,
    #[token("range")]
    Range,
    #[token("select")]
    Select,

    // Identifiers and literals
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", priority = 1, callback = |lex| lex.slice().to_string())]
    Ident(String),

    // Float literals: 3.14, 1e-9, 1.5e10, etc.
    #[regex(r"[0-9]+\.[0-9]+([eE][+-]?[0-9]+)?|[0-9]+[eE][+-]?[0-9]+", priority = 2, callback = |lex| lex.slice().parse::<f64>().ok())]
    Float(f64),

    // Integer literals: decimal, hex (0x), binary (0b), octal (0o or leading 0)
    #[regex(r"0[xX][0-9a-fA-F]+|0[bB][01]+|0[oO][0-7]+|0[0-7]+|[0-9]+", priority = 3, callback = parse_integer)]
    Integer(i64),

    #[regex(r#""([^"\\]|\\["\\bnfrt])*""#, |lex| {
        let s = lex.slice();
        s[1..s.len()-1].to_string()
    })]
    String(String),

    // Raw string (backtick): `...` - no escape processing, can span lines
    #[regex(r"`[^`]*`", allow_greedy = true, callback = |lex| {
        let s = lex.slice();
        s[1..s.len()-1].to_string()
    })]
    RawString(String),

    // Rune literal (character): 'a', '\n', '\t', etc.
    #[regex(r"'([^'\\]|\\['\\bnfrt0])'", |lex| {
        let s = lex.slice();
        s[1..s.len()-1].to_string()
    })]
    Rune(String),

    // Operators
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("==")]
    Eq,
    #[token("!=")]
    Ne,
    #[token("<")]
    Lt,
    #[token("<-")]
    Arrow,
    #[token("<=")]
    Le,
    #[token(">")]
    Gt,
    #[token(">=")]
    Ge,
    #[token("&&")]
    And,
    #[token("||")]
    Or,
    #[token("!")]
    Not,
    #[token("&")]
    Ampersand,
    #[token("^")]
    Caret,
    #[token("<<")]
    Shl,
    #[token(">>")]
    Shr,

    // Compound assignment operators
    #[token("+=")]
    PlusAssign,
    #[token("-=")]
    MinusAssign,
    #[token("*=")]
    StarAssign,
    #[token("/=")]
    SlashAssign,
    #[token("%=")]
    PercentAssign,
    #[token("&=")]
    AmpersandAssign,
    #[token("|=")]
    PipeAssign,
    #[token("^=")]
    CaretAssign,
    #[token("<<=")]
    ShlAssign,
    #[token(">>=")]
    ShrAssign,

    // Increment/decrement
    #[token("++")]
    PlusPlus,
    #[token("--")]
    MinusMinus,

    // Delimiters
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,

    // Punctuation
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token("=")]
    Assign,
    #[token(":=")]
    ColonAssign,
    #[token("...")]
    DotDotDot,
    #[token(".")]
    Dot,
    #[token("|")]
    Pipe,
    #[token("_", priority = 2)]
    Underscore,
    #[token(";")]
    Semicolon,
    #[token("?")]
    Question,
}

/// A comment with its span information
#[derive(Debug, Clone)]
pub struct Comment {
    pub text: String,
    pub span: Span,
    pub is_block: bool,
}

pub struct Lexer<'a> {
    source: &'a str,
    file: FileId,
    lexer: logos::Lexer<'a, Token>,
    comments: Vec<Comment>,
    /// Byte offset to add to all spans (for parsing sub-expressions like string interpolation)
    byte_offset: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str, file: FileId) -> Self {
        Self {
            source,
            file,
            lexer: Token::lexer(source),
            comments: Vec::new(),
            byte_offset: 0,
        }
    }

    /// Create a lexer with a byte offset for all spans.
    /// Used when parsing sub-expressions (e.g., string interpolation) that need
    /// their spans to point to the correct location in the original source.
    pub fn new_with_offset(source: &'a str, file: FileId, byte_offset: usize) -> Self {
        Self {
            source,
            file,
            lexer: Token::lexer(source),
            comments: Vec::new(),
            byte_offset,
        }
    }

    /// Get all collected comments
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// Take ownership of collected comments
    pub fn take_comments(&mut self) -> Vec<Comment> {
        std::mem::take(&mut self.comments)
    }

    fn byte_offset_to_line_col(&self, offset: usize) -> LineColumn {
        let mut line = 1;
        let mut col = 1;

        for (i, ch) in self.source.char_indices() {
            if i >= offset {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }

        LineColumn { line, col }
    }

    pub fn next_token(&mut self) -> Option<(Token, Span)> {
        loop {
            let token = self.lexer.next()?;
            let byte_span = self.lexer.span();

            let start = self.byte_offset_to_line_col(byte_span.start);
            let end = self.byte_offset_to_line_col(byte_span.end);

            // Apply byte offset for sub-expression parsing (e.g., string interpolation)
            let span = Span::with_bytes(
                start,
                end,
                self.file,
                byte_span.start + self.byte_offset,
                byte_span.end + self.byte_offset,
            );

            match token.ok()? {
                Token::LineComment(text) => {
                    self.comments.push(Comment {
                        text,
                        span,
                        is_block: false,
                    });
                }
                Token::BlockComment(text) => {
                    self.comments.push(Comment {
                        text,
                        span,
                        is_block: true,
                    });
                }
                token => return Some((token, span)),
            }
        }
    }

    pub fn collect_all(&mut self) -> Vec<(Token, Span)> {
        let mut tokens = Vec::new();
        while let Some(token) = self.next_token() {
            tokens.push(token);
        }
        tokens
    }
}

/// Parse integer literals with various bases (decimal, hex, binary, octal)
fn parse_integer(lex: &logos::Lexer<Token>) -> Option<i64> {
    let s = lex.slice();
    if s.starts_with("0x") || s.starts_with("0X") {
        i64::from_str_radix(&s[2..], 16).ok()
    } else if s.starts_with("0b") || s.starts_with("0B") {
        i64::from_str_radix(&s[2..], 2).ok()
    } else if s.starts_with("0o") || s.starts_with("0O") {
        i64::from_str_radix(&s[2..], 8).ok()
    } else if s.starts_with('0') && s.len() > 1 && s.chars().all(|c| c.is_ascii_digit()) {
        // Legacy octal: 0755 (but not 0 itself or 09 which would be invalid)
        i64::from_str_radix(&s[1..], 8).ok()
    } else {
        s.parse::<i64>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keywords() {
        let source = "func type match case return enum struct";
        let mut lexer = Lexer::new(source, FileId(0));
        let tokens: Vec<_> = lexer.collect_all().into_iter().map(|(t, _)| t).collect();

        assert_eq!(
            tokens,
            vec![
                Token::Func,
                Token::Type,
                Token::Match,
                Token::Case,
                Token::Return,
                Token::Enum,
                Token::Struct
            ]
        );
    }

    #[test]
    fn test_identifiers_and_literals() {
        let source = r#"foo 42 "hello""#;
        let mut lexer = Lexer::new(source, FileId(0));
        let tokens: Vec<_> = lexer.collect_all().into_iter().map(|(t, _)| t).collect();

        assert_eq!(
            tokens,
            vec![
                Token::Ident("foo".to_string()),
                Token::Integer(42),
                Token::String("hello".to_string())
            ]
        );
    }

    #[test]
    fn test_operators() {
        let source = "+ - * / == != < <= > >= && || !";
        let mut lexer = Lexer::new(source, FileId(0));
        let tokens: Vec<_> = lexer.collect_all().into_iter().map(|(t, _)| t).collect();

        assert_eq!(
            tokens,
            vec![
                Token::Plus,
                Token::Minus,
                Token::Star,
                Token::Slash,
                Token::Eq,
                Token::Ne,
                Token::Lt,
                Token::Le,
                Token::Gt,
                Token::Ge,
                Token::And,
                Token::Or,
                Token::Not
            ]
        );
    }

    #[test]
    fn test_span_tracking() {
        let source = "func\nfoo";
        let mut lexer = Lexer::new(source, FileId(0));
        let tokens = lexer.collect_all();

        assert_eq!(tokens.len(), 3); // func, newline, foo

        let (_, func_span) = &tokens[0];
        assert_eq!(func_span.start.line, 1);
        assert_eq!(func_span.start.col, 1);

        let (tok, _) = &tokens[1];
        assert_eq!(*tok, Token::Newline);

        let (_, foo_span) = &tokens[2];
        assert_eq!(foo_span.start.line, 2);
        assert_eq!(foo_span.start.col, 1);
    }

    #[test]
    fn test_semicolon() {
        let source = "x = 1; y = 2";
        let mut lexer = Lexer::new(source, FileId(0));
        let tokens: Vec<_> = lexer.collect_all().into_iter().map(|(t, _)| t).collect();

        assert_eq!(
            tokens,
            vec![
                Token::Ident("x".to_string()),
                Token::Assign,
                Token::Integer(1),
                Token::Semicolon,
                Token::Ident("y".to_string()),
                Token::Assign,
                Token::Integer(2)
            ]
        );
    }

    #[test]
    fn test_integer_literals() {
        // Decimal
        let mut lexer = Lexer::new("42", FileId(0));
        let tokens: Vec<_> = lexer.collect_all().into_iter().map(|(t, _)| t).collect();
        assert_eq!(tokens, vec![Token::Integer(42)]);

        // Hex
        let mut lexer = Lexer::new("0xFF 0x1a 0X10", FileId(0));
        let tokens: Vec<_> = lexer.collect_all().into_iter().map(|(t, _)| t).collect();
        assert_eq!(
            tokens,
            vec![Token::Integer(255), Token::Integer(26), Token::Integer(16)]
        );

        // Binary
        let mut lexer = Lexer::new("0b1010 0B11", FileId(0));
        let tokens: Vec<_> = lexer.collect_all().into_iter().map(|(t, _)| t).collect();
        assert_eq!(tokens, vec![Token::Integer(10), Token::Integer(3)]);

        // Octal (new style 0o)
        let mut lexer = Lexer::new("0o755 0O644", FileId(0));
        let tokens: Vec<_> = lexer.collect_all().into_iter().map(|(t, _)| t).collect();
        assert_eq!(tokens, vec![Token::Integer(493), Token::Integer(420)]);

        // Octal (legacy style leading 0)
        let mut lexer = Lexer::new("0755 0644", FileId(0));
        let tokens: Vec<_> = lexer.collect_all().into_iter().map(|(t, _)| t).collect();
        assert_eq!(tokens, vec![Token::Integer(493), Token::Integer(420)]);

        // Zero itself should still work
        let mut lexer = Lexer::new("0", FileId(0));
        let tokens: Vec<_> = lexer.collect_all().into_iter().map(|(t, _)| t).collect();
        assert_eq!(tokens, vec![Token::Integer(0)]);
    }
}
