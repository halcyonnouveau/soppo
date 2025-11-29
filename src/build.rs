use std::fs;
use std::path::Path;

use miette::{IntoDiagnostic, NamedSource, Result};

use crate::codegen::Codegen;
use crate::deps::DepGraph;
use crate::go::Project;
use crate::syntax::{Decl, FileId, ModuleId, Parser};
use crate::types::{GlobalCtxt, Infer};

/// Result of compiling a project - maps relative paths to generated Go code
pub type BuildResult = Vec<(String, String)>;

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
    let mut parser = Parser::new(source, FileId(0));
    let file = parser.parse_file().map_err(|e| {
        miette::Report::from(e).with_source_code(NamedSource::new(filename, source.to_string()))
    })?;

    let mut infer = Infer::new()?;
    infer.process_imports(&file.imports);

    for decl in &file.decls {
        infer_decl(&mut infer, decl, source, filename)?;
    }

    Ok(())
}

fn infer_decl(infer: &mut Infer, decl: &Decl, source: &str, filename: &str) -> Result<()> {
    // Create NamedSource once, only clone if an error occurs
    let add_source = |e| {
        miette::Report::from(e).with_source_code(NamedSource::new(filename, source.to_string()))
    };

    match decl {
        Decl::Const(const_decl) => {
            infer.infer_const_decl(const_decl).map_err(add_source)?;
        }
        Decl::Type(type_decl) => {
            infer.infer_type_decl(type_decl).map_err(add_source)?;
        }
        Decl::Func(func) => {
            infer.infer_func_decl(func).map_err(add_source)?;
        }
    }
    Ok(())
}
