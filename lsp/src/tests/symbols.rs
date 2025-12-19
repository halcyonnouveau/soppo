use std::path::PathBuf;

use soppo::go::SourceLocation;
use soppo::types::SymbolKind as SoppoSymbolKind;
use tower_lsp::lsp_types::SymbolKind;

use crate::{Backend, go_location_to_lsp, span_to_range};

#[test]
fn to_lsp_symbol_kind_converts_correctly() {
    assert_eq!(
        Backend::to_lsp_symbol_kind(SoppoSymbolKind::Function),
        SymbolKind::FUNCTION
    );
    assert_eq!(
        Backend::to_lsp_symbol_kind(SoppoSymbolKind::Type),
        SymbolKind::STRUCT
    );
    assert_eq!(
        Backend::to_lsp_symbol_kind(SoppoSymbolKind::Constant),
        SymbolKind::CONSTANT
    );
    assert_eq!(
        Backend::to_lsp_symbol_kind(SoppoSymbolKind::Variable),
        SymbolKind::VARIABLE
    );
    assert_eq!(
        Backend::to_lsp_symbol_kind(SoppoSymbolKind::Method),
        SymbolKind::METHOD
    );
    assert_eq!(
        Backend::to_lsp_symbol_kind(SoppoSymbolKind::Field),
        SymbolKind::FIELD
    );
    assert_eq!(
        Backend::to_lsp_symbol_kind(SoppoSymbolKind::Variant),
        SymbolKind::ENUM_MEMBER
    );
    assert_eq!(
        Backend::to_lsp_symbol_kind(SoppoSymbolKind::Package),
        SymbolKind::MODULE
    );
}

#[test]
fn analyse_document_returns_symbols_for_valid_code() {
    let code = r#"
package main

func main() {
    x := 42
    println(x)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
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
fn analyse_document_returns_diagnostics_for_invalid_code() {
    let code = r#"
package main

func main() {
    x := undefined_var
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
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
    let (_, symbols) = Backend::analyse_document(code, "test.sop");
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
    let (_, symbols) = Backend::analyse_document(code, "test.sop");
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
    let (_, symbols) = Backend::analyse_document(code, "test.sop");
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
    let (_, symbols) = Backend::analyse_document(code, "test.sop");
    let symbols = symbols.expect("Should have symbols");

    let x_usage_offset = code.rfind("(x)").unwrap() + 1;

    let symbol = symbols.find_at(x_usage_offset);
    let symbol = symbol.expect("Should find symbol");
    let def_span = symbol.definition_span.expect("Should have definition span");

    let range = span_to_range(def_span);

    assert_eq!(range.start.line, 4, "Definition range should be on line 4");
}

#[test]
fn analyse_document_includes_function_symbols() {
    let code = r#"
package main

func add(a int, b int) int {
    return a + b
}

func main() {
    x := add(1, 2)
    println(x)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
    assert!(
        diagnostics.is_empty(),
        "Should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Check that we have the 'add' function in symbols (when it's called)
    let add_symbols: Vec<_> = symbols
        .all_symbols()
        .values()
        .filter(|s| s.name == "add")
        .collect();

    assert!(
        !add_symbols.is_empty(),
        "Should have 'add' symbol recorded (from call site). All symbols: {:?}",
        symbols
            .all_symbols()
            .values()
            .map(|s| (&s.name, s.kind))
            .collect::<Vec<_>>()
    );
}

#[test]
fn analyse_document_includes_type_symbols() {
    let code = r#"
package main

type Point struct {
    X int
    Y int
}

func main() {
    p := Point{X: 1, Y: 2}
    println(p.X)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
    assert!(
        diagnostics.is_empty(),
        "Should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Check that we have the 'Point' type in symbols (from struct literal usage)
    let point_symbols: Vec<_> = symbols
        .all_symbols()
        .values()
        .filter(|s| s.name == "Point")
        .collect();

    assert!(
        !point_symbols.is_empty(),
        "Should have 'Point' symbol recorded (from struct literal). All symbols: {:?}",
        symbols
            .all_symbols()
            .values()
            .map(|s| (&s.name, s.kind))
            .collect::<Vec<_>>()
    );
}

#[test]
fn analyse_document_includes_constant_symbols() {
    let code = r#"
package main

const MaxSize = 100

func main() {
    x := MaxSize
    println(x)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
    assert!(
        diagnostics.is_empty(),
        "Should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Check that we have 'MaxSize' constant in symbols (from usage)
    let const_symbols: Vec<_> = symbols
        .all_symbols()
        .values()
        .filter(|s| s.name == "MaxSize")
        .collect();

    assert!(
        !const_symbols.is_empty(),
        "Should have 'MaxSize' symbol recorded (from usage). All symbols: {:?}",
        symbols
            .all_symbols()
            .values()
            .map(|s| (&s.name, s.kind))
            .collect::<Vec<_>>()
    );
}

#[test]
fn analyse_document_symbols_in_scope() {
    let code = r#"
package main

func main() {
    x := 42
    y := x + 1
    println(y)
}
"#;
    let (_, symbols) = Backend::analyse_document(code, "test.sop");
    let symbols = symbols.expect("Should have symbols");

    // Find the 'y' usage in println(y)
    let y_offset = code.rfind("(y)").unwrap() + 1;
    let symbol = symbols.find_at(y_offset);
    assert!(symbol.is_some(), "Should find 'y' symbol");
    assert_eq!(symbol.unwrap().name, "y");

    // Also check that we can find 'x' where it's used
    let x_offset = code.find("y := x").unwrap() + 5;
    let symbol = symbols.find_at(x_offset);
    assert!(symbol.is_some(), "Should find 'x' symbol");
    assert_eq!(symbol.unwrap().name, "x");
}

#[test]
fn symbol_lookup_finds_try_expression_variable() {
    let code = r#"
package main

func readLines(path string) ([]string, error) {
    return []string{}, nil
}

func main() error {
    lines := readLines("input.txt") ?
    println(lines)
    return nil
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
    assert!(
        diagnostics.is_empty(),
        "Should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Find 'lines' at its definition
    let lines_def_offset = code.find("lines :=").unwrap();
    let symbol = symbols.find_at(lines_def_offset);
    assert!(symbol.is_some(), "Should find 'lines' symbol at definition");
    let symbol = symbol.unwrap();
    assert_eq!(symbol.name, "lines");
    // The type should be []string (error stripped by ?)
    assert_eq!(symbol.ty.to_string(), "[]string");
}

#[test]
fn function_call_has_doc_comment() {
    let code = r#"
package main

// help me doc
func readLines(path string) ([]string, error) {
    return []string{}, nil
}

func main() error {
    lines := readLines("input.txt") ?
    println(lines)
    return nil
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
    assert!(
        diagnostics.is_empty(),
        "Should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Find 'readLines' at the call site
    let call_offset = code.find("readLines(\"input").unwrap();
    let symbol = symbols.find_at(call_offset);
    assert!(
        symbol.is_some(),
        "Should find 'readLines' symbol at call site"
    );
    let symbol = symbol.unwrap();
    assert_eq!(symbol.name, "readLines");
    assert!(
        symbol.doc_comment.is_some(),
        "Function symbol should have doc comment. Symbol: {:?}",
        symbol
    );
    assert_eq!(symbol.doc_comment.as_ref().unwrap(), " help me doc");
}

#[test]
fn go_package_function_has_go_location() {
    let code = r#"
package main

import "bufio"

func main() {
    scanner := bufio.NewScanner(nil)
    println(scanner)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
    assert!(
        diagnostics.is_empty(),
        "Should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Find 'NewScanner' at the call site
    let call_offset = code.find("NewScanner").unwrap();
    let symbol = symbols.find_at(call_offset);
    assert!(
        symbol.is_some(),
        "Should find 'NewScanner' symbol at call site"
    );
    let symbol = symbol.unwrap();
    assert_eq!(symbol.name, "NewScanner");
    assert_eq!(symbol.kind, SoppoSymbolKind::Function);

    // Should have a Go source location
    assert!(
        symbol.go_location.is_some(),
        "Go package function should have go_location. Symbol: {:?}",
        symbol
    );

    let go_loc = symbol.go_location.as_ref().unwrap();
    // The location should point to the bufio package
    assert!(
        go_loc.file.to_string_lossy().contains("bufio"),
        "Go location should be in bufio package, got: {:?}",
        go_loc.file
    );
    assert!(go_loc.start_line > 0, "Line should be positive");
}

#[test]
fn go_package_type_annotation_has_go_location() {
    // Test that Go types in type annotations (e.g., var x *bufio.Scanner) record symbols with go_location
    let code = r#"
package main

import "bufio"

func main() {
    var scanner ?*bufio.Scanner = nil
    println(scanner)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
    assert!(
        diagnostics.is_empty(),
        "Should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Find 'Scanner' in the type annotation
    let type_offset = code.find("Scanner").unwrap();
    let symbol = symbols.find_at(type_offset);
    assert!(
        symbol.is_some(),
        "Should find 'Scanner' symbol in type annotation"
    );
    let symbol = symbol.unwrap();
    assert_eq!(symbol.name, "Scanner");
    assert_eq!(symbol.kind, SoppoSymbolKind::Type);

    // Should have a Go source location
    assert!(
        symbol.go_location.is_some(),
        "Go package type in annotation should have go_location. Symbol: {:?}",
        symbol
    );

    let go_loc = symbol.go_location.as_ref().unwrap();
    assert!(
        go_loc.file.to_string_lossy().contains("bufio"),
        "Go location should be in bufio package, got: {:?}",
        go_loc.file
    );
}

#[test]
fn go_location_to_lsp_converts_correctly() {
    let go_loc = SourceLocation {
        file: PathBuf::from("/home/user/go/src/bufio/scan.go"),
        start_line: 10,
        start_col: 5,
        end_line: 10,
        end_col: 15,
    };

    let lsp_loc = go_location_to_lsp(&go_loc);
    assert!(lsp_loc.is_some(), "Should convert to LSP location");

    let lsp_loc = lsp_loc.unwrap();
    // LSP uses 0-based line/col, SourceLocation uses 1-based
    assert_eq!(lsp_loc.range.start.line, 9);
    assert_eq!(lsp_loc.range.start.character, 4);
    assert_eq!(lsp_loc.range.end.line, 9);
    assert_eq!(lsp_loc.range.end.character, 14);
    assert!(lsp_loc.uri.path().ends_with("bufio/scan.go"));
}

#[test]
fn go_package_function_has_doc_comment() {
    let code = r#"
package main

import "bufio"

func main() {
    scanner := bufio.NewScanner(nil)
    println(scanner)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
    assert!(
        diagnostics.is_empty(),
        "Should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Find 'NewScanner' at the call site
    let call_offset = code.find("NewScanner").unwrap();
    let symbol = symbols.find_at(call_offset);
    assert!(
        symbol.is_some(),
        "Should find 'NewScanner' symbol at call site"
    );
    let symbol = symbol.unwrap();
    assert_eq!(symbol.name, "NewScanner");

    // Should have a doc comment from Go source
    assert!(
        symbol.doc_comment.is_some(),
        "Go package function should have doc_comment. Symbol: {:?}",
        symbol
    );

    // NewScanner's doc comment should mention Scanner or io.Reader
    let doc = symbol.doc_comment.as_ref().unwrap();
    assert!(
        doc.contains("Scanner") || doc.contains("io.Reader") || doc.contains("NewScanner"),
        "Doc comment should be relevant: {:?}",
        doc
    );
}

#[test]
fn go_package_type_has_doc_comment() {
    let code = r#"
package main

import "bufio"

func main() {
    var scanner ?*bufio.Scanner = nil
    println(scanner)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
    assert!(
        diagnostics.is_empty(),
        "Should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Find 'Scanner' in the type annotation
    let type_offset = code.find("Scanner").unwrap();
    let symbol = symbols.find_at(type_offset);
    assert!(
        symbol.is_some(),
        "Should find 'Scanner' symbol in type annotation"
    );
    let symbol = symbol.unwrap();
    assert_eq!(symbol.name, "Scanner");
    assert_eq!(symbol.kind, SoppoSymbolKind::Type);

    // Should have a doc comment from Go source
    assert!(
        symbol.doc_comment.is_some(),
        "Go package type should have doc_comment. Symbol: {:?}",
        symbol
    );

    // Scanner's doc comment should be relevant
    let doc = symbol.doc_comment.as_ref().unwrap();
    assert!(
        doc.contains("Scanner") || doc.contains("split") || doc.contains("token"),
        "Doc comment should be relevant: {:?}",
        doc
    );
}

#[test]
fn package_name_has_symbol_for_goto() {
    let code = r#"
package main

import "bufio"

func main() {
    scanner := bufio.NewScanner(nil)
    println(scanner)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
    assert!(
        diagnostics.is_empty(),
        "Should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Find 'bufio' (the package name) at the usage site
    let bufio_offset = code.find("bufio.New").unwrap();
    let symbol = symbols.find_at(bufio_offset);
    assert!(
        symbol.is_some(),
        "Should find 'bufio' symbol at package usage"
    );
    let symbol = symbol.unwrap();
    assert_eq!(symbol.name, "bufio");
    assert_eq!(symbol.kind, SoppoSymbolKind::Package);

    // Should have a definition span pointing to the import
    assert!(
        symbol.definition_span.is_some(),
        "Package symbol should have definition_span pointing to import. Symbol: {:?}",
        symbol
    );

    // The type should show the import path
    assert_eq!(symbol.ty.to_string(), "bufio");

    // Doc comment should show the import
    assert!(
        symbol.doc_comment.is_some(),
        "Package symbol should have doc_comment"
    );
    let doc = symbol.doc_comment.as_ref().unwrap();
    assert!(
        doc.contains("import") && doc.contains("bufio"),
        "Doc comment should show import: {:?}",
        doc
    );
}
