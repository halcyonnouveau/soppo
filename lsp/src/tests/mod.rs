mod completion;
mod diagnostics;
mod references;
mod rename;
mod signature_help;
mod sniff;
mod symbols;

use soppo::syntax::{FileId, LineColumn};

use crate::Span;

/// Create a span for testing
pub fn make_span(start_line: usize, start_col: usize, end_line: usize, end_col: usize) -> Span {
    Span {
        start: LineColumn {
            line: start_line,
            col: start_col,
        },
        end: LineColumn {
            line: end_line,
            col: end_col,
        },
        file: FileId(0),
        byte_start: 0,
        byte_end: 0,
    }
}
