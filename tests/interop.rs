//! Integration tests for Go interop.
//!
//! These tests compile .sop files to .go and verify the generated Go code compiles.
//! Each test directory contains a go.mod and .sop files.

use std::path::Path;
use std::process::Command;

use test_generator::test_resources;

/// Run an interop test: compile .sop to .go, then build with Go.
fn run_interop_test(fixture_path: &Path) -> Result<(), String> {
    // Build the entire project (outputs .go files next to .sop files)
    let results = soppo::build::build_project(fixture_path, None)
        .map_err(|e| format!("sop build failed:\n{:?}", e))?;

    // Write all generated Go files
    for (relative_path, go_code) in &results {
        let go_file = fixture_path.join(relative_path);
        if let Some(parent) = go_file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {}", e))?;
        }
        std::fs::write(&go_file, go_code).map_err(|e| format!("Failed to write Go file: {}", e))?;
    }

    // Build with Go to verify the generated code compiles
    let output = Command::new("go")
        .arg("build")
        .arg("-o")
        .arg("/dev/null")
        .arg(".")
        .current_dir(fixture_path)
        .output()
        .map_err(|e| format!("Failed to run go build: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("go build failed:\n{}", stderr));
    }

    // Clean up generated .go files
    for (relative_path, _) in &results {
        let go_file = fixture_path.join(relative_path);
        let _ = std::fs::remove_file(go_file);
    }

    Ok(())
}

#[test_resources("tests/fixtures/interop/*/")]
fn test_interop(resource: &str) {
    let fixture_path = Path::new(resource);
    if let Err(e) = run_interop_test(fixture_path) {
        panic!("Interop test failed for {}:\n{}", resource, e);
    }
}
