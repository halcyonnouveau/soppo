mod common;

use common::compile_soppo_file;

#[test]
fn test_basic_go() {
    let output =
        compile_soppo_file("tests/fixtures/pass/basic_go.sop").expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_simple_add() {
    let output = compile_soppo_file("tests/fixtures/pass/simple_add.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_enum_match() {
    let output = compile_soppo_file("tests/fixtures/pass/enum_match.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_generics() {
    let output =
        compile_soppo_file("tests/fixtures/pass/generics.sop").expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_error_type() {
    let output = compile_soppo_file("tests/fixtures/pass/error_type.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_nil_safety() {
    let output = compile_soppo_file("tests/fixtures/pass/nil_safety.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_try_operator() {
    let output = compile_soppo_file("tests/fixtures/pass/try_operator.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}
