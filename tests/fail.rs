mod common;

use common::compile_soppo_file;

#[test]
fn test_type_mismatch() {
    let result = compile_soppo_file("tests/fixtures/fail/type_mismatch.sop");
    assert!(result.is_err(), "Expected type error");
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_undeclared_variable() {
    let result = compile_soppo_file("tests/fixtures/fail/undeclared_variable.sop");
    assert!(result.is_err(), "Expected undeclared variable error");
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_assign_wrong_type() {
    let result = compile_soppo_file("tests/fixtures/fail/assign_wrong_type.sop");
    assert!(result.is_err(), "Expected type mismatch on assignment");
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_non_exhaustive() {
    let result = compile_soppo_file("tests/fixtures/fail/non_exhaustive.sop");
    assert!(result.is_err(), "Expected non-exhaustive match error");
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_var_no_type_or_value() {
    let result = compile_soppo_file("tests/fixtures/fail/var_no_type_or_value.sop");
    assert!(
        result.is_err(),
        "Expected parse error for var without type or value"
    );
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_const_no_value() {
    let result = compile_soppo_file("tests/fixtures/fail/const_no_value.sop");
    assert!(
        result.is_err(),
        "Expected parse error for const without value"
    );
    insta::assert_snapshot!(result.unwrap_err());
}
