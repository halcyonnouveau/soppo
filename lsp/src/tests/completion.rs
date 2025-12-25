use soppo::types::{SymbolKind as SoppoSymbolKind, Type};
use tower_lsp::lsp_types::CompletionItemKind;

use crate::Backend;

#[test]
fn to_completion_kind_converts_correctly() {
    assert_eq!(
        Backend::to_completion_kind(SoppoSymbolKind::Function),
        CompletionItemKind::FUNCTION
    );
    assert_eq!(
        Backend::to_completion_kind(SoppoSymbolKind::Variable),
        CompletionItemKind::VARIABLE
    );
    assert_eq!(
        Backend::to_completion_kind(SoppoSymbolKind::Type),
        CompletionItemKind::STRUCT
    );
    assert_eq!(
        Backend::to_completion_kind(SoppoSymbolKind::Constant),
        CompletionItemKind::CONSTANT
    );
    assert_eq!(
        Backend::to_completion_kind(SoppoSymbolKind::Method),
        CompletionItemKind::METHOD
    );
}

#[test]
fn get_package_prefix_after_dot() {
    let text = "helpers.";
    let cursor = text.len();
    let result = Backend::get_package_prefix(text, cursor);
    assert_eq!(result, Some("helpers".to_string()));
}

#[test]
fn get_package_prefix_no_dot() {
    let text = "helpers";
    let cursor = text.len();
    let result = Backend::get_package_prefix(text, cursor);
    assert_eq!(result, None);
}

#[test]
fn get_package_prefix_after_newline() {
    let text = "func main() {\n    helpers.";
    let cursor = text.len();
    let result = Backend::get_package_prefix(text, cursor);
    assert_eq!(result, Some("helpers".to_string()));
}

#[test]
fn get_package_prefix_with_underscore() {
    let text = "my_package.";
    let cursor = text.len();
    let result = Backend::get_package_prefix(text, cursor);
    assert_eq!(result, Some("my_package".to_string()));
}

#[test]
fn get_package_prefix_after_assignment() {
    let text = "result := helpers.";
    let cursor = text.len();
    let result = Backend::get_package_prefix(text, cursor);
    assert_eq!(result, Some("helpers".to_string()));
}

#[test]
fn func_def_to_type_no_params_no_return() {
    let func_def = soppo::types::FuncDef {
        name: "foo".to_string(),
        generics: vec![],
        params: vec![],
        return_types: vec![],
        span: None,
        name_span: None,
        doc_comment: None,
        must_use: false,
    };
    let result = Backend::func_def_to_type(&func_def);
    assert_eq!(result, "func()");
}

#[test]
fn func_def_to_type_with_params_and_return() {
    let func_def = soppo::types::FuncDef {
        name: "add".to_string(),
        generics: vec![],
        params: vec![
            ("a".to_string(), Type::simple("int")),
            ("b".to_string(), Type::simple("int")),
        ],
        return_types: vec![Type::simple("int")],
        span: None,
        name_span: None,
        doc_comment: None,
        must_use: false,
    };
    let result = Backend::func_def_to_type(&func_def);
    assert_eq!(result, "func(a int, b int) int");
}

#[test]
fn func_def_to_type_multiple_returns() {
    let func_def = soppo::types::FuncDef {
        name: "divmod".to_string(),
        generics: vec![],
        params: vec![
            ("a".to_string(), Type::simple("int")),
            ("b".to_string(), Type::simple("int")),
        ],
        return_types: vec![Type::simple("int"), Type::simple("int")],
        span: None,
        name_span: None,
        doc_comment: None,
        must_use: false,
    };
    let result = Backend::func_def_to_type(&func_def);
    assert_eq!(result, "func(a int, b int) (int, int)");
}
