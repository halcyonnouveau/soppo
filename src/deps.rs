use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use miette::{Result, miette};

use crate::syntax::{FileId, Parser};

/// Dependency graph for Soppo files based on local imports
#[derive(Debug)]
pub struct DepGraph {
    /// Map from file path to its local Soppo dependencies (resolved to file paths)
    edges: HashMap<PathBuf, Vec<PathBuf>>,
    /// All files in the graph
    files: HashSet<PathBuf>,
}

impl DepGraph {
    /// Build a dependency graph from a list of source files
    ///
    /// Parses each file to extract local Soppo imports and builds the graph.
    /// Local imports are identified by checking if the import path starts with
    /// the module path and corresponds to a local directory with .sop files.
    pub fn build(sources: &[PathBuf], project_root: &Path, module_path: &str) -> Result<Self> {
        let mut edges: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        let mut files: HashSet<PathBuf> = HashSet::new();

        for source_path in sources {
            files.insert(source_path.clone());

            // Parse just enough to get imports
            let source = std::fs::read_to_string(source_path)
                .map_err(|e| miette!("Failed to read {}: {}", source_path.display(), e))?;

            let mut parser = Parser::new(&source, FileId(0));
            let file = parser
                .parse_file()
                .map_err(|e| miette!("Failed to parse {}: {:?}", source_path.display(), e))?;

            // Extract local Soppo imports and resolve to package files
            let mut deps = Vec::new();
            for import in &file.imports {
                if let Some(local_path) = get_local_package_path(&import.path, module_path) {
                    // Check if this is actually a Soppo package (has .sop files)
                    if let Some(package_files) = resolve_local_package(local_path, project_root) {
                        deps.extend(package_files);
                    }
                    // If no .sop files, it's a local Go package - not a dependency for us
                }
            }

            edges.insert(source_path.clone(), deps);
        }

        Ok(Self { edges, files })
    }

    /// Topologically sort the files so dependencies come before dependents
    ///
    /// Returns an error if there's a circular dependency.
    pub fn topological_sort(&self) -> Result<Vec<PathBuf>> {
        // Kahn's algorithm
        let mut in_degree: HashMap<&PathBuf, usize> = HashMap::new();
        let mut reverse_edges: HashMap<&PathBuf, Vec<&PathBuf>> = HashMap::new();

        // Initialise in-degree for all files
        for file in &self.files {
            in_degree.insert(file, 0);
            reverse_edges.insert(file, Vec::new());
        }

        // Build reverse edges and calculate in-degrees
        for (file, deps) in &self.edges {
            for dep in deps {
                if self.files.contains(dep) {
                    *in_degree.get_mut(file).unwrap() += 1;
                    reverse_edges.get_mut(dep).unwrap().push(file);
                }
            }
        }

        // Start with files that have no dependencies
        let mut queue: VecDeque<&PathBuf> = VecDeque::new();
        for (file, &degree) in &in_degree {
            if degree == 0 {
                queue.push_back(*file);
            }
        }

        let mut result = Vec::new();
        while let Some(file) = queue.pop_front() {
            result.push(file.clone());

            // Reduce in-degree of dependents
            for dependent in &reverse_edges[file] {
                let degree = in_degree.get_mut(*dependent).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(*dependent);
                }
            }
        }

        // Check for cycles
        if result.len() != self.files.len() {
            let remaining: Vec<_> = self
                .files
                .iter()
                .filter(|f| !result.contains(f))
                .map(|f| f.display().to_string())
                .collect();

            return Err(miette!(
                "Circular dependency detected involving: {}",
                remaining.join(", ")
            ));
        }

        Ok(result)
    }

    /// Check if a file has any sop: dependencies
    pub fn has_sop_deps(&self, file: &Path) -> bool {
        self.edges.get(file).is_some_and(|deps| !deps.is_empty())
    }

    /// Get sop: dependencies for a file
    pub fn get_deps(&self, file: &Path) -> Option<&Vec<PathBuf>> {
        self.edges.get(file)
    }
}

/// Check if an import path is a local package (starts with module path)
/// Returns the local path portion if it is, None otherwise.
///
/// Example: "github.com/user/project/helpers" with module "github.com/user/project"
/// returns Some("helpers")
pub fn get_local_package_path<'a>(import_path: &'a str, module_path: &str) -> Option<&'a str> {
    if let Some(remainder) = import_path.strip_prefix(module_path) {
        // Strip leading slash if present
        let local_path = remainder.strip_prefix('/').unwrap_or(remainder);
        if local_path.is_empty() {
            None
        } else {
            Some(local_path)
        }
    } else {
        None
    }
}

/// Check if a local path corresponds to a Soppo package (directory with .sop files)
/// Returns the list of .sop files if it is, None otherwise.
pub fn resolve_local_package(local_path: &str, project_root: &Path) -> Option<Vec<PathBuf>> {
    let package_dir = project_root.join(local_path);

    if !package_dir.is_dir() {
        return None;
    }

    let mut files = Vec::new();
    let entries = std::fs::read_dir(&package_dir).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "sop") {
            files.push(path);
        }
    }

    if files.is_empty() { None } else { Some(files) }
}

/// Check if an import is a local Soppo package
pub fn is_soppo_import(import_path: &str, module_path: &str, project_root: &Path) -> bool {
    if let Some(local_path) = get_local_package_path(import_path, module_path) {
        resolve_local_package(local_path, project_root).is_some()
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    const TEST_MODULE: &str = "github.com/test/myproject";

    fn create_test_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_get_local_package_path() {
        assert_eq!(
            get_local_package_path(
                "github.com/test/myproject/helpers",
                "github.com/test/myproject"
            ),
            Some("helpers")
        );
        assert_eq!(
            get_local_package_path(
                "github.com/test/myproject/util/helpers",
                "github.com/test/myproject"
            ),
            Some("util/helpers")
        );
        assert_eq!(
            get_local_package_path("fmt", "github.com/test/myproject"),
            None
        );
        assert_eq!(
            get_local_package_path("github.com/other/lib", "github.com/test/myproject"),
            None
        );
    }

    #[test]
    fn test_no_deps() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let a = create_test_file(root, "a.sop", "func main() {}");
        let b = create_test_file(root, "b.sop", "func foo() {}");

        let graph = DepGraph::build(&[a.clone(), b.clone()], root, TEST_MODULE).unwrap();
        let sorted = graph.topological_sort().unwrap();

        assert_eq!(sorted.len(), 2);
    }

    #[test]
    fn test_simple_deps() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // main depends on package a
        let a = create_test_file(root, "a/lib.sop", "package a\nfunc helper() {}");
        let main = create_test_file(
            root,
            "main/main.sop",
            &format!(
                r#"package main
import "{}/a"
func main() {{}}"#,
                TEST_MODULE
            ),
        );

        let graph = DepGraph::build(&[a.clone(), main.clone()], root, TEST_MODULE).unwrap();
        let sorted = graph.topological_sort().unwrap();

        let a_idx = sorted.iter().position(|p| p == &a).unwrap();
        let main_idx = sorted.iter().position(|p| p == &main).unwrap();
        assert!(a_idx < main_idx);
    }

    #[test]
    fn test_chain_deps() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // c -> b -> a (each in its own package directory)
        let a = create_test_file(root, "a/lib.sop", "package a\nfunc A() {}");
        let b = create_test_file(
            root,
            "b/lib.sop",
            &format!(
                r#"package b
import "{}/a"
func B() {{}}"#,
                TEST_MODULE
            ),
        );
        let c = create_test_file(
            root,
            "c/lib.sop",
            &format!(
                r#"package c
import "{}/b"
func C() {{}}"#,
                TEST_MODULE
            ),
        );

        let graph = DepGraph::build(&[a.clone(), b.clone(), c.clone()], root, TEST_MODULE).unwrap();
        let sorted = graph.topological_sort().unwrap();

        let a_idx = sorted.iter().position(|p| p == &a).unwrap();
        let b_idx = sorted.iter().position(|p| p == &b).unwrap();
        let c_idx = sorted.iter().position(|p| p == &c).unwrap();

        assert!(a_idx < b_idx);
        assert!(b_idx < c_idx);
    }

    #[test]
    fn test_circular_dep_detection() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // a -> b -> a (circular)
        let a = create_test_file(
            root,
            "a/lib.sop",
            &format!(
                r#"package a
import "{}/b"
func A() {{}}"#,
                TEST_MODULE
            ),
        );
        let b = create_test_file(
            root,
            "b/lib.sop",
            &format!(
                r#"package b
import "{}/a"
func B() {{}}"#,
                TEST_MODULE
            ),
        );

        let graph = DepGraph::build(&[a, b], root, TEST_MODULE).unwrap();
        let result = graph.topological_sort();

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Circular dependency"));
    }

    #[test]
    fn test_missing_local_import_is_ignored() {
        // If a local import path doesn't have .sop files, it's treated as Go
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let a = create_test_file(
            root,
            "a/lib.sop",
            &format!(
                r#"package a
import "{}/nonexistent"
func A() {{}}"#,
                TEST_MODULE
            ),
        );

        // This should succeed - nonexistent is treated as a Go package
        let result = DepGraph::build(&[a], root, TEST_MODULE);
        assert!(result.is_ok());
    }

    #[test]
    fn test_nested_path() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let helper = create_test_file(
            root,
            "util/helpers/lib.sop",
            "package helpers\nfunc Help() {}",
        );
        let main = create_test_file(
            root,
            "main/main.sop",
            &format!(
                r#"package main
import "{}/util/helpers"
func main() {{}}"#,
                TEST_MODULE
            ),
        );

        let graph = DepGraph::build(&[helper.clone(), main.clone()], root, TEST_MODULE).unwrap();
        let sorted = graph.topological_sort().unwrap();

        let helper_idx = sorted.iter().position(|p| p == &helper).unwrap();
        let main_idx = sorted.iter().position(|p| p == &main).unwrap();
        assert!(helper_idx < main_idx);
    }

    #[test]
    fn test_go_imports_ignored() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let main = create_test_file(
            root,
            "main/main.sop",
            r#"package main
import "fmt"
import "github.com/other/lib"
func main() {}"#,
        );

        let graph = DepGraph::build(&[main.clone()], root, TEST_MODULE).unwrap();
        // Should have no dependencies (Go imports are ignored)
        assert!(!graph.has_sop_deps(&main));
    }
}
