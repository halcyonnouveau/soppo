mod common;

use std::path::Path;

use common::{format_results, parse_test_toml, run_multi_test, sanitise_error};

mod pass {
    use super::*;

    macro_rules! multi_pass_tests {
        ($($name:ident),* $(,)?) => {
            $(
                #[test]
                fn $name() {
                    let fixture_dir = format!("tests/fixtures/multi/pass/{}", stringify!($name));
                    let results = run_multi_test(Path::new(&fixture_dir))
                        .expect(&format!("Multi-file test '{}' should succeed", stringify!($name)));
                    insta::assert_snapshot!(stringify!($name), format_results(&results));
                }
            )*
        };
    }

    multi_pass_tests!(
        simple_import,
        aliased_import,
        chain_dependencies,
        multiple_returns,
        enum_cross_package,
        nilable_cross_package,
        import_generated_enum,
        import_generated_nilable,
    );
}

mod fail {
    use super::*;

    macro_rules! multi_fail_tests {
        ($($name:ident),* $(,)?) => {
            $(
                #[test]
                fn $name() {
                    let fixture_dir = format!("tests/fixtures/multi/fail/{}", stringify!($name));
                    let fixture_path = Path::new(&fixture_dir);
                    let config = parse_test_toml(&fixture_path.join("test.toml"));

                    let result = run_multi_test(fixture_path);
                    assert!(result.is_err(), "Test '{}' should fail", stringify!($name));

                    let err = result.unwrap_err();
                    if let Some(expected) = &config.expected_error {
                        assert!(
                            err.contains(expected),
                            "Error should contain '{}'\nGot: {}",
                            expected,
                            err
                        );
                    }
                    insta::assert_snapshot!(stringify!($name), sanitise_error(&err));
                }
            )*
        };
    }

    multi_fail_tests!(circular_dependency, missing_import, deref_nilable_field,);
}
