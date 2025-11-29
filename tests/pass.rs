mod common;

use common::compile_soppo_file;

#[test]
fn test_basic_go() {
    let output = compile_soppo_file("tests/fixtures/single/pass/basic_go.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_simple_add() {
    let output = compile_soppo_file("tests/fixtures/single/pass/simple_add.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_enum_match() {
    let output = compile_soppo_file("tests/fixtures/single/pass/enum_match.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_generics() {
    let output = compile_soppo_file("tests/fixtures/single/pass/generics.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_error_type() {
    let output = compile_soppo_file("tests/fixtures/single/pass/error_type.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_nil_safety() {
    let output = compile_soppo_file("tests/fixtures/single/pass/nil_safety.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_try_operator() {
    let output = compile_soppo_file("tests/fixtures/single/pass/try_operator.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_if_init() {
    let output = compile_soppo_file("tests/fixtures/single/pass/if_init.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_struct_match() {
    let output = compile_soppo_file("tests/fixtures/single/pass/struct_match.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_named_args() {
    let output = compile_soppo_file("tests/fixtures/single/pass/named_args.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_builtins() {
    let output = compile_soppo_file("tests/fixtures/single/pass/builtins.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_string_interpolation() {
    let output = compile_soppo_file("tests/fixtures/single/pass/string_interpolation.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_nullable_types() {
    let output = compile_soppo_file("tests/fixtures/single/pass/nullable_types.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_const_grouped_block() {
    let output = compile_soppo_file("tests/fixtures/single/pass/const_grouped_block.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_type_alias() {
    let output = compile_soppo_file("tests/fixtures/single/pass/type_alias.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}
