use soppo::syntax::{FileId, LineColumn};
use soppo::types::Type;
use tower_lsp::lsp_types::{DiagnosticSeverity, Position};

use crate::{Backend, SoppoError, Span, check_document, soppo_error_to_diagnostics, span_to_range};

fn make_span(start_line: usize, start_col: usize, end_line: usize, end_col: usize) -> Span {
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

    let x_usage_offset = code.rfind("(x)").unwrap() + 1;

    let symbol = symbols.find_at(x_usage_offset);
    assert!(symbol.is_some(), "Should find symbol at x usage");
    let symbol = symbol.unwrap();
    assert_eq!(symbol.name, "x");
    assert_eq!(symbol.ty.to_string(), "int");
}

#[test]
fn symbol_has_definition_span() {
    let code = r#"
package main

func main() {
    x := 42
    println(x)
}
"#;
    let (_, symbols) = Backend::analyze_document(code, "test.sop");
    let symbols = symbols.expect("Should have symbols");

    let x_usage_offset = code.rfind("(x)").unwrap() + 1;

    let symbol = symbols.find_at(x_usage_offset);
    assert!(symbol.is_some(), "Should find symbol at x usage");
    let symbol = symbol.unwrap();

    assert!(
        symbol.definition_span.is_some(),
        "Symbol should have a definition span"
    );

    let def_span = symbol.definition_span.unwrap();
    assert_eq!(def_span.start.line, 5, "Definition should be on line 5");
}

#[test]
fn function_parameter_has_definition_span() {
    let code = r#"
package main

func add(a int, b int) int {
    return a + b
}
"#;
    let (_, symbols) = Backend::analyze_document(code, "test.sop");
    let symbols = symbols.expect("Should have symbols");

    let a_usage_offset = code.find("return a").unwrap() + 7;

    let symbol = symbols.find_at(a_usage_offset);
    assert!(symbol.is_some(), "Should find symbol at a usage");
    let symbol = symbol.unwrap();
    assert_eq!(symbol.name, "a");

    assert!(
        symbol.definition_span.is_some(),
        "Parameter should have a definition span"
    );

    let def_span = symbol.definition_span.unwrap();
    assert_eq!(
        def_span.start.line, 4,
        "Parameter definition should be on line 4"
    );
}

#[test]
fn goto_definition_returns_location_for_variable() {
    let code = r#"
package main

func main() {
    x := 42
    println(x)
}
"#;
    let (_, symbols) = Backend::analyze_document(code, "test.sop");
    let symbols = symbols.expect("Should have symbols");

    let x_usage_offset = code.rfind("(x)").unwrap() + 1;

    let symbol = symbols.find_at(x_usage_offset);
    let symbol = symbol.expect("Should find symbol");
    let def_span = symbol.definition_span.expect("Should have definition span");

    let range = span_to_range(def_span);

    assert_eq!(range.start.line, 4, "Definition range should be on line 4");
}
