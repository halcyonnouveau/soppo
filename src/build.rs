use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use miette::{IntoDiagnostic, NamedSource, Result};

use crate::codegen::Codegen;
use crate::deps::DepGraph;
use crate::go::Project;
use crate::syntax::{Decl, FileId, FileRegistry, ModuleId, Parser};
use crate::types::{GlobalCtxt, Infer, SymbolTable};

/// Result of compiling a project - maps relative paths to generated Go code
pub type BuildResult = Vec<(String, String)>;

/// Result of type-checking a workspace - used by the LSP
#[derive(Debug)]
pub struct WorkspaceResult {
    /// Registry mapping FileId to file paths
    pub file_registry: FileRegistry,
    /// Global type context with all modules
    pub global_ctxt: GlobalCtxt,
    /// Symbol tables per file for LSP features
    pub symbol_tables: HashMap<FileId, SymbolTable>,
}

/// Build a project from a directory containing go.mod
pub fn build_project(root: &Path, output_dir: Option<&Path>) -> Result<BuildResult> {
    let project = Project::discover(root)?;

    // Compute output directory (absolute) and relative path for Go imports
    let (output_dir_abs, output_dir_relative) = match output_dir {
        Some(dir) => {
            let relative = dir
                .strip_prefix(&project.root)
                .map(|p| p.to_string_lossy().to_string())
                .ok();
            (dir.to_path_buf(), relative)
        }
        None => (project.root.join("gen"), Some("gen".to_string())),
    };

    let sources = project.find_sources();
    if sources.is_empty() {
        return Ok(Vec::new());
    }

    // Build dependency graph and topologically sort
    let dep_graph = DepGraph::build(&sources, &project.root, &project.module_path)?;
    let ordered_sources = dep_graph.topological_sort()?;

    // Compile files in dependency order
    let mut global_ctxt = GlobalCtxt::new();
    let mut results = Vec::new();

    for source_path in &ordered_sources {
        let output_path = project.output_path(source_path, &output_dir_abs);

        // Compute module ID from package directory
        let module_id = source_path
            .strip_prefix(&project.root)
            .ok()
            .and_then(|p| p.parent())
            .and_then(|p| p.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("main");

        let source = fs::read_to_string(source_path)
            .into_diagnostic()
            .map_err(|e| e.context(format!("Failed to read file: {}", source_path.display())))?;

        let filename = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("input.sop");

        let (go_code, new_global_ctxt) = compile_with_context(
            &source,
            filename,
            global_ctxt,
            &project.module_path,
            output_dir_relative.as_deref(),
            module_id,
            &project.root,
        )?;

        // Compute relative output path for the result
        let relative_path = output_path
            .strip_prefix(&output_dir_abs)
            .unwrap_or(&output_path)
            .to_string_lossy()
            .to_string();

        results.push((relative_path, go_code));
        global_ctxt = new_global_ctxt;
    }

    Ok(results)
}

/// Build a project and write output files to disk
pub fn build_project_to_disk(root: &Path, output_dir: Option<&Path>) -> Result<usize> {
    let project = Project::discover(root)?;

    let output_dir_abs = output_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| project.root.join("gen"));

    let results = build_project(root, output_dir)?;
    let count = results.len();

    for (relative_path, go_code) in results {
        let output_path = output_dir_abs.join(&relative_path);

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).into_diagnostic().map_err(|e| {
                e.context(format!("Failed to create directory: {}", parent.display()))
            })?;
        }

        fs::write(&output_path, go_code)
            .into_diagnostic()
            .map_err(|e| e.context(format!("Failed to write file: {}", output_path.display())))?;
    }

    Ok(count)
}

/// Compile a single source string
pub fn compile(source: &str, filename: &str) -> Result<String> {
    let mut parser = Parser::new(source, FileId(0));
    let file = parser.parse_file().map_err(|e| {
        miette::Report::from(e).with_source_code(NamedSource::new(filename, source.to_string()))
    })?;

    let mut infer = Infer::new()?;
    infer.process_imports(&file.imports);

    // Two-pass type checking:
    // Pass 1: Register all type definitions and function signatures
    for decl in &file.decls {
        register_decl(&mut infer, decl, source, filename)?;
    }

    // Pass 2: Infer and check function bodies
    for decl in &file.decls {
        infer_decl(&mut infer, decl, source, filename)?;
    }

    let global_state = infer.into_global_state();
    let mut codegen = Codegen::with_global_state(global_state);
    codegen.gen_file(&file).map_err(|e| {
        miette::Report::from(e).with_source_code(NamedSource::new(filename, source.to_string()))
    })?;

    Ok(codegen.output().to_string())
}

/// Compile with existing GlobalCtxt, returning the code and updated GlobalCtxt
pub fn compile_with_context(
    source: &str,
    filename: &str,
    mut global_ctxt: GlobalCtxt,
    module_path: &str,
    output_dir: Option<&str>,
    module_id: &str,
    project_root: &Path,
) -> Result<(String, GlobalCtxt)> {
    let mut parser = Parser::new(source, FileId(0));
    let file = parser.parse_file().map_err(|e| {
        miette::Report::from(e).with_source_code(NamedSource::new(filename, source.to_string()))
    })?;

    global_ctxt.set_current_module(ModuleId::new(module_id));

    // Create project context for import resolution
    let project = Project {
        root: project_root.to_path_buf(),
        module_path: module_path.to_string(),
    };

    let mut infer = Infer::with_global_state_and_project(global_ctxt, project)?;
    infer.process_imports(&file.imports);

    // Two-pass type checking:
    // Pass 1: Register all type definitions and function signatures
    for decl in &file.decls {
        register_decl(&mut infer, decl, source, filename)?;
    }

    // Pass 2: Infer and check function bodies
    for decl in &file.decls {
        infer_decl(&mut infer, decl, source, filename)?;
    }

    let global_state = infer.into_global_state();
    let mut codegen = Codegen::with_module_info(
        global_state.clone(),
        module_path.to_string(),
        output_dir.map(String::from),
        project_root.to_path_buf(),
    );
    codegen.gen_file(&file).map_err(|e| {
        miette::Report::from(e).with_source_code(NamedSource::new(filename, source.to_string()))
    })?;

    Ok((codegen.output().to_string(), global_state))
}

/// Type-check a single source string without generating code
pub fn typecheck(source: &str, filename: &str) -> Result<()> {
    typecheck_with_symbols(source, filename).map(|_| ())
}

/// Type-check a single source string and return the symbol table.
/// Used by the LSP for hover and go-to-definition.
pub fn typecheck_with_symbols(source: &str, filename: &str) -> Result<SymbolTable> {
    let mut parser = Parser::new(source, FileId(0));
    let file = parser.parse_file().map_err(|e| {
        miette::Report::from(e).with_source_code(NamedSource::new(filename, source.to_string()))
    })?;

    let mut infer = Infer::new()?;
    infer.process_imports(&file.imports);

    // Two-pass type checking:
    // Pass 1: Register all type definitions and function signatures
    for decl in &file.decls {
        register_decl(&mut infer, decl, source, filename)?;
    }

    // Pass 2: Infer and check function bodies
    for decl in &file.decls {
        infer_decl(&mut infer, decl, source, filename)?;
    }

    Ok(infer.into_symbols())
}

/// Pass 1: Register type definitions and function signatures (no body checking)
fn register_decl(infer: &mut Infer, decl: &Decl, source: &str, filename: &str) -> Result<()> {
    let add_source = |e| {
        miette::Report::from(e).with_source_code(NamedSource::new(filename, source.to_string()))
    };

    match decl {
        Decl::Const(const_decl) => {
            // Consts are fully processed in pass 1 since they don't have bodies
            infer.infer_const_decl(const_decl).map_err(add_source)?;
        }
        Decl::ConstBlock(consts) => {
            // Process each const in the block
            for const_decl in consts {
                infer.infer_const_decl(const_decl).map_err(add_source)?;
            }
        }
        Decl::Type(type_decl) => {
            // Types are fully processed in pass 1
            infer.infer_type_decl(type_decl).map_err(add_source)?;
        }
        Decl::Func(func) => {
            // Only register the signature, don't check the body yet
            infer.register_func_signature(func).map_err(add_source)?;
        }
    }
    Ok(())
}

/// Pass 2: Infer and check function bodies (consts and types already processed in pass 1)
fn infer_decl(infer: &mut Infer, decl: &Decl, source: &str, filename: &str) -> Result<()> {
    let add_source = |e| {
        miette::Report::from(e).with_source_code(NamedSource::new(filename, source.to_string()))
    };

    match decl {
        // Consts and types were fully processed in pass 1
        Decl::Const(_) | Decl::ConstBlock(_) | Decl::Type(_) => {}
        // Only functions need body checking in pass 2
        Decl::Func(func) => {
            infer.infer_func_decl(func).map_err(add_source)?;
        }
    }
    Ok(())
}

/// Type-check an entire workspace, returning the FileRegistry, GlobalCtxt, and SymbolTables.
/// Used by the LSP for cross-file features like go-to-definition.
///
/// `file_overrides` can provide in-memory content for files (e.g., unsaved changes in the editor).
pub fn typecheck_workspace(
    root: &Path,
    file_overrides: &HashMap<PathBuf, String>,
) -> Result<WorkspaceResult> {
    let project = Project::discover(root)?;
    let sources = project.find_sources();

    if sources.is_empty() {
        return Ok(WorkspaceResult {
            file_registry: FileRegistry::new(),
            global_ctxt: GlobalCtxt::new(),
            symbol_tables: HashMap::new(),
        });
    }

    // Build dependency graph and topologically sort
    let dep_graph = DepGraph::build(&sources, &project.root, &project.module_path)?;
    let ordered_sources = dep_graph.topological_sort()?;

    let mut file_registry = FileRegistry::new();
    let mut global_ctxt = GlobalCtxt::new();
    let mut symbol_tables = HashMap::new();

    for source_path in &ordered_sources {
        // Register the file and get its FileId
        let file_id = file_registry.register(source_path.clone());

        // Use override content if available, otherwise read from disk
        let source = if let Some(content) = file_overrides.get(source_path) {
            content.clone()
        } else {
            fs::read_to_string(source_path)
                .into_diagnostic()
                .map_err(|e| e.context(format!("Failed to read file: {}", source_path.display())))?
        };

        let filename = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("input.sop");

        // Compute module ID from package directory
        let module_id = source_path
            .strip_prefix(&project.root)
            .ok()
            .and_then(|p| p.parent())
            .and_then(|p| p.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("main");

        // Parse with correct FileId
        let mut parser = Parser::new(&source, file_id);
        let file = parser.parse_file().map_err(|e| {
            miette::Report::from(e).with_source_code(NamedSource::new(filename, source.to_string()))
        })?;

        // Set up for this module
        global_ctxt.set_current_module(ModuleId::new(module_id));

        let mut infer = Infer::with_global_state_and_project(global_ctxt, project.clone())?;
        infer.process_imports(&file.imports);

        // Two-pass type checking
        for decl in &file.decls {
            register_decl(&mut infer, decl, &source, filename)?;
        }
        for decl in &file.decls {
            infer_decl(&mut infer, decl, &source, filename)?;
        }

        // Extract results
        let symbols = infer.symbols().clone();
        global_ctxt = infer.into_global_state();
        symbol_tables.insert(file_id, symbols);
    }

    Ok(WorkspaceResult {
        file_registry,
        global_ctxt,
        symbol_tables,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn typecheck_workspace_cross_file_symbols() {
        // Set up a temp project with go.mod
        let temp = TempDir::new().expect("Failed to create temp dir");
        let root = temp.path();

        // Create go.mod
        fs::write(
            root.join("go.mod"),
            "module github.com/test/simple\n\ngo 1.23\n",
        )
        .expect("Failed to write go.mod");

        // Create helpers/lib.sop
        fs::create_dir_all(root.join("helpers")).expect("Failed to create helpers dir");
        fs::write(
            root.join("helpers/lib.sop"),
            r#"package helpers

func Add(a int, b int) int {
    return a + b
}
"#,
        )
        .expect("Failed to write helpers/lib.sop");

        // Create cmd/main.sop
        fs::create_dir_all(root.join("cmd")).expect("Failed to create cmd dir");
        fs::write(
            root.join("cmd/main.sop"),
            r#"package main

import (
    "fmt"
    "github.com/test/simple/helpers"
)

func main() {
    result := helpers.Add(1, 2)
    fmt.Println(result)
}
"#,
        )
        .expect("Failed to write cmd/main.sop");

        // Typecheck the workspace
        let result = typecheck_workspace(root, &HashMap::new())
            .expect("Workspace should typecheck successfully");

        // Find the main file
        let main_file_id = result
            .file_registry
            .file_ids()
            .find(|id| {
                result
                    .file_registry
                    .get_path(*id)
                    .map(|p| p.ends_with("cmd/main.sop"))
                    .unwrap_or(false)
            })
            .expect("Should find main.sop");

        // Find the helpers file
        let helpers_file_id = result
            .file_registry
            .file_ids()
            .find(|id| {
                result
                    .file_registry
                    .get_path(*id)
                    .map(|p| p.ends_with("helpers/lib.sop"))
                    .unwrap_or(false)
            })
            .expect("Should find helpers/lib.sop");

        // Get symbols for main file
        let main_symbols = result
            .symbol_tables
            .get(&main_file_id)
            .expect("Should have symbols for main file");

        // Look for the "Add" symbol (from helpers.Add call)
        let add_symbol = main_symbols
            .all_symbols()
            .values()
            .find(|s| s.name == "Add")
            .expect("Should have Add symbol in main file");

        // Verify it has a definition span pointing to the helpers file
        let def_span = add_symbol
            .definition_span
            .expect("Add symbol should have a definition span");

        assert_eq!(
            def_span.file, helpers_file_id,
            "Add symbol definition should point to helpers file"
        );
    }
}
