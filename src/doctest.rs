use std::path::Path;

use miette::Result;

use crate::codegen::Codegen;
use crate::syntax::Import as AstImport;
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

impl From<&AstImport> for Import {
    fn from(ast_import: &AstImport) -> Self {
        Import {
            alias: ast_import.alias.clone(),
            path: ast_import.path.clone(),
        }
    }
}

/// A single doctest extracted from a doc comment.
#[derive(Debug, Clone)]
pub struct Doctest {
    /// The name of the declaration this doctest documents
    pub decl_name: String,
    /// The transpiled Go code for the doctest body
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

    let package = file.package.name.clone();
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

        // Extract expected output first (before parsing)
        let (code_without_output, expected_output) = parse_expected_output(&block.code);

        // Skip empty doctests
        if code_without_output.trim().is_empty() {
            continue;
        }

        // Parse the doctest code using the Soppo parser
        let mut parser = Parser::new(&code_without_output, FileId(0));
        let parsed = match parser.parse_doctest() {
            Ok(parsed) => parsed,
            Err(_e) => {
                // If parsing fails, skip this doctest
                // TODO: Consider reporting parse errors
                continue;
            }
        };

        // Convert AST imports to our Import type
        let imports: Vec<Import> = parsed.imports.iter().map(Import::from).collect();

        // Transpile statements to Go code using codegen
        let mut codegen = Codegen::new();
        codegen.gen_statements(&parsed.stmts);
        let go_code = codegen.output().to_string();

        // Skip if transpilation resulted in empty code
        if go_code.trim().is_empty() {
            continue;
        }

        doctests.push(Doctest {
            decl_name: decl_name.to_string(),
            code: go_code,
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
/// Returns None if there are no runnable doctests.
pub fn generate_example_file(
    file_doctests: &FileDocTests,
    module_path: &str,
    source_path: &Path,
) -> Result<Option<String>> {
    // Check if there are any runnable doctests (not compile_fail or no_run)
    let runnable_doctests: Vec<_> = file_doctests
        .doctests
        .iter()
        .filter(|d| !d.attrs.compile_fail && !d.attrs.no_run)
        .collect();

    if runnable_doctests.is_empty() {
        return Ok(None);
    }

    let mut output = String::new();

    // Package declaration (use _test suffix for external test package)
    output.push_str(&format!("package {}_test\n\n", file_doctests.package));

    // Collect all unique imports from runnable doctests only
    let mut all_imports = Vec::new();

    // Add the package being tested with dot import so exported symbols
    // are available without qualification (doctest code is written as if
    // inside the package)
    let package_import_path = compute_import_path(module_path, source_path);
    all_imports.push(Import {
        alias: Some(".".to_string()),
        path: package_import_path,
    });

    // Add imports from runnable doctests only
    for doctest in &runnable_doctests {
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
        // Skip compile_fail and no_run doctests (they're for documentation only)
        if doctest.attrs.compile_fail || doctest.attrs.no_run {
            continue;
        }

        let func_name = generate_example_name(&doctest.decl_name, doctest.index);

        output.push_str(&format!("func {}() {{\n", func_name));

        // Handle should_panic
        if doctest.attrs.should_panic {
            output.push_str("\tdefer func() {\n");
            output.push_str("\t\tif r := recover(); r == nil {\n");
            output.push_str("\t\t\tpanic(\"expected panic but none occurred\")\n");
            output.push_str("\t\t}\n");
            output.push_str("\t}()\n");
        }

        // Add the transpiled Go code (already indented by codegen)
        // We need to add one level of indentation since we're inside a function
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

    Ok(Some(output))
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

    #[test]
    fn test_doctest_parsing_with_parser() {
        // Test that we can parse a simple doctest
        let code = r#"import "fmt"

compat := GoCompatFor("0.5.0")
fmt.Println(compat.Min)"#;

        let mut parser = Parser::new(code, FileId(0));
        let parsed = parser.parse_doctest().unwrap();

        // Should have 1 import
        assert_eq!(parsed.imports.len(), 1);
        assert_eq!(parsed.imports[0].path, "fmt");

        // Should have 2 statements
        assert_eq!(parsed.stmts.len(), 2);
    }
}
