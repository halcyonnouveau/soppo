use crate::project::Project;
use miette::{Diagnostic, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[derive(Error, Diagnostic, Debug)]
pub enum ResolveError {
    #[error("Failed to run `go env`: {0}")]
    GoEnv(String),

    #[error("Failed to resolve module {0}: {1}")]
    ModuleResolution(String, String),

    #[error("Module not found in cache: {0}")]
    ModuleNotInCache(String),
}

/// The kind of import and where to find its source
#[derive(Debug, Clone, PartialEq)]
pub enum ImportKind {
    /// Local Soppo package in this project
    LocalSoppo {
        /// Path to directory containing .sop files
        source_dir: PathBuf,
    },
    /// Go standard library package
    GoStdlib {
        /// Package path (e.g., "fmt", "net/http")
        package: String,
        /// Path to source directory
        source_dir: PathBuf,
    },
    /// External Go package (from module cache)
    ExternalGo {
        /// Full import path
        import_path: String,
        /// Path to source directory in module cache
        source_dir: PathBuf,
    },
}

/// Resolver for import paths
pub struct Resolver {
    goroot: PathBuf,
    gomodcache: PathBuf,
}

impl Resolver {
    /// Create a new resolver by querying Go environment
    pub fn new() -> Result<Self> {
        let goroot = get_go_env("GOROOT")?;
        let gomodcache = get_go_env("GOMODCACHE")?;

        Ok(Self {
            goroot: PathBuf::from(goroot),
            gomodcache: PathBuf::from(gomodcache),
        })
    }

    /// Resolve an import path to its kind and source location
    ///
    /// - Stdlib packages (fmt, strings, etc.) work without a project
    /// - External modules require a project (go.mod) to resolve versions
    pub fn resolve(&self, import_path: &str, project: Option<&Project>) -> Result<ImportKind> {
        // Check for local Soppo package first (requires project)
        if let Some(proj) = project
            && let Some(kind) = self.resolve_local_soppo(import_path, proj) {
                return Ok(kind);
            }

        // Check stdlib (no project needed)
        if let Some(kind) = self.resolve_stdlib(import_path) {
            return Ok(kind);
        }

        // External modules require a project for version resolution
        if let Some(proj) = project {
            self.resolve_external(import_path, proj)
        } else {
            Err(ResolveError::ModuleResolution(
                import_path.to_string(),
                "External modules require a go.mod (no project context available)".to_string(),
            )
            .into())
        }
    }

    /// Check if import is a local Soppo package
    ///
    /// Import path like "github.com/user/project/gen/pkg/util"
    /// maps to source at "pkg/util/*.sop"
    fn resolve_local_soppo(&self, import_path: &str, project: &Project) -> Option<ImportKind> {
        // Check if import starts with our module path + "/gen/"
        let gen_prefix = format!("{}/gen/", project.module_path);

        if let Some(rest) = import_path.strip_prefix(&gen_prefix) {
            // Map gen/pkg/util -> pkg/util
            let source_dir = project.root.join(rest);
            // Check if there are .sop files there
            if source_dir.is_dir() && has_files_with_extension(&source_dir, "sop") {
                return Some(ImportKind::LocalSoppo { source_dir });
            }
        }

        None
    }

    /// Check if import is a Go stdlib package
    fn resolve_stdlib(&self, import_path: &str) -> Option<ImportKind> {
        let source_dir = self.goroot.join("src").join(import_path);

        if source_dir.is_dir() && has_files_with_extension(&source_dir, "go") {
            return Some(ImportKind::GoStdlib {
                package: import_path.to_string(),
                source_dir,
            });
        }

        None
    }

    /// Resolve an external Go module
    fn resolve_external(&self, import_path: &str, project: &Project) -> Result<ImportKind> {
        // Use `go list` to get module info
        let module_info = get_module_info(import_path, &project.root)?;

        // Module path might be a prefix of import path
        // e.g., import "github.com/foo/bar/pkg" where module is "github.com/foo/bar"
        let subpath = import_path
            .strip_prefix(&module_info.path)
            .unwrap_or("")
            .trim_start_matches('/');

        // Build path in module cache: $GOMODCACHE/module@version/subpath
        let cache_path = self
            .gomodcache
            .join(encode_module_path(&module_info.path))
            .join(format!("@{}", module_info.version));

        let source_dir = if subpath.is_empty() {
            cache_path
        } else {
            cache_path.join(subpath)
        };

        if !source_dir.exists() {
            return Err(ResolveError::ModuleNotInCache(import_path.to_string()).into());
        }

        Ok(ImportKind::ExternalGo {
            import_path: import_path.to_string(),
            source_dir,
        })
    }
}

/// Get a Go environment variable
fn get_go_env(name: &str) -> Result<String> {
    let output = Command::new("go")
        .args(["env", name])
        .output()
        .map_err(|e| ResolveError::GoEnv(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ResolveError::GoEnv(stderr.to_string()).into());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Module info from `go list -m -json`
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ModuleInfo {
    path: String,
    version: String,
}

/// Get module info using `go list -m -json`
fn get_module_info(import_path: &str, project_root: &Path) -> Result<ModuleInfo> {
    let output = Command::new("go")
        .args(["list", "-m", "-json", import_path])
        .current_dir(project_root)
        .output()
        .map_err(|e| ResolveError::ModuleResolution(import_path.to_string(), e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(
            ResolveError::ModuleResolution(import_path.to_string(), stderr.to_string()).into(),
        );
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|e| ResolveError::ModuleResolution(import_path.to_string(), e.to_string()).into())
}

/// Encode module path for filesystem (handles uppercase)
/// e.g., "github.com/BurntSushi/toml" -> "github.com/!burnt!sushi/toml"
fn encode_module_path(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    for c in path.chars() {
        if c.is_uppercase() {
            result.push('!');
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }
    result
}

/// Check if directory contains files with the given extension
fn has_files_with_extension(dir: &Path, ext: &str) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.path().extension().is_some_and(|x| x == ext))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_module_path() {
        assert_eq!(
            encode_module_path("github.com/BurntSushi/toml"),
            "github.com/!burnt!sushi/toml"
        );
        assert_eq!(
            encode_module_path("github.com/user/repo"),
            "github.com/user/repo"
        );
    }

    #[test]
    fn test_resolver_stdlib() {
        let resolver = Resolver::new().unwrap();

        // "fmt" should be stdlib
        let kind = resolver.resolve_stdlib("fmt");
        assert!(matches!(kind, Some(ImportKind::GoStdlib { package, .. }) if package == "fmt"));

        // "net/http" should be stdlib
        let kind = resolver.resolve_stdlib("net/http");
        assert!(
            matches!(kind, Some(ImportKind::GoStdlib { package, .. }) if package == "net/http")
        );

        // Random path should not be stdlib
        let kind = resolver.resolve_stdlib("github.com/foo/bar");
        assert!(kind.is_none());
    }
}
