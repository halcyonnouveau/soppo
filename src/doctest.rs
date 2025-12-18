use std::path::Path;

use miette::Result;

use crate::syntax::{Decl, File, FileId, Parser, Span};

/// Attributes that can be applied to doctest code blocks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DoctestAttrs {
    /// Skip this doctest entirely
    pub ignore: bool,
    /// Expect a panic (test passes if it panics)
    pub should_panic: bool,
    /// Parse and type-check but don't run
    pub no_run: bool,
    /// Expect compilation to fail
    pub compile_fail: bool,
}

/// An import statement parsed from a doctest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// Optional alias (e.g., `stdmath` in `import stdmath "math"`)
    pub alias: Option<String>,
    /// The import path (e.g., `"fmt"` or `"math"`)
    pub path: String,
}

/// A single doctest extracted from a doc comment.
#[derive(Debug, Clone)]
pub struct Doctest {
    /// The name of the declaration this doctest documents
    pub decl_name: String,
    /// The Soppo code in the doctest (without imports)
    pub code: String,
    /// Imports parsed from the doctest
    pub imports: Vec<Import>,
    /// Expected output (from `// Output:` section)
    pub expected_output: Option<String>,
    /// Attributes parsed from the code fence
    pub attrs: DoctestAttrs,
    /// Source location for error reporting
    pub span: Span,
    /// Index for multiple doctests on the same declaration
    pub index: usize,
}

/// Result of extracting doctests from a file.
#[derive(Debug)]
pub struct FileDocTests {
    /// The source file path
    pub source_path: String,
    /// Package name from the file
    pub package: String,
    /// Extracted doctests
    pub doctests: Vec<Doctest>,
}

/// Extract doctests from a source file.
pub fn extract_from_source(source: &str, filename: &str) -> Result<FileDocTests> {
    let mut parser = Parser::new(source, FileId(0));
    let file = parser.parse_file()?;
    let _ = filename; // Used for error context in future

    let package = file.package.clone();
    let doctests = extract_from_ast(&file)?;

    Ok(FileDocTests {
        source_path: filename.to_string(),
        package,
        doctests,
    })
}

/// Extract doctests from a parsed AST.
fn extract_from_ast(file: &File) -> Result<Vec<Doctest>> {
    let mut doctests = Vec::new();

    for decl in &file.decls {
        match decl {
            Decl::Func(func) => {
                if let Some(ref doc) = func.doc_comment {
                    let decl_name = func.ident.name.clone();
                    let span = func.span;
                    let extracted = extract_doctests_from_comment(doc, &decl_name, &span)?;
                    doctests.extend(extracted);
                }
            }
            Decl::Type(ty) => {
                if let Some(ref doc) = ty.doc_comment {
                    let decl_name = ty.ident.name.clone();
                    let span = ty.span;
                    let extracted = extract_doctests_from_comment(doc, &decl_name, &span)?;
                    doctests.extend(extracted);
                }
            }
            Decl::Const(constant) => {
                if let Some(ref doc) = constant.doc_comment {
                    let decl_name = constant.ident.name.clone();
                    let span = constant.span;
                    let extracted = extract_doctests_from_comment(doc, &decl_name, &span)?;
                    doctests.extend(extracted);
                }
            }
            // Import and ConstGroup don't have doc comments that make sense for doctests
            _ => {}
        }
    }

    Ok(doctests)
}

/// Extract doctests from a doc comment string.
fn extract_doctests_from_comment(doc: &str, decl_name: &str, span: &Span) -> Result<Vec<Doctest>> {
    let code_blocks = parse_code_blocks(doc);
    let mut doctests = Vec::new();

    for (index, block) in code_blocks.into_iter().enumerate() {
        // Skip if ignore attribute is set
        if block.attrs.ignore {
            continue;
        }

        // Parse imports from the code
        let (imports, remaining_code) = parse_imports(&block.code);

        // Parse expected output
        let (code, expected_output) = parse_expected_output(&remaining_code);

        // Skip empty doctests
        if code.trim().is_empty() {
            continue;
        }

        doctests.push(Doctest {
            decl_name: decl_name.to_string(),
            code,
            imports,
            expected_output,
            attrs: block.attrs,
            span: *span,
            index,
        });
    }

    Ok(doctests)
}

/// A raw code block parsed from a doc comment.
#[derive(Debug)]
struct CodeBlock {
    /// The code content
    code: String,
    /// Parsed attributes
    attrs: DoctestAttrs,
}

/// Parse code blocks from a doc comment.
fn parse_code_blocks(doc: &str) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = doc.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // Check for opening fence with sop or soppo marker
        if let Some(attrs_str) = line
            .strip_prefix("```sop")
            .or_else(|| line.strip_prefix("```soppo"))
        {
            // Parse attributes from the fence
            let attrs = parse_fence_attrs(attrs_str);

            // Collect lines until closing fence
            let mut code_lines = Vec::new();
            i += 1;

            while i < lines.len() {
                let content = lines[i].trim();
                if content == "```" {
                    break;
                }
                code_lines.push(content);
                i += 1;
            }

            blocks.push(CodeBlock {
                code: code_lines.join("\n"),
                attrs,
            });
        }

        i += 1;
    }

    blocks
}

/// Parse attributes from the fence line (e.g., `,ignore,should_panic`).
fn parse_fence_attrs(attrs_str: &str) -> DoctestAttrs {
    let mut attrs = DoctestAttrs::default();

    for part in attrs_str.split(',') {
        let attr = part.trim();
        match attr {
            "ignore" => attrs.ignore = true,
            "should_panic" => attrs.should_panic = true,
            "no_run" => attrs.no_run = true,
            "compile_fail" => attrs.compile_fail = true,
            _ => {} // Ignore unknown attributes
        }
    }

    attrs
}

/// Parse import statements from the beginning of doctest code.
///
/// Returns the parsed imports and the remaining code.
fn parse_imports(code: &str) -> (Vec<Import>, String) {
    let mut imports = Vec::new();
    let mut remaining_lines = Vec::new();
    let mut in_import_section = true;

    for line in code.lines() {
        let trimmed = line.trim();

        if in_import_section {
            // Check for single import: `import "pkg"` or `import alias "pkg"`
            if let Some(import) = parse_import_line(trimmed) {
                imports.push(import);
                continue;
            }

            // Check for import block start
            if trimmed == "import (" {
                // Parse multi-line import block
                continue;
            }

            // Check for import block entry or end
            if trimmed == ")" {
                continue;
            }

            // Check for import line within a block (just the path or alias path)
            if let Some(import) = parse_import_block_line(trimmed) {
                imports.push(import);
                continue;
            }

            // Empty lines are ok in import section
            if trimmed.is_empty() {
                remaining_lines.push(line);
                continue;
            }

            // First non-import, non-empty line ends the import section
            in_import_section = false;
        }

        remaining_lines.push(line);
    }

    // Trim leading empty lines from remaining code
    while remaining_lines.first().is_some_and(|l| l.trim().is_empty()) {
        remaining_lines.remove(0);
    }

    (imports, remaining_lines.join("\n"))
}

/// Parse a single import line: `import "pkg"` or `import alias "pkg"`.
fn parse_import_line(line: &str) -> Option<Import> {
    let line = line.strip_prefix("import ")?.trim();

    // Check for aliased import: `alias "pkg"`
    if let Some((alias, path)) = line.split_once(' ') {
        let alias = alias.trim();
        let path = path.trim().trim_matches('"');
        if !alias.starts_with('"') {
            return Some(Import {
                alias: Some(alias.to_string()),
                path: path.to_string(),
            });
        }
    }

    // Simple import: `"pkg"`
    let path = line.trim_matches('"');
    Some(Import {
        alias: None,
        path: path.to_string(),
    })
}

/// Parse an import line within an import block: `"pkg"` or `alias "pkg"`.
fn parse_import_block_line(line: &str) -> Option<Import> {
    let line = line.trim();

    if line.is_empty() || !line.contains('"') {
        return None;
    }

    // Check for aliased import: `alias "pkg"`
    if let Some((alias, path)) = line.split_once(' ') {
        let alias = alias.trim();
        let path = path.trim().trim_matches('"');
        if !alias.starts_with('"') && !alias.is_empty() {
            return Some(Import {
                alias: Some(alias.to_string()),
                path: path.to_string(),
            });
        }
    }

    // Simple import: `"pkg"`
    let path = line.trim_matches('"');
    Some(Import {
        alias: None,
        path: path.to_string(),
    })
}

/// Parse expected output from `// Output:` comment at the end of code.
fn parse_expected_output(code: &str) -> (String, Option<String>) {
    let lines: Vec<&str> = code.lines().collect();
    let mut output_start = None;

    // Find the `// Output:` marker
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed == "// Output:" {
            output_start = Some(i);
            break;
        }
    }

    if let Some(start) = output_start {
        let code_lines = &lines[..start];
        let output_lines = &lines[start + 1..];

        // Extract output, stripping `// ` prefix
        let output: Vec<&str> = output_lines
            .iter()
            .map(|l| {
                let trimmed = l.trim();
                trimmed.strip_prefix("// ").unwrap_or(trimmed)
            })
            .collect();

        let code = code_lines.join("\n");
        let output_str = output.join("\n");

        (code, Some(output_str))
    } else {
        (code.to_string(), None)
    }
}

/// Generate a Go Example test file from extracted doctests.
pub fn generate_example_file(
    file_doctests: &FileDocTests,
    module_path: &str,
    source_path: &Path,
) -> Result<String> {
    let mut output = String::new();

    // Package declaration (use _test suffix for external test package)
    output.push_str(&format!("package {}_test\n\n", file_doctests.package));

    // Collect all unique imports
    let mut all_imports = Vec::new();

    // Add the package being tested
    let package_import_path = compute_import_path(module_path, source_path);
    all_imports.push(Import {
        alias: None,
        path: package_import_path,
    });

    // Add imports from all doctests
    for doctest in &file_doctests.doctests {
        for import in &doctest.imports {
            if !all_imports.iter().any(|i| i.path == import.path) {
                all_imports.push(import.clone());
            }
        }
    }

    // Write imports
    if !all_imports.is_empty() {
        output.push_str("import (\n");
        for import in &all_imports {
            if let Some(ref alias) = import.alias {
                output.push_str(&format!("\t{} \"{}\"\n", alias, import.path));
            } else {
                output.push_str(&format!("\t\"{}\"\n", import.path));
            }
        }
        output.push_str(")\n\n");
    }

    // Generate Example functions
    for doctest in &file_doctests.doctests {
        // Skip compile_fail doctests (they're for documentation only)
        if doctest.attrs.compile_fail {
            continue;
        }

        let func_name = generate_example_name(&doctest.decl_name, doctest.index);

        // Add no_run build constraint if needed
        if doctest.attrs.no_run {
            output.push_str("//go:build ignore\n\n");
        }

        output.push_str(&format!("func {}() {{\n", func_name));

        // Handle should_panic
        if doctest.attrs.should_panic {
            output.push_str("\tdefer func() {\n");
            output.push_str("\t\tif r := recover(); r == nil {\n");
            output.push_str("\t\t\tpanic(\"expected panic but none occurred\")\n");
            output.push_str("\t\t}\n");
            output.push_str("\t}()\n");
        }

        // Add the doctest code (indented)
        for line in doctest.code.lines() {
            if line.trim().is_empty() {
                output.push('\n');
            } else {
                output.push_str(&format!("\t{}\n", line));
            }
        }

        // Add expected output comment if present
        if let Some(ref expected) = doctest.expected_output {
            output.push_str("\t// Output:\n");
            for line in expected.lines() {
                output.push_str(&format!("\t// {}\n", line));
            }
        }

        output.push_str("}\n\n");
    }

    Ok(output)
}

/// Compute the import path for the package being tested.
fn compute_import_path(module_path: &str, source_path: &Path) -> String {
    // Get the directory containing the source file
    let dir = source_path.parent().unwrap_or(Path::new(""));

    if dir.as_os_str().is_empty() {
        // Root package
        module_path.to_string()
    } else {
        // Subpackage
        format!("{}/{}", module_path, dir.display())
    }
}

/// Generate a unique Example function name.
/// Go's convention: ExampleIdentifier, ExampleIdentifier_suffix
fn generate_example_name(decl_name: &str, index: usize) -> String {
    if index == 0 {
        format!("Example{}", decl_name)
    } else {
        // Subsequent examples use lowercase suffix
        format!("Example{}_{}", decl_name, index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_fence_attrs() {
        let attrs = parse_fence_attrs(",ignore");
        assert!(attrs.ignore);
        assert!(!attrs.should_panic);

        let attrs = parse_fence_attrs(",should_panic,no_run");
        assert!(attrs.should_panic);
        assert!(attrs.no_run);
        assert!(!attrs.ignore);
    }

    #[test]
    fn test_parse_import_line() {
        let import = parse_import_line(r#"import "fmt""#).unwrap();
        assert_eq!(import.path, "fmt");
        assert!(import.alias.is_none());

        let import = parse_import_line(r#"import stdmath "math""#).unwrap();
        assert_eq!(import.path, "math");
        assert_eq!(import.alias, Some("stdmath".to_string()));
    }

    #[test]
    fn test_parse_imports() {
        let code = r#"import "fmt"
import stdmath "math"

result := Add(1, 2)
fmt.Println(result)"#;

        let (imports, remaining) = parse_imports(code);

        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].path, "fmt");
        assert_eq!(imports[1].path, "math");
        assert_eq!(imports[1].alias, Some("stdmath".to_string()));

        assert!(remaining.contains("result := Add(1, 2)"));
        assert!(remaining.contains("fmt.Println(result)"));
    }

    #[test]
    fn test_parse_expected_output() {
        let code = r#"result := Add(1, 2)
fmt.Println(result)
// Output:
// 3"#;

        let (code, output) = parse_expected_output(code);

        assert!(code.contains("result := Add(1, 2)"));
        assert!(!code.contains("Output:"));
        assert_eq!(output, Some("3".to_string()));
    }

    #[test]
    fn test_parse_code_blocks() {
        let doc = r#"Add returns the sum of two numbers.

```sop
import "fmt"
fmt.Println(Add(1, 2))
```

This example shows panic handling:

```sop,should_panic
Add(MaxInt, 1)
```
"#;

        let blocks = parse_code_blocks(doc);

        assert_eq!(blocks.len(), 2);
        assert!(!blocks[0].attrs.should_panic);
        assert!(blocks[1].attrs.should_panic);
    }

    #[test]
    fn test_generate_example_name() {
        assert_eq!(generate_example_name("Add", 0), "ExampleAdd");
        assert_eq!(generate_example_name("Add", 1), "ExampleAdd_1");
        assert_eq!(generate_example_name("Add", 2), "ExampleAdd_2");
    }
}
