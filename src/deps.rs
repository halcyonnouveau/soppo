use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use miette::{NamedSource, Result};

use crate::syntax::{FileId, Parser};

/// An import edge: source file imports a target via an import path
#[derive(Debug, Clone)]
struct ImportEdge {
    /// The resolved target file(s)
    targets: Vec<PathBuf>,
    /// The original import path (e.g., "github.com/test/pkg")
    import_path: String,
}

/// Dependency graph for Soppo files based on local imports
#[derive(Debug)]
pub struct DepGraph {
    /// Map from file path to its imports (with import paths preserved)
    imports: HashMap<PathBuf, Vec<ImportEdge>>,
    /// Flattened edges for topological sort (file -> all dependency files)
    edges: HashMap<PathBuf, Vec<PathBuf>>,
    /// All files in the graph
    files: HashSet<PathBuf>,
    /// Project root for making paths relative in error messages
    project_root: PathBuf,
}

impl DepGraph {
    /// Build a dependency graph from a list of source files
    ///
    /// Parses each file to extract local Soppo imports and builds the graph.
    /// Local imports are identified by checking if the import path starts with
    /// the module path and corresponds to a local directory with .sop files.
    ///
    /// Test files (`*_test.sop`) in the same package implicitly depend on
    /// all non-test files in their package, ensuring they're processed after
    /// the package's symbols are available.
    pub fn build(sources: &[PathBuf], project_root: &Path, module_path: &str) -> Result<Self> {
        let mut imports: HashMap<PathBuf, Vec<ImportEdge>> = HashMap::new();
        let mut edges: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        let mut files: HashSet<PathBuf> = HashSet::new();
        let mut file_packages: HashMap<PathBuf, String> = HashMap::new();

        // First pass: parse all files to get their packages
        for source_path in sources {
            files.insert(source_path.clone());

            let source = std::fs::read_to_string(source_path)
                .map_err(|e| miette::miette!("Failed to read {}: {}", source_path.display(), e))?;

            let filename = source_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| source_path.display().to_string());

            let mut parser = Parser::new(&source, FileId(0));
            let file = parser.parse_file().map_err(|e| {
                miette::Report::from(e).with_source_code(NamedSource::new(filename, source.clone()))
            })?;

            file_packages.insert(source_path.clone(), file.package.name.clone());

            // Extract local Soppo imports and resolve to package files
            let mut import_edges = Vec::new();
            let mut all_deps = Vec::new();
            for import in &file.imports {
                if let Some(local_path) = get_local_package_path(&import.path, module_path) {
                    // Check if this is actually a Soppo package (has .sop files)
                    if let Some(package_files) = resolve_local_package(local_path, project_root) {
                        import_edges.push(ImportEdge {
                            targets: package_files.clone(),
                            import_path: import.path.clone(),
                        });
                        all_deps.extend(package_files);
                    }
                    // If no .sop files, it's a local Go package - not a dependency for us
                }
            }

            imports.insert(source_path.clone(), import_edges);
            edges.insert(source_path.clone(), all_deps);
        }

        // Second pass: add implicit dependencies for test files
        // Test files depend on all non-test files in the same package
        for source_path in sources {
            let filename = source_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            if filename.ends_with("_test.sop") {
                let test_package = file_packages.get(source_path).cloned().unwrap_or_default();
                let test_dir = source_path.parent();

                // Find all non-test files in the same directory with the same package
                for other_path in sources {
                    if other_path == source_path {
                        continue;
                    }

                    let other_filename = other_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");

                    // Skip other test files
                    if other_filename.ends_with("_test.sop") {
                        continue;
                    }

                    // Check same directory
                    if other_path.parent() != test_dir {
                        continue;
                    }

                    // Check same package
                    let other_package = file_packages.get(other_path).cloned().unwrap_or_default();
                    if other_package == test_package {
                        // Add implicit dependency
                        edges.get_mut(source_path).unwrap().push(other_path.clone());
                    }
                }
            }
        }

        Ok(Self {
            imports,
            edges,
            files,
            project_root: project_root.to_path_buf(),
        })
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
            let cycle = self.find_cycle(&result);
            return Err(crate::error::SoppoError::CircularDependency { cycle }.into());
        }

        Ok(result)
    }

    /// Find a cycle in the dependency graph, returning (source_file, import_path) pairs
    fn find_cycle(&self, processed: &[PathBuf]) -> Vec<(String, String)> {
        // Get files that weren't processed (part of a cycle)
        let remaining: HashSet<_> = self
            .files
            .iter()
            .filter(|f| !processed.contains(f))
            .collect();

        if remaining.is_empty() {
            return Vec::new();
        }

        // Pick a deterministic starting file (sorted order) and trace the cycle using DFS
        let mut remaining_sorted: Vec<_> = remaining.iter().copied().collect();
        remaining_sorted.sort();
        let start = remaining_sorted[0];
        let mut visited: HashSet<&PathBuf> = HashSet::new();
        let mut path: Vec<&PathBuf> = Vec::new();

        if let Some(cycle_start_idx) =
            self.dfs_find_cycle(start, &remaining, &mut visited, &mut path)
        {
            // Extract the cycle portion of the path
            let cycle_path = &path[cycle_start_idx..];

            // Build the result with import paths
            let mut result = Vec::new();
            for i in 0..cycle_path.len() {
                let source = cycle_path[i];
                let target = cycle_path[(i + 1) % cycle_path.len()];

                // Find the import path from source to target
                let import_path = self
                    .imports
                    .get(source)
                    .and_then(|edges| {
                        edges
                            .iter()
                            .find(|e| e.targets.contains(target))
                            .map(|e| e.import_path.clone())
                    })
                    .unwrap_or_else(|| "unknown".to_string());

                let source_rel = source
                    .strip_prefix(&self.project_root)
                    .unwrap_or(source)
                    .display()
                    .to_string();

                result.push((source_rel, import_path));
            }
            result
        } else {
            // Fallback: just list remaining files
            remaining
                .iter()
                .map(|f| {
                    let rel = f
                        .strip_prefix(&self.project_root)
                        .unwrap_or(f)
                        .display()
                        .to_string();
                    (rel, "unknown".to_string())
                })
                .collect()
        }
    }

    /// DFS to find a cycle, returns the index in path where the cycle starts
    fn dfs_find_cycle<'a>(
        &'a self,
        node: &'a PathBuf,
        remaining: &HashSet<&'a PathBuf>,
        visited: &mut HashSet<&'a PathBuf>,
        path: &mut Vec<&'a PathBuf>,
    ) -> Option<usize> {
        // Check if we've found a cycle
        if let Some(idx) = path.iter().position(|&p| p == node) {
            return Some(idx);
        }

        // Skip if already fully processed or not in remaining set
        if visited.contains(&node) || !remaining.contains(&node) {
            return None;
        }

        visited.insert(node);
        path.push(node);

        // Follow dependencies (sorted for deterministic order)
        if let Some(deps) = self.edges.get(node) {
            let mut sorted_deps: Vec<_> = deps.iter().filter(|d| remaining.contains(d)).collect();
            sorted_deps.sort();
            for dep in sorted_deps {
                if let Some(idx) = self.dfs_find_cycle(dep, remaining, visited, path) {
                    return Some(idx);
                }
            }
        }

        path.pop();
        None
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

        let graph = DepGraph::build(std::slice::from_ref(&main), root, TEST_MODULE).unwrap();
        // Should have no dependencies (Go imports are ignored)
        assert!(!graph.has_sop_deps(&main));
    }

    #[test]
    fn test_test_file_implicit_dep() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // main.sop has the Add function
        let main = create_test_file(
            root,
            "main.sop",
            r#"package main
func Add(a int, b int) int { return a + b }"#,
        );

        // main_test.sop is in the same package, should depend on main.sop
        let test = create_test_file(
            root,
            "main_test.sop",
            r#"package main
import "testing"
func TestAdd(t *testing.T) { Add(1, 2) }"#,
        );

        let graph = DepGraph::build(&[main.clone(), test.clone()], root, TEST_MODULE).unwrap();
        let sorted = graph.topological_sort().unwrap();

        // main.sop should come before main_test.sop
        let main_idx = sorted.iter().position(|p| p == &main).unwrap();
        let test_idx = sorted.iter().position(|p| p == &test).unwrap();
        assert!(
            main_idx < test_idx,
            "main.sop should be processed before main_test.sop"
        );
    }

    #[test]
    fn test_test_file_implicit_dep_math() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // math.sop has the Add function (using package basic like the fixture)
        let math = create_test_file(
            root,
            "math.sop",
            r#"package basic
func Add(a int, b int) int { return a + b }"#,
        );

        // math_test.sop is in the same package
        let test = create_test_file(
            root,
            "math_test.sop",
            r#"package basic
import "testing"
func TestAdd(t *testing.T) { Add(1, 2) }"#,
        );

        let graph = DepGraph::build(&[math.clone(), test.clone()], root, TEST_MODULE).unwrap();
        let sorted = graph.topological_sort().unwrap();

        println!(
            "Sorted order: {:?}",
            sorted.iter().map(|p| p.file_name()).collect::<Vec<_>>()
        );

        // math.sop should come before math_test.sop
        let math_idx = sorted.iter().position(|p| p == &math).unwrap();
        let test_idx = sorted.iter().position(|p| p == &test).unwrap();
        assert!(
            math_idx < test_idx,
            "math.sop should be processed before math_test.sop"
        );
    }
}
