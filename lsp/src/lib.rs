#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use miette::Diagnostic as MietteDiagnostic;
use soppo::build::{typecheck, typecheck_to_typed_with_symbols, typecheck_workspace};
use soppo::error::{SoppoError, SoppoErrors};
use soppo::fmt::format_source;
use soppo::go::SourceLocation;
use soppo::sniff::{self, LintConfig, LintWarning};
use soppo::syntax::{FileId, FileRegistry, Span};
use soppo::types::{GlobalCtxt, SymbolKind as SoppoSymbolKind, SymbolTable};
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

/// Command-line options for the language server
#[derive(Parser)]
#[command(name = "sopls", about = "Soppo language server")]
pub struct Cli {
    /// Disable the sniff linter
    #[arg(long)]
    pub no_sniff: bool,
}

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

/// Convert a Go SourceLocation to an LSP Location.
/// SourceLocation uses 1-based line/col, LSP uses 0-based.
pub fn go_location_to_lsp(loc: &SourceLocation) -> Option<Location> {
    let uri = Url::from_file_path(&loc.file).ok()?;
    Some(Location {
        uri,
        range: Range {
            start: Position {
                line: loc.start_line.saturating_sub(1) as u32,
                character: loc.start_col.saturating_sub(1) as u32,
            },
            end: Position {
                line: loc.end_line.saturating_sub(1) as u32,
                character: loc.end_col.saturating_sub(1) as u32,
            },
        },
    })
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
        SoppoError::ShadowsImport { name, span } => (
            format!("variable `{}` shadows imported package", name),
            Some(*span),
        ),
        SoppoError::Redeclared { name, span, .. } => (
            format!("variable `{}` redeclared in this block", name),
            Some(*span),
        ),
        SoppoError::UnusedImport { name, span, .. } => (
            format!("imported package `{}` is not used", name),
            Some(*span),
        ),
        SoppoError::UnusedVariable { name, span } => (
            format!("variable `{}` is declared but not used", name),
            Some(*span),
        ),
        SoppoError::TryCapturesError { span } => (
            "cannot capture error with `?` operator".to_string(),
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

/// Convert a LintWarning to an LSP diagnostic.
pub fn lint_warning_to_diagnostic(warning: &LintWarning, source: &str) -> Diagnostic {
    let range = byte_offset_to_range(
        source,
        warning.span.offset(),
        warning.span.offset() + warning.span.len(),
    );

    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::WARNING),
        code: Some(NumberOrString::String(warning.code.to_string())),
        source: Some("sniff".to_string()),
        message: warning.message.clone(),
        ..Default::default()
    }
}

/// Run typecheck and convert result to diagnostics.
pub fn check_document(text: &str, filename: &str) -> Vec<Diagnostic> {
    match typecheck(text, filename) {
        Ok(()) => vec![],
        Err(report) => {
            // First try to downcast to MultiError for multiple diagnostics
            if let Some(multi_err) = report.downcast_ref::<SoppoErrors>() {
                return multi_err
                    .iter()
                    .flat_map(soppo_error_to_diagnostics)
                    .collect();
            }

            // Try to downcast to single SoppoError
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
    /// Typed AST per file for linting
    typed_files: HashMap<FileId, soppo::types::TypedFile>,
    /// Diagnostics per file (type errors, unused variables, etc.)
    diagnostics: HashMap<FileId, Vec<SoppoError>>,
    /// Sniff linter configuration from sop.mod
    sniff_config: LintConfig,
}

/// LSP initialization options from the editor
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct InitOptions {
    sniff: SniffOptions,
}

#[derive(Debug, serde::Deserialize)]
#[serde(default)]
struct SniffOptions {
    enabled: bool,
}

impl Default for SniffOptions {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug)]
pub struct Backend {
    client: Client,
    /// Single-file document state (fallback when no workspace)
    documents: Arc<RwLock<HashMap<Url, DocumentState>>>,
    /// Text content of open documents (may have unsaved changes)
    open_documents: Arc<RwLock<HashMap<PathBuf, String>>>,
    /// Workspace state (initialised on first file open if project found)
    workspace: Arc<RwLock<Option<Workspace>>>,
    /// Whether sniff linting is enabled (can be disabled via CLI or initializationOptions)
    sniff_enabled: Arc<RwLock<bool>>,
}

impl Backend {
    pub fn new(client: Client, sniff_enabled: bool) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
            open_documents: Arc::new(RwLock::new(HashMap::new())),
            workspace: Arc::new(RwLock::new(None)),
            sniff_enabled: Arc::new(RwLock::new(sniff_enabled)),
        }
    }

    /// Analyse a document, returning diagnostics and symbol table (single-file mode)
    pub fn analyse_document(
        text: &str,
        filename: &str,
        sniff_enabled: bool,
    ) -> (Vec<Diagnostic>, Option<SymbolTable>) {
        match typecheck_to_typed_with_symbols(text, filename) {
            Ok(result) => {
                // Run sniff linter on successful typecheck (if enabled)
                let diagnostics = if sniff_enabled {
                    let config = LintConfig::default();
                    let warnings = sniff::lint_file(&result.typed_file, filename, text, &config);
                    warnings
                        .iter()
                        .map(|w| lint_warning_to_diagnostic(w, text))
                        .collect()
                } else {
                    vec![]
                };

                (diagnostics, Some(result.symbols))
            }
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

    /// Try to discover a project from a file path and initialise workspace
    async fn try_init_workspace(&self, file_path: &Path) -> bool {
        // Already initialised?
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
                // Build LintConfig from sop.mod sniff settings
                let sniff_config = result
                    .config
                    .as_ref()
                    .and_then(|c| c.sniff.as_ref())
                    .and_then(|s| s.ignore.as_ref())
                    .map(|ignored| LintConfig {
                        ignored: ignored.iter().cloned().collect(),
                    })
                    .unwrap_or_default();

                *ws = Some(Workspace {
                    project_root: result.project_root,
                    file_registry: result.file_registry,
                    global_ctxt: result.global_ctxt,
                    symbol_tables: result.symbol_tables,
                    typed_files: result.typed_files,
                    diagnostics: result.diagnostics,
                    sniff_config,
                });
                true
            }
            Err(e) => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("Failed to initialise workspace: {}", e),
                    )
                    .await;
                false
            }
        }
    }

    /// Rebuild the workspace after a file change.
    /// Returns Ok(()) on success, or the error on failure.
    async fn rebuild_workspace(&self) -> std::result::Result<(), miette::Report> {
        let ws_guard = self.workspace.read().await;
        let Some(ws) = ws_guard.as_ref() else {
            return Ok(());
        };
        let project_root = ws.project_root.clone();
        drop(ws_guard);

        let open_docs = self.open_documents.read().await.clone();

        match typecheck_workspace(&project_root, &open_docs) {
            Ok(result) => {
                let mut ws = self.workspace.write().await;
                // Build LintConfig from sop.mod sniff settings
                let sniff_config = result
                    .config
                    .as_ref()
                    .and_then(|c| c.sniff.as_ref())
                    .and_then(|s| s.ignore.as_ref())
                    .map(|ignored| LintConfig {
                        ignored: ignored.iter().cloned().collect(),
                    })
                    .unwrap_or_default();

                *ws = Some(Workspace {
                    project_root: result.project_root,
                    file_registry: result.file_registry,
                    global_ctxt: result.global_ctxt,
                    symbol_tables: result.symbol_tables,
                    typed_files: result.typed_files,
                    diagnostics: result.diagnostics,
                    sniff_config,
                });
                Ok(())
            }
            Err(e) => {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!("Workspace rebuild failed: {}", e),
                    )
                    .await;
                Err(e)
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

            // Try to initialise or use workspace
            if self.try_init_workspace(path).await {
                // Rebuild workspace
                if self.rebuild_workspace().await.is_ok() {
                    // Publish diagnostics from workspace (success = clear diagnostics)
                    self.publish_workspace_diagnostics().await;
                    return;
                }
                // Workspace rebuild failed - fall through to single-file mode
                // to show diagnostics for the current file
            }
        }

        // Fallback to single-file mode
        let filename = uri
            .path_segments()
            .and_then(|mut s| s.next_back())
            .unwrap_or("input.sop");

        let sniff_enabled = *self.sniff_enabled.read().await;
        let (diagnostics, symbols) = Self::analyse_document(&text, filename, sniff_enabled);

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

    /// Publish diagnostics for all files in the workspace.
    async fn publish_workspace_diagnostics(&self) {
        let ws_guard = self.workspace.read().await;
        let Some(ws) = ws_guard.as_ref() else {
            return;
        };

        let open_docs = self.open_documents.read().await;

        // Publish diagnostics for each file
        for file_id in ws.file_registry.file_ids() {
            let Some(file_path) = ws.file_registry.get_path(file_id) else {
                continue;
            };
            let Ok(uri) = Url::from_file_path(file_path) else {
                continue;
            };

            // Get compile errors for this file
            let mut diagnostics: Vec<Diagnostic> = ws
                .diagnostics
                .get(&file_id)
                .map(|errors| errors.iter().flat_map(soppo_error_to_diagnostics).collect())
                .unwrap_or_default();

            // Run sniff on files without compile errors (if enabled)
            let sniff_enabled = *self.sniff_enabled.read().await;
            if sniff_enabled
                && diagnostics.is_empty()
                && let Some(typed_file) = ws.typed_files.get(&file_id)
            {
                // Get source text
                let source = open_docs
                    .get(file_path)
                    .cloned()
                    .or_else(|| std::fs::read_to_string(file_path).ok());

                if let Some(source) = source {
                    let filename = file_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("input.sop");

                    let warnings =
                        sniff::lint_file(typed_file, filename, &source, &ws.sniff_config);
                    diagnostics.extend(
                        warnings
                            .iter()
                            .map(|w| lint_warning_to_diagnostic(w, &source)),
                    );
                }
            }

            self.client
                .publish_diagnostics(uri, diagnostics, None)
                .await;
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
            SoppoSymbolKind::Package => SymbolKind::MODULE,
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
            SoppoSymbolKind::Package => CompletionItemKind::MODULE,
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
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Parse initializationOptions if present
        if let Some(opts) = params.initialization_options
            && let Ok(init_opts) = serde_json::from_value::<InitOptions>(opts)
        {
            // Editor can disable sniff via initializationOptions
            if !init_opts.sniff.enabled {
                *self.sniff_enabled.write().await = false;
            }
        }

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
                references_provider: Some(OneOf::Left(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Soppo language server initialised")
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

        // Helper to format hover content with optional doc comment
        fn format_hover(symbol: &soppo::types::SymbolInfo) -> String {
            use soppo::types::SymbolKind;

            // Format header based on symbol kind (Go-style)
            let header = match symbol.kind {
                SymbolKind::Variable => format!("var {} {}", symbol.name, symbol.ty),
                SymbolKind::Parameter => format!("{} {}", symbol.name, symbol.ty),
                SymbolKind::Constant => format!("const {} {}", symbol.name, symbol.ty),
                SymbolKind::Type => format!("type {} {}", symbol.name, symbol.ty),
                SymbolKind::Field => format!("{} {}", symbol.name, symbol.ty),
                SymbolKind::Variant => symbol.name.clone(),
                SymbolKind::Package => format!("package {}", symbol.name),
                SymbolKind::Function | SymbolKind::Method => {
                    // Type displays as "func(...) ret", so strip the "func" prefix
                    let ty_str = symbol.ty.to_string();
                    if let Some(rest) = ty_str.strip_prefix("func") {
                        format!("func {}{}", symbol.name, rest)
                    } else {
                        format!("func {} {}", symbol.name, ty_str)
                    }
                }
            };
            let mut content = format!("```soppo\n{}\n```", header);
            if let Some(ref doc) = symbol.doc_comment {
                content.push_str("\n\n---\n\n");
                // Clean up code block language identifiers for better syntax highlighting
                // Strip doctest attributes like ,no_run ,should_panic ,compile_fail
                let cleaned_doc = clean_doctest_fences(doc);
                content.push_str(&cleaned_doc);
            }
            content
        }

        // Strip doctest attributes from code fences for better syntax highlighting
        // Converts ```sop,no_run to ```sop, etc.
        fn clean_doctest_fences(doc: &str) -> String {
            let mut result = String::with_capacity(doc.len());
            for line in doc.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("```sop,") {
                    result.push_str("```sop");
                } else if trimmed.starts_with("```soppo,") {
                    result.push_str("```soppo");
                } else {
                    result.push_str(line);
                }
                result.push('\n');
            }
            // Remove trailing newline if original didn't have one
            if !doc.ends_with('\n') && result.ends_with('\n') {
                result.pop();
            }
            result
        }

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
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: format_hover(symbol),
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

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format_hover(symbol),
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
                    if let Some(symbol) = symbols.find_at(offset) {
                        // Check for Go source location first (external Go packages)
                        if let Some(ref go_loc) = symbol.go_location
                            && let Some(location) = go_location_to_lsp(go_loc)
                        {
                            return Ok(Some(GotoDefinitionResponse::Scalar(location)));
                        }

                        // Fall back to Soppo definition span
                        if let Some(def_span) = symbol.definition_span {
                            // Use name_span for highlighting if available, otherwise fall back to def_span
                            let highlight_span = symbol.name_span.unwrap_or(def_span);

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
                                    range: span_to_range(highlight_span),
                                })));
                            }
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

        // Check for Go source location first (external Go packages)
        if let Some(ref go_loc) = symbol.go_location
            && let Some(location) = go_location_to_lsp(go_loc)
        {
            return Ok(Some(GotoDefinitionResponse::Scalar(location)));
        }

        // Fall back to Soppo definition span
        let Some(def_span) = symbol.definition_span else {
            return Ok(None);
        };

        // Use name_span for highlighting if available, otherwise fall back to def_span
        let highlight_span = symbol.name_span.unwrap_or(def_span);

        // In single-file mode, assume definition is in the same file
        let location = Location {
            uri: uri.clone(),
            range: span_to_range(highlight_span),
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

        // Format the signature in Go style: func name(params) return
        let ty_str = func_info.ty.to_string();
        let signature_label = if let Some(rest) = ty_str.strip_prefix("func") {
            format!("func {}{}", func_info.name, rest)
        } else {
            format!("func {} {}", func_info.name, ty_str)
        };

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

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_declaration = params.context.include_declaration;

        // Get the symbol at cursor position
        let (text, symbols) = if let Some(path) = Self::uri_to_path(&uri) {
            let ws_guard = self.workspace.read().await;
            if let Some(ws) = ws_guard.as_ref()
                && let Some(fid) = ws.file_registry.get_id(&path)
                && let Some(symbols) = ws.symbol_tables.get(&fid)
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

        let Some(text) = text else {
            return Ok(None);
        };
        let Some(symbols) = symbols else {
            return Ok(None);
        };

        let offset = Self::position_to_byte_offset(&text, position);

        // Find the symbol at cursor
        let Some(symbol) = symbols.find_at(offset) else {
            return Ok(None);
        };

        // Get the definition span - this uniquely identifies the symbol
        let Some(def_span) = symbol.definition_span else {
            return Ok(None);
        };

        let mut locations = Vec::new();

        // Search in workspace mode if available
        let ws_guard = self.workspace.read().await;
        if let Some(ws) = ws_guard.as_ref() {
            let open_docs = self.open_documents.read().await;

            // Search all symbol tables in the workspace
            for (fid, file_symbols) in &ws.symbol_tables {
                let file_path = if let Some(path) = ws.file_registry.get_path(*fid) {
                    path
                } else {
                    continue;
                };

                let file_uri = match Url::from_file_path(file_path) {
                    Ok(uri) => uri,
                    Err(_) => continue,
                };

                // Find all symbols that reference the same definition
                for ((start, end), info) in file_symbols.all_symbols() {
                    if let Some(info_def_span) = info.definition_span {
                        // Check if this symbol references the same definition
                        if info_def_span.file == def_span.file
                            && info_def_span.byte_start == def_span.byte_start
                            && info_def_span.byte_end == def_span.byte_end
                        {
                            // Skip the declaration itself unless include_declaration is true
                            let is_declaration = *start == def_span.byte_start
                                && *end == def_span.byte_end
                                && *fid == def_span.file;

                            if is_declaration && !include_declaration {
                                continue;
                            }

                            // Get proper range from byte offsets
                            let range = if let Some(text) = open_docs.get(file_path) {
                                byte_offset_to_range(text, *start, *end)
                            } else if let Ok(text) = std::fs::read_to_string(file_path) {
                                byte_offset_to_range(&text, *start, *end)
                            } else {
                                Range::default()
                            };

                            locations.push(Location {
                                uri: file_uri.clone(),
                                range,
                            });
                        }
                    }
                }
            }
        } else {
            // Single-file mode: only search current file
            for ((start, end), info) in symbols.all_symbols() {
                if let Some(info_def_span) = info.definition_span
                    && info_def_span.byte_start == def_span.byte_start
                    && info_def_span.byte_end == def_span.byte_end
                {
                    let is_declaration = *start == def_span.byte_start && *end == def_span.byte_end;

                    if is_declaration && !include_declaration {
                        continue;
                    }

                    let range = byte_offset_to_range(&text, *start, *end);

                    locations.push(Location {
                        uri: uri.clone(),
                        range,
                    });
                }
            }
        }

        if locations.is_empty() {
            Ok(None)
        } else {
            Ok(Some(locations))
        }
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;

        // Get the document text
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

        // Format the source
        let formatted = match format_source(&text) {
            Ok(f) => f,
            Err(_) => return Ok(None), // Can't format if parsing fails
        };

        // If no change, return empty
        if formatted == text {
            return Ok(Some(vec![]));
        }

        // Return a single edit that replaces the entire document
        let line_count = text.lines().count() as u32;
        let last_line_len = text.lines().last().map(|l| l.len()).unwrap_or(0) as u32;

        Ok(Some(vec![TextEdit {
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: line_count,
                    character: last_line_len,
                },
            },
            new_text: formatted,
        }]))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let position = params.position;

        // Get the symbol at cursor position
        let (text, symbols) = if let Some(path) = Self::uri_to_path(&uri) {
            let ws_guard = self.workspace.read().await;
            if let Some(ws) = ws_guard.as_ref()
                && let Some(fid) = ws.file_registry.get_id(&path)
                && let Some(symbols) = ws.symbol_tables.get(&fid)
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

        let Some(text) = text else {
            return Ok(None);
        };
        let Some(symbols) = symbols else {
            return Ok(None);
        };

        let offset = Self::position_to_byte_offset(&text, position);

        // Find the symbol at cursor
        let Some(symbol) = symbols.find_at(offset) else {
            return Ok(None);
        };

        // Can't rename builtins or Go package symbols
        if symbol.definition_span.is_none() || symbol.go_location.is_some() {
            return Ok(None);
        }

        // Can't rename package imports (for now)
        if symbol.kind == SoppoSymbolKind::Package {
            return Ok(None);
        }

        // Find the range of the symbol name at cursor position
        // Search through all symbols to find the one at the cursor offset
        for ((start, end), info) in symbols.all_symbols() {
            if *start <= offset && offset < *end && info.name == symbol.name {
                let range = byte_offset_to_range(&text, *start, *end);
                return Ok(Some(PrepareRenameResponse::Range(range)));
            }
        }

        Ok(None)
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;

        // Get the symbol at cursor position
        let (text, symbols) = if let Some(path) = Self::uri_to_path(&uri) {
            let ws_guard = self.workspace.read().await;
            if let Some(ws) = ws_guard.as_ref()
                && let Some(fid) = ws.file_registry.get_id(&path)
                && let Some(symbols) = ws.symbol_tables.get(&fid)
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

        let Some(text) = text else {
            return Ok(None);
        };
        let Some(symbols) = symbols else {
            return Ok(None);
        };

        let offset = Self::position_to_byte_offset(&text, position);

        // Find the symbol at cursor
        let Some(symbol) = symbols.find_at(offset) else {
            return Ok(None);
        };

        // Can't rename builtins or Go package symbols
        if symbol.go_location.is_some() {
            return Ok(None);
        }

        // Get the definition span - this uniquely identifies the symbol
        let Some(def_span) = symbol.definition_span else {
            return Ok(None);
        };

        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();

        // Search in workspace mode if available
        let ws_guard = self.workspace.read().await;
        if let Some(ws) = ws_guard.as_ref() {
            let open_docs = self.open_documents.read().await;

            // Search all symbol tables in the workspace
            for (fid, file_symbols) in &ws.symbol_tables {
                let file_path = if let Some(path) = ws.file_registry.get_path(*fid) {
                    path
                } else {
                    continue;
                };

                let file_uri = match Url::from_file_path(file_path) {
                    Ok(uri) => uri,
                    Err(_) => continue,
                };

                // Find all symbols that reference the same definition
                for ((start, end), info) in file_symbols.all_symbols() {
                    if let Some(info_def_span) = info.definition_span {
                        // Check if this symbol references the same definition
                        if info_def_span.file == def_span.file
                            && info_def_span.byte_start == def_span.byte_start
                            && info_def_span.byte_end == def_span.byte_end
                        {
                            // Get proper range from byte offsets
                            let range = if let Some(text) = open_docs.get(file_path) {
                                byte_offset_to_range(text, *start, *end)
                            } else if let Ok(text) = std::fs::read_to_string(file_path) {
                                byte_offset_to_range(&text, *start, *end)
                            } else {
                                Range::default()
                            };

                            changes.entry(file_uri.clone()).or_default().push(TextEdit {
                                range,
                                new_text: new_name.clone(),
                            });
                        }
                    }
                }
            }
        } else {
            // Single-file mode: only search current file
            for ((start, end), info) in symbols.all_symbols() {
                if let Some(info_def_span) = info.definition_span
                    && info_def_span.byte_start == def_span.byte_start
                    && info_def_span.byte_end == def_span.byte_end
                {
                    let range = byte_offset_to_range(&text, *start, *end);

                    changes.entry(uri.clone()).or_default().push(TextEdit {
                        range,
                        new_text: new_name.clone(),
                    });
                }
            }
        }

        if changes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }))
        }
    }
}

/// Run the LSP server on stdin/stdout
pub async fn run_server(cli: Cli) {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let sniff_enabled = !cli.no_sniff;
    let (service, socket) = LspService::new(|client| Backend::new(client, sniff_enabled));
    Server::new(stdin, stdout, socket).serve(service).await;
}
