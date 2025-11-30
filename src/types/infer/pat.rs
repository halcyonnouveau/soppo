use std::collections::HashMap;

use super::Infer;
use crate::error::Result;
use crate::syntax::{EnumVariant, FieldPattern, Pattern, PatternKind};
use crate::types::Type;
use crate::types::ctx::TypeDefKind;

impl Infer {
    /// Add pattern bindings to the current scope
    pub(super) fn add_pattern_bindings(
        &mut self,
        pattern: &Pattern,
        scrutinee_ty: &Type,
    ) -> Result<()> {
        match &pattern.kind {
            PatternKind::Default => {
                // Default doesn't bind anything
                Ok(())
            }
            PatternKind::Variant(_name) => {
                // Qualified variant names like Colour.Red don't create bindings
                // They just match against enum variants
                Ok(())
            }
            PatternKind::Literal(_) => {
                // Literal pattern doesn't bind anything
                Ok(())
            }
            PatternKind::Destructor { name, binding } => {
                // For destructor patterns like Ok(value), extract the inner type from variant
                let variant_name = name.rsplit('.').next().unwrap_or(name);

                // Try to find the actual type from the enum variant
                if let Type::Con {
                    name: type_name, ..
                } = scrutinee_ty
                    && let Some(type_def) = self.global_state.lookup_type(&type_name.name)
                    && let TypeDefKind::Enum { variants } = &type_def.kind
                {
                    for variant in variants {
                        if let EnumVariant::Single {
                            name: vname, ty, ..
                        } = variant
                            && vname == variant_name
                        {
                            let binding_ty = Type::simple(&ty.name);
                            self.insert_var(binding.clone(), binding_ty, Some(pattern.span));
                            return Ok(());
                        }
                    }
                }
                // Fallback to fresh type variable if we can't determine the type
                let binding_ty = self.fresh_ty_var();
                self.insert_var(binding.clone(), binding_ty, Some(pattern.span));
                Ok(())
            }
            PatternKind::StructDestructor {
                name,
                fields,
                rest: _,
            } => {
                // For struct destructor patterns like Circle{radius: r, ...} or Point{x: 0, y}
                let variant_name = name.rsplit('.').next().unwrap_or(name);

                // Collect field types first to avoid borrow conflicts
                let mut bindings: Vec<(String, Type)> = Vec::new();
                let mut found_type = false;

                // Look up the type - could be an enum variant or a regular struct
                if let Type::Con {
                    name: type_name, ..
                } = scrutinee_ty
                    && let Some(type_def) = self.global_state.lookup_type(&type_name.name)
                {
                    match &type_def.kind {
                        TypeDefKind::Enum { variants } => {
                            // Enum struct variant
                            for variant in variants {
                                if let EnumVariant::Struct {
                                    name: vname,
                                    fields: variant_fields,
                                    ..
                                } = variant
                                    && vname == variant_name
                                {
                                    found_type = true;
                                    for (field_name, field_pattern) in fields {
                                        if let FieldPattern::Bind(binding_name) = field_pattern
                                            && let Some(field) = variant_fields
                                                .iter()
                                                .find(|f| &f.name == field_name)
                                        {
                                            let field_ty = Type::simple(&field.ty.name);
                                            bindings.push((binding_name.clone(), field_ty));
                                        }
                                        // Literals don't create bindings
                                    }
                                    break;
                                }
                            }
                        }
                        TypeDefKind::Struct {
                            fields: struct_fields,
                        } => {
                            // Regular struct matching
                            found_type = true;
                            for (field_name, field_pattern) in fields {
                                if let FieldPattern::Bind(binding_name) = field_pattern
                                    && let Some((_, field_ty)) =
                                        struct_fields.iter().find(|(name, _)| name == field_name)
                                {
                                    bindings.push((binding_name.clone(), field_ty.clone()));
                                }
                                // Literals don't create bindings
                            }
                        }
                        _ => {}
                    }
                }

                // Insert bindings after borrows are released
                if found_type {
                    for (binding_name, field_ty) in bindings {
                        self.insert_var(binding_name, field_ty, Some(pattern.span));
                    }
                } else {
                    // Fallback: add bindings with fresh type variables
                    for (_field_name, field_pattern) in fields {
                        if let FieldPattern::Bind(binding_name) = field_pattern {
                            let binding_ty = self.fresh_ty_var();
                            self.insert_var(binding_name.clone(), binding_ty, Some(pattern.span));
                        }
                    }
                }
                Ok(())
            }
            PatternKind::Guard(_) => {
                // Guard expressions don't bind anything
                Ok(())
            }
        }
    }

    /// Collect pattern bindings without inserting them (for multi-pattern validation)
    pub(super) fn collect_pattern_bindings(
        &mut self,
        pattern: &Pattern,
        scrutinee_ty: &Type,
    ) -> Result<HashMap<String, Type>> {
        let mut bindings = HashMap::new();

        match &pattern.kind {
            PatternKind::Default | PatternKind::Literal(_) | PatternKind::Guard(_) => {
                // These patterns don't bind anything
            }
            PatternKind::Variant(_name) => {
                // Qualified variant names like Colour.Red don't create bindings
                // They just match against enum variants
            }
            PatternKind::Destructor { name, binding } => {
                let variant_name = name.rsplit('.').next().unwrap_or(name);

                let binding_ty = if let Type::Con {
                    name: type_name, ..
                } = scrutinee_ty
                    && let Some(type_def) = self.global_state.lookup_type(&type_name.name)
                    && let TypeDefKind::Enum { variants } = &type_def.kind
                {
                    variants
                        .iter()
                        .find_map(|variant| {
                            if let EnumVariant::Single {
                                name: vname, ty, ..
                            } = variant
                                && vname == variant_name
                            {
                                Some(Type::simple(&ty.name))
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| self.fresh_ty_var())
                } else {
                    self.fresh_ty_var()
                };

                bindings.insert(binding.clone(), binding_ty);
            }
            PatternKind::StructDestructor {
                name,
                fields,
                rest: _,
            } => {
                let variant_name = name.rsplit('.').next().unwrap_or(name);

                if let Type::Con {
                    name: type_name, ..
                } = scrutinee_ty
                    && let Some(type_def) = self.global_state.lookup_type(&type_name.name)
                {
                    match &type_def.kind {
                        TypeDefKind::Enum { variants } => {
                            for variant in variants {
                                if let EnumVariant::Struct {
                                    name: vname,
                                    fields: variant_fields,
                                    ..
                                } = variant
                                    && vname == variant_name
                                {
                                    for (field_name, field_pattern) in fields {
                                        if let FieldPattern::Bind(binding_name) = field_pattern
                                            && let Some(field) = variant_fields
                                                .iter()
                                                .find(|f| &f.name == field_name)
                                        {
                                            let field_ty = Type::simple(&field.ty.name);
                                            bindings.insert(binding_name.clone(), field_ty);
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                        TypeDefKind::Struct {
                            fields: struct_fields,
                        } => {
                            for (field_name, field_pattern) in fields {
                                if let FieldPattern::Bind(binding_name) = field_pattern
                                    && let Some((_, field_ty)) =
                                        struct_fields.iter().find(|(name, _)| name == field_name)
                                {
                                    bindings.insert(binding_name.clone(), field_ty.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Fallback for any bindings not found
                for (_field_name, field_pattern) in fields {
                    if let FieldPattern::Bind(binding_name) = field_pattern {
                        bindings
                            .entry(binding_name.clone())
                            .or_insert_with(|| self.fresh_ty_var());
                    }
                }
            }
        }

        Ok(bindings)
    }
}
