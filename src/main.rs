use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

use clap::{Parser as ClapParser, Subcommand};
use miette::{IntoDiagnostic, Result};
use soppo::build;
use soppo::config::{ConfigError, resolve_globs};
use soppo::format;
use soppo::go::Project;
use soppo::test::TestConfig;

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
    /// Run tests
    Test {
        /// Packages or files to test (e.g., "./pkg/...", "path/to/test.sop").
        /// If empty, runs all tests in the project.
        #[arg()]
        packages: Vec<String>,

        /// Run only tests matching this pattern (passed to `go test -run`)
        #[arg(short, long)]
        run: Option<String>,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,

        /// Keep temp directory on failure (for debugging)
        #[arg(long)]
        keep_temp: bool,

        /// Additional arguments to pass to `go test`
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Format source files
    Fmt {
        /// Files or glob patterns to format.
        /// If empty, uses sop.mod config or errors if no config exists.
        #[arg()]
        files: Vec<String>,

        /// Write result to (source) file instead of stdout
        #[arg(short, long)]
        write: bool,

        /// List files whose formatting differs from sop fmt's
        #[arg(short, long)]
        list: bool,

        /// Display diffs instead of rewriting files
        #[arg(short, long)]
        diff: bool,
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
        Command::Test {
            packages,
            run,
            verbose,
            keep_temp,
            args,
        } => {
            let cwd = std::env::current_dir().into_diagnostic()?;

            // If the first package argument is a directory with go.mod, use it as root
            let (root, packages) = if let Some(first) = packages.first() {
                let path = PathBuf::from(first);
                if path.is_dir() && path.join("go.mod").exists() {
                    // Use this directory as the project root
                    let root = if path.is_absolute() {
                        path
                    } else {
                        cwd.join(path)
                    };
                    (root, packages.into_iter().skip(1).collect())
                } else {
                    (cwd, packages)
                }
            } else {
                (cwd, packages)
            };

            let config = TestConfig {
                root,
                packages,
                run_pattern: run,
                verbose,
                keep_temp,
                passthrough_args: args,
            };

            soppo::test::run_tests(&config)?;
        }
        Command::Fmt {
            files,
            write,
            list,
            diff,
        } => {
            let cwd = std::env::current_dir().into_diagnostic()?;

            let resolved = if files.is_empty() {
                // No CLI args - try sop.mod
                let project = Project::discover(&cwd)?;
                if project.config.is_none() {
                    return Err(ConfigError::NoFilesSpecified.into());
                }
                project.find_sources()
            } else {
                resolve_globs(&files, &cwd)?
            };

            if resolved.is_empty() {
                return Err(ConfigError::NoFilesSpecified.into());
            }

            format_files(&resolved, write, list, diff)?;
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

/// Format files
fn format_files(files: &[PathBuf], write: bool, list: bool, diff: bool) -> Result<()> {
    let mut exit_code = 0;

    for file in files {
        let source = fs::read_to_string(file)
            .into_diagnostic()
            .map_err(|e| e.context(format!("Failed to read file: {}", file.display())))?;

        let formatted = format::format_source(&source)?;

        if source == formatted {
            // Already formatted
            continue;
        }

        if list {
            println!("{}", file.display());
            exit_code = 1;
        } else if diff {
            // Show diff using diff -u
            show_diff(file, &source, &formatted)?;
            exit_code = 1;
        } else if write {
            fs::write(file, &formatted)
                .into_diagnostic()
                .map_err(|e| e.context(format!("Failed to write file: {}", file.display())))?;
        } else {
            // Print to stdout
            print!("{}", formatted);
        }
    }

    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}

/// Show diff between original and formatted using diff -u
fn show_diff(file: &Path, original: &str, formatted: &str) -> Result<()> {
    use tempfile::NamedTempFile;

    // Create temp files for diff
    let mut orig_file = NamedTempFile::new().into_diagnostic()?;
    let mut fmt_file = NamedTempFile::new().into_diagnostic()?;

    orig_file.write_all(original.as_bytes()).into_diagnostic()?;
    fmt_file.write_all(formatted.as_bytes()).into_diagnostic()?;

    // Run diff -u
    let output = ProcessCommand::new("diff")
        .arg("-u")
        .arg("--label")
        .arg(format!("a/{}", file.display()))
        .arg("--label")
        .arg(format!("b/{}", file.display()))
        .arg(orig_file.path())
        .arg(fmt_file.path())
        .stdout(Stdio::piped())
        .output()
        .into_diagnostic()?;

    // Print diff output (diff returns exit code 1 when files differ, which is expected)
    print!("{}", String::from_utf8_lossy(&output.stdout));

    Ok(())
}
