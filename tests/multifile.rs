use std::fs;
use std::process::Command;

use tempfile::TempDir;

/// Create a test project with go.mod and compile it using the library directly
fn build_test_project(
    module_name: &str,
    files: &[(&str, &str)],
) -> Result<Vec<(String, String)>, String> {
    let temp = TempDir::new().expect("Failed to create temp dir");
    let root = temp.path();

    // Create go.mod
    fs::write(
        root.join("go.mod"),
        format!("module {}\n\ngo 1.25\n", module_name),
    )
    .expect("Failed to write go.mod");

    // Create source files
    for (path, content) in files {
        let file_path = root.join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create directories");
        }
        fs::write(&file_path, content).expect("Failed to write source file");
    }

    // Build using the library directly
    let results = soppo::build::build_project(root, None).map_err(|e| e.to_string())?;

    // Write generated files to disk for go build validation
    let gen_dir = root.join("gen");
    for (relative_path, go_code) in &results {
        let output_path = gen_dir.join(relative_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create output directories");
        }
        fs::write(&output_path, go_code).expect("Failed to write generated file");
    }

    // Add replace directive to go.mod so Go can find local packages
    let go_mod_path = root.join("go.mod");
    let go_mod_content = fs::read_to_string(&go_mod_path).expect("Failed to read go.mod");
    let updated_go_mod = format!(
        "{}\nreplace {}/gen => ./gen\n",
        go_mod_content.trim(),
        module_name
    );
    fs::write(&go_mod_path, updated_go_mod).expect("Failed to update go.mod");

    // Run go build to verify the generated code compiles
    if gen_dir.exists() {
        let go_output = Command::new("go")
            .args(["build", "./..."])
            .current_dir(&gen_dir)
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
    }

    Ok(results)
}

#[test]
fn test_simple_sop_import() {
    // Each package must be in its own directory for Go to find it
    // helpers/lib.sop with package helpers → gen/helpers/lib.go
    // Import: module/helpers → Go import: module/gen/helpers (directory)
    let files = &[
        (
            "helpers/lib.sop",
            r#"package helpers

func Add(a int, b int) int {
    return a + b
}
"#,
        ),
        (
            "cmd/main.sop",
            r#"package main

import (
    "fmt"
    "github.com/test/myproject/helpers"
)

func main() {
    result := helpers.Add(1, 2)
    fmt.Println(result)
}
"#,
        ),
    ];

    let results =
        build_test_project("github.com/test/myproject", files).expect("Build should succeed");

    // Should have generated 2 files
    assert_eq!(results.len(), 2);

    // Check main.go has correct import path (directory)
    let (_, main_content) = results.iter().find(|(p, _)| p.contains("main.go")).unwrap();
    assert!(
        main_content.contains("\"github.com/test/myproject/gen/helpers\""),
        "Import path should point to directory.\nGot:\n{}",
        main_content
    );
}

#[test]
fn test_aliased_sop_import() {
    let files = &[
        (
            "mathutil/lib.sop",
            r#"package mathutil

func Multiply(a int, b int) int {
    return a * b
}
"#,
        ),
        (
            "cmd/main.sop",
            r#"package main

import (
    "fmt"
    u "github.com/test/aliased/mathutil"
)

func main() {
    result := u.Multiply(3, 4)
    fmt.Println(result)
}
"#,
        ),
    ];

    let results =
        build_test_project("github.com/test/aliased", files).expect("Build should succeed");

    let (_, main_content) = results.iter().find(|(p, _)| p.contains("main.go")).unwrap();
    assert!(
        main_content.contains("u \"github.com/test/aliased/gen/mathutil\""),
        "Aliased import should preserve alias.\nGot:\n{}",
        main_content
    );
}

#[test]
fn test_chain_dependencies() {
    // main imports pkgb, pkgb imports pkga - tests topological sorting
    // Each package must be in its own directory for Go
    let files = &[
        (
            "pkga/lib.sop",
            r#"package pkga

func GetA() int {
    return 1
}
"#,
        ),
        (
            "pkgb/lib.sop",
            r#"package pkgb

import "github.com/test/chain/pkga"

func GetB() int {
    return pkga.GetA() + 1
}
"#,
        ),
        (
            "cmd/main.sop",
            r#"package main

import (
    "fmt"
    "github.com/test/chain/pkgb"
)

func main() {
    fmt.Println(pkgb.GetB())
}
"#,
        ),
    ];

    let results = build_test_project("github.com/test/chain", files)
        .expect("Build should succeed with chained dependencies");

    assert_eq!(results.len(), 3);

    // Verify import paths in generated code
    let (_, b_content) = results.iter().find(|(p, _)| p.contains("pkgb")).unwrap();
    assert!(
        b_content.contains("\"github.com/test/chain/gen/pkga\""),
        "pkgb should import pkga with correct path.\nGot:\n{}",
        b_content
    );
}

#[test]
fn test_circular_dependency_fails() {
    // Each package in its own directory
    let files = &[
        (
            "pkga/lib.sop",
            r#"package pkga

import "github.com/test/circular/pkgb"

func GetA() int {
    return pkgb.GetB()
}
"#,
        ),
        (
            "pkgb/lib.sop",
            r#"package pkgb

import "github.com/test/circular/pkga"

func GetB() int {
    return pkga.GetA()
}
"#,
        ),
    ];

    let result = build_test_project("github.com/test/circular", files);
    assert!(result.is_err(), "Circular dependency should fail");
    assert!(
        result.unwrap_err().contains("Circular dependency"),
        "Error should mention circular dependency"
    );
}

#[test]
fn test_missing_local_import_treated_as_go() {
    // When importing a local path that doesn't have .sop files,
    // it's treated as a Go import and will fail during Go build
    let files = &[(
        "cmd/main.sop",
        r#"package main

import "github.com/test/missing/nonexistent"

func main() {}
"#,
    )];

    let result = build_test_project("github.com/test/missing", files);
    // Should fail during Go build (treated as a Go import)
    assert!(
        result.is_err(),
        "Missing Go import should fail during go build"
    );
    let err = result.unwrap_err();
    // Go will report it can't find the package
    assert!(
        err.contains("go build")
            || err.contains("cannot find")
            || err.contains("no required module"),
        "Error should be from Go build.\nGot:\n{}",
        err
    );
}

#[test]
fn test_multiple_return_types() {
    // Test that functions with multiple return types work across packages
    let files = &[
        (
            "helpers/lib.sop",
            r#"package helpers

func GetPair() (int, string) {
    return 42, "hello"
}
"#,
        ),
        (
            "cmd/main.sop",
            r#"package main

import (
    "fmt"
    "github.com/test/multiret/helpers"
)

func main() {
    num, str := helpers.GetPair()
    fmt.Println(num, str)
}
"#,
        ),
    ];

    let results =
        build_test_project("github.com/test/multiret", files).expect("Build should succeed");

    assert_eq!(results.len(), 2);

    // Check that the generated code compiles (go build runs in build_test_project)
    let (_, main_content) = results.iter().find(|(p, _)| p.contains("main.go")).unwrap();
    assert!(
        main_content.contains("helpers.GetPair"),
        "Should call helpers.GetPair.\nGot:\n{}",
        main_content
    );
}
