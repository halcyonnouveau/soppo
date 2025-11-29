use std::collections::HashMap;

use super::ty::Type;
use crate::syntax::{ConstDecl, EnumVariant, FuncDecl, ModuleId, TypeDecl, TypeKind};

/// Kind of Soppo type discovered from Go package markers
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoSoppoKind {
    /// Enum type from //soppo:enum marker
    Enum,
    /// Nilable field from //soppo:nilable marker
    Nilable,
}

/// Global context tracking all modules and types
#[derive(Debug, Clone)]
pub struct GlobalCtxt {
    /// All modules indexed by ID
    modules: HashMap<ModuleId, Module>,

    /// Currently active module
    current_module: ModuleId,

    /// Soppo import mappings for current module: package alias → ModuleId
    /// This is set per-file during compilation
    soppo_imports: HashMap<String, ModuleId>,

    /// Soppo types discovered from Go packages via //soppo: markers
    /// Maps package alias to type name to type kind
    go_soppo_types: HashMap<String, HashMap<String, GoSoppoKind>>,
}

/// A module containing type, function, and constant definitions
#[derive(Debug, Clone)]
pub struct Module {
    pub id: ModuleId,
    pub name: String,

    /// Type definitions (enums, structs, aliases)
    pub types: HashMap<String, TypeDef>,

    /// Function definitions
    pub functions: HashMap<String, FuncDef>,

    /// Constant definitions
    pub constants: HashMap<String, ConstDef>,
}

/// Type definition in a module
#[derive(Debug, Clone)]
pub struct TypeDef {
    pub name: String,
    pub generics: Vec<String>,
    pub kind: TypeDefKind,
}

#[derive(Debug, Clone)]
pub enum TypeDefKind {
    /// Type alias: type X = Y (X is exactly Y)
    Alias {
        target: Type,
    },
    /// Type definition: type X Y (X is a new distinct type based on Y)
    Definition {
        target: Type,
    },
    Enum {
        variants: Vec<EnumVariant>,
    },
    Struct {
        fields: Vec<(String, Type)>,
    },
    Interface {
        methods: Vec<MethodSig>,
    },
}

/// Method signature for interfaces
#[derive(Debug, Clone)]
pub struct MethodSig {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub returns: Vec<Type>,
}

/// Function definition in a module
#[derive(Debug, Clone)]
pub struct FuncDef {
    pub name: String,
    pub generics: Vec<String>,
    pub params: Vec<(String, Type)>,
    pub return_types: Vec<Type>,
}

/// Constant definition in a module
#[derive(Debug, Clone)]
pub struct ConstDef {
    pub name: String,
    pub ty: Type,
}

impl GlobalCtxt {
    pub fn new() -> Self {
        let mut gs = Self {
            modules: HashMap::new(),
            current_module: ModuleId::new("main"),
            soppo_imports: HashMap::new(),
            go_soppo_types: HashMap::new(),
        };

        // Create main module
        gs.modules.insert(
            ModuleId::new("main"),
            Module {
                id: ModuleId::new("main"),
                name: "main".to_string(),
                types: HashMap::new(),
                functions: HashMap::new(),
                constants: HashMap::new(),
            },
        );

        gs
    }

    /// Get or create a module by ID
    pub fn get_or_create_module(&mut self, id: ModuleId) -> &mut Module {
        self.modules.entry(id.clone()).or_insert_with(|| Module {
            id: id.clone(),
            name: id.0.clone(),
            types: HashMap::new(),
            functions: HashMap::new(),
            constants: HashMap::new(),
        })
    }

    /// Set the current module
    pub fn set_current_module(&mut self, id: ModuleId) {
        if !self.modules.contains_key(&id) {
            self.get_or_create_module(id.clone());
        }
        self.current_module = id;
    }

    /// Get a module by ID
    pub fn get_module(&self, id: &ModuleId) -> Option<&Module> {
        self.modules.get(id)
    }

    /// Get the current module
    pub fn current_module(&self) -> &Module {
        self.modules.get(&self.current_module).unwrap()
    }

    /// Get the current module mutably
    pub fn current_module_mut(&mut self) -> &mut Module {
        self.modules.get_mut(&self.current_module).unwrap()
    }

    /// Register a type definition
    pub fn register_type(&mut self, type_decl: &TypeDecl) {
        let type_def = TypeDef {
            name: type_decl.name.clone(),
            generics: type_decl.generics.iter().map(|g| g.name.clone()).collect(),
            kind: match &type_decl.kind {
                TypeKind::Alias { target } => TypeDefKind::Alias {
                    target: Type::from_ast(target),
                },
                TypeKind::Definition { target } => TypeDefKind::Definition {
                    target: Type::from_ast(target),
                },
                TypeKind::Enum { variants } => TypeDefKind::Enum {
                    variants: variants.clone(),
                },
                TypeKind::Struct { fields } => TypeDefKind::Struct {
                    fields: fields
                        .iter()
                        .map(|f| (f.name.clone(), Type::from_ast(&f.ty)))
                        .collect(),
                },
                TypeKind::Interface { methods } => TypeDefKind::Interface {
                    methods: methods
                        .iter()
                        .map(|m| MethodSig {
                            name: m.name.clone(),
                            params: m
                                .params
                                .iter()
                                .map(|p| (p.name.clone(), Type::from_ast(&p.ty)))
                                .collect(),
                            returns: m.returns.iter().map(Type::from_ast).collect(),
                        })
                        .collect(),
                },
            },
        };

        self.current_module_mut()
            .types
            .insert(type_decl.name.clone(), type_def);
    }

    /// Register a function definition
    pub fn register_function(&mut self, func_decl: &FuncDecl) {
        let func_def = FuncDef {
            name: func_decl.name.clone(),
            generics: func_decl.generics.iter().map(|g| g.name.clone()).collect(),
            params: func_decl
                .params
                .iter()
                .map(|p| (p.name.clone(), Type::from_ast(&p.ty)))
                .collect(),
            return_types: func_decl.return_types.iter().map(Type::from_ast).collect(),
        };

        self.current_module_mut()
            .functions
            .insert(func_decl.name.clone(), func_def);
    }

    /// Register a constant definition
    pub fn register_constant(&mut self, const_decl: &ConstDecl, ty: Type) {
        let const_def = ConstDef {
            name: const_decl.name.clone(),
            ty,
        };

        self.current_module_mut()
            .constants
            .insert(const_decl.name.clone(), const_def);
    }

    /// Lookup a type definition in current module
    pub fn lookup_type(&self, name: &str) -> Option<&TypeDef> {
        self.current_module().types.get(name)
    }

    /// Lookup a type definition in a specific module
    pub fn lookup_type_in(&self, module: &ModuleId, name: &str) -> Option<&TypeDef> {
        self.modules.get(module)?.types.get(name)
    }

    /// Lookup a function definition in current module
    pub fn lookup_function(&self, name: &str) -> Option<&FuncDef> {
        self.current_module().functions.get(name)
    }

    /// Lookup a function definition in a specific module
    pub fn lookup_function_in(&self, module: &ModuleId, name: &str) -> Option<&FuncDef> {
        self.modules.get(module)?.functions.get(name)
    }

    /// Lookup a constant definition in current module
    pub fn lookup_constant(&self, name: &str) -> Option<&ConstDef> {
        self.current_module().constants.get(name)
    }

    /// Lookup a constant definition in a specific module
    pub fn lookup_constant_in(&self, module: &ModuleId, name: &str) -> Option<&ConstDef> {
        self.modules.get(module)?.constants.get(name)
    }

    /// Check if a type is registered in current module
    pub fn has_type(&self, name: &str) -> bool {
        self.current_module().types.contains_key(name)
    }

    /// Register a Soppo import mapping
    pub fn register_soppo_import(&mut self, alias: String, module_id: ModuleId) {
        self.soppo_imports.insert(alias, module_id);
    }

    /// Clear Soppo imports (call when moving to a new file)
    pub fn clear_soppo_imports(&mut self) {
        self.soppo_imports.clear();
    }

    /// Get the ModuleId for a Soppo import alias
    pub fn get_soppo_module(&self, alias: &str) -> Option<&ModuleId> {
        self.soppo_imports.get(alias)
    }

    /// Check if pkg.Type refers to a Soppo enum (either from Soppo imports or Go packages)
    pub fn is_soppo_enum(&self, pkg: &str, type_name: &str) -> bool {
        // Check Soppo imports first
        if let Some(module_id) = self.soppo_imports.get(pkg)
            && let Some(type_def) = self.lookup_type_in(module_id, type_name)
        {
            return matches!(type_def.kind, TypeDefKind::Enum { .. });
        }
        // Check Go package enums (from //soppo:enum markers)
        if let Some(types) = self.go_soppo_types.get(pkg) {
            return types.get(type_name) == Some(&GoSoppoKind::Enum);
        }
        false
    }

    /// Register a Soppo type discovered from a Go package via //soppo: markers
    pub fn register_go_soppo_type(&mut self, pkg: &str, type_name: &str, kind: GoSoppoKind) {
        self.go_soppo_types
            .entry(pkg.to_string())
            .or_default()
            .insert(type_name.to_string(), kind);
    }

    /// Get the kind of a Soppo type in a Go package
    pub fn get_go_soppo_kind(&self, pkg: &str, type_name: &str) -> Option<&GoSoppoKind> {
        self.go_soppo_types.get(pkg)?.get(type_name)
    }

    /// Clear Go soppo types for current file
    pub fn clear_go_soppo_types(&mut self) {
        self.go_soppo_types.clear();
    }

    /// Check if a type in current module is an enum
    pub fn is_local_enum(&self, type_name: &str) -> bool {
        if let Some(type_def) = self.lookup_type(type_name) {
            return matches!(type_def.kind, TypeDefKind::Enum { .. });
        }
        false
    }
}

impl Default for GlobalCtxt {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::{Generic, Span};

    #[test]
    fn test_register_type() {
        let mut gs = GlobalCtxt::new();

        let type_decl = TypeDecl {
            name: "Colour".to_string(),
            generics: vec![],
            kind: TypeKind::Enum { variants: vec![] },
            span: Span::dummy(),
        };

        gs.register_type(&type_decl);

        let type_def = gs.lookup_type("Colour").unwrap();
        assert_eq!(type_def.name, "Colour");
        assert_eq!(type_def.generics.len(), 0);
    }

    #[test]
    fn test_register_generic_type() {
        let mut gs = GlobalCtxt::new();

        let type_decl = TypeDecl {
            name: "Result".to_string(),
            generics: vec![
                Generic {
                    name: "T".to_string(),
                    constraint: "any".to_string(),
                    span: Span::dummy(),
                },
                Generic {
                    name: "E".to_string(),
                    constraint: "any".to_string(),
                    span: Span::dummy(),
                },
            ],
            kind: TypeKind::Enum { variants: vec![] },
            span: Span::dummy(),
        };

        gs.register_type(&type_decl);

        let type_def = gs.lookup_type("Result").unwrap();
        assert_eq!(type_def.name, "Result");
        assert_eq!(type_def.generics.len(), 2);
        assert_eq!(type_def.generics[0], "T");
        assert_eq!(type_def.generics[1], "E");
    }
}
