//! Cache for parsed Go packages.

use crate::go_extract::{self, GoPackage};
use crate::project::Project;
use crate::resolve::{ImportKind, Resolver};
use miette::Result;
use std::collections::HashMap;
use std::path::Path;

/// Cache for parsed Go packages.
/// Avoids re-parsing the same package multiple times during compilation.
pub struct GoCache {
    /// Cached packages by import path
    packages: HashMap<String, GoPackage>,
    /// Resolver for finding package sources
    resolver: Resolver,
}

impl GoCache {
    /// Create a new cache with a resolver
    pub fn new() -> Result<Self> {
        Ok(Self {
            packages: HashMap::new(),
            resolver: Resolver::new()?,
        })
    }

    /// Get a package, parsing it if not cached
    ///
    /// - Stdlib packages work without a project
    /// - External modules require a project for version resolution
    pub fn get_or_parse(
        &mut self,
        import_path: &str,
        project: Option<&Project>,
    ) -> Result<&GoPackage> {
        // Check cache first
        if self.packages.contains_key(import_path) {
            return Ok(self.packages.get(import_path).unwrap());
        }

        // Resolve and parse
        let kind = self.resolver.resolve(import_path, project)?;
        let source_dir = match &kind {
            ImportKind::LocalSoppo { source_dir } => source_dir,
            ImportKind::GoStdlib { source_dir, .. } => source_dir,
            ImportKind::ExternalGo { source_dir, .. } => source_dir,
        };

        let pkg = go_extract::extract(source_dir)?;
        self.packages.insert(import_path.to_string(), pkg);
        Ok(self.packages.get(import_path).unwrap())
    }

    /// Check if a package is cached
    pub fn is_cached(&self, import_path: &str) -> bool {
        self.packages.contains_key(import_path)
    }

    /// Clear the cache
    pub fn clear(&mut self) {
        self.packages.clear();
    }

    /// Get the resolver for direct access
    pub fn resolver(&self) -> &Resolver {
        &self.resolver
    }
}

/// Parse a single Go file or directory (convenience function)
pub fn parse_go_path(path: &Path) -> Result<GoPackage> {
    go_extract::extract(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_creation() {
        let cache = GoCache::new();
        assert!(cache.is_ok());
    }

    #[test]
    fn test_parse_go_file() {
        use std::fs::{self, File};
        use std::io::Write;

        let dir = std::env::temp_dir().join("soppo-cache-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let file_path = dir.join("test.go");
        let mut file = File::create(&file_path).unwrap();
        writeln!(
            file,
            r#"
package test

func Hello() string {{
    return "hello"
}}
"#
        )
        .unwrap();

        let pkg = parse_go_path(&file_path).unwrap();
        assert!(pkg.functions.contains_key("Hello"));

        fs::remove_dir_all(&dir).unwrap();
    }
}
