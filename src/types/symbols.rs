//! Symbol table for LSP features (hover, go-to-definition)

use std::collections::HashMap;

use super::ty::Type;
use crate::syntax::Span;

/// Information about a symbol at a specific location
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    /// The symbol's name
    pub name: String,
    /// The symbol's inferred type
    pub ty: Type,
    /// Where this symbol is defined (None for builtins/externals)
    pub definition_span: Option<Span>,
    /// The kind of symbol
    pub kind: SymbolKind,
}

/// What kind of symbol this is
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// Local variable
    Variable,
    /// Function parameter
    Parameter,
    /// Function
    Function,
    /// Type (struct, enum, alias)
    Type,
    /// Struct field
    Field,
    /// Enum variant
    Variant,
    /// Constant
    Constant,
    /// Method
    Method,
}

/// Symbol table mapping byte ranges to symbol information.
/// Used by the LSP for hover and go-to-definition.
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    /// Maps (byte_start, byte_end) to symbol info
    symbols: HashMap<(usize, usize), SymbolInfo>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            symbols: HashMap::new(),
        }
    }

    /// Record a symbol at the given span
    pub fn record(&mut self, span: Span, info: SymbolInfo) {
        self.symbols.insert((span.byte_start, span.byte_end), info);
    }

    /// Find a symbol at the given byte offset
    pub fn find_at(&self, offset: usize) -> Option<&SymbolInfo> {
        // Find any symbol whose range contains the offset
        for ((start, end), info) in &self.symbols {
            if offset >= *start && offset < *end {
                return Some(info);
            }
        }
        None
    }

    /// Get all recorded symbols (for testing)
    pub fn all_symbols(&self) -> &HashMap<(usize, usize), SymbolInfo> {
        &self.symbols
    }

    /// Check if the table is empty
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// Get the number of symbols
    pub fn len(&self) -> usize {
        self.symbols.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{FileId, LineColumn};

    fn make_span(byte_start: usize, byte_end: usize) -> Span {
        Span {
            start: LineColumn { line: 1, col: 1 },
            end: LineColumn { line: 1, col: 1 },
            file: FileId(0),
            byte_start,
            byte_end,
        }
    }

    #[test]
    fn test_record_and_find() {
        let mut table = SymbolTable::new();

        table.record(
            make_span(10, 15),
            SymbolInfo {
                name: "foo".to_string(),
                ty: Type::simple("int"),
                definition_span: Some(make_span(10, 15)),
                kind: SymbolKind::Variable,
            },
        );

        // Find within range
        let found = table.find_at(12);
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "foo");

        // Find at start
        let found = table.find_at(10);
        assert!(found.is_some());

        // Not found before
        let found = table.find_at(9);
        assert!(found.is_none());

        // Not found at end (exclusive)
        let found = table.find_at(15);
        assert!(found.is_none());
    }

    #[test]
    fn test_multiple_symbols() {
        let mut table = SymbolTable::new();

        table.record(
            make_span(0, 5),
            SymbolInfo {
                name: "a".to_string(),
                ty: Type::simple("int"),
                definition_span: None,
                kind: SymbolKind::Variable,
            },
        );

        table.record(
            make_span(10, 20),
            SymbolInfo {
                name: "b".to_string(),
                ty: Type::simple("string"),
                definition_span: None,
                kind: SymbolKind::Function,
            },
        );

        assert_eq!(table.len(), 2);
        assert_eq!(table.find_at(2).unwrap().name, "a");
        assert_eq!(table.find_at(15).unwrap().name, "b");
        assert!(table.find_at(7).is_none());
    }
}
