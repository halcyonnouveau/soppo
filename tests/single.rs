mod common;

use std::path::Path;

use common::compile_soppo_file;
use test_generator::test_resources;

#[test_resources("tests/fixtures/single/pass/*.sop")]
fn test_pass(resource: &str) {
    let output = compile_soppo_file(resource)
        .unwrap_or_else(|e| panic!("Pass test '{}' should succeed:\n{}", resource, e));

    let file_name = Path::new(resource)
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let name = format!("pass__{}", file_name);

    insta::assert_snapshot!(name, output);
}

#[test_resources("tests/fixtures/single/fail/*.sop")]
fn test_fail(resource: &str) {
    let result = compile_soppo_file(resource);
    assert!(result.is_err(), "Fail test '{}' should fail", resource);

    let file_name = Path::new(resource)
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let name = format!("fail__{}", file_name);

    insta::assert_snapshot!(name, result.unwrap_err());
}
