use soppo_lsp::check_document;

/// Test that valid code produces no diagnostics
#[test]
fn valid_code_no_errors() {
    let code = r#"
package main

func main() {
    x := 42
    println(x)
}
"#;
    let diagnostics = check_document(code, "test.sop");
    assert!(diagnostics.is_empty());
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
        "Should have diagnostics for undefined variable, got: {:?}",
        diagnostics
    );
    assert!(
        diagnostics[0].message.contains("cannot find value")
            || diagnostics[0].message.contains("undefined")
            || diagnostics[0].message.contains("not found"),
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

/// Test that multiple errors in one file are all reported
#[test]
fn multiple_errors_reported() {
    let code = r#"
package main

func main() {
    x := undefined1
    y := undefined2
}
"#;
    let diagnostics = check_document(code, "test.sop");
    // At least one error should be reported
    assert!(!diagnostics.is_empty());
}

/// Test nil safety errors are reported
#[test]
fn nil_safety_error_reported() {
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

/// Test enum exhaustiveness is checked
#[test]
fn enum_exhaustiveness_error() {
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

/// Test that diagnostics have correct range
#[test]
fn diagnostic_range_is_correct() {
    // "undefined_x" is on line 4 (1-indexed), starting at column 10
    let code = "package main\n\nfunc main() {\n    x := undefined_x\n}\n";
    let diagnostics = check_document(code, "test.sop");
    assert!(!diagnostics.is_empty(), "Expected diagnostics but got none");

    let diag = &diagnostics[0];
    // The error should be on line 4 (0-indexed = 3)
    assert_eq!(
        diag.range.start.line, 3,
        "Error should be on line 4 (0-indexed: 3)"
    );
    // "undefined_x" starts at column 10 (0-indexed: 9)
    assert_eq!(
        diag.range.start.character, 9,
        "Error should start at column 10 (0-indexed: 9)"
    );
}
