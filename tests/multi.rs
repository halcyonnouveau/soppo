mod common;

use std::path::Path;

use common::{format_results, parse_test_toml, run_multi_test, sanitise_error};
use test_generator::test_resources;

#[test_resources("tests/fixtures/multi/pass/*/")]
fn test_pass(resource: &str) {
    let fixture_path = Path::new(resource);
    let results = run_multi_test(fixture_path)
        .unwrap_or_else(|e| panic!("Multi-file test '{}' should succeed:\n{}", resource, e));

    let dir_name = fixture_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let name = format!("pass__{}", dir_name);

    insta::assert_snapshot!(name, format_results(&results));
}

#[test_resources("tests/fixtures/multi/fail/*/")]
fn test_fail(resource: &str) {
    let fixture_path = Path::new(resource);
    let config = parse_test_toml(&fixture_path.join("test.toml"));

    let result = run_multi_test(fixture_path);
    assert!(result.is_err(), "Test '{}' should fail", resource);

    let err = result.unwrap_err();
    if let Some(expected) = &config.expected_error {
        assert!(
            err.contains(expected),
            "Error should contain '{}'\nGot: {}",
            expected,
            err
        );
    }

    let dir_name = fixture_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let name = format!("fail__{}", dir_name);

    insta::assert_snapshot!(name, sanitise_error(&err));
}
