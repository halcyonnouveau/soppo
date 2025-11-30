use std::collections::HashMap;
use std::sync::Arc;

use soppo::build::typecheck;
use soppo::error::SoppoError;
use soppo::syntax::Span;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Debug)]
struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, String>>>,
}

impl Backend {
    fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn publish_diagnostics(&self, uri: Url, text: &str) {
        let filename = uri
            .path_segments()
            .and_then(|s| s.last())
            .unwrap_or("input.sop");

        let diagnostics = match typecheck(text, filename) {
            Ok(()) => vec![],
            Err(report) => {
                // Try to downcast to SoppoError for span info
                if let Some(err) = report.downcast_ref::<SoppoError>() {
                    self.soppo_error_to_diagnostics(err)
                } else {
                    // Fallback: use error message without span
                    vec![Diagnostic {
                        range: Range::default(),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("soppo".to_string()),
                        message: report.to_string(),
                        ..Default::default()
                    }]
                }
            }
        };

        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    fn span_to_range(&self, span: Span) -> Range {
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

    fn soppo_error_to_diagnostics(&self, err: &SoppoError) -> Vec<Diagnostic> {
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

        let range = span.map(|s| self.span_to_range(s)).unwrap_or_default();

        vec![Diagnostic {
            range,
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("soppo".to_string()),
            message,
            ..Default::default()
        }]
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

        self.documents
            .write()
            .await
            .insert(uri.clone(), text.clone());
        self.publish_diagnostics(uri, &text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(change) = params.content_changes.into_iter().last() {
            let text = change.text;
            self.documents
                .write()
                .await
                .insert(uri.clone(), text.clone());
            self.publish_diagnostics(uri, &text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .write()
            .await
            .remove(&params.text_document.uri);
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
