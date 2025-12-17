use std::fs;
use std::path::PathBuf;

use clap::{Parser as ClapParser, Subcommand};
use miette::{IntoDiagnostic, Result};
use soppo::build;
use soppo::config::{ConfigError, resolve_globs};
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
        /// Files or glob patterns to compile (e.g., "**/*.sop", "src/**/*.sop").
        /// If empty, uses sop.mod config or errors if no config exists.
        #[arg()]
        files: Vec<String>,

        /// Output directory (preserves source directory structure)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Type-check without generating code
    Check {
        /// Files or glob patterns to check.
        /// If empty, uses sop.mod config or errors if no config exists.
        #[arg()]
        files: Vec<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Build { files, output } => {
            let cwd = std::env::current_dir().into_diagnostic()?;

            if files.is_empty() {
                // No CLI args - try sop.mod
                build_from_config(&cwd, output)?;
            } else {
                // CLI args provided - resolve globs and build
                let resolved = resolve_globs(&files, &cwd)?;
                if resolved.is_empty() {
                    return Err(ConfigError::NoFilesSpecified.into());
                }
                build_files(&resolved, output)?;
            }
        }
        Command::Check { files } => {
            let cwd = std::env::current_dir().into_diagnostic()?;

            if files.is_empty() {
                // No CLI args - try sop.mod
                check_from_config(&cwd)?;
            } else {
                // CLI args provided - resolve globs and check
                let resolved = resolve_globs(&files, &cwd)?;
                if resolved.is_empty() {
                    return Err(ConfigError::NoFilesSpecified.into());
                }
                check_files(&resolved)?;
            }
        }
    }

    Ok(())
}

/// Build using sop.mod configuration
fn build_from_config(cwd: &std::path::Path, output: Option<PathBuf>) -> Result<()> {
    let project = Project::discover(cwd)?;

    if project.config.is_none() {
        return Err(ConfigError::NoFilesSpecified.into());
    }

    let count = build::build_project_to_disk(cwd, output.as_deref())?;

    if count == 0 {
        println!("No .sop files found matching patterns");
    } else {
        println!("✓ Built {} file(s)", count);
    }

    Ok(())
}

/// Build specific files (after glob resolution)
fn build_files(files: &[PathBuf], output: Option<PathBuf>) -> Result<()> {
    for file in files {
        let output_path = if let Some(ref dir) = output {
            let mut out = dir.join(file.file_name().unwrap_or_default());
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

/// Check using sop.mod configuration
fn check_from_config(cwd: &std::path::Path) -> Result<()> {
    let project = Project::discover(cwd)?;

    if project.config.is_none() {
        return Err(ConfigError::NoFilesSpecified.into());
    }

    let sources = project.find_sources();
    if sources.is_empty() {
        println!("No .sop files found matching patterns");
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

/// Check specific files (after glob resolution)
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
