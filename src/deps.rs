use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use miette::{Result, miette};

use crate::syntax::{FileId, Parser};

/// Dependency graph for Soppo files based on sop: imports
#[derive(Debug)]
pub struct DepGraph {
    /// Map from file path to its sop: dependencies (resolved to file paths)
    edges: HashMap<PathBuf, Vec<PathBuf>>,
    /// All files in the graph
    files: HashSet<PathBuf>,
}

impl DepGraph {
    /// Build a dependency graph from a list of source files
    ///
    /// Parses each file to extract sop: imports and builds the graph.
    pub fn build(sources: &[PathBuf], project_root: &Path) -> Result<Self> {
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

            // Extract sop: imports and resolve to package files
            let mut deps = Vec::new();
            for import in &file.imports {
                if import.path.starts_with("sop:") {
                    let sop_path = &import.path[4..]; // Strip "sop:"
                    let package_files = resolve_sop_package(sop_path, project_root)?;
                    deps.extend(package_files);
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

/// Resolve a sop: import path to source files
///
/// Like Go, imports are package-based (directory-based):
/// `sop:mathutil` -> all `.sop` files in `<project_root>/mathutil/`
fn resolve_sop_package(sop_path: &str, project_root: &Path) -> Result<Vec<PathBuf>> {
    let package_dir = project_root.join(sop_path);

    if !package_dir.is_dir() {
        return Err(miette!(
            "Soppo package not found: sop:{} (expected directory at {})",
            sop_path,
            package_dir.display()
        ));
    }

    let mut files = Vec::new();
    for entry in std::fs::read_dir(&package_dir).map_err(|e| {
        miette!(
            "Failed to read package directory {}: {}",
            package_dir.display(),
            e
        )
    })? {
        let entry = entry.map_err(|e| miette!("Failed to read entry: {}", e))?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "sop") {
            files.push(path);
        }
    }

    if files.is_empty() {
        return Err(miette!("Soppo package {} contains no .sop files", sop_path));
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn create_test_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_no_deps() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let a = create_test_file(root, "a.sop", "func main() {}");
        let b = create_test_file(root, "b.sop", "func foo() {}");

        let graph = DepGraph::build(&[a.clone(), b.clone()], root).unwrap();
        let sorted = graph.topological_sort().unwrap();

        assert_eq!(sorted.len(), 2);
        // Both files have no deps, order doesn't matter
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
            r#"package main
import "sop:a"
func main() {}"#,
        );

        let graph = DepGraph::build(&[a.clone(), main.clone()], root).unwrap();
        let sorted = graph.topological_sort().unwrap();

        // a must come before main
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
            r#"package b
import "sop:a"
func B() {}"#,
        );
        let c = create_test_file(
            root,
            "c/lib.sop",
            r#"package c
import "sop:b"
func C() {}"#,
        );

        let graph = DepGraph::build(&[a.clone(), b.clone(), c.clone()], root).unwrap();
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

        // a -> b -> a (circular, each in its own package directory)
        let a = create_test_file(
            root,
            "a/lib.sop",
            r#"package a
import "sop:b"
func A() {}"#,
        );
        let b = create_test_file(
            root,
            "b/lib.sop",
            r#"package b
import "sop:a"
func B() {}"#,
        );

        let graph = DepGraph::build(&[a, b], root).unwrap();
        let result = graph.topological_sort();

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Circular dependency"));
    }

    #[test]
    fn test_missing_dep() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // a depends on nonexistent package
        let a = create_test_file(
            root,
            "a/lib.sop",
            r#"package a
import "sop:nonexistent"
func A() {}"#,
        );

        let result = DepGraph::build(&[a], root);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
    }

    #[test]
    fn test_nested_path() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Nested package: util/helpers is a package at util/helpers/
        let helper = create_test_file(
            root,
            "util/helpers/lib.sop",
            "package helpers\nfunc Help() {}",
        );
        let main = create_test_file(
            root,
            "main/main.sop",
            r#"package main
import "sop:util/helpers"
func main() {}"#,
        );

        let graph = DepGraph::build(&[helper.clone(), main.clone()], root).unwrap();
        let sorted = graph.topological_sort().unwrap();

        let helper_idx = sorted.iter().position(|p| p == &helper).unwrap();
        let main_idx = sorted.iter().position(|p| p == &main).unwrap();
        assert!(helper_idx < main_idx);
    }
}
