use soppo::types::Type;
use tower_lsp::lsp_types::{DiagnosticSeverity, Position};

use super::make_span;
use crate::{Backend, SoppoError, check_document, soppo_error_to_diagnostics, span_to_range};

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
fn check_document_valid_code_no_diagnostics() {
    let code = r#"
package main

func add(a int, b int) int {
    return a + b
}
"#;
    let diagnostics = check_document(code, "test.sop");
    assert!(
        diagnostics.is_empty(),
        "Valid code should have no diagnostics"
    );
}

#[test]
fn check_document_undefined_variable() {
    let code = r#"
package main

func main() {
    x := undefined_var
}
"#;
    let diagnostics = check_document(code, "test.sop");
    assert!(
        !diagnostics.is_empty(),
        "Should have diagnostics for undefined variable"
    );
    assert!(
        diagnostics[0]
            .message
            .to_lowercase()
            .contains("cannot find value"),
        "Unexpected error message: {}",
        diagnostics[0].message
    );
}

#[test]
fn check_document_type_mismatch() {
    let code = r#"
package main

func main() {
    var x int = "hello"
}
"#;
    let diagnostics = check_document(code, "test.sop");
    assert!(
        !diagnostics.is_empty(),
        "Should have diagnostics for type mismatch"
    );
}

#[test]
fn check_document_parse_error() {
    let code = r#"
package main

func main() {
    if {
}
"#;
    let diagnostics = check_document(code, "test.sop");
    assert!(
        !diagnostics.is_empty(),
        "Should have diagnostics for parse error"
    );
}

#[test]
fn check_document_multiple_errors() {
    let code = r#"
package main

func main() {
    x := undefined1
    y := undefined2
}
"#;
    let diagnostics = check_document(code, "test.sop");
    assert!(!diagnostics.is_empty());
}

#[test]
fn check_document_nil_safety_error() {
    let code = r#"
package main

func main() {
    var p ?*int = nil
    x := *p  // should error - dereferencing potentially nil pointer
}
"#;
    let diagnostics = check_document(code, "test.sop");
    assert!(!diagnostics.is_empty(), "Should report nil safety error");
}

#[test]
fn check_document_enum_exhaustiveness() {
    let code = r#"
package main

enum Color {
    Red
    Green
    Blue
}

func main() {
    c := Color.Red
    match c {
        .Red => println("red")
        // Missing Green and Blue
    }
}
"#;
    let diagnostics = check_document(code, "test.sop");
    assert!(
        !diagnostics.is_empty(),
        "Should report non-exhaustive match"
    );
}

#[test]
fn check_document_diagnostic_range_is_correct() {
    // "undefined_x" is on line 4 (1-indexed), starting at column 10
    let code = "package main\n\nfunc main() {\n    x := undefined_x\n}\n";
    let diagnostics = check_document(code, "test.sop");
    assert!(!diagnostics.is_empty(), "Expected diagnostics");

    let diag = &diagnostics[0];
    assert_eq!(
        diag.range.start.line, 3,
        "Error should be on line 3 (0-indexed)"
    );
    assert_eq!(
        diag.range.start.character, 9,
        "Error should start at column 9 (0-indexed)"
    );
}

#[test]
fn check_document_string_interpolation_error_range() {
    let code = r#"
package main

func main() {
    msg := "hello {undefined_var}"
}
"#;
    let diagnostics = check_document(code, "test.sop");
    assert!(
        !diagnostics.is_empty(),
        "Expected diagnostics for undefined variable in interpolation"
    );

    let diag = &diagnostics[0];
    assert!(
        diag.range.start.line > 0,
        "Error in string interpolation should not be at line 0, got line {}",
        diag.range.start.line
    );
}

#[test]
fn analyse_document_returns_diagnostics_on_error() {
    // This tests the fallback behavior - when workspace typecheck fails,
    // the LSP falls back to single-file analysis using analyse_document
    let code = r#"
package main

func main() {
    x := undefined_var
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");

    // Should have diagnostics for the error
    assert!(
        !diagnostics.is_empty(),
        "Should have diagnostics for undefined variable"
    );

    // Should NOT have symbols when there's an error
    assert!(
        symbols.is_none(),
        "Should not produce symbols when there's an error"
    );

    // Verify the diagnostic message is useful
    assert!(
        diagnostics[0]
            .message
            .to_lowercase()
            .contains("cannot find value"),
        "Diagnostic should mention undefined variable: {}",
        diagnostics[0].message
    );
}

#[test]
fn analyse_document_clears_diagnostics_on_success() {
    // When code is valid, diagnostics should be empty
    let code = r#"
package main

func main() {
    x := 42
    println(x)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");

    // Should have NO diagnostics for valid code
    assert!(
        diagnostics.is_empty(),
        "Valid code should have no diagnostics"
    );

    // Should have symbols
    assert!(symbols.is_some(), "Valid code should produce symbols");
}
