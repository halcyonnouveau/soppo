use logos::Logos;

use super::source::{FileId, LineColumn, Span};

#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\f]+")]
#[logos(skip r"//[^\n]*")]
pub enum Token {
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

    // Identifiers and literals
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", priority = 1, callback = |lex| lex.slice().to_string())]
    Ident(String),

    // Float literals: 3.14, 1e-9, 1.5e10, etc.
    #[regex(r"[0-9]+\.[0-9]+([eE][+-]?[0-9]+)?|[0-9]+[eE][+-]?[0-9]+", priority = 2, callback = |lex| lex.slice().parse::<f64>().ok())]
    Float(f64),

    #[regex(r"[0-9]+", |lex| lex.slice().parse::<i64>().ok())]
    Integer(i64),

    #[regex(r#""([^"\\]|\\["\\bnfrt])*""#, |lex| {
        let s = lex.slice();
        s[1..s.len()-1].to_string()
    })]
    String(String),

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
}

pub struct Lexer<'a> {
    source: &'a str,
    file: FileId,
    lexer: logos::Lexer<'a, Token>,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str, file: FileId) -> Self {
        Self {
            source,
            file,
            lexer: Token::lexer(source),
        }
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
        let token = self.lexer.next()?;
        let byte_span = self.lexer.span();

        let start = self.byte_offset_to_line_col(byte_span.start);
        let end = self.byte_offset_to_line_col(byte_span.end);

        let span = Span::with_bytes(start, end, self.file, byte_span.start, byte_span.end);

        token.ok().map(|t| (t, span))
    }

    pub fn collect_all(&mut self) -> Vec<(Token, Span)> {
        let mut tokens = Vec::new();
        while let Some(token) = self.next_token() {
            tokens.push(token);
        }
        tokens
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
}
