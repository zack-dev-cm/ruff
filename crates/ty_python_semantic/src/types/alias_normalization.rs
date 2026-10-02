use std::cell::RefCell;

use rustc_hash::FxHashSet;
use ty_python_core::definition::Definition;

use crate::Db;
use crate::types::{ApplyTypeMappingVisitor, Type, TypeContext, TypeMapping};

/// Expands aliases while rebuilding their enclosing types. Recursive applications retain their
/// normalized arguments, so subsequent dependency searches can recognize equivalent backedges.
#[derive(Default)]
pub(super) struct AliasNormalizationVisitor<'db> {
    active_types: RefCell<FxHashSet<(Type<'db>, usize)>>,
    active_definitions: RefCell<FxHashSet<Definition<'db>>>,
}

impl<'db> AliasNormalizationVisitor<'db> {
    /// Preserve a recursive reference without caching a context-dependent partial expansion.
    pub(super) fn visit_type(
        &self,
        ty: Type<'db>,
        compute: impl FnOnce() -> Type<'db>,
    ) -> Type<'db> {
        // Entering another alias changes which references are backedges. Revisit the type in
        // that scope so its recursive applications still receive normalized arguments.
        let key = (ty, self.active_definitions.borrow().len());
        if !self.active_types.borrow_mut().insert(key) {
            return ty;
        }
        let result = compute();
        self.active_types.borrow_mut().remove(&key);
        result
    }

    /// Normalize supplied arguments before entering the alias body. Finite nesting such as
    /// `Identity[Identity[T]]` is independent of recursive references introduced by that body.
    pub(super) fn normalize_alias(
        &self,
        db: &'db dyn Db,
        ty: Type<'db>,
        visitor: &ApplyTypeMappingVisitor<'_, 'db>,
    ) -> Type<'db> {
        self.visit_type(ty, || {
            let mapping = &TypeMapping::NormalizeAliases;
            let (definition, normalized) = match ty {
                Type::TypeAlias(alias) => (
                    alias.definition(db),
                    Type::TypeAlias(alias.map_stored_specialization(db, mapping, visitor)),
                ),
                Type::Recursive(recursive) => (
                    recursive.definition(db),
                    Type::Recursive(recursive.with_arguments(
                        db,
                        recursive.arguments(db).map(|arguments| {
                            arguments.apply_type_mapping_impl(db, mapping, &[], visitor)
                        }),
                    )),
                ),
                _ => return ty,
            };
            if !self.active_definitions.borrow_mut().insert(definition) {
                return normalized;
            }
            let body = match normalized {
                Type::TypeAlias(alias) => alias.value_type(db),
                Type::Recursive(recursive) => recursive.unfold(db, visitor.env).into_type(),
                _ => normalized,
            };
            let result = body.apply_type_mapping_impl(db, mapping, TypeContext::default(), visitor);
            self.active_definitions.borrow_mut().remove(&definition);
            result
        })
    }
}
