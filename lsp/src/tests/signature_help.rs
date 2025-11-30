//! Signature help tests

#[test]
fn find_function_call_context_simple() {
    // Test the logic of finding function call context
    let text = "func main() {\n    add(1, 2)\n}";
    let cursor_pos = text.find("add(1").unwrap() + 5; // Position after "add(1"

    // Simulate the signature help logic
    let before_cursor = &text[..cursor_pos];
    let mut paren_depth = 0;
    let mut func_call_start = None;
    let mut comma_count = 0;

    for (i, c) in before_cursor.char_indices().rev() {
        match c {
            ')' => paren_depth += 1,
            '(' => {
                if paren_depth == 0 {
                    func_call_start = Some(i);
                    break;
                }
                paren_depth -= 1;
            }
            ',' if paren_depth == 0 => comma_count += 1,
            _ => {}
        }
    }

    assert!(func_call_start.is_some(), "Should find opening paren");
    assert_eq!(comma_count, 0, "Should have 0 commas before cursor");
}

#[test]
fn find_function_call_context_with_comma() {
    let text = "func main() {\n    add(1, 2)\n}";
    let cursor_pos = text.find("add(1, 2").unwrap() + 7; // Position after "add(1, "

    let before_cursor = &text[..cursor_pos];
    let mut paren_depth = 0;
    let mut func_call_start = None;
    let mut comma_count = 0;

    for (i, c) in before_cursor.char_indices().rev() {
        match c {
            ')' => paren_depth += 1,
            '(' => {
                if paren_depth == 0 {
                    func_call_start = Some(i);
                    break;
                }
                paren_depth -= 1;
            }
            ',' if paren_depth == 0 => comma_count += 1,
            _ => {}
        }
    }

    assert!(func_call_start.is_some(), "Should find opening paren");
    assert_eq!(comma_count, 1, "Should have 1 comma before cursor");
}

#[test]
fn find_function_call_context_nested() {
    let text = "func main() {\n    outer(inner(1), 2)\n}";
    let cursor_pos = text.find("outer(inner(1), 2").unwrap() + 16; // Position after ", 2"

    let before_cursor = &text[..cursor_pos];
    let mut paren_depth = 0;
    let mut func_call_start = None;
    let mut comma_count = 0;

    for (i, c) in before_cursor.char_indices().rev() {
        match c {
            ')' => paren_depth += 1,
            '(' => {
                if paren_depth == 0 {
                    func_call_start = Some(i);
                    break;
                }
                paren_depth -= 1;
            }
            ',' if paren_depth == 0 => comma_count += 1,
            _ => {}
        }
    }

    assert!(func_call_start.is_some(), "Should find opening paren");
    assert_eq!(
        comma_count, 1,
        "Should have 1 comma at outer level before cursor"
    );
}

#[test]
fn extract_function_name_simple() {
    let text = "add(1, 2)";
    let paren_pos = text.find('(').unwrap();

    let before_paren = &text[..paren_pos];
    let func_name_end = before_paren.trim_end().len();
    let func_name_start = before_paren[..func_name_end]
        .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
        .map(|i| i + 1)
        .unwrap_or(0);
    let func_name = before_paren[func_name_start..func_name_end].trim();

    assert_eq!(func_name, "add");
}

#[test]
fn extract_function_name_qualified() {
    let text = "pkg.Func(1, 2)";
    let paren_pos = text.find('(').unwrap();

    let before_paren = &text[..paren_pos];
    let func_name_end = before_paren.trim_end().len();
    let func_name_start = before_paren[..func_name_end]
        .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
        .map(|i| i + 1)
        .unwrap_or(0);
    let func_name = before_paren[func_name_start..func_name_end].trim();

    assert_eq!(func_name, "pkg.Func");

    // The search should extract just the function name for lookup
    let search_name = func_name.split('.').next_back().unwrap_or(func_name);
    assert_eq!(search_name, "Func");
}

#[test]
fn extract_function_name_with_spaces() {
    let text = "    myFunc   (1, 2)";
    let paren_pos = text.find('(').unwrap();

    let before_paren = &text[..paren_pos];
    let func_name_end = before_paren.trim_end().len();
    let func_name_start = before_paren[..func_name_end]
        .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
        .map(|i| i + 1)
        .unwrap_or(0);
    let func_name = before_paren[func_name_start..func_name_end].trim();

    assert_eq!(func_name, "myFunc");
}
