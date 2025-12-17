use std::fs;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use miette::Diagnostic;
use serde::Deserialize;
use thiserror::Error;

#[derive(Error, Diagnostic, Debug)]
pub enum ConfigError {
    #[error("Failed to read sop.mod: {0}")]
    ReadFailed(String),

    #[error("Failed to parse sop.mod: {0}")]
    ParseFailed(String),

    #[error("Invalid glob pattern '{pattern}': {reason}")]
    InvalidGlob { pattern: String, reason: String },

    #[error("No files to compile")]
    #[diagnostic(
        code(soppo::no_files),
        help("Provide files/globs as arguments, or create a sop.mod file with include patterns")
    )]
    NoFilesSpecified,
}

/// Raw config as parsed from sop.mod TOML
#[derive(Debug, Deserialize, Default)]
pub struct SopModRaw {
    pub include: Option<Vec<String>>,
    pub exclude: Option<Vec<String>>,
    pub output: Option<PathBuf>,
}

/// Processed config with compiled glob patterns
#[derive(Debug, Clone)]
pub struct SopConfig {
    /// Compiled include patterns
    include: GlobSet,
    /// Compiled exclude patterns
    exclude: GlobSet,
    /// Optional output directory
    pub output: Option<PathBuf>,
    /// Directory containing sop.mod (patterns are relative to this)
    pub config_dir: PathBuf,
}

impl SopConfig {
    /// Load config from sop.mod in the given directory.
    /// Returns Ok(None) if sop.mod doesn't exist.
    /// Returns Ok(Some(config)) if sop.mod exists and is valid.
    pub fn load(dir: &Path) -> Result<Option<Self>, ConfigError> {
        let config_path = dir.join("sop.mod");

        if !config_path.exists() {
            return Ok(None);
        }

        let content =
            fs::read_to_string(&config_path).map_err(|e| ConfigError::ReadFailed(e.to_string()))?;

        let raw: SopModRaw =
            toml::from_str(&content).map_err(|e| ConfigError::ParseFailed(e.to_string()))?;

        // Default to **/*.sop if no include patterns specified
        let include_patterns = raw.include.unwrap_or_else(|| vec!["**/*.sop".to_string()]);
        let exclude_patterns = raw.exclude.unwrap_or_default();

        let include = compile_globs(&include_patterns)?;
        let exclude = compile_globs(&exclude_patterns)?;

        Ok(Some(SopConfig {
            include,
            exclude,
            output: raw.output,
            config_dir: dir.to_path_buf(),
        }))
    }

    /// Find files matching include patterns, excluding those matching exclude patterns.
    pub fn find_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        find_matching_files(
            &self.config_dir,
            &self.config_dir,
            &self.include,
            &self.exclude,
            &mut files,
        );
        files.sort();
        files
    }
}

/// Compile glob patterns from strings to a GlobSet
fn compile_globs(patterns: &[String]) -> Result<GlobSet, ConfigError> {
    let mut builder = GlobSetBuilder::new();

    for pattern in patterns {
        let glob = Glob::new(pattern).map_err(|e| ConfigError::InvalidGlob {
            pattern: pattern.clone(),
            reason: e.to_string(),
        })?;
        builder.add(glob);
    }

    builder.build().map_err(|e| ConfigError::InvalidGlob {
        pattern: "<combined>".to_string(),
        reason: e.to_string(),
    })
}

/// Recursively find files matching include patterns, excluding those matching exclude patterns
fn find_matching_files(
    dir: &Path,
    base: &Path,
    include: &GlobSet,
    exclude: &GlobSet,
    files: &mut Vec<PathBuf>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();

            // Skip hidden directories
            if name.starts_with('.') {
                continue;
            }

            find_matching_files(&path, base, include, exclude, files);
        } else if path.extension().is_some_and(|ext| ext == "sop") {
            // Get relative path for glob matching
            let relative = path.strip_prefix(base).unwrap_or(&path);

            // Check include patterns
            if !include.is_match(relative) {
                continue;
            }

            // Check exclude patterns
            if exclude.is_match(relative) {
                continue;
            }

            files.push(path);
        }
    }
}

/// Resolve glob patterns from CLI arguments to file paths.
/// Patterns are resolved relative to the given base directory.
pub fn resolve_globs(patterns: &[String], base: &Path) -> Result<Vec<PathBuf>, ConfigError> {
    let mut files = Vec::new();

    for pattern in patterns {
        // Check if this looks like a glob pattern
        if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
            // Build a GlobSet for this pattern
            let glob = Glob::new(pattern).map_err(|e| ConfigError::InvalidGlob {
                pattern: pattern.clone(),
                reason: e.to_string(),
            })?;
            let globset =
                GlobSetBuilder::new()
                    .add(glob)
                    .build()
                    .map_err(|e| ConfigError::InvalidGlob {
                        pattern: pattern.clone(),
                        reason: e.to_string(),
                    })?;

            // Find matching files
            let empty_exclude = GlobSetBuilder::new().build().unwrap();
            find_matching_files(base, base, &globset, &empty_exclude, &mut files);
        } else {
            // Treat as literal path
            let path = if Path::new(pattern).is_absolute() {
                PathBuf::from(pattern)
            } else {
                base.join(pattern)
            };

            if path.exists() && path.extension().is_some_and(|ext| ext == "sop") {
                files.push(path);
            }
        }
    }

    files.sort();
    files.dedup();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("soppo-config-test-{}-{}", std::process::id(), id));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_load_no_config() {
        let root = temp_dir();
        let config = SopConfig::load(&root).unwrap();
        assert!(config.is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_load_empty_config() {
        let root = temp_dir();
        File::create(root.join("sop.mod")).unwrap();

        let config = SopConfig::load(&root).unwrap().unwrap();
        // Should default to **/*.sop
        assert!(config.output.is_none());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_load_with_patterns() {
        let root = temp_dir();
        let mut config_file = File::create(root.join("sop.mod")).unwrap();
        writeln!(
            config_file,
            r#"
include = ["src/**/*.sop"]
exclude = ["testdata/**"]
output = "gen"
"#
        )
        .unwrap();

        let config = SopConfig::load(&root).unwrap().unwrap();
        assert_eq!(config.output, Some(PathBuf::from("gen")));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_find_files_default_pattern() {
        let root = temp_dir();

        // Create sop.mod with default patterns
        File::create(root.join("sop.mod")).unwrap();

        // Create some .sop files
        fs::create_dir_all(root.join("src")).unwrap();
        File::create(root.join("main.sop")).unwrap();
        File::create(root.join("src/lib.sop")).unwrap();

        let config = SopConfig::load(&root).unwrap().unwrap();
        let files = config.find_files();

        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|p| p.ends_with("main.sop")));
        assert!(files.iter().any(|p| p.ends_with("src/lib.sop")));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_find_files_with_exclude() {
        let root = temp_dir();

        // Create sop.mod excluding testdata
        let mut config_file = File::create(root.join("sop.mod")).unwrap();
        writeln!(
            config_file,
            r#"
include = ["**/*.sop"]
exclude = ["testdata/**"]
"#
        )
        .unwrap();

        // Create some .sop files
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("testdata")).unwrap();
        File::create(root.join("src/lib.sop")).unwrap();
        File::create(root.join("testdata/fixture.sop")).unwrap();

        let config = SopConfig::load(&root).unwrap().unwrap();
        let files = config.find_files();

        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("src/lib.sop"));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_resolve_globs_literal_path() {
        let root = temp_dir();

        File::create(root.join("main.sop")).unwrap();

        let files = resolve_globs(&["main.sop".to_string()], &root).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("main.sop"));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_resolve_globs_pattern() {
        let root = temp_dir();

        fs::create_dir_all(root.join("src")).unwrap();
        File::create(root.join("main.sop")).unwrap();
        File::create(root.join("src/lib.sop")).unwrap();

        let files = resolve_globs(&["**/*.sop".to_string()], &root).unwrap();
        assert_eq!(files.len(), 2);

        fs::remove_dir_all(&root).unwrap();
    }
}
