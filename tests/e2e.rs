use std::fs;
use std::process::Command;

fn compile_soppo_file(input_file: &str) -> Result<String, String> {
    let output_file = input_file.replace(".sop", ".go");

    let output = Command::new("cargo")
        .env("RUSTFLAGS", "-A warnings")
        .args(&["run", "--quiet", "--bin", "soppo", "--", input_file])
        .output()
        .expect("Failed to run soppo");

    if output.status.success() {
        let go_code = fs::read_to_string(&output_file).expect("Failed to read generated .go file");

        let vet_output = Command::new("go")
            .args(&["vet", &output_file])
            .output()
            .expect("Failed to run go vet");

        if !vet_output.status.success() {
            let vet_error = String::from_utf8_lossy(&vet_output.stderr);
            fs::remove_file(&output_file).ok();
            panic!(
                "Generated Go code failed go vet:\n{}\n\nGenerated code:\n{}",
                vet_error, go_code
            );
        }

        fs::remove_file(&output_file).ok();
        Ok(go_code)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

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
fn test_error_type_mismatch() {
    let result = compile_soppo_file("tests/fixtures/fail/error_type_mismatch.sop");
    assert!(result.is_err(), "Expected type error");
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_error_undeclared_variable() {
    let result = compile_soppo_file("tests/fixtures/fail/error_undeclared_variable.sop");
    assert!(result.is_err(), "Expected undeclared variable error");
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_error_assign_wrong_type() {
    let result = compile_soppo_file("tests/fixtures/fail/error_assign_wrong_type.sop");
    assert!(result.is_err(), "Expected type mismatch on assignment");
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_enum_match() {
    let output = compile_soppo_file("tests/fixtures/pass/enum_match.sop")
        .expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}

#[test]
fn test_error_non_exhaustive() {
    let result = compile_soppo_file("tests/fixtures/fail/error_non_exhaustive.sop");
    assert!(result.is_err(), "Expected non-exhaustive match error");
    insta::assert_snapshot!(result.unwrap_err());
}

#[test]
fn test_generics() {
    let output =
        compile_soppo_file("tests/fixtures/pass/generics.sop").expect("Compilation should succeed");
    insta::assert_snapshot!(output);
}
