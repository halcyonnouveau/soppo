use std::path::PathBuf;

use soppo::test::{TestConfig, run_tests};

#[test]
fn test_basic_fixture() {
    let fixture_dir = PathBuf::from("tests/fixtures/test/basic");

    let config = TestConfig {
        root: fixture_dir,
        packages: vec![],
        run_pattern: None,
        verbose: false,
        keep_temp: false,
        passthrough_args: vec![],
    };

    let result = run_tests(&config).expect("sop test should succeed");
    assert!(result.success, "All tests should pass");
    assert_eq!(result.exit_code, 0);
}
