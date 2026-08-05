use std::collections::HashMap;

use super::Infer;
use crate::error::SoppoResult;
use crate::syntax::{EnumVariant, FieldPattern, Pattern, PatternKind};
use crate::types::ast::TypedFieldPattern;
use crate::types::ctx::TypeDefKind;
use crate::types::{Type, TypedPattern, TypedPatternKind};

impl Infer {
    /// Add pattern bindings to the current scope
    pub(super) fn add_pattern_bindings(
        &mut self,
        pattern: &Pattern,
        scrutinee_ty: &Type,
    ) -> SoppoResult<()> {
        match &pattern.kind {
            PatternKind::Default => {
                // Default doesn't bind anything
                Ok(())
            }
            PatternKind::Variant { .. } => {
                // Qualified variant names like Colour.Red don't create bindings
                // They just match against enum variants
                Ok(())
            }
            PatternKind::Literal(_) => {
                // Literal pattern doesn't bind anything
                Ok(())
            }
            PatternKind::Destructor { name, binding, .. } => {
                // For destructor patterns like Ok(value), extract the inner type from variant
                let variant_name = name.rsplit('.').next().unwrap_or(name);

                // Try to find the actual type from the enum variant
                if let Type::Con { sym: type_name, .. } = scrutinee_ty
                    && let Some(type_def) = self.global_state.lookup_type(&type_name.name)
                    && let TypeDefKind::Enum { variants } = &type_def.kind
                {
                    for variant in variants {
                        if let EnumVariant::Single {
                            ident: vname, ty, ..
                        } = variant
                            && vname.name == variant_name
                        {
                            let binding_ty = Type::simple(&ty.name);
                            self.insert_var(binding.name.clone(), binding_ty, Some(binding.span))?;
                            return Ok(());
                        }
                    }
                }
                // Fallback to fresh type variable if we can't determine the type
                let binding_ty = self.fresh_ty_var();
                self.insert_var(binding.name.clone(), binding_ty, Some(binding.span))?;
                Ok(())
            }
            PatternKind::StructDestructor { name, fields, .. } => {
                // For struct destructor patterns like Circle{radius: r, ...} or Point{x: 0, y}
                let variant_name = name.rsplit('.').next().unwrap_or(name);

                // Collect field types first to avoid borrow conflicts
                let mut bindings: Vec<(String, Type)> = Vec::new();
                let mut found_type = false;

                // Look up the type - could be an enum variant or a regular struct
                if let Type::Con { sym: type_name, .. } = scrutinee_ty
                    && let Some(type_def) = self.global_state.lookup_type(&type_name.name)
                {
                    match &type_def.kind {
                        TypeDefKind::Enum { variants } => {
                            // Enum struct variant
                            for variant in variants {
                                if let EnumVariant::Struct {
                                    ident: vname,
                                    fields: variant_fields,
                                    ..
                                } = variant
                                    && vname.name == variant_name
                                {
                                    found_type = true;
                                    for (field_name, field_pattern) in fields {
                                        if let FieldPattern::Bind(binding_ident) = field_pattern
                                            && let Some(field) = variant_fields
                                                .iter()
                                                .find(|f| &f.ident.name == field_name)
                                        {
                                            let field_ty = Type::simple(&field.ty.name);
                                            bindings.push((binding_ident.name.clone(), field_ty));
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
                                if let FieldPattern::Bind(binding_ident) = field_pattern
                                    && let Some((_, field_ty, _)) =
                                        struct_fields.iter().find(|(name, _, _)| name == field_name)
                                {
                                    bindings.push((binding_ident.name.clone(), field_ty.clone()));
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
                        self.insert_var(binding_name, field_ty, Some(pattern.span))?;
                    }
                } else {
                    // Fallback: add bindings with fresh type variables
                    for (_field_name, field_pattern) in fields {
                        if let FieldPattern::Bind(binding_ident) = field_pattern {
                            let binding_ty = self.fresh_ty_var();
                            self.insert_var(
                                binding_ident.name.clone(),
                                binding_ty,
                                Some(binding_ident.span),
                            )?;
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
    ) -> SoppoResult<HashMap<String, Type>> {
        let mut bindings = HashMap::new();

        match &pattern.kind {
            PatternKind::Default | PatternKind::Literal(_) | PatternKind::Guard(_) => {
                // These patterns don't bind anything
            }
            PatternKind::Variant { .. } => {
                // Qualified variant names like Colour.Red don't create bindings
                // They just match against enum variants
            }
            PatternKind::Destructor { name, binding, .. } => {
                let variant_name = name.rsplit('.').next().unwrap_or(name);

                let binding_ty = if let Type::Con { sym: type_name, .. } = scrutinee_ty
                    && let Some(type_def) = self.global_state.lookup_type(&type_name.name)
                    && let TypeDefKind::Enum { variants } = &type_def.kind
                {
                    variants
                        .iter()
                        .find_map(|variant| {
                            if let EnumVariant::Single {
                                ident: vname, ty, ..
                            } = variant
                                && vname.name == variant_name
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

                bindings.insert(binding.name.clone(), binding_ty);
            }
            PatternKind::StructDestructor { name, fields, .. } => {
                let variant_name = name.rsplit('.').next().unwrap_or(name);

                if let Type::Con { sym: type_name, .. } = scrutinee_ty
                    && let Some(type_def) = self.global_state.lookup_type(&type_name.name)
                {
                    match &type_def.kind {
                        TypeDefKind::Enum { variants } => {
                            for variant in variants {
                                if let EnumVariant::Struct {
                                    ident: vname,
                                    fields: variant_fields,
                                    ..
                                } = variant
                                    && vname.name == variant_name
                                {
                                    for (field_name, field_pattern) in fields {
                                        if let FieldPattern::Bind(binding_ident) = field_pattern
                                            && let Some(field) = variant_fields
                                                .iter()
                                                .find(|f| &f.ident.name == field_name)
                                        {
                                            let field_ty = Type::simple(&field.ty.name);
                                            bindings.insert(binding_ident.name.clone(), field_ty);
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
                                if let FieldPattern::Bind(binding_ident) = field_pattern
                                    && let Some((_, field_ty, _)) =
                                        struct_fields.iter().find(|(name, _, _)| name == field_name)
                                {
                                    bindings.insert(binding_ident.name.clone(), field_ty.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Fallback for any bindings not found
                for (_field_name, field_pattern) in fields {
                    if let FieldPattern::Bind(binding_ident) = field_pattern {
                        bindings
                            .entry(binding_ident.name.clone())
                            .or_insert_with(|| self.fresh_ty_var());
                    }
                }
            }
        }

        Ok(bindings)
    }

    /// Build a TypedPattern from a Pattern during inference.
    pub fn build_typed_pattern(
        &mut self,
        pattern: &crate::syntax::Pattern,
        matched_ty: &Type,
    ) -> TypedPattern {
        let kind = match &pattern.kind {
            PatternKind::Default => TypedPatternKind::Default,

            PatternKind::Variant {
                name, type_args, ..
            } => {
                let resolved_type_args: Vec<Type> =
                    type_args.iter().map(|t| self.resolve_type(t)).collect();

                // Try to resolve enum type from name
                let enum_ty = if let Some((enum_name, _)) = name.split_once('.') {
                    Type::simple(enum_name)
                } else {
                    Type::error()
                };
                // Determine if this is a Soppo enum from the matched type
                let is_soppo_enum = self.is_soppo_enum_type(matched_ty);
                TypedPatternKind::Variant {
                    enum_ty,
                    variant_name: name.clone(),
                    type_args: resolved_type_args,
                    is_soppo_enum,
                }
            }

            PatternKind::Literal(lit) => TypedPatternKind::Literal(lit.clone()),

            PatternKind::Destructor {
                name,
                type_args,
                binding,
            } => {
                let resolved_type_args: Vec<Type> =
                    type_args.iter().map(|t| self.resolve_type(t)).collect();
                let enum_ty = if let Some((enum_name, _)) = name.split_once('.') {
                    Type::simple(enum_name)
                } else {
                    Type::error()
                };

                // Look up binding type from variant definition
                let variant_name_part = name.rsplit('.').next().unwrap_or(name);
                let binding_ty = if let Type::Con { sym: type_name, .. } = matched_ty
                    && let Some(type_def) = self.global_state.lookup_type(&type_name.name)
                    && let TypeDefKind::Enum { variants } = &type_def.kind
                {
                    variants
                        .iter()
                        .find_map(|variant| {
                            if let EnumVariant::Single {
                                ident: vname, ty, ..
                            } = variant
                                && vname.name == variant_name_part
                            {
                                Some(Type::simple(&ty.name))
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(Type::error)
                } else {
                    Type::error()
                };

                TypedPatternKind::Destructor {
                    enum_ty,
                    variant_name: name.clone(),
                    type_args: resolved_type_args,
                    binding: binding.clone(),
                    binding_ty,
                }
            }

            PatternKind::StructDestructor {
                name,
                type_args,
                fields,
                rest,
            } => {
                let resolved_type_args: Vec<Type> =
                    type_args.iter().map(|t| self.resolve_type(t)).collect();
                let struct_ty = if let Some((type_name, _)) = name.split_once('.') {
                    Type::simple(type_name)
                } else {
                    Type::simple(name)
                };

                // Build a map of field name -> type from the struct/variant definition
                let variant_name_part = name.rsplit('.').next().unwrap_or(name);
                let field_type_map: std::collections::HashMap<&str, Type> =
                    if let Type::Con { sym: type_name, .. } = matched_ty
                        && let Some(type_def) = self.global_state.lookup_type(&type_name.name)
                    {
                        match &type_def.kind {
                            TypeDefKind::Enum { variants } => {
                                // Look for matching struct variant
                                variants
                                    .iter()
                                    .find_map(|variant| {
                                        if let EnumVariant::Struct {
                                            ident: vname,
                                            fields: variant_fields,
                                            ..
                                        } = variant
                                            && vname.name == variant_name_part
                                        {
                                            Some(
                                                variant_fields
                                                    .iter()
                                                    .map(|f| {
                                                        (
                                                            f.ident.name.as_str(),
                                                            Type::simple(&f.ty.name),
                                                        )
                                                    })
                                                    .collect(),
                                            )
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap_or_default()
                            }
                            TypeDefKind::Struct {
                                fields: struct_fields,
                            } => struct_fields
                                .iter()
                                .map(|(name, ty, _is_const)| (name.as_str(), ty.clone()))
                                .collect(),
                            _ => std::collections::HashMap::new(),
                        }
                    } else {
                        std::collections::HashMap::new()
                    };

                let typed_fields: Vec<(String, TypedFieldPattern)> = fields
                    .iter()
                    .map(|(field_name, fp)| {
                        let typed_fp = match fp {
                            crate::syntax::FieldPattern::Bind(ident) => {
                                let field_ty = field_type_map
                                    .get(field_name.as_str())
                                    .cloned()
                                    .unwrap_or_else(Type::error);
                                TypedFieldPattern::Bind(ident.clone(), field_ty)
                            }
                            crate::syntax::FieldPattern::Literal(lit) => {
                                TypedFieldPattern::Literal(lit.clone())
                            }
                        };
                        (field_name.clone(), typed_fp)
                    })
                    .collect();
                TypedPatternKind::StructDestructor {
                    pattern_name: name.clone(),
                    struct_ty,
                    type_args: resolved_type_args,
                    fields: typed_fields,
                    rest: *rest,
                }
            }

            PatternKind::Guard(expr) => TypedPatternKind::Guard(Box::new(self.infer_expr(expr))),
        };

        TypedPattern {
            kind,
            span: pattern.span,
            matched_ty: matched_ty.clone(),
        }
    }

    /// Check if a type is a Soppo enum (vs a Go interface/constant).
    fn is_soppo_enum_type(&self, ty: &Type) -> bool {
        if let Type::Con { sym, .. } = ty {
            if sym.module.0.is_empty() {
                // Local type in current module
                self.global_state
                    .lookup_type(&sym.name)
                    .map(|td| matches!(td.kind, TypeDefKind::Enum { .. }))
                    .unwrap_or(false)
            } else {
                // Cross-package type
                self.global_state.is_soppo_enum(&sym.module.0, &sym.name)
            }
        } else {
            false
        }
    }
}
