use tower_lsp::lsp_types::{DiagnosticSeverity, NumberOrString};

use crate::Backend;

#[test]
fn analyse_document_returns_sniff_warnings_for_if_err_nil() {
    let code = r#"
package main

import "errors"

func parsePort(s string) (int, error) {
    if s == "" {
        return 0, errors.New("empty port")
    }
    return 8080, nil
}

func connect() (string, error) {
    port, err := parsePort("8080")
    if err != nil {
        return "", err
    }
    return "Connected on port {port}", nil
}
"#;

    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");

    // Should have symbols (no compile errors)
    assert!(symbols.is_some(), "Should produce symbols for valid code");

    // Should have sniff warning for if err != nil pattern
    assert!(
        !diagnostics.is_empty(),
        "Should have sniff warning for if err != nil pattern"
    );

    let warning = &diagnostics[0];
    assert_eq!(
        warning.severity,
        Some(DiagnosticSeverity::WARNING),
        "Sniff diagnostics should be warnings, not errors"
    );
    assert_eq!(
        warning.source,
        Some("sniff".to_string()),
        "Sniff diagnostics should have source 'sniff'"
    );
    assert_eq!(
        warning.code,
        Some(NumberOrString::String("try_operator".to_string())),
        "Should have code 'try_operator'"
    );
    assert!(
        warning.message.contains("?"),
        "Warning should mention the ? operator"
    );
}

#[test]
fn analyse_document_no_warnings_for_try_operator() {
    let code = r#"
package main

import "errors"

func parsePort(s string) (int, error) {
    if s == "" {
        return 0, errors.New("empty port")
    }
    return 8080, nil
}

func connect() (string, error) {
    port := parsePort("8080") ?
    return "Connected on port {port}", nil
}
"#;

    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");

    // Should have symbols (no compile errors)
    assert!(symbols.is_some(), "Should produce symbols for valid code");

    // Should have no warnings when using ? operator
    assert!(
        diagnostics.is_empty(),
        "Should have no warnings when using ? operator, got: {:?}",
        diagnostics
    );
}

#[test]
fn analyse_document_sniff_ignored_with_comment() {
    let code = r#"
package main

import "errors"

func parsePort(s string) (int, error) {
    if s == "" {
        return 0, errors.New("empty port")
    }
    return 8080, nil
}

func connect() (string, error) {
    port, err := parsePort("8080")
    //sniff:ignore try_operator
    if err != nil {
        return "", err
    }
    return "Connected on port {port}", nil
}
"#;

    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");

    // Should have symbols (no compile errors)
    assert!(symbols.is_some(), "Should produce symbols for valid code");

    // Should have no warnings when ignored
    assert!(
        diagnostics.is_empty(),
        "Should have no warnings when sniff:ignore is used, got: {:?}",
        diagnostics
    );
}

#[test]
fn analyse_document_compile_error_no_sniff() {
    let code = r#"
package main

func main() {
    x := undefined_var
}
"#;

    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");

    // Should NOT have symbols (compile error)
    assert!(
        symbols.is_none(),
        "Should not produce symbols when there's a compile error"
    );

    // Should have compile error
    assert!(
        !diagnostics.is_empty(),
        "Should have compile error diagnostic"
    );

    // Compile error should be ERROR severity, not WARNING
    assert_eq!(
        diagnostics[0].severity,
        Some(DiagnosticSeverity::ERROR),
        "Compile errors should be ERROR severity"
    );
    assert_eq!(
        diagnostics[0].source,
        Some("soppo".to_string()),
        "Compile errors should have source 'soppo'"
    );
}

#[test]
fn sniff_warning_has_correct_range() {
    let code = r#"package main

import "errors"

func parsePort(s string) (int, error) {
    if s == "" {
        return 0, errors.New("empty port")
    }
    return 8080, nil
}

func connect() (string, error) {
    port, err := parsePort("8080")
    if err != nil {
        return "", err
    }
    return "Connected on port {port}", nil
}
"#;

    let (diagnostics, _) = Backend::analyse_document(code, "test.sop");

    assert!(!diagnostics.is_empty(), "Should have sniff warning");

    let warning = &diagnostics[0];

    // The warning should point to the "if err != nil" line (line 13, 0-indexed)
    assert!(
        warning.range.start.line >= 12,
        "Warning should be around line 13-14, got line {}",
        warning.range.start.line
    );
}
