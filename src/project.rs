use miette::{Diagnostic, Result};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Diagnostic, Debug)]
pub enum ProjectError {
    #[error("No go.mod found (searched from {0} to filesystem root)")]
    NoGoMod(PathBuf),

    #[error("Failed to read go.mod: {0}")]
    ReadGoMod(String),

    #[error("Invalid go.mod: missing module declaration")]
    InvalidGoMod,
}

pub struct Project {
    /// Directory containing go.mod
    pub root: PathBuf,
    /// Go module path (e.g., "github.com/user/project")
    pub module_path: String,
}

impl Project {
    /// Walk up from `start` to find go.mod and create Project
    pub fn discover(start: &Path) -> Result<Project> {
        let mut current = start.to_path_buf();

        loop {
            let go_mod = current.join("go.mod");
            if go_mod.exists() {
                let module_path = parse_go_mod(&go_mod)?;
                return Ok(Project {
                    root: current,
                    module_path,
                });
            }

            if !current.pop() {
                return Err(ProjectError::NoGoMod(start.to_path_buf()).into());
            }
        }
    }

    /// Find all .sop files in the project (recursive, excluding gen/, vendor/, testdata/, dotdirs)
    pub fn find_sources(&self) -> Vec<PathBuf> {
        let mut sources = Vec::new();
        find_sop_files(&self.root, &mut sources);
        sources.sort();
        sources
    }

    /// Map a source path to its output path under the output directory
    /// e.g., /project/cmd/app/main.sop -> /project/gen/cmd/app/main.go
    pub fn output_path(&self, source: &Path, output_dir: &Path) -> PathBuf {
        let relative = source
            .strip_prefix(&self.root)
            .expect("source should be under project root");

        let mut output = output_dir.join(relative);
        output.set_extension("go");
        output
    }
}

/// Parse go.mod to extract module path
fn parse_go_mod(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path).map_err(|e| ProjectError::ReadGoMod(e.to_string()))?;

    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("module") {
            let module_path = rest.trim();
            if !module_path.is_empty() {
                return Ok(module_path.to_string());
            }
        }
    }

    Err(ProjectError::InvalidGoMod.into())
}

/// Recursively find .sop files, excluding special directories
fn find_sop_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();

            // Skip excluded directories
            if name.starts_with('.') || name == "vendor" || name == "gen" || name == "testdata" {
                continue;
            }

            find_sop_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "sop") {
            files.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("soppo-test-{}-{}", std::process::id(), id));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_discover_finds_go_mod() {
        let root = temp_dir();
        let mut go_mod = File::create(root.join("go.mod")).unwrap();
        writeln!(go_mod, "module github.com/test/project").unwrap();

        let subdir = root.join("cmd/app");
        fs::create_dir_all(&subdir).unwrap();

        let project = Project::discover(&subdir).unwrap();
        assert_eq!(project.root, root);
        assert_eq!(project.module_path, "github.com/test/project");

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_discover_no_go_mod() {
        let root = temp_dir();
        let result = Project::discover(&root);
        assert!(result.is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_find_sources_excludes_special_dirs() {
        let root = temp_dir();
        fs::create_dir_all(root.join("cmd")).unwrap();
        fs::create_dir_all(root.join("gen")).unwrap();
        fs::create_dir_all(root.join("vendor")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("testdata")).unwrap();

        File::create(root.join("cmd/main.sop")).unwrap();
        File::create(root.join("gen/main.sop")).unwrap();
        File::create(root.join("vendor/lib.sop")).unwrap();
        File::create(root.join(".git/hooks.sop")).unwrap();
        File::create(root.join("testdata/fixture.sop")).unwrap();

        let mut go_mod = File::create(root.join("go.mod")).unwrap();
        writeln!(go_mod, "module test").unwrap();

        let project = Project::discover(&root).unwrap();
        let sources = project.find_sources();

        assert_eq!(sources.len(), 1);
        assert!(sources[0].ends_with("cmd/main.sop"));

        fs::remove_dir_all(&root).unwrap();
    }
}
