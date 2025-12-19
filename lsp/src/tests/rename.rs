use soppo::types::SymbolKind as SoppoSymbolKind;

use crate::Backend;

#[test]
fn prepare_rename_returns_range_for_variable() {
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
    let symbols = symbols.expect("Should have symbols");

    // Find x at the definition site
    let x_def_offset = code.find("x :=").unwrap();
    let symbol = symbols.find_at(x_def_offset);
    assert!(symbol.is_some(), "Should find x at definition");
    let symbol = symbol.unwrap();
    assert_eq!(symbol.name, "x");
    assert_eq!(symbol.kind, SoppoSymbolKind::Variable);

    // Should have a definition span (can be renamed)
    assert!(
        symbol.definition_span.is_some(),
        "Variable should have definition span"
    );

    // Should not have a go_location (not a Go package symbol)
    assert!(
        symbol.go_location.is_none(),
        "Local variable should not have go_location"
    );
}

#[test]
fn rename_finds_all_variable_occurrences() {
    let code = r#"
package main

func main() {
    x := 42
    println(x)
    y := x + 1
    println(y)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
    assert!(
        diagnostics.is_empty(),
        "Valid code should have no diagnostics"
    );
    let symbols = symbols.expect("Should have symbols");

    // Find x at a usage site
    let x_usage_offset = code.find("println(x)").unwrap() + 8;
    let symbol = symbols.find_at(x_usage_offset);
    assert!(symbol.is_some(), "Should find x at usage");
    let symbol = symbol.unwrap();
    let def_span = symbol.definition_span.expect("Should have definition span");

    // Count all occurrences that would be renamed
    let rename_targets: Vec<_> = symbols
        .all_symbols()
        .iter()
        .filter(|(_, info)| {
            if let Some(info_def_span) = info.definition_span {
                info_def_span.byte_start == def_span.byte_start
                    && info_def_span.byte_end == def_span.byte_end
            } else {
                false
            }
        })
        .collect();

    // Should find at least 2 occurrences: println(x), x + 1
    // (definition is recorded but may be at same position)
    assert!(
        rename_targets.len() >= 2,
        "Should find at least 2 occurrences of x, found {}",
        rename_targets.len()
    );
}

#[test]
fn rename_finds_function_definition_and_calls() {
    let code = r#"
package main

func add(a int, b int) int {
    return a + b
}

func main() {
    x := add(1, 2)
    y := add(3, 4)
    println(x + y)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop");
    assert!(
        diagnostics.is_empty(),
        "Valid code should have no diagnostics"
    );
    let symbols = symbols.expect("Should have symbols");

    // Find add at a call site
    let add_call_offset = code.find("x := add").unwrap() + 5;
    let symbol = symbols.find_at(add_call_offset);
    assert!(symbol.is_some(), "Should find add at call site");
    let symbol = symbol.unwrap();
    assert_eq!(symbol.name, "add");
    let def_span = symbol.definition_span.expect("Should have definition span");

    // Count all occurrences that would be renamed
    let rename_targets: Vec<_> = symbols
        .all_symbols()
        .iter()
        .filter(|(_, info)| {
            if let Some(info_def_span) = info.definition_span {
                info_def_span.byte_start == def_span.byte_start
                    && info_def_span.byte_end == def_span.byte_end
            } else {
                false
            }
        })
        .collect();

    // Should find: definition (func add), add(1, 2), add(3, 4)
    assert!(
        rename_targets.len() >= 3,
        "Should find at least 3 occurrences of add, found {}",
        rename_targets.len()
    );
}

#[test]
fn cannot_rename_go_package_symbols() {
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
        "Valid code should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Find NewScanner at the call site
    let call_offset = code.find("NewScanner").unwrap();
    let symbol = symbols.find_at(call_offset);
    assert!(symbol.is_some(), "Should find NewScanner at call site");
    let symbol = symbol.unwrap();
    assert_eq!(symbol.name, "NewScanner");

    // Should have a go_location (cannot be renamed)
    assert!(
        symbol.go_location.is_some(),
        "Go package function should have go_location"
    );
}

#[test]
fn cannot_rename_package_import_name() {
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
        "Valid code should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Find 'bufio' at usage site
    let bufio_offset = code.find("bufio.New").unwrap();
    let symbol = symbols.find_at(bufio_offset);
    assert!(symbol.is_some(), "Should find bufio at usage");
    let symbol = symbol.unwrap();
    assert_eq!(symbol.name, "bufio");
    assert_eq!(symbol.kind, SoppoSymbolKind::Package);
}
