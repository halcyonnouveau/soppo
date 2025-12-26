use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

use clap::{Parser as ClapParser, Subcommand};
use miette::{IntoDiagnostic, NamedSource, Result};
use soppo::build;
use soppo::config::{ConfigError, SopConfig, resolve_globs};
use soppo::fmt;
use soppo::go::Project;
use soppo::sniff::{self, LintConfig};
use soppo::test::TestConfig;
use tempfile::NamedTempFile;

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

        /// Keep temp directory
        #[arg(long)]
        keep_tmp: bool,

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
    /// Checks source files to catch common mistakes and improve you Soppo code
    Sniff {
        /// Files or glob patterns to lint.
        /// If empty, uses sop.mod config or errors if no config exists.
        #[arg()]
        files: Vec<String>,

        /// Disable specific lint rules
        #[arg(long)]
        ignore: Vec<String>,
    },
}

fn main() -> Result<()> {
    // Check if sop.mod version requirements match current environment
    check_version_requirements()?;

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
            keep_tmp,
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
                keep_tmp,
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
        Command::Sniff { files, ignore } => {
            let cwd = std::env::current_dir().into_diagnostic()?;

            // Try to discover project, but don't fail if there's no go.mod
            let project = Project::discover(&cwd).ok();

            let resolved = if files.is_empty() {
                // No files provided - need project config
                let proj = project.as_ref().ok_or(ConfigError::NoFilesSpecified)?;
                if proj.config.is_none() {
                    return Err(ConfigError::NoFilesSpecified.into());
                }
                proj.find_sources()
            } else {
                resolve_globs(&files, &cwd)?
            };

            if resolved.is_empty() {
                return Err(ConfigError::NoFilesSpecified.into());
            }

            // Build lint config from sop.mod + CLI flags
            let mut ignored: std::collections::HashSet<String> = ignore.into_iter().collect();

            // Merge in ignored/disabled rules from sop.mod config
            if let Some(ref proj) = project
                && let Some(ref config) = proj.config
                && let Some(ref sniff_config) = config.sniff
                && let Some(ref config_ignored) = sniff_config.ignore
            {
                ignored.extend(config_ignored.iter().cloned());
            }

            let config = LintConfig { ignored };

            sniff_files(&resolved, &config)?;
        }
    }

    Ok(())
}

/// Build using sop.mod configuration
fn build_from_config(cwd: &std::path::Path, output: Option<PathBuf>) -> Result<()> {
    let project = Project::discover(cwd)?;

    let config = project
        .config
        .as_ref()
        .ok_or(ConfigError::NoFilesSpecified)?;

    // CLI --output overrides config output, otherwise use config's output
    let output_dir = output.or_else(|| config.output.clone());
    let count = build::build_project_to_disk(cwd, output_dir.as_deref())?;

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
    let checked = build::typecheck_project(cwd)?;

    if checked.is_empty() {
        println!("No .sop files found matching patterns");
        return Ok(());
    }

    for path in &checked {
        println!("  ✓ {}", path.display());
    }

    println!("✓ Checked {} file(s)", checked.len());
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

        let filename = file.display().to_string();
        let formatted = fmt::format_source(&source).map_err(|e| {
            miette::Report::from(e)
                .with_source_code(NamedSource::new(filename.clone(), source.clone()))
        })?;

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

/// Lint files for code quality issues
fn sniff_files(files: &[PathBuf], config: &LintConfig) -> Result<()> {
    // Typecheck all files together for proper cross-file resolution
    let typed_files = match build::typecheck_project_to_typed(files) {
        Ok(f) => f,
        Err(e) => {
            // If there are compile errors, report them
            eprintln!("{:?}", e);
            return Ok(());
        }
    };

    // Run lints on each typed file
    for (path, source, typed_file) in typed_files {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("input.sop");

        let warnings = sniff::lint_file(&typed_file, filename, &source, config);

        for warning in warnings {
            eprintln!("{:?}", miette::Report::new(warning));
        }
    }

    Ok(())
}

/// Check if sop.mod in current or parent directories has version requirements
/// that don't match the current environment.
fn check_version_requirements() -> Result<()> {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => return Ok(()), // Can't check, just continue
    };

    // Walk up to find sop.mod
    let mut current = cwd;
    loop {
        if let Ok(Some(config)) = SopConfig::load(&current) {
            config.check_version_requirements()?;
            return Ok(());
        }

        if !current.pop() {
            break;
        }
    }

    Ok(())
}
