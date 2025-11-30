use std::collections::HashMap;
use std::sync::Arc;

use miette::Diagnostic as MietteDiagnostic;
use soppo::build::{typecheck, typecheck_with_symbols};
use soppo::error::SoppoError;
use soppo::syntax::Span;
use soppo::types::SymbolTable;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

/// Convert a Soppo span to an LSP range.
/// Soppo uses 1-based line/col, LSP uses 0-based.
pub fn span_to_range(span: Span) -> Range {
    Range {
        start: Position {
            line: span.start.line.saturating_sub(1) as u32,
            character: span.start.col.saturating_sub(1) as u32,
        },
        end: Position {
            line: span.end.line.saturating_sub(1) as u32,
            character: span.end.col.saturating_sub(1) as u32,
        },
    }
}

/// Convert a SoppoError to LSP diagnostics.
pub fn soppo_error_to_diagnostics(err: &SoppoError) -> Vec<Diagnostic> {
    let (message, span) = match err {
        SoppoError::Parse { message, span } => (message.clone(), Some(*span)),
        SoppoError::Type { message, span } => (message.clone(), Some(*span)),
        SoppoError::TypeMismatch {
            expected,
            found,
            span,
        } => (
            format!("expected `{}`, found `{}`", expected, found),
            Some(*span),
        ),
        SoppoError::UndefinedVariable { name, span } => (
            format!("cannot find value `{}` in this scope", name),
            Some(*span),
        ),
        SoppoError::NonExhaustive { missing, span } => (
            format!("non-exhaustive match, missing: {}", missing.join(", ")),
            Some(*span),
        ),
        SoppoError::MissingModuleContext { import_path, span } => (
            format!("cannot resolve sop: import `{}`", import_path),
            Some(*span),
        ),
        SoppoError::NilPointer { name, span } => (
            format!("potential nil pointer dereference: `{}`", name),
            Some(*span),
        ),
        SoppoError::TryNoErrorReturn { span } => (
            "`?` requires enclosing function to return error".to_string(),
            Some(*span),
        ),
        SoppoError::TryExprNoError { span } => (
            "`?` requires expression to return error".to_string(),
            Some(*span),
        ),
        SoppoError::NilToNonNilable { ty, span } => (
            format!("cannot assign `nil` to non-nilable type `{}`", ty),
            Some(*span),
        ),
        SoppoError::NonNilableNoInit { ty, span } => (
            format!("non-nilable type `{}` requires initialisation", ty),
            Some(*span),
        ),
        SoppoError::NilableToNonNilable {
            expected,
            found,
            span,
        } => (
            format!(
                "cannot assign nilable `{}` to non-nilable `{}`",
                found, expected
            ),
            Some(*span),
        ),
        SoppoError::CircularDependency { cycle } => {
            let msg = cycle
                .iter()
                .map(|(f, i)| format!("{} -> {}", f, i))
                .collect::<Vec<_>>()
                .join(" -> ");
            (format!("circular dependency: {}", msg), None)
        }
        SoppoError::GenericUnitVariant {
            enum_name,
            variant_name,
            span,
        } => (
            format!(
                "generic unit variant `{}.{}` requires type arguments",
                enum_name, variant_name
            ),
            Some(*span),
        ),
        SoppoError::ConstraintNotSatisfied {
            ty,
            constraint,
            span,
            ..
        } => (
            format!("type `{}` does not satisfy constraint `{}`", ty, constraint),
            Some(*span),
        ),
        _ => (err.to_string(), None),
    };

    let range = span.map(span_to_range).unwrap_or_default();

    vec![Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("soppo".to_string()),
        message,
        ..Default::default()
    }]
}

/// Run typecheck and convert result to diagnostics.
pub fn check_document(text: &str, filename: &str) -> Vec<Diagnostic> {
    match typecheck(text, filename) {
        Ok(()) => vec![],
        Err(report) => {
            // First try to downcast to SoppoError for rich diagnostics
            if let Some(err) = report.downcast_ref::<SoppoError>() {
                return soppo_error_to_diagnostics(err);
            }

            // Fallback: extract info from miette's Diagnostic trait
            // miette::Report implements Diagnostic, so we can call labels() on it
            let mut range = Range::default();
            if let Some(labels) = MietteDiagnostic::labels(&*report)
                && let Some(label) = labels.into_iter().next()
            {
                // Convert byte offset to line/column
                range = byte_offset_to_range(text, label.offset(), label.offset() + label.len());
            }

            vec![Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("soppo".to_string()),
                message: report.to_string(),
                ..Default::default()
            }]
        }
    }
}

/// Convert byte offset to LSP Range using line/column calculation
pub fn byte_offset_to_range(text: &str, start: usize, end: usize) -> Range {
    let mut line = 0u32;
    let mut col = 0u32;
    let mut start_pos = Position {
        line: 0,
        character: 0,
    };
    let mut end_pos = Position {
        line: 0,
        character: 0,
    };

    for (i, c) in text.char_indices() {
        if i == start {
            start_pos = Position {
                line,
                character: col,
            };
        }
        if i == end {
            end_pos = Position {
                line,
                character: col,
            };
            break;
        }
        if c == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }

    // Handle case where end is at or past end of text
    if end >= text.len() {
        end_pos = Position {
            line,
            character: col,
        };
    }

    Range {
        start: start_pos,
        end: end_pos,
    }
}

/// Cached document state including source and symbol table
#[derive(Debug)]
struct DocumentState {
    text: String,
    symbols: Option<SymbolTable>,
}

#[derive(Debug)]
pub struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, DocumentState>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Analyze a document, returning diagnostics and symbol table
    fn analyze_document(text: &str, filename: &str) -> (Vec<Diagnostic>, Option<SymbolTable>) {
        match typecheck_with_symbols(text, filename) {
            Ok(symbols) => (vec![], Some(symbols)),
            Err(report) => {
                // First try to downcast to SoppoError for rich diagnostics
                if let Some(err) = report.downcast_ref::<SoppoError>() {
                    return (soppo_error_to_diagnostics(err), None);
                }

                // Fallback: extract info from miette's Diagnostic trait
                let mut range = Range::default();
                if let Some(labels) = MietteDiagnostic::labels(&*report)
                    && let Some(label) = labels.into_iter().next()
                {
                    range =
                        byte_offset_to_range(text, label.offset(), label.offset() + label.len());
                }

                (
                    vec![Diagnostic {
                        range,
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("soppo".to_string()),
                        message: report.to_string(),
                        ..Default::default()
                    }],
                    None,
                )
            }
        }
    }

    async fn update_document(&self, uri: Url, text: String) {
        let filename = uri
            .path_segments()
            .and_then(|mut s| s.next_back())
            .unwrap_or("input.sop");

        let (diagnostics, symbols) = Self::analyze_document(&text, filename);

        // Update document state
        self.documents
            .write()
            .await
            .insert(uri.clone(), DocumentState { text, symbols });

        // Publish diagnostics
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    /// Convert an LSP position to a byte offset in the document
    fn position_to_byte_offset(text: &str, position: Position) -> usize {
        let mut offset = 0;
        let mut current_line = 0;

        for (i, c) in text.char_indices() {
            if current_line == position.line as usize {
                // Count characters in this line
                for (col, (j, ch)) in text[i..].char_indices().enumerate() {
                    if col == position.character as usize {
                        return i + j;
                    }
                    if ch == '\n' {
                        break;
                    }
                }
                return i + text[i..].find('\n').unwrap_or(text.len() - i);
            }
            if c == '\n' {
                current_line += 1;
            }
            offset = i + c.len_utf8();
        }
        offset
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Soppo language server initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        self.update_document(uri, text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(change) = params.content_changes.into_iter().last() {
            self.update_document(uri, change.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .write()
            .await
            .remove(&params.text_document.uri);
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let docs = self.documents.read().await;
        let Some(doc) = docs.get(&uri) else {
            return Ok(None);
        };

        let Some(ref symbols) = doc.symbols else {
            return Ok(None);
        };

        // Convert position to byte offset
        let offset = Self::position_to_byte_offset(&doc.text, position);

        // Find symbol at this position
        let Some(symbol) = symbols.find_at(offset) else {
            return Ok(None);
        };

        // Format hover content
        let content = format!("```soppo\n{}: {}\n```", symbol.name, symbol.ty);

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        }))
    }
}

/// Run the LSP server on stdin/stdout
pub async fn run_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use soppo::syntax::{FileId, LineColumn};
    use soppo::types::Type;

    use super::*;

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

    #[test]
    fn span_to_range_converts_1based_to_0based() {
        let span = make_span(1, 1, 1, 10);
        let range = span_to_range(span);

        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 0);
        assert_eq!(range.end.line, 0);
        assert_eq!(range.end.character, 9);
    }

    #[test]
    fn span_to_range_multiline() {
        let span = make_span(5, 3, 10, 15);
        let range = span_to_range(span);

        assert_eq!(range.start.line, 4);
        assert_eq!(range.start.character, 2);
        assert_eq!(range.end.line, 9);
        assert_eq!(range.end.character, 14);
    }

    #[test]
    fn span_to_range_handles_zero_gracefully() {
        let span = make_span(0, 0, 0, 0);
        let range = span_to_range(span);

        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 0);
    }

    #[test]
    fn error_to_diagnostics_type_mismatch() {
        let err = SoppoError::TypeMismatch {
            expected: Box::new(Type::simple("int")),
            found: Box::new(Type::simple("string")),
            span: make_span(5, 10, 5, 20),
        };

        let diagnostics = soppo_error_to_diagnostics(&err);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "expected `int`, found `string`");
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(diagnostics[0].source, Some("soppo".to_string()));
        assert_eq!(diagnostics[0].range.start.line, 4);
        assert_eq!(diagnostics[0].range.start.character, 9);
    }

    #[test]
    fn error_to_diagnostics_undefined_variable() {
        let err = SoppoError::UndefinedVariable {
            name: "foo".to_string(),
            span: make_span(1, 1, 1, 4),
        };

        let diagnostics = soppo_error_to_diagnostics(&err);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "cannot find value `foo` in this scope"
        );
    }

    #[test]
    fn error_to_diagnostics_nil_pointer() {
        let err = SoppoError::NilPointer {
            name: "ptr".to_string(),
            span: make_span(10, 5, 10, 8),
        };

        let diagnostics = soppo_error_to_diagnostics(&err);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "potential nil pointer dereference: `ptr`"
        );
    }

    #[test]
    fn error_to_diagnostics_non_exhaustive() {
        let err = SoppoError::NonExhaustive {
            missing: vec!["A".to_string(), "B".to_string()],
            span: make_span(1, 1, 1, 10),
        };

        let diagnostics = soppo_error_to_diagnostics(&err);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "non-exhaustive match, missing: A, B"
        );
    }

    #[test]
    fn error_to_diagnostics_circular_dependency_no_span() {
        let err = SoppoError::CircularDependency {
            cycle: vec![
                ("a.sop".to_string(), "b".to_string()),
                ("b.sop".to_string(), "a".to_string()),
            ],
        };

        let diagnostics = soppo_error_to_diagnostics(&err);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("circular dependency"));
        assert_eq!(diagnostics[0].range.start.line, 0);
        assert_eq!(diagnostics[0].range.start.character, 0);
    }

    #[test]
    fn position_to_byte_offset_first_line() {
        let text = "hello world";
        let pos = Position {
            line: 0,
            character: 6,
        };
        assert_eq!(Backend::position_to_byte_offset(text, pos), 6);
    }

    #[test]
    fn position_to_byte_offset_second_line() {
        let text = "hello\nworld";
        let pos = Position {
            line: 1,
            character: 2,
        };
        assert_eq!(Backend::position_to_byte_offset(text, pos), 8); // "hello\n" = 6 bytes + 2
    }

    #[test]
    fn position_to_byte_offset_start_of_line() {
        let text = "line1\nline2\nline3";
        let pos = Position {
            line: 2,
            character: 0,
        };
        assert_eq!(Backend::position_to_byte_offset(text, pos), 12); // "line1\nline2\n" = 12 bytes
    }

    #[test]
    fn analyze_document_returns_symbols_for_valid_code() {
        let code = r#"
package main

func main() {
    x := 42
    println(x)
}
"#;
        let (diagnostics, symbols) = Backend::analyze_document(code, "test.sop");
        assert!(
            diagnostics.is_empty(),
            "Valid code should have no diagnostics"
        );
        assert!(symbols.is_some(), "Valid code should produce symbols");

        let symbols = symbols.unwrap();
        assert!(
            !symbols.is_empty(),
            "Should have recorded at least one symbol"
        );
    }

    #[test]
    fn analyze_document_returns_diagnostics_for_invalid_code() {
        let code = r#"
package main

func main() {
    x := undefined_var
}
"#;
        let (diagnostics, symbols) = Backend::analyze_document(code, "test.sop");
        assert!(
            !diagnostics.is_empty(),
            "Invalid code should have diagnostics"
        );
        assert!(symbols.is_none(), "Invalid code should not produce symbols");
    }

    #[test]
    fn symbol_lookup_finds_variable() {
        let code = r#"
package main

func main() {
    x := 42
    println(x)
}
"#;
        let (_, symbols) = Backend::analyze_document(code, "test.sop");
        let symbols = symbols.expect("Should have symbols");

        // Find the byte offset of the second 'x' (in println(x))
        // "x" appears at position after "println("
        let x_usage_offset = code.rfind("(x)").unwrap() + 1;

        let symbol = symbols.find_at(x_usage_offset);
        assert!(symbol.is_some(), "Should find symbol at x usage");
        let symbol = symbol.unwrap();
        assert_eq!(symbol.name, "x");
        assert_eq!(symbol.ty.to_string(), "int");
    }
}
