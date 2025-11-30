#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use miette::Diagnostic as MietteDiagnostic;
use soppo::build::{typecheck, typecheck_with_symbols, typecheck_workspace};
use soppo::error::SoppoError;
use soppo::syntax::{FileId, FileRegistry, Span};
use soppo::types::{GlobalCtxt, SymbolKind as SoppoSymbolKind, SymbolTable};
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

    /// Convert our SymbolKind to LSP SymbolKind
    fn to_lsp_symbol_kind(kind: SoppoSymbolKind) -> SymbolKind {
        match kind {
            SoppoSymbolKind::Variable => SymbolKind::VARIABLE,
            SoppoSymbolKind::Parameter => SymbolKind::VARIABLE,
            SoppoSymbolKind::Function => SymbolKind::FUNCTION,
            SoppoSymbolKind::Type => SymbolKind::STRUCT,
            SoppoSymbolKind::Field => SymbolKind::FIELD,
            SoppoSymbolKind::Variant => SymbolKind::ENUM_MEMBER,
            SoppoSymbolKind::Constant => SymbolKind::CONSTANT,
            SoppoSymbolKind::Method => SymbolKind::METHOD,
        }
    }

    /// Convert our SymbolKind to LSP CompletionItemKind
    fn to_completion_kind(kind: SoppoSymbolKind) -> CompletionItemKind {
        match kind {
            SoppoSymbolKind::Variable => CompletionItemKind::VARIABLE,
            SoppoSymbolKind::Parameter => CompletionItemKind::VARIABLE,
            SoppoSymbolKind::Function => CompletionItemKind::FUNCTION,
            SoppoSymbolKind::Type => CompletionItemKind::STRUCT,
            SoppoSymbolKind::Field => CompletionItemKind::FIELD,
            SoppoSymbolKind::Variant => CompletionItemKind::ENUM_MEMBER,
            SoppoSymbolKind::Constant => CompletionItemKind::CONSTANT,
            SoppoSymbolKind::Method => CompletionItemKind::METHOD,
        }
    }

    /// Extract the package name if cursor is after `pkg.`
    /// Returns Some("pkg") if text before cursor ends with "pkg." pattern
    fn get_package_prefix(text: &str, cursor_offset: usize) -> Option<String> {
        let before_cursor = &text[..cursor_offset];

        // Check if we just typed a `.` after an identifier
        if !before_cursor.ends_with('.') {
            return None;
        }

        // Find the identifier before the dot
        let before_dot = &before_cursor[..before_cursor.len() - 1];
        let ident_start = before_dot
            .rfind(|c: char| !c.is_alphanumeric() && c != '_')
            .map(|i| i + 1)
            .unwrap_or(0);

        let ident = before_dot[ident_start..].trim();
        if ident.is_empty() || !ident.chars().next().unwrap().is_alphabetic() {
            return None;
        }

        Some(ident.to_string())
    }

    /// Convert a FuncDef to a type string for display
    fn func_def_to_type(func_def: &soppo::types::FuncDef) -> String {
        let params: Vec<String> = func_def
            .params
            .iter()
            .map(|(name, ty)| format!("{} {}", name, ty))
            .collect();

        let returns = if func_def.return_types.is_empty() {
            String::new()
        } else if func_def.return_types.len() == 1 {
            format!(" {}", func_def.return_types[0])
        } else {
            format!(
                " ({})",
                func_def
                    .return_types
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        format!("func({}){}", params.join(", "), returns)
    }

    /// Convert a TypeDef to a string for display
    fn type_def_to_string(type_def: &soppo::types::TypeDef) -> String {
        use soppo::types::TypeDefKind;
        match &type_def.kind {
            TypeDefKind::Struct { fields } => {
                let field_count = fields.len();
                format!("struct ({} fields)", field_count)
            }
            TypeDefKind::Enum { variants } => {
                let variant_count = variants.len();
                format!("enum ({} variants)", variant_count)
            }
            TypeDefKind::Alias { target } => format!("= {}", target),
            TypeDefKind::Definition { target } => format!("type {}", target),
            TypeDefKind::Interface { methods } => {
                let method_count = methods.len();
                format!("interface ({} methods)", method_count)
            }
        }
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
                document_symbol_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string()]),
                    ..Default::default()
                }),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    retrigger_characters: None,
                    work_done_progress_options: Default::default(),
                }),
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

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;

        // Get symbols for this file
        let symbols = if let Some(path) = Self::uri_to_path(&uri) {
            let ws_guard = self.workspace.read().await;
            if let Some(ws) = ws_guard.as_ref()
                && let Some(file_id) = ws.file_registry.get_id(&path)
                && let Some(symbols) = ws.symbol_tables.get(&file_id)
            {
                Some(symbols.clone())
            } else {
                drop(ws_guard);
                let docs = self.documents.read().await;
                docs.get(&uri).and_then(|d| d.symbols.clone())
            }
        } else {
            let docs = self.documents.read().await;
            docs.get(&uri).and_then(|d| d.symbols.clone())
        };

        let Some(symbols) = symbols else {
            return Ok(None);
        };

        // Filter to top-level symbols (functions, types, constants)
        let doc_symbols: Vec<_> = symbols
            .all_symbols()
            .iter()
            .filter(|(_, info)| {
                matches!(
                    info.kind,
                    SoppoSymbolKind::Function
                        | SoppoSymbolKind::Type
                        | SoppoSymbolKind::Constant
                        | SoppoSymbolKind::Method
                )
            })
            .filter_map(|((start, end), info)| {
                // Only include symbols that have a definition span (i.e., they are defined here)
                let def_span = info.definition_span?;
                // Check that the definition is at the same location as the symbol reference
                if def_span.byte_start != *start || def_span.byte_end != *end {
                    return None;
                }
                #[allow(deprecated)]
                Some(DocumentSymbol {
                    name: info.name.clone(),
                    detail: Some(info.ty.to_string()),
                    kind: Self::to_lsp_symbol_kind(info.kind),
                    tags: None,
                    deprecated: None,
                    range: span_to_range(def_span),
                    selection_range: span_to_range(def_span),
                    children: None,
                })
            })
            .collect();

        Ok(Some(DocumentSymbolResponse::Nested(doc_symbols)))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let mut items = Vec::new();

        // Get document text and symbols
        let (text, symbols) = if let Some(path) = Self::uri_to_path(&uri) {
            let ws_guard = self.workspace.read().await;
            if let Some(ws) = ws_guard.as_ref()
                && let Some(file_id) = ws.file_registry.get_id(&path)
                && let Some(symbols) = ws.symbol_tables.get(&file_id)
            {
                let open_docs = self.open_documents.read().await;
                (open_docs.get(&path).cloned(), Some(symbols.clone()))
            } else {
                drop(ws_guard);
                let docs = self.documents.read().await;
                let doc = docs.get(&uri);
                (
                    doc.map(|d| d.text.clone()),
                    doc.and_then(|d| d.symbols.clone()),
                )
            }
        } else {
            let docs = self.documents.read().await;
            let doc = docs.get(&uri);
            (
                doc.map(|d| d.text.clone()),
                doc.and_then(|d| d.symbols.clone()),
            )
        };

        let cursor_offset = text
            .as_ref()
            .map(|t| Self::position_to_byte_offset(t, position))
            .unwrap_or(0);

        // Check if we're completing after a package name followed by `.`
        // e.g., `helpers.` should show exports from the helpers module
        if let Some(ref text) = text
            && let Some(ref symbols) = symbols
            && let Some(pkg_name) = Self::get_package_prefix(text, cursor_offset)
        {
            // Look up the module for this package in the symbol table's imports
            if let Some(module_id) = symbols.imports().get(&pkg_name) {
                // Get the module from GlobalCtxt
                let ws_guard = self.workspace.read().await;
                if let Some(ws) = ws_guard.as_ref()
                    && let Some(module) = ws.global_ctxt.get_module(module_id)
                {
                    // Add exported functions (uppercase names)
                    for (name, func_def) in &module.functions {
                        if name.starts_with(char::is_uppercase) {
                            let ty = Self::func_def_to_type(func_def);
                            items.push(CompletionItem {
                                label: name.clone(),
                                kind: Some(CompletionItemKind::FUNCTION),
                                detail: Some(ty),
                                ..Default::default()
                            });
                        }
                    }

                    // Add exported types
                    for (name, type_def) in &module.types {
                        if name.starts_with(char::is_uppercase) {
                            items.push(CompletionItem {
                                label: name.clone(),
                                kind: Some(CompletionItemKind::STRUCT),
                                detail: Some(Self::type_def_to_string(type_def)),
                                ..Default::default()
                            });
                        }
                    }

                    // Add exported constants
                    for (name, const_def) in &module.constants {
                        if name.starts_with(char::is_uppercase) {
                            items.push(CompletionItem {
                                label: name.clone(),
                                kind: Some(CompletionItemKind::CONSTANT),
                                detail: Some(const_def.ty.to_string()),
                                ..Default::default()
                            });
                        }
                    }

                    // Return only cross-module completions when after `pkg.`
                    return Ok(Some(CompletionResponse::Array(items)));
                }
            }
        }

        // Add keywords
        const KEYWORDS: &[&str] = &[
            "break",
            "case",
            "const",
            "continue",
            "default",
            "defer",
            "else",
            "enum",
            "fallthrough",
            "for",
            "func",
            "go",
            "if",
            "import",
            "interface",
            "map",
            "match",
            "package",
            "range",
            "return",
            "select",
            "struct",
            "type",
            "var",
        ];

        for kw in KEYWORDS {
            items.push(CompletionItem {
                label: (*kw).to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                ..Default::default()
            });
        }

        // Add builtin types
        const BUILTIN_TYPES: &[&str] = &[
            "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32",
            "uint64", "float32", "float64", "string", "bool", "byte", "rune", "error", "any",
        ];

        for ty in BUILTIN_TYPES {
            items.push(CompletionItem {
                label: (*ty).to_string(),
                kind: Some(CompletionItemKind::TYPE_PARAMETER),
                ..Default::default()
            });
        }

        // Add builtin functions
        const BUILTIN_FUNCS: &[&str] = &[
            "len", "cap", "make", "new", "append", "copy", "delete", "panic", "recover", "close",
            "print", "println",
        ];

        for func in BUILTIN_FUNCS {
            items.push(CompletionItem {
                label: (*func).to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                ..Default::default()
            });
        }

        // Add symbols from the current file
        if let Some(symbols) = symbols {
            // Collect unique symbol names that are in scope
            let mut seen = std::collections::HashSet::new();
            for ((start, _end), info) in symbols.all_symbols() {
                // Only include symbols defined before the cursor
                if let Some(def_span) = info.definition_span {
                    if def_span.byte_start <= cursor_offset && seen.insert(info.name.clone()) {
                        items.push(CompletionItem {
                            label: info.name.clone(),
                            kind: Some(Self::to_completion_kind(info.kind)),
                            detail: Some(info.ty.to_string()),
                            ..Default::default()
                        });
                    }
                } else if *start <= cursor_offset && seen.insert(info.name.clone()) {
                    // For builtins without definition span
                    items.push(CompletionItem {
                        label: info.name.clone(),
                        kind: Some(Self::to_completion_kind(info.kind)),
                        detail: Some(info.ty.to_string()),
                        ..Default::default()
                    });
                }
            }
        }

        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        // Get document text
        let text = if let Some(path) = Self::uri_to_path(&uri) {
            let open_docs = self.open_documents.read().await;
            open_docs.get(&path).cloned()
        } else {
            let docs = self.documents.read().await;
            docs.get(&uri).map(|d| d.text.clone())
        };

        let Some(text) = text else {
            return Ok(None);
        };

        let offset = Self::position_to_byte_offset(&text, position);

        // Find the function call context by scanning backwards for '('
        let before_cursor = &text[..offset];
        let mut paren_depth = 0;
        let mut func_call_start = None;
        let mut comma_count = 0;

        for (i, c) in before_cursor.char_indices().rev() {
            match c {
                ')' => paren_depth += 1,
                '(' => {
                    if paren_depth == 0 {
                        func_call_start = Some(i);
                        break;
                    }
                    paren_depth -= 1;
                }
                ',' if paren_depth == 0 => comma_count += 1,
                _ => {}
            }
        }

        let Some(paren_pos) = func_call_start else {
            return Ok(None);
        };

        // Find the function name by scanning backwards from the opening paren
        let before_paren = &text[..paren_pos];
        let func_name_end = before_paren.trim_end().len();
        let func_name_start = before_paren[..func_name_end]
            .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
            .map(|i| i + 1)
            .unwrap_or(0);
        let func_name = before_paren[func_name_start..func_name_end].trim();

        if func_name.is_empty() {
            return Ok(None);
        }

        // Look up the function in symbols
        let symbols = if let Some(path) = Self::uri_to_path(&uri) {
            let ws_guard = self.workspace.read().await;
            if let Some(ws) = ws_guard.as_ref()
                && let Some(file_id) = ws.file_registry.get_id(&path)
                && let Some(symbols) = ws.symbol_tables.get(&file_id)
            {
                Some(symbols.clone())
            } else {
                drop(ws_guard);
                let docs = self.documents.read().await;
                docs.get(&uri).and_then(|d| d.symbols.clone())
            }
        } else {
            let docs = self.documents.read().await;
            docs.get(&uri).and_then(|d| d.symbols.clone())
        };

        let Some(symbols) = symbols else {
            return Ok(None);
        };

        // Find the function symbol - handle qualified names like "pkg.Func"
        let search_name = func_name.split('.').next_back().unwrap_or(func_name);

        let func_info = symbols.all_symbols().values().find(|info| {
            info.name == search_name
                && matches!(
                    info.kind,
                    SoppoSymbolKind::Function | SoppoSymbolKind::Method
                )
        });

        let Some(func_info) = func_info else {
            return Ok(None);
        };

        // Format the signature
        let signature_label = format!("{}: {}", func_info.name, func_info.ty);

        Ok(Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label: signature_label,
                documentation: None,
                parameters: None, // Could parse params from type string
                active_parameter: Some(comma_count as u32),
            }],
            active_signature: Some(0),
            active_parameter: Some(comma_count as u32),
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
