use crate::Backend;

#[test]
fn find_references_variable_used_multiple_times() {
    let code = r#"
package main

func main() {
    x := 42
    println(x)
    y := x + 1
    println(y)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop", true);
    assert!(
        diagnostics.is_empty(),
        "Valid code should have no diagnostics"
    );
    let symbols = symbols.expect("Should have symbols");

    // Find x at a usage site - println(x)
    let x_usage_offset = code.find("println(x)").unwrap() + 8; // position at 'x' inside println
    let x_symbol = symbols.find_at(x_usage_offset);
    assert!(x_symbol.is_some(), "Should find x at usage");
    let x_symbol = x_symbol.unwrap();
    assert_eq!(x_symbol.name, "x");
    let x_def_span = x_symbol
        .definition_span
        .expect("Should have definition span");

    // Count all references to x (including definition)
    let x_refs: Vec<_> = symbols
        .all_symbols()
        .iter()
        .filter(|(_, info)| {
            if let Some(def_span) = info.definition_span {
                def_span.byte_start == x_def_span.byte_start
                    && def_span.byte_end == x_def_span.byte_end
            } else {
                false
            }
        })
        .collect();

    // Should find at least the usages: println(x), y := x + 1
    // The symbol table records symbols at usage sites during type inference
    assert!(
        x_refs.len() >= 2,
        "Should find at least 2 references to x, found {}",
        x_refs.len()
    );
}

#[test]
fn find_references_function_calls() {
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
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop", true);
    assert!(
        diagnostics.is_empty(),
        "Valid code should have no diagnostics"
    );
    let symbols = symbols.expect("Should have symbols");

    // Find 'add' at a call site
    let add_call_offset = code.find("add(1").unwrap();
    let add_symbol = symbols.find_at(add_call_offset);
    assert!(add_symbol.is_some(), "Should find add at call site");
    let add_symbol = add_symbol.unwrap();
    assert_eq!(add_symbol.name, "add");
    let add_def_span = add_symbol
        .definition_span
        .expect("Should have definition span");

    // Count all references to add
    let add_refs: Vec<_> = symbols
        .all_symbols()
        .iter()
        .filter(|(_, info)| {
            if let Some(def_span) = info.definition_span {
                def_span.byte_start == add_def_span.byte_start
                    && def_span.byte_end == add_def_span.byte_end
            } else {
                false
            }
        })
        .collect();

    // Should find: definition (func add) + 2 call sites
    assert!(
        add_refs.len() >= 2,
        "Should find at least 2 references to add (2 call sites), found {}",
        add_refs.len()
    );
}

#[test]
fn find_references_parameter() {
    let code = r#"
package main

func multiply(n int, times int) int {
    result := 0
    for i := 0; i < times; i++ {
        result = result + n
    }
    return result
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop", true);
    assert!(
        diagnostics.is_empty(),
        "Valid code should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Find 'n' in the function body (result + n)
    // "result + n" - find position of 'n' after "+ "
    let plus_n_pos = code.find("+ n").unwrap() + 2; // position at 'n' after "+ "
    let n_symbol = symbols.find_at(plus_n_pos);
    assert!(
        n_symbol.is_some(),
        "Should find n at usage. All symbols: {:?}",
        symbols
            .all_symbols()
            .values()
            .map(|s| (&s.name, s.kind))
            .collect::<Vec<_>>()
    );
    let n_symbol = n_symbol.unwrap();
    assert_eq!(n_symbol.name, "n");
    let n_def_span = n_symbol
        .definition_span
        .expect("Should have definition span");

    // Count all references to n
    let n_refs: Vec<_> = symbols
        .all_symbols()
        .iter()
        .filter(|(_, info)| {
            if let Some(def_span) = info.definition_span {
                def_span.byte_start == n_def_span.byte_start
                    && def_span.byte_end == n_def_span.byte_end
            } else {
                false
            }
        })
        .collect();

    // Should find at least the usage in the body
    // The symbol table records symbols at usage sites during type inference
    assert!(
        !n_refs.is_empty(),
        "Should find at least 1 reference to n (usage in body), found {}",
        n_refs.len()
    );
}

#[test]
fn find_references_excludes_different_variables() {
    let code = r#"
package main

func main() {
    x := 1
    y := 2
    z := x + y
    println(z)
}
"#;
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop", true);
    assert!(
        diagnostics.is_empty(),
        "Valid code should have no diagnostics"
    );
    let symbols = symbols.expect("Should have symbols");

    // Find 'x' at usage site (z := x + y)
    let x_usage_offset = code.find("z := x").unwrap() + 5; // position at 'x' in "z := x"
    let x_symbol = symbols.find_at(x_usage_offset);
    assert!(x_symbol.is_some(), "Should find x at usage");
    let x_symbol = x_symbol.unwrap();
    assert_eq!(x_symbol.name, "x");
    let x_def_span = x_symbol
        .definition_span
        .expect("Should have definition span");

    // Find 'y' at usage site (z := x + y)
    let y_usage_offset = code.find("+ y").unwrap() + 2; // position at 'y' after "+ "
    let y_symbol = symbols.find_at(y_usage_offset);
    assert!(y_symbol.is_some(), "Should find y at usage");
    let y_symbol = y_symbol.unwrap();
    assert_eq!(y_symbol.name, "y");
    let y_def_span = y_symbol
        .definition_span
        .expect("Should have definition span");

    // x and y should have different definition spans
    assert!(
        x_def_span.byte_start != y_def_span.byte_start
            || x_def_span.byte_end != y_def_span.byte_end,
        "x and y should have different definition spans"
    );

    // References to x should not include y
    let x_refs: Vec<_> = symbols
        .all_symbols()
        .iter()
        .filter(|(_, info)| {
            if let Some(def_span) = info.definition_span {
                def_span.byte_start == x_def_span.byte_start
                    && def_span.byte_end == x_def_span.byte_end
            } else {
                false
            }
        })
        .collect();

    for ((_, _), info) in &x_refs {
        assert_eq!(info.name, "x", "All x references should be named 'x'");
    }
}

#[test]
fn find_references_type_usage() {
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
    let (diagnostics, symbols) = Backend::analyse_document(code, "test.sop", true);
    assert!(
        diagnostics.is_empty(),
        "Valid code should have no diagnostics: {:?}",
        diagnostics
    );
    let symbols = symbols.expect("Should have symbols");

    // Find 'Point' at struct literal usage
    let point_usage_offset = code.find("Point{").unwrap();
    let point_symbol = symbols.find_at(point_usage_offset);
    assert!(
        point_symbol.is_some(),
        "Should find Point at struct literal usage"
    );
    let point_symbol = point_symbol.unwrap();
    assert_eq!(point_symbol.name, "Point");
}
