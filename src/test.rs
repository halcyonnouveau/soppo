use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use miette::{IntoDiagnostic, Result, miette};
use tempfile::TempDir;

use crate::build;
use crate::doctest;
use crate::go::Project;

/// Configuration for running tests.
#[derive(Debug)]
pub struct TestConfig {
    /// Root directory of the project
    pub root: PathBuf,
    /// Specific packages to test (e.g., "./pkg/...")
    pub packages: Vec<String>,
    /// Pattern to filter tests (passed to `go test -run`)
    pub run_pattern: Option<String>,
    /// Whether to show verbose output
    pub verbose: bool,
    /// Keep temp directory (for debugging)
    pub keep_temp: bool,
    /// Additional arguments to pass to `go test`
    pub passthrough_args: Vec<String>,
}

/// Result of running tests.
#[derive(Debug)]
pub struct TestResult {
    /// Exit code from `go test`
    pub exit_code: i32,
    /// Whether all tests passed
    pub success: bool,
}

/// Run tests for a Soppo project.
pub fn run_tests(config: &TestConfig) -> Result<TestResult> {
    // Discover the project
    let project = Project::discover(&config.root)?;

    if config.verbose {
        println!("Running tests for {}", project.module_path);
    }

    // Create temp directory
    let temp_dir = TempDir::new()
        .into_diagnostic()
        .map_err(|e| e.context("Failed to create temp directory"))?;

    let temp_root = temp_dir.path();

    if config.verbose {
        println!("  Using temp directory: {}", temp_root.display());
    }

    // Copy go.mod and go.sum
    copy_go_files(&config.root, temp_root, &project.module_path)?;

    // Find and transpile all .sop files (including test files)
    let sources = find_all_sources(&project, true)?;

    if sources.is_empty() {
        println!("No .sop files found");
        return Ok(TestResult {
            exit_code: 0,
            success: true,
        });
    }

    if config.verbose {
        println!("  Found {} source file(s)", sources.len());
    }

    // Transpile to temp directory
    let transpiled = transpile_to_temp(&config.root, temp_root)?;

    if config.verbose {
        println!("  Transpiled {} file(s)", transpiled.len());
    }

    // Extract and generate doctests
    let doctest_files = generate_doctests(&config.root, temp_root, &sources, &project.module_path)?;

    if config.verbose && !doctest_files.is_empty() {
        println!("  Generated {} doctest file(s)", doctest_files.len());
    }

    // Run go test
    let result = run_go_test(temp_root, config)?;

    // Handle temp directory cleanup
    if config.keep_temp {
        // Persist the temp directory for debugging
        let persisted = temp_dir.keep();
        println!("  Temp directory preserved at: {}", persisted.display());
    }

    Ok(result)
}

/// Copy go.mod and go.sum to temp directory, adding replace directive.
fn copy_go_files(src_root: &Path, dest_root: &Path, module_path: &str) -> Result<()> {
    let go_mod_src = src_root.join("go.mod");
    let go_mod_dest = dest_root.join("go.mod");

    if go_mod_src.exists() {
        let content = fs::read_to_string(&go_mod_src)
            .into_diagnostic()
            .map_err(|e| e.context("Failed to read go.mod"))?;

        // Add replace directive to point to temp directory
        let modified = format!(
            "{}\nreplace {} => {}\n",
            content.trim(),
            module_path,
            dest_root.display()
        );

        fs::write(&go_mod_dest, modified)
            .into_diagnostic()
            .map_err(|e| e.context("Failed to write go.mod"))?;
    } else {
        return Err(miette!("No go.mod found in project root"));
    }

    // Copy go.sum if it exists
    let go_sum_src = src_root.join("go.sum");
    let go_sum_dest = dest_root.join("go.sum");

    if go_sum_src.exists() {
        fs::copy(&go_sum_src, &go_sum_dest)
            .into_diagnostic()
            .map_err(|e| e.context("Failed to copy go.sum"))?;
    }

    Ok(())
}

/// Find all .sop source files, optionally including test files.
fn find_all_sources(project: &Project, include_tests: bool) -> Result<Vec<PathBuf>> {
    let mut sources = project.find_sources();

    if include_tests {
        // Also find *_test.sop files
        let test_files = find_test_files(&project.root)?;
        sources.extend(test_files);
    }

    // Remove duplicates and sort
    sources.sort();
    sources.dedup();

    Ok(sources)
}

/// Find all *_test.sop files in the project.
fn find_test_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut test_files = Vec::new();
    find_test_files_recursive(root, &mut test_files)?;
    Ok(test_files)
}

fn find_test_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir)
        .into_diagnostic()
        .map_err(|e| e.context(format!("Failed to read directory: {}", dir.display())))?;

    for entry in entries {
        let entry = entry.into_diagnostic()?;
        let path = entry.path();

        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip hidden dirs, vendor, etc.
            if !name.starts_with('.') && name != "vendor" && name != "testdata" {
                find_test_files_recursive(&path, files)?;
            }
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && name.ends_with("_test.sop")
        {
            files.push(path);
        }
    }

    Ok(())
}

/// Transpile .sop files to .go files in the temp directory.
fn transpile_to_temp(src_root: &Path, dest_root: &Path) -> Result<Vec<PathBuf>> {
    // Build the project to the temp directory
    let results = build::build_project(src_root, Some(dest_root))?;

    // Write the generated files
    let mut output_paths = Vec::new();
    for (relative_path, go_code) in results {
        let output_path = dest_root.join(&relative_path);

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).into_diagnostic().map_err(|e| {
                e.context(format!("Failed to create directory: {}", parent.display()))
            })?;
        }

        fs::write(&output_path, go_code)
            .into_diagnostic()
            .map_err(|e| e.context(format!("Failed to write: {}", output_path.display())))?;

        output_paths.push(output_path);
    }

    Ok(output_paths)
}

/// Extract doctests from source files and generate Go Example files.
fn generate_doctests(
    src_root: &Path,
    dest_root: &Path,
    sources: &[PathBuf],
    module_path: &str,
) -> Result<Vec<PathBuf>> {
    let mut generated = Vec::new();

    for source_path in sources {
        // Skip test files - they don't have doctests
        if source_path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("_test.sop"))
        {
            continue;
        }

        // Read and parse the source file
        let source = fs::read_to_string(source_path)
            .into_diagnostic()
            .map_err(|e| e.context(format!("Failed to read: {}", source_path.display())))?;

        let filename = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("input.sop");

        // Extract doctests
        let file_doctests = doctest::extract_from_source(&source, filename)?;

        if file_doctests.doctests.is_empty() {
            continue;
        }

        // Compute relative path for output
        let relative = source_path.strip_prefix(src_root).unwrap_or(source_path);

        // Generate the doctest file
        let doctest_code = doctest::generate_example_file(&file_doctests, module_path, relative)?;

        // Write to temp directory
        let output_name = relative
            .with_extension("")
            .to_string_lossy()
            .replace(['/', '\\'], "_")
            + "_doctest_test.go";

        // Put in the same package directory
        let package_dir = relative.parent().unwrap_or(Path::new(""));
        let output_path = dest_root.join(package_dir).join(&output_name);

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).into_diagnostic().map_err(|e| {
                e.context(format!("Failed to create directory: {}", parent.display()))
            })?;
        }

        fs::write(&output_path, doctest_code)
            .into_diagnostic()
            .map_err(|e| {
                e.context(format!(
                    "Failed to write doctest: {}",
                    output_path.display()
                ))
            })?;

        generated.push(output_path);
    }

    Ok(generated)
}

/// Run `go test` in the temp directory.
fn run_go_test(temp_root: &Path, config: &TestConfig) -> Result<TestResult> {
    let mut cmd = Command::new("go");
    cmd.arg("test");

    // Add verbose flag if requested
    if config.verbose {
        cmd.arg("-v");
    }

    // Add run pattern if specified
    if let Some(ref pattern) = config.run_pattern {
        cmd.arg("-run");
        cmd.arg(pattern);
    }

    // Add package patterns or default to ./...
    if config.packages.is_empty() {
        cmd.arg("./...");
    } else {
        for pkg in &config.packages {
            cmd.arg(pkg);
        }
    }

    // Add passthrough args
    for arg in &config.passthrough_args {
        cmd.arg(arg);
    }

    cmd.current_dir(temp_root);

    let output = cmd
        .output()
        .into_diagnostic()
        .map_err(|e| e.context("Failed to run go test"))?;

    // Print output
    if !output.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }

    let exit_code = output.status.code().unwrap_or(1);

    Ok(TestResult {
        exit_code,
        success: output.status.success(),
    })
}
