use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Position in source code (line and column)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineColumn {
    pub line: usize,
    pub col: usize,
}

/// Unique identifier for a source file
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub usize);

/// Registry mapping FileId <-> file paths.
/// Used for cross-file go-to-definition and workspace-wide operations.
#[derive(Debug, Clone, Default)]
pub struct FileRegistry {
    id_to_path: HashMap<FileId, PathBuf>,
    path_to_id: HashMap<PathBuf, FileId>,
    next_id: usize,
}

impl FileRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a file path and get its FileId.
    /// If the path is already registered, returns the existing FileId.
    pub fn register(&mut self, path: PathBuf) -> FileId {
        if let Some(&id) = self.path_to_id.get(&path) {
            return id;
        }
        let id = FileId(self.next_id);
        self.next_id += 1;
        self.id_to_path.insert(id, path.clone());
        self.path_to_id.insert(path, id);
        id
    }

    /// Get the file path for a FileId.
    pub fn get_path(&self, id: FileId) -> Option<&PathBuf> {
        self.id_to_path.get(&id)
    }

    /// Get the FileId for a file path.
    pub fn get_id(&self, path: &Path) -> Option<FileId> {
        self.path_to_id.get(path).copied()
    }

    /// Check if a path is registered.
    pub fn contains(&self, path: &Path) -> bool {
        self.path_to_id.contains_key(path)
    }

    /// Get all registered file IDs.
    pub fn file_ids(&self) -> impl Iterator<Item = FileId> + '_ {
        self.id_to_path.keys().copied()
    }
}

/// Span in source code - tracks location for error messages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: LineColumn,
    pub end: LineColumn,
    pub file: FileId,
    /// Byte offsets for miette error reporting
    pub byte_start: usize,
    pub byte_end: usize,
}

// Convert Span to miette's SourceSpan for error reporting
impl From<Span> for miette::SourceSpan {
    fn from(span: Span) -> Self {
        miette::SourceSpan::from(span.byte_start..span.byte_end)
    }
}

impl From<&Span> for miette::SourceSpan {
    fn from(span: &Span) -> Self {
        miette::SourceSpan::from(span.byte_start..span.byte_end)
    }
}

impl Span {
    pub fn new(start: LineColumn, end: LineColumn, file: FileId) -> Self {
        Self {
            start,
            end,
            file,
            byte_start: 0,
            byte_end: 1,
        }
    }

    pub fn with_bytes(
        start: LineColumn,
        end: LineColumn,
        file: FileId,
        byte_start: usize,
        byte_end: usize,
    ) -> Self {
        Self {
            start,
            end,
            file,
            byte_start,
            byte_end,
        }
    }

    pub fn dummy() -> Self {
        Self {
            start: LineColumn { line: 0, col: 0 },
            end: LineColumn { line: 0, col: 0 },
            file: FileId(0),
            byte_start: 0,
            byte_end: 1,
        }
    }

    /// Create a span pointing to the last character of this span.
    /// Useful for "expected X after Y" errors where we want to highlight
    /// where the missing token should be.
    pub fn at_end(&self) -> Self {
        Self {
            start: self.end,
            end: self.end,
            file: self.file,
            byte_start: self.byte_end.saturating_sub(1),
            byte_end: self.byte_end,
        }
    }
}

/// Unique identifier for a module
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleId(pub String);

impl ModuleId {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn empty() -> Self {
        Self(String::new())
    }
}

/// A symbol with module context - used for type names, function names, etc.
/// Note: PartialEq/Hash ignore span - we compare symbols by module and name only.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub module: ModuleId,
    pub name: String,
    pub span: Span,
}

impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        self.module == other.module && self.name == other.name
    }
}

impl Eq for Symbol {}

impl std::hash::Hash for Symbol {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.module.hash(state);
        self.name.hash(state);
    }
}

impl Symbol {
    pub fn new(module: ModuleId, name: String, span: Span) -> Self {
        Self { module, name, span }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_registry_register_new_file() {
        let mut registry = FileRegistry::new();
        let path = PathBuf::from("/test/file.sop");
        let id = registry.register(path.clone());

        assert_eq!(id, FileId(0));
        assert_eq!(registry.get_path(id), Some(&path));
        assert_eq!(registry.get_id(&path), Some(id));
    }

    #[test]
    fn file_registry_register_same_file_returns_same_id() {
        let mut registry = FileRegistry::new();
        let path = PathBuf::from("/test/file.sop");

        let id1 = registry.register(path.clone());
        let id2 = registry.register(path.clone());

        assert_eq!(id1, id2);
    }

    #[test]
    fn file_registry_register_multiple_files() {
        let mut registry = FileRegistry::new();
        let path1 = PathBuf::from("/test/file1.sop");
        let path2 = PathBuf::from("/test/file2.sop");
        let path3 = PathBuf::from("/test/file3.sop");

        let id1 = registry.register(path1.clone());
        let id2 = registry.register(path2.clone());
        let id3 = registry.register(path3.clone());

        assert_eq!(id1, FileId(0));
        assert_eq!(id2, FileId(1));
        assert_eq!(id3, FileId(2));

        assert_eq!(registry.get_path(id1), Some(&path1));
        assert_eq!(registry.get_path(id2), Some(&path2));
        assert_eq!(registry.get_path(id3), Some(&path3));
    }

    #[test]
    fn file_registry_get_unknown_file() {
        let registry = FileRegistry::new();
        let path = PathBuf::from("/test/file.sop");

        assert_eq!(registry.get_id(&path), None);
        assert_eq!(registry.get_path(FileId(0)), None);
    }

    #[test]
    fn file_registry_contains() {
        let mut registry = FileRegistry::new();
        let path = PathBuf::from("/test/file.sop");
        let other = PathBuf::from("/test/other.sop");

        assert!(!registry.contains(&path));
        registry.register(path.clone());
        assert!(registry.contains(&path));
        assert!(!registry.contains(&other));
    }

    #[test]
    fn file_registry_file_ids_iterator() {
        let mut registry = FileRegistry::new();
        registry.register(PathBuf::from("/a.sop"));
        registry.register(PathBuf::from("/b.sop"));
        registry.register(PathBuf::from("/c.sop"));

        let ids: Vec<_> = registry.file_ids().collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&FileId(0)));
        assert!(ids.contains(&FileId(1)));
        assert!(ids.contains(&FileId(2)));
    }
}
