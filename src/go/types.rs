//! Convert Go type strings to Soppo types.
//!
//! This module parses Go type syntax (as extracted by tree-sitter) and converts
//! it to Soppo's internal Type representation.

use crate::parse::{ModuleId, Span, Symbol};
use crate::types::Type;

/// Parse a Go type string into a Soppo Type.
///
/// Examples:
/// - `"int"` → `Type::simple("int")`
/// - `"[]string"` → `Type::generic("slice", [Type::simple("string")])`
/// - `"map[string]int"` → `Type::generic("map", [Type::simple("string"), Type::simple("int")])`
/// - `"*Point"` → `Type::generic("ptr", [Type::simple("Point")])`
/// - `"func(int) string"` → `Type::Fun { args: [int], ret: string }`
pub fn parse_go_type(s: &str) -> Type {
    let s = s.trim();

    if s.is_empty() {
        return Type::unit();
    }

    // Pointer type: *T
    if let Some(inner) = s.strip_prefix('*') {
        return Type::generic("ptr", vec![parse_go_type(inner)]);
    }

    // Slice type: []T
    if let Some(inner) = s.strip_prefix("[]") {
        return Type::generic("slice", vec![parse_go_type(inner)]);
    }

    // Variadic type: ...T (use special variadic type marker)
    if let Some(inner) = s.strip_prefix("...") {
        return Type::generic("variadic", vec![parse_go_type(inner)]);
    }

    // Array type: [N]T or [...]T
    if s.starts_with('[')
        && let Some(bracket_end) = s.find(']')
    {
        let inner = &s[bracket_end + 1..];
        // We treat arrays as slices for simplicity
        return Type::generic("array", vec![parse_go_type(inner)]);
    }

    // Map type: map[K]V
    if let Some(rest) = s.strip_prefix("map[")
        && let Some((key, value)) = parse_map_types(rest)
    {
        return Type::generic("map", vec![parse_go_type(key), parse_go_type(value)]);
    }

    // Channel type: chan T
    if let Some(inner) = s.strip_prefix("chan ") {
        return Type::generic("chan", vec![parse_go_type(inner)]);
    }

    // Function type: func(A, B) C or func(A, B) (C, D)
    if let Some(rest) = s.strip_prefix("func") {
        return parse_func_type(rest);
    }

    // Interface type
    if s == "interface{}" || s == "any" {
        return Type::simple("any");
    }

    // Struct type (anonymous)
    if s == "struct{}" {
        return Type::simple("struct");
    }

    // Tuple type: (A, B, C) - multiple returns
    if s.starts_with('(') && s.ends_with(')') {
        let inner = &s[1..s.len() - 1];
        let types = split_type_list(inner);
        if types.len() == 1 {
            return parse_go_type(types[0]);
        }
        // Multiple return types - create a tuple
        let type_args: Vec<Type> = types.iter().map(|t| parse_go_type(t)).collect();
        return Type::generic("tuple", type_args);
    }

    // Generic type: T[A, B]
    if let Some(bracket_pos) = s.find('[')
        && s.ends_with(']')
    {
        let name = &s[..bracket_pos];
        let args_str = &s[bracket_pos + 1..s.len() - 1];
        let args = split_type_list(args_str);
        let type_args: Vec<Type> = args.iter().map(|t| parse_go_type(t)).collect();
        return make_type_with_module(name, type_args);
    }

    // Qualified type: pkg.Type
    if let Some(dot_pos) = s.find('.') {
        let pkg = &s[..dot_pos];
        let name = &s[dot_pos + 1..];
        return Type::Con {
            name: Symbol {
                module: ModuleId::new(pkg),
                name: name.to_string(),
                span: Span::dummy(),
            },
            args: vec![],
        };
    }

    // Primitive types - pass through as Type::simple
    // Aliases map to their underlying types
    match s {
        "byte" => Type::simple("uint8"),
        "rune" => Type::simple("int32"),
        _ => Type::simple(s),
    }
}

/// Parse map[K]V, returning (K, V)
fn parse_map_types(s: &str) -> Option<(&str, &str)> {
    // Need to find matching ] accounting for nested brackets
    let mut depth = 1;
    let mut key_end = 0;

    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    key_end = i;
                    break;
                }
            }
            _ => {}
        }
    }

    if key_end == 0 {
        return None;
    }

    let key = &s[..key_end];
    let value = &s[key_end + 1..];
    Some((key, value))
}

/// Parse func(A, B) C or func(A, B) (C, D)
fn parse_func_type(s: &str) -> Type {
    let s = s.trim();

    // Find the parameter list
    if !s.starts_with('(') {
        // Just "func" with no details
        return Type::fun(vec![], Type::unit());
    }

    // Find matching closing paren for params
    let params_end = find_matching_paren(s, 0);
    if params_end.is_none() {
        return Type::fun(vec![], Type::unit());
    }
    let params_end = params_end.unwrap();

    let params_str = &s[1..params_end];
    let params = parse_param_list(params_str);

    let rest = s[params_end + 1..].trim();

    let ret = if rest.is_empty() {
        Type::unit()
    } else {
        parse_go_type(rest)
    };

    Type::fun(params, ret)
}

/// Parse a parameter list like "a int, b string" or "int, string"
fn parse_param_list(s: &str) -> Vec<Type> {
    if s.trim().is_empty() {
        return vec![];
    }

    let parts = split_type_list(s);
    let mut types = Vec::new();

    for part in parts {
        let part = part.trim();
        // Parameters can be "name type" or just "type"
        // Take the last space-separated token as the type
        let tokens: Vec<&str> = part.split_whitespace().collect();
        if let Some(ty_str) = tokens.last() {
            types.push(parse_go_type(ty_str));
        }
    }

    types
}

/// Split a comma-separated type list, respecting brackets
fn split_type_list(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0;
    let mut start = 0;

    for (i, c) in s.char_indices() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                result.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }

    let last = s[start..].trim();
    if !last.is_empty() {
        result.push(last);
    }

    result
}

/// Find the index of the closing paren matching the one at `start`
fn find_matching_paren(s: &str, start: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s[start..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Create a type, handling qualified names (pkg.Type)
fn make_type_with_module(name: &str, args: Vec<Type>) -> Type {
    if let Some(dot_pos) = name.find('.') {
        let pkg = &name[..dot_pos];
        let type_name = &name[dot_pos + 1..];
        Type::Con {
            name: Symbol {
                module: ModuleId::new(pkg),
                name: type_name.to_string(),
                span: Span::dummy(),
            },
            args,
        }
    } else {
        Type::Con {
            name: Symbol {
                module: ModuleId::empty(),
                name: name.to_string(),
                span: Span::dummy(),
            },
            args,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitives() {
        assert_eq!(parse_go_type("int").to_string(), "int");
        assert_eq!(parse_go_type("string").to_string(), "string");
        assert_eq!(parse_go_type("bool").to_string(), "bool");
        assert_eq!(parse_go_type("float64").to_string(), "float64");
        assert_eq!(parse_go_type("byte").to_string(), "uint8");
        assert_eq!(parse_go_type("uint").to_string(), "uint");
        assert_eq!(parse_go_type("uint32").to_string(), "uint32");
        assert_eq!(parse_go_type("uintptr").to_string(), "uintptr");
        assert_eq!(parse_go_type("error").to_string(), "error");
    }

    #[test]
    fn test_pointer() {
        assert_eq!(parse_go_type("*int").to_string(), "ptr[int]");
        assert_eq!(parse_go_type("*string").to_string(), "ptr[string]");
        assert_eq!(parse_go_type("**int").to_string(), "ptr[ptr[int]]");
    }

    #[test]
    fn test_slice() {
        assert_eq!(parse_go_type("[]int").to_string(), "slice[int]");
        assert_eq!(parse_go_type("[]string").to_string(), "slice[string]");
        assert_eq!(parse_go_type("[][]int").to_string(), "slice[slice[int]]");
    }

    #[test]
    fn test_array() {
        assert_eq!(parse_go_type("[10]int").to_string(), "array[int]");
        assert_eq!(parse_go_type("[...]string").to_string(), "array[string]");
    }

    #[test]
    fn test_map() {
        assert_eq!(
            parse_go_type("map[string]int").to_string(),
            "map[string, int]"
        );
        assert_eq!(
            parse_go_type("map[string][]int").to_string(),
            "map[string, slice[int]]"
        );
    }

    #[test]
    fn test_channel() {
        assert_eq!(parse_go_type("chan int").to_string(), "chan[int]");
    }

    #[test]
    fn test_interface() {
        assert_eq!(parse_go_type("interface{}").to_string(), "any");
        assert_eq!(parse_go_type("any").to_string(), "any");
    }

    #[test]
    fn test_function() {
        assert_eq!(parse_go_type("func()").to_string(), "fn() -> ()");
        assert_eq!(parse_go_type("func(int)").to_string(), "fn(int) -> ()");
        assert_eq!(
            parse_go_type("func(int) string").to_string(),
            "fn(int) -> string"
        );
        assert_eq!(
            parse_go_type("func(a int, b string) bool").to_string(),
            "fn(int, string) -> bool"
        );
    }

    #[test]
    fn test_qualified() {
        let ty = parse_go_type("fmt.Stringer");
        match ty {
            Type::Con { name, args } => {
                assert_eq!(name.module.0, "fmt");
                assert_eq!(name.name, "Stringer");
                assert!(args.is_empty());
            }
            _ => panic!("Expected Con type"),
        }
    }

    #[test]
    fn test_generic() {
        assert_eq!(
            parse_go_type("Result[int, string]").to_string(),
            "Result[int, string]"
        );
        assert_eq!(
            parse_go_type("Option[*int]").to_string(),
            "Option[ptr[int]]"
        );
    }

    #[test]
    fn test_variadic() {
        // Variadic uses special marker type
        assert_eq!(parse_go_type("...int").to_string(), "variadic[int]");
        assert_eq!(parse_go_type("...any").to_string(), "variadic[any]");
    }

    #[test]
    fn test_multiple_returns() {
        assert_eq!(
            parse_go_type("(int, error)").to_string(),
            "tuple[int, error]"
        );
        assert_eq!(
            parse_go_type("(string, int, bool)").to_string(),
            "tuple[string, int, bool]"
        );
    }
}
