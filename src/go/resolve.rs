use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use miette::{Diagnostic, Result};
use serde::Deserialize;
use thiserror::Error;

use super::project::Project;

/// Cached Go environment variables.
static GO_ENV: OnceLock<GoEnv> = OnceLock::new();

struct GoEnv {
    goroot: PathBuf,
}

fn get_cached_go_env() -> Result<&'static GoEnv> {
    if let Some(env) = GO_ENV.get() {
        return Ok(env);
    }

    let goroot = get_go_env("GOROOT")?;

    let _ = GO_ENV.set(GoEnv {
        goroot: PathBuf::from(goroot),
    });

    Ok(GO_ENV.get().unwrap())
}

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
}

impl Resolver {
    /// Create a new resolver using cached Go environment
    pub fn new() -> Result<Self> {
        let env = get_cached_go_env()?;
        Ok(Self {
            goroot: env.goroot.clone(),
        })
    }

    /// Resolve an import path to its kind and source location
    ///
    /// - Stdlib packages (fmt, strings, etc.) work without a project
    /// - External modules require a project (go.mod) to resolve versions
    pub fn resolve(&self, import_path: &str, project: Option<&Project>) -> Result<ImportKind> {
        // Check for local Soppo package first (requires project)
        if let Some(proj) = project
            && let Some(kind) = self.resolve_local_soppo(import_path, proj)
        {
            return Ok(kind);
        }

        // Check for local Go package (requires project)
        if let Some(proj) = project
            && let Some(kind) = self.resolve_local_go(import_path, proj)
        {
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

    /// Check if import is a local Go package
    ///
    /// Import path like "github.com/user/project/pkg"
    /// maps to source at "pkg/*.go"
    fn resolve_local_go(&self, import_path: &str, project: &Project) -> Option<ImportKind> {
        // Check if import starts with our module path + "/"
        let module_prefix = format!("{}/", project.module_path);

        if let Some(rest) = import_path.strip_prefix(&module_prefix) {
            // Map module/pkg -> pkg
            let source_dir = project.root.join(rest);
            // Check if there are .go files there
            if source_dir.is_dir() && has_files_with_extension(&source_dir, "go") {
                return Some(ImportKind::ExternalGo {
                    import_path: import_path.to_string(),
                    source_dir,
                });
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

    /// Resolve an external Go package
    fn resolve_external(&self, import_path: &str, project: &Project) -> Result<ImportKind> {
        // Use `go list -json` to get package info (works for subpackages too)
        let pkg_info = get_package_info(import_path, &project.root)?;
        let source_dir = PathBuf::from(&pkg_info.dir);

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

/// Package info from `go list -json`
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PackageInfo {
    /// Source directory
    dir: String,
}

/// Get package info using `go list -json`
fn get_package_info(import_path: &str, project_root: &Path) -> Result<PackageInfo> {
    let output = Command::new("go")
        .args(["list", "-json", import_path])
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
