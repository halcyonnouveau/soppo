/// Position in source code (line and column)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineColumn {
    pub line: usize,
    pub col: usize,
}

/// Unique identifier for a source file
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub usize);

/// Span in source code - tracks location for error messages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: LineColumn,
    pub end: LineColumn,
    pub file: FileId,
    /// Byte offsets for miette error reporting
    pub byte_start: usize,
    pub byte_end: usize,
}

// Convert Span to miette's SourceSpan for error reporting
impl From<Span> for miette::SourceSpan {
    fn from(span: Span) -> Self {
        miette::SourceSpan::from(span.byte_start..span.byte_end)
    }
}

impl From<&Span> for miette::SourceSpan {
    fn from(span: &Span) -> Self {
        miette::SourceSpan::from(span.byte_start..span.byte_end)
    }
}

impl Span {
    pub fn new(start: LineColumn, end: LineColumn, file: FileId) -> Self {
        Self {
            start,
            end,
            file,
            byte_start: 0,
            byte_end: 1,
        }
    }

    pub fn with_bytes(
        start: LineColumn,
        end: LineColumn,
        file: FileId,
        byte_start: usize,
        byte_end: usize,
    ) -> Self {
        Self {
            start,
            end,
            file,
            byte_start,
            byte_end,
        }
    }

    pub fn dummy() -> Self {
        Self {
            start: LineColumn { line: 0, col: 0 },
            end: LineColumn { line: 0, col: 0 },
            file: FileId(0),
            byte_start: 0,
            byte_end: 1,
        }
    }
}

/// Unique identifier for a module
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleId(pub String);

impl ModuleId {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn empty() -> Self {
        Self(String::new())
    }
}

/// A symbol with module context - used for type names, function names, etc.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol {
    pub module: ModuleId,
    pub name: String,
    pub span: Span,
}

impl Symbol {
    pub fn new(module: ModuleId, name: String, span: Span) -> Self {
        Self { module, name, span }
    }
}
