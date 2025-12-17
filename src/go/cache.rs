use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use miette::Result;

use super::extract::{self as extract, GoPackage};
use super::project::Project;
use super::resolve::{ImportKind, Resolver};

/// Global cache for parsed Go stdlib packages.
/// These never change during a process's lifetime, so safe to cache globally.
static STDLIB_CACHE: OnceLock<Mutex<HashMap<String, GoPackage>>> = OnceLock::new();

fn get_stdlib_cache() -> &'static Mutex<HashMap<String, GoPackage>> {
    STDLIB_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Cache for parsed Go packages.
/// Stdlib packages use a global cache; project-specific packages are per-instance.
pub struct GoCache {
    /// Per-instance cache for project-specific packages (local, external)
    local_packages: HashMap<String, GoPackage>,
    /// Resolver for finding package sources
    resolver: Resolver,
}

impl GoCache {
    /// Create a new cache with a resolver
    pub fn new() -> Result<Self> {
        Ok(Self {
            local_packages: HashMap::new(),
            resolver: Resolver::new()?,
        })
    }

    /// Get a package, parsing it if not cached
    ///
    /// - Stdlib packages use the global cache
    /// - Local/external packages use the per-instance cache
    pub fn get_or_parse(
        &mut self,
        import_path: &str,
        project: Option<&Project>,
    ) -> Result<&GoPackage> {
        let kind = self.resolver.resolve(import_path, project)?;

        match &kind {
            ImportKind::GoStdlib { source_dir, .. } => {
                // Use global cache for stdlib
                let mut cache = get_stdlib_cache().lock().unwrap();
                if !cache.contains_key(import_path) {
                    let pkg = extract::extract(source_dir)?;
                    cache.insert(import_path.to_string(), pkg);
                }
                drop(cache);

                // Return reference from global cache
                let cache = get_stdlib_cache().lock().unwrap();
                // Clone into local cache to return a reference with correct lifetime
                let pkg = cache.get(import_path).unwrap().clone();
                drop(cache);
                self.local_packages
                    .entry(import_path.to_string())
                    .or_insert(pkg);
                Ok(self.local_packages.get(import_path).unwrap())
            }
            ImportKind::LocalSoppo { source_dir } | ImportKind::ExternalGo { source_dir, .. } => {
                // Use per-instance cache for project-specific packages
                if !self.local_packages.contains_key(import_path) {
                    let pkg = extract::extract(source_dir)?;
                    self.local_packages.insert(import_path.to_string(), pkg);
                }
                Ok(self.local_packages.get(import_path).unwrap())
            }
        }
    }

    /// Check if a package is cached (either globally or locally)
    pub fn is_cached(&self, import_path: &str) -> bool {
        self.local_packages.contains_key(import_path)
            || get_stdlib_cache().lock().unwrap().contains_key(import_path)
    }

    /// Clear the per-instance cache (does not clear global stdlib cache)
    pub fn clear(&mut self) {
        self.local_packages.clear();
    }

    /// Get the resolver for direct access
    pub fn resolver(&self) -> &Resolver {
        &self.resolver
    }
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

        let pkg = extract::extract(&file_path).unwrap();
        assert!(pkg.functions.contains_key("Hello"));

        fs::remove_dir_all(&dir).unwrap();
    }
}
