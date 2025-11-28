use std::fs;
use std::path::PathBuf;

use clap::{Parser as ClapParser, Subcommand};
use miette::{IntoDiagnostic, Result};
use soppo::build;
use soppo::go::Project;

#[derive(ClapParser)]
#[command(name = "sop", about = "Sop is a tool for managing Soppo source code")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build project or specific files
    Build {
        /// Files to compile. If empty, builds entire project from go.mod root
        #[arg()]
        files: Vec<PathBuf>,

        /// Output directory
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Type-check without generating code
    Check {
        /// Files to check
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
    let cwd = std::env::current_dir().into_diagnostic()?;
    let count = build::build_project_to_disk(&cwd, output.as_deref())?;

    if count == 0 {
        println!("No .sop files found in project");
    } else {
        println!("✓ Built {} file(s)", count);
    }

    Ok(())
}

fn build_files(files: &[PathBuf], output: Option<PathBuf>) -> Result<()> {
    for file in files {
        let output_path = if let Some(ref dir) = output {
            let mut out = dir.join(file);
            out.set_extension("go");
            out
        } else {
            let mut out = file.clone();
            out.set_extension("go");
            out
        };

        let source = fs::read_to_string(file)
            .into_diagnostic()
            .map_err(|e| e.context(format!("Failed to read file: {}", file.display())))?;

        let filename = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("input.sop");

        let go_code = build::compile(&source, filename)?;

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).into_diagnostic().map_err(|e| {
                e.context(format!("Failed to create directory: {}", parent.display()))
            })?;
        }

        fs::write(&output_path, go_code)
            .into_diagnostic()
            .map_err(|e| e.context(format!("Failed to write file: {}", output_path.display())))?;

        println!("  {} → {}", file.display(), output_path.display());
    }

    println!("✓ Compiled {} file(s)", files.len());
    Ok(())
}

fn check_project() -> Result<()> {
    let cwd = std::env::current_dir().into_diagnostic()?;
    let project = Project::discover(&cwd)?;

    let sources = project.find_sources();
    if sources.is_empty() {
        println!("No .sop files found in project");
        return Ok(());
    }

    for source_path in &sources {
        let source = fs::read_to_string(source_path)
            .into_diagnostic()
            .map_err(|e| e.context(format!("Failed to read file: {}", source_path.display())))?;

        let filename = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("input.sop");

        build::typecheck(&source, filename)?;
        println!("  ✓ {}", source_path.display());
    }

    println!("✓ Checked {} file(s)", sources.len());
    Ok(())
}

fn check_files(files: &[PathBuf]) -> Result<()> {
    for file in files {
        let source = fs::read_to_string(file)
            .into_diagnostic()
            .map_err(|e| e.context(format!("Failed to read file: {}", file.display())))?;

        let filename = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("input.sop");

        build::typecheck(&source, filename)?;
        println!("  ✓ {}", file.display());
    }

    println!("✓ Checked {} file(s)", files.len());
    Ok(())
}
