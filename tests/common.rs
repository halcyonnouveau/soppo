#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;
use tempfile::TempDir;

/// Configuration for a multi-file test, parsed from test.toml
#[derive(Deserialize)]
pub struct TestConfig {
    /// Go module name for go.mod
    pub module: String,
    /// Optional expected error substring for fail tests
    pub expected_error: Option<String>,
    /// Optional output directory for generated Go files
    pub output: Option<String>,
}

pub fn compile_soppo_file(input_file: &str) -> Result<String, String> {
    let output_file = input_file.replace(".sop", ".go");

    let output = Command::new("cargo")
        .env("RUSTFLAGS", "-A warnings")
        .args(["run", "--quiet", "--bin", "sop", "--", "build", input_file])
        .output()
        .expect("Failed to run sop");

    if output.status.success() {
        let go_code = fs::read_to_string(&output_file).expect("Failed to read generated .go file");

        let vet_output = Command::new("go")
            .args(["vet", &output_file])
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

/// Parse test.toml configuration from a fixture directory
pub fn parse_test_toml(path: &Path) -> TestConfig {
    let content = fs::read_to_string(path).expect("Failed to read test.toml");
    toml::from_str(&content).expect("Failed to parse test.toml")
}

/// Recursively find all .sop files in a directory
pub fn find_sop_files(dir: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    find_sop_files_recursive(dir, dir, &mut files);
    files
}

fn find_sop_files_recursive(base: &Path, dir: &Path, files: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("Failed to read directory") {
        let entry = entry.expect("Failed to read entry");
        let path = entry.path();

        if path.is_dir() {
            find_sop_files_recursive(base, &path, files);
        } else if path.extension().is_some_and(|e| e == "sop") {
            let relative = path.strip_prefix(base).expect("Failed to strip prefix");
            let content = fs::read_to_string(&path).expect("Failed to read .sop file");
            files.push((relative.to_string_lossy().to_string(), content));
        }
    }
}

/// Recursively find all .go files in a directory (for testing import of generated Go)
pub fn find_go_files(dir: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    find_go_files_recursive(dir, dir, &mut files);
    files
}

fn find_go_files_recursive(base: &Path, dir: &Path, files: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("Failed to read directory") {
        let entry = entry.expect("Failed to read entry");
        let path = entry.path();

        if path.is_dir() {
            find_go_files_recursive(base, &path, files);
        } else if path.extension().is_some_and(|e| e == "go") {
            let relative = path.strip_prefix(base).expect("Failed to strip prefix");
            let content = fs::read_to_string(&path).expect("Failed to read .go file");
            files.push((relative.to_string_lossy().to_string(), content));
        }
    }
}

/// Build a multi-file test project and return generated Go code
pub fn run_multi_test(fixture_dir: &Path) -> Result<Vec<(String, String)>, String> {
    let config = parse_test_toml(&fixture_dir.join("test.toml"));
    let sop_files = find_sop_files(fixture_dir);
    let go_files = find_go_files(fixture_dir);

    build_test_project(
        &config.module,
        &sop_files,
        &go_files,
        config.output.as_deref(),
    )
}

/// Create a test project with go.mod and compile it using the library directly
fn build_test_project(
    module_name: &str,
    sop_files: &[(String, String)],
    go_files: &[(String, String)],
    output_dir: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let temp = TempDir::new().expect("Failed to create temp dir");
    let root = temp.path();

    // Create go.mod
    fs::write(
        root.join("go.mod"),
        format!("module {}\n\ngo 1.25\n", module_name),
    )
    .expect("Failed to write go.mod");

    // Create .sop source files
    for (path, content) in sop_files {
        let file_path = root.join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create directories");
        }
        fs::write(&file_path, content).expect("Failed to write source file");
    }

    // Create .go source files (for testing import of generated Go packages)
    for (path, content) in go_files {
        let file_path = root.join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create directories");
        }
        fs::write(&file_path, content).expect("Failed to write go file");
    }

    // Build using the library directly
    let output_path = output_dir.map(|d| root.join(d));
    let results = soppo::build::build_project(root, output_path.as_deref()).map_err(|e| {
        // Render the full miette diagnostic for better error messages
        format!("{:?}", e)
    })?;

    // Write generated files to disk for go build validation
    // When output_dir is set, files go there; otherwise next to source files
    let root_buf = root.to_path_buf();
    let base_dir = output_path.as_ref().unwrap_or(&root_buf);
    for (relative_path, go_code) in &results {
        let file_path = base_dir.join(relative_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create output directories");
        }
        fs::write(&file_path, go_code).expect("Failed to write generated file");
    }

    // Run go build to verify the generated code compiles
    // Use -o /dev/null to avoid creating binaries (which can conflict with dir names)
    let go_output = Command::new("go")
        .args(["build", "-o", "/dev/null", "./..."])
        .current_dir(root)
        .output()
        .expect("Failed to run go build");

    if !go_output.status.success() {
        let go_error = String::from_utf8_lossy(&go_output.stderr);
        return Err(format!(
            "Generated Go code failed to compile:\n{}\n\nGenerated files:\n{}",
            go_error,
            results
                .iter()
                .map(|(p, c)| format!("=== {} ===\n{}", p, c))
                .collect::<Vec<_>>()
                .join("\n\n")
        ));
    }

    Ok(results)
}

/// Format multi-file results for snapshot comparison
pub fn format_results(results: &[(String, String)]) -> String {
    let mut sorted = results.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted
        .iter()
        .map(|(path, content)| format!("=== {} ===\n{}", path, content))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Sanitise error messages by replacing temp paths and stripping ANSI codes
pub fn sanitise_error(err: &str) -> String {
    // Strip ANSI escape codes
    let ansi_re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    let stripped = ansi_re.replace_all(err, "");

    // Replace temp paths like /tmp/.tmpXXXXXX/ with [TEMP]/
    let path_re = regex::Regex::new(r"/tmp/\.tmp[A-Za-z0-9]+/").unwrap();
    path_re.replace_all(&stripped, "[TEMP]/").to_string()
}
