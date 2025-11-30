use soppo::types::SymbolKind as SoppoSymbolKind;
use tower_lsp::lsp_types::SymbolKind;

use crate::{Backend, span_to_range};

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

#[test]
fn analyze_document_includes_function_symbols() {
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
    let (diagnostics, symbols) = Backend::analyze_document(code, "test.sop");
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
fn analyze_document_includes_type_symbols() {
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
    let (diagnostics, symbols) = Backend::analyze_document(code, "test.sop");
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
fn analyze_document_includes_constant_symbols() {
    let code = r#"
package main

const MaxSize = 100

func main() {
    x := MaxSize
    println(x)
}
"#;
    let (diagnostics, symbols) = Backend::analyze_document(code, "test.sop");
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
fn analyze_document_symbols_in_scope() {
    let code = r#"
package main

func main() {
    x := 42
    y := x + 1
    println(y)
}
"#;
    let (_, symbols) = Backend::analyze_document(code, "test.sop");
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
