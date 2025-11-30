#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use miette::Diagnostic as MietteDiagnostic;
use soppo::build::{typecheck, typecheck_with_symbols, typecheck_workspace};
use soppo::error::SoppoError;
use soppo::syntax::{FileId, FileRegistry, Span};
use soppo::types::{GlobalCtxt, SymbolTable};
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

/// Cached document state including source and symbol table (single-file mode)
#[derive(Debug)]
struct DocumentState {
    text: String,
    symbols: Option<SymbolTable>,
}

/// Workspace state for multi-file projects
#[derive(Debug)]
struct Workspace {
    /// Project root directory (where go.mod is)
    project_root: PathBuf,
    /// Registry mapping FileId to file paths
    file_registry: FileRegistry,
    /// Global type context with all modules (for type/function lookups)
    #[allow(dead_code)]
    global_ctxt: GlobalCtxt,
    /// Symbol tables per file for LSP features
    symbol_tables: HashMap<FileId, SymbolTable>,
}

#[derive(Debug)]
pub struct Backend {
    client: Client,
    /// Single-file document state (fallback when no workspace)
    documents: Arc<RwLock<HashMap<Url, DocumentState>>>,
    /// Text content of open documents (may have unsaved changes)
    open_documents: Arc<RwLock<HashMap<PathBuf, String>>>,
    /// Workspace state (initialized on first file open if project found)
    workspace: Arc<RwLock<Option<Workspace>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
            open_documents: Arc::new(RwLock::new(HashMap::new())),
            workspace: Arc::new(RwLock::new(None)),
        }
    }

    /// Analyze a document, returning diagnostics and symbol table (single-file mode)
    pub fn analyze_document(text: &str, filename: &str) -> (Vec<Diagnostic>, Option<SymbolTable>) {
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

    /// Try to discover a project from a file path and initialize workspace
    async fn try_init_workspace(&self, file_path: &Path) -> bool {
        // Already initialized?
        if self.workspace.read().await.is_some() {
            return true;
        }

        // Try to find project root by walking up from file
        let start_dir = file_path.parent().unwrap_or(file_path);

        // Try to typecheck the workspace
        let open_docs = self.open_documents.read().await;
        match typecheck_workspace(start_dir, &open_docs) {
            Ok(result) => {
                let mut ws = self.workspace.write().await;
                *ws = Some(Workspace {
                    project_root: start_dir.to_path_buf(),
                    file_registry: result.file_registry,
                    global_ctxt: result.global_ctxt,
                    symbol_tables: result.symbol_tables,
                });
                true
            }
            Err(_) => false,
        }
    }

    /// Rebuild the workspace after a file change
    async fn rebuild_workspace(&self) {
        let ws_guard = self.workspace.read().await;
        let Some(ws) = ws_guard.as_ref() else {
            return;
        };
        let project_root = ws.project_root.clone();
        drop(ws_guard);

        let open_docs = self.open_documents.read().await.clone();

        match typecheck_workspace(&project_root, &open_docs) {
            Ok(result) => {
                let mut ws = self.workspace.write().await;
                *ws = Some(Workspace {
                    project_root,
                    file_registry: result.file_registry,
                    global_ctxt: result.global_ctxt,
                    symbol_tables: result.symbol_tables,
                });
            }
            Err(e) => {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!("Workspace rebuild failed: {}", e),
                    )
                    .await;
            }
        }
    }

    /// Update a document (handles both workspace and single-file modes)
    async fn update_document(&self, uri: Url, text: String) {
        // Convert URI to file path
        let file_path = uri.to_file_path().ok();

        // Try workspace mode first
        if let Some(ref path) = file_path {
            // Store the document content
            self.open_documents
                .write()
                .await
                .insert(path.clone(), text.clone());

            // Try to initialize or use workspace
            if self.try_init_workspace(path).await {
                // Rebuild workspace
                self.rebuild_workspace().await;

                // Publish diagnostics from workspace
                self.publish_workspace_diagnostics().await;
                return;
            }
        }

        // Fallback to single-file mode
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

    /// Publish diagnostics for all files in the workspace
    async fn publish_workspace_diagnostics(&self) {
        let ws_guard = self.workspace.read().await;
        let Some(ws) = ws_guard.as_ref() else {
            return;
        };

        // For now, if workspace typechecked successfully, clear diagnostics for all open files
        // TODO: Collect and publish actual diagnostics per file
        for path in ws.file_registry.file_ids() {
            if let Some(file_path) = ws.file_registry.get_path(path)
                && let Ok(uri) = Url::from_file_path(file_path)
            {
                self.client.publish_diagnostics(uri, vec![], None).await;
            }
        }
    }

    /// Convert an LSP position to a byte offset in the document
    pub fn position_to_byte_offset(text: &str, position: Position) -> usize {
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

    /// Get the file path for a URI
    fn uri_to_path(uri: &Url) -> Option<PathBuf> {
        uri.to_file_path().ok()
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
                definition_provider: Some(OneOf::Left(true)),
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

        // Try workspace mode first
        if let Some(path) = Self::uri_to_path(&uri) {
            let ws_guard = self.workspace.read().await;
            if let Some(ws) = ws_guard.as_ref()
                && let Some(file_id) = ws.file_registry.get_id(&path)
                && let Some(symbols) = ws.symbol_tables.get(&file_id)
            {
                // Get document text
                let open_docs = self.open_documents.read().await;
                if let Some(text) = open_docs.get(&path) {
                    let offset = Self::position_to_byte_offset(text, position);
                    if let Some(symbol) = symbols.find_at(offset) {
                        let content = format!("```soppo\n{}: {}\n```", symbol.name, symbol.ty);
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: content,
                            }),
                            range: None,
                        }));
                    }
                }
            }
        }

        // Fallback to single-file mode
        let docs = self.documents.read().await;
        let Some(doc) = docs.get(&uri) else {
            return Ok(None);
        };

        let Some(ref symbols) = doc.symbols else {
            return Ok(None);
        };

        let offset = Self::position_to_byte_offset(&doc.text, position);
        let Some(symbol) = symbols.find_at(offset) else {
            return Ok(None);
        };

        let content = format!("```soppo\n{}: {}\n```", symbol.name, symbol.ty);

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        }))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        // Try workspace mode first
        if let Some(path) = Self::uri_to_path(&uri) {
            let ws_guard = self.workspace.read().await;
            if let Some(ws) = ws_guard.as_ref()
                && let Some(file_id) = ws.file_registry.get_id(&path)
                && let Some(symbols) = ws.symbol_tables.get(&file_id)
            {
                // Get document text
                let open_docs = self.open_documents.read().await;
                if let Some(text) = open_docs.get(&path) {
                    let offset = Self::position_to_byte_offset(text, position);
                    if let Some(symbol) = symbols.find_at(offset)
                        && let Some(def_span) = symbol.definition_span
                    {
                        // Check if definition is in a different file
                        let def_uri = if def_span.file != file_id {
                            // Cross-file: look up the path
                            if let Some(def_path) = ws.file_registry.get_path(def_span.file) {
                                Url::from_file_path(def_path).ok()
                            } else {
                                None
                            }
                        } else {
                            // Same file
                            Some(uri.clone())
                        };

                        if let Some(def_uri) = def_uri {
                            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                                uri: def_uri,
                                range: span_to_range(def_span),
                            })));
                        }
                    }
                }
            }
        }

        // Fallback to single-file mode
        let docs = self.documents.read().await;
        let Some(doc) = docs.get(&uri) else {
            return Ok(None);
        };

        let Some(ref symbols) = doc.symbols else {
            return Ok(None);
        };

        let offset = Self::position_to_byte_offset(&doc.text, position);
        let Some(symbol) = symbols.find_at(offset) else {
            return Ok(None);
        };

        let Some(def_span) = symbol.definition_span else {
            return Ok(None);
        };

        // In single-file mode, assume definition is in the same file
        let location = Location {
            uri: uri.clone(),
            range: span_to_range(def_span),
        };

        Ok(Some(GotoDefinitionResponse::Scalar(location)))
    }
}

/// Run the LSP server on stdin/stdout
pub async fn run_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
