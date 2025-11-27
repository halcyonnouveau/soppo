use std::fs;
use std::path::PathBuf;

use clap::{Parser as ClapParser, Subcommand};
use miette::{IntoDiagnostic, NamedSource, Result};
use soppo::codegen::Codegen;
use soppo::go::Project;
use soppo::parse::{Decl, FileId, Parser};
use soppo::types::Infer;

#[derive(ClapParser)]
#[command(name = "sop")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build project or specific files
    Build {
        /// Files to compile (glob patterns expanded by shell)
        /// If empty, builds entire project from go.mod root
        #[arg()]
        files: Vec<PathBuf>,

        /// Output directory
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Type-check without generating code
    Check {
        /// Files to check (glob patterns expanded by shell)
        #[arg()]
        files: Vec<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Build { files, output } => {
            if files.is_empty() {
                build_project(output)?;
            } else {
                build_files(&files, output)?;
            }
        }
        Command::Check { files } => {
            if files.is_empty() {
                check_project()?;
            } else {
                check_files(&files)?;
            }
        }
    }

    Ok(())
}

fn build_project(output: Option<PathBuf>) -> Result<()> {
    let project = Project::discover(&std::env::current_dir().into_diagnostic()?)?;
    let output_dir = output.unwrap_or_else(|| project.root.join("gen"));

    let sources = project.find_sources();
    if sources.is_empty() {
        println!("No .sop files found in project");
        return Ok(());
    }

    for source_path in &sources {
        let output_path = project.output_path(source_path, &output_dir);
        compile_file(source_path, &output_path)?;
    }

    println!("✓ Built {} file(s)", sources.len());
    Ok(())
}

fn build_files(files: &[PathBuf], output: Option<PathBuf>) -> Result<()> {
    for file in files {
        let output_path = if let Some(ref dir) = output {
            // Preserve structure: --out-dir gen/ with pkg/main.sop -> gen/pkg/main.go
            let mut out = dir.join(file);
            out.set_extension("go");
            out
        } else {
            // Default: .go next to .sop
            let mut out = file.clone();
            out.set_extension("go");
            out
        };

        compile_file(file, &output_path)?;
    }

    println!("✓ Compiled {} file(s)", files.len());
    Ok(())
}

fn check_project() -> Result<()> {
    let project = Project::discover(&std::env::current_dir().into_diagnostic()?)?;

    let sources = project.find_sources();
    if sources.is_empty() {
        println!("No .sop files found in project");
        return Ok(());
    }

    for source_path in &sources {
        check_file(source_path)?;
    }

    println!("✓ Checked {} file(s)", sources.len());
    Ok(())
}

fn check_files(files: &[PathBuf]) -> Result<()> {
    for file in files {
        check_file(file)?;
    }

    println!("✓ Checked {} file(s)", files.len());
    Ok(())
}

fn compile_file(input: &PathBuf, output: &PathBuf) -> Result<()> {
    let source = fs::read_to_string(input)
        .into_diagnostic()
        .map_err(|e| e.context(format!("Failed to read file: {}", input.display())))?;

    let filename = input
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("input.sop");

    let go_code = compile(&source, filename)?;

    // Create parent directories if needed
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .into_diagnostic()
            .map_err(|e| e.context(format!("Failed to create directory: {}", parent.display())))?;
    }

    fs::write(output, go_code)
        .into_diagnostic()
        .map_err(|e| e.context(format!("Failed to write file: {}", output.display())))?;

    println!("  {} → {}", input.display(), output.display());
    Ok(())
}

fn check_file(input: &PathBuf) -> Result<()> {
    let source = fs::read_to_string(input)
        .into_diagnostic()
        .map_err(|e| e.context(format!("Failed to read file: {}", input.display())))?;

    let filename = input
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("input.sop");

    typecheck(&source, filename)?;

    println!("  ✓ {}", input.display());
    Ok(())
}

fn typecheck(source: &str, filename: &str) -> Result<()> {
    let mut parser = Parser::new(source, FileId(0));
    let file = parser.parse_file().map_err(|e| {
        miette::Report::from(e).with_source_code(NamedSource::new(filename, source.to_string()))
    })?;

    let mut infer = Infer::new()?;
    infer.process_imports(&file.imports);

    for decl in &file.decls {
        match decl {
            Decl::Const(const_decl) => {
                infer.infer_const_decl(const_decl).map_err(|e| {
                    miette::Report::from(e)
                        .with_source_code(NamedSource::new(filename, source.to_string()))
                })?;
            }
            Decl::Type(type_decl) => {
                infer.infer_type_decl(type_decl).map_err(|e| {
                    miette::Report::from(e)
                        .with_source_code(NamedSource::new(filename, source.to_string()))
                })?;
            }
            Decl::Func(func) => {
                infer.infer_func_decl(func).map_err(|e| {
                    miette::Report::from(e)
                        .with_source_code(NamedSource::new(filename, source.to_string()))
                })?;
            }
        }
    }

    Ok(())
}

fn compile(source: &str, filename: &str) -> Result<String> {
    // Parse
    let mut parser = Parser::new(source, FileId(0));
    let file = parser.parse_file().map_err(|e| {
        miette::Report::from(e).with_source_code(NamedSource::new(filename, source.to_string()))
    })?;

    // Type check
    let mut infer = Infer::new()?;

    // Process imports to add package names to scope
    infer.process_imports(&file.imports);

    for decl in &file.decls {
        match decl {
            Decl::Const(const_decl) => {
                infer.infer_const_decl(const_decl).map_err(|e| {
                    miette::Report::from(e)
                        .with_source_code(NamedSource::new(filename, source.to_string()))
                })?;
            }
            Decl::Type(type_decl) => {
                infer.infer_type_decl(type_decl).map_err(|e| {
                    miette::Report::from(e)
                        .with_source_code(NamedSource::new(filename, source.to_string()))
                })?;
            }
            Decl::Func(func) => {
                infer.infer_func_decl(func).map_err(|e| {
                    miette::Report::from(e)
                        .with_source_code(NamedSource::new(filename, source.to_string()))
                })?;
            }
        }
    }

    // Generate Go code
    let global_state = infer.global_state();
    let mut codegen = Codegen::with_global_state(global_state);
    codegen.gen_file(&file);

    Ok(codegen.output().to_string())
}
