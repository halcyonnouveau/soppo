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

#[test]
fn test_go_unknown_function() {
    let result = compile_soppo_file("tests/fixtures/fail/go_unknown_function.sop");
    assert!(result.is_err(), "Expected error for unknown Go function");
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_go_wrong_arg_count() {
    let result = compile_soppo_file("tests/fixtures/fail/go_wrong_arg_count.sop");
    assert!(result.is_err(), "Expected error for wrong argument count");
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_go_wrong_arg_type() {
    let result = compile_soppo_file("tests/fixtures/fail/go_wrong_arg_type.sop");
    assert!(result.is_err(), "Expected error for wrong argument type");
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_duration_string_mul() {
    let result = compile_soppo_file("tests/fixtures/fail/duration_string_mul.sop");
    assert!(
        result.is_err(),
        "Expected error for Duration * string (incompatible types)"
    );
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_nil_deref_no_check() {
    let result = compile_soppo_file("tests/fixtures/fail/nil_deref_no_check.sop");
    assert!(
        result.is_err(),
        "Expected error for nil pointer dereference without check"
    );
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_nil_access_wrong_branch() {
    let result = compile_soppo_file("tests/fixtures/fail/nil_access_wrong_branch.sop");
    assert!(
        result.is_err(),
        "Expected error for accessing nil pointer in wrong branch"
    );
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_nil_nested_no_check() {
    let result = compile_soppo_file("tests/fixtures/fail/nil_nested_no_check.sop");
    assert!(
        result.is_err(),
        "Expected error for accessing nested pointer without check"
    );
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_nil_reassign_resets() {
    let result = compile_soppo_file("tests/fixtures/fail/nil_reassign_resets.sop");
    assert!(
        result.is_err(),
        "Expected error after reassignment resets nil state"
    );
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_try_no_return_error() {
    let result = compile_soppo_file("tests/fixtures/fail/try_no_return_error.sop");
    assert!(
        result.is_err(),
        "Expected error for ? in function not returning error"
    );
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_try_expr_no_error() {
    let result = compile_soppo_file("tests/fixtures/fail/try_expr_no_error.sop");
    assert!(
        result.is_err(),
        "Expected error for ? on expression not returning error"
    );
    insta::assert_snapshot!(result.unwrap_err());
}
