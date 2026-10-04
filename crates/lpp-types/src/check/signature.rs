use std::collections::BTreeMap;

use lpp_hir::{
    ArenaId, BindingTarget, DefId, HirItemId, HirItemKind, LocalId, ModuleId, OriginId, Symbol,
    TypeParamId, TypeRefId, TypeRefKind,
};

use crate::{InferenceLevel, PrimitiveType, TypeId, TypeKind};

use super::{ShadowTypeError, TypeChecker};

impl<'hir> TypeChecker<'hir> {
    pub(super) fn register_item(
        &mut self,
        item_id: HirItemId,
        item: lpp_hir::HirItem,
    ) -> Result<(), ShadowTypeError> {
        let parameters = self.collect_parameters(item_id, item.kind);
        let context = self.parameter_context(item_id);
        for parameter in &parameters {
            if let Some(bound) = self.package.type_parameters[*parameter].bound {
                self.resolve_type_ref(bound, item.module, &context)?;
            }
        }
        let type_id = match item.kind {
            HirItemKind::Function(function) => {
                let local_ids = self.package.locals(function.parameters).to_vec();
                let mut parameter_types = Vec::with_capacity(local_ids.len());
                for local in &local_ids {
                    let type_id = self.ensure_local(*local, &context)?;
                    parameter_types.push(type_id);
                }
                // A variadic rest parameter is declared with its *element* type
                // (`...items: Str`) but is a `List[element]` both inside the body
                // and in the public signature. Rewrite the trailing slot and the
                // cached local type so the body sees a list and call sites can
                // recover the element type.
                if function.variadic {
                    if let (Some(&last_local), Some(&element)) =
                        (local_ids.last(), parameter_types.last())
                    {
                        let list = self
                            .interner
                            .intern(TypeKind::List(element))
                            .map_err(|error| self.at(item.origin, error.into()))?;
                        *parameter_types.last_mut().unwrap() = list;
                        self.assignments.locals[last_local.index()] = Some(list);
                    }
                }
                let parameters = self
                    .interner
                    .intern_list(&parameter_types)
                    .map_err(|error| self.at(item.origin, error.into()))?;
                let body_result = if let Some(type_ref) = function.return_type {
                    self.resolve_type_ref(type_ref, item.module, &context)?
                } else {
                    self.interner.primitive(PrimitiveType::Void)
                };
                self.function_results[item_id.index()] = Some(body_result);
                let public_result = if function.is_async {
                    self.interner
                        .intern(TypeKind::Task(body_result))
                        .map_err(|error| self.at(item.origin, error.into()))?
                } else {
                    body_result
                };
                self.interner
                    .intern(TypeKind::Function {
                        parameters,
                        result: public_result,
                    })
                    .map_err(|error| self.at(item.origin, error.into()))?
            }
            HirItemKind::Struct(struct_) => {
                let arguments = parameters
                    .iter()
                    .map(|parameter| self.interner.intern(TypeKind::GenericParameter(*parameter)))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| self.at(item.origin, error.into()))?;
                let arguments = self
                    .interner
                    .intern_list(&arguments)
                    .map_err(|error| self.at(item.origin, error.into()))?;
                let nominal = self.nominal_item_type(item.definition, arguments, item.origin)?;
                let fields = self.package.fields(struct_.fields).to_vec();
                let mut field_types = Vec::with_capacity(fields.len());
                for field in fields {
                    field_types.push(self.resolve_type_ref(
                        self.package.fields[field].type_ref,
                        item.module,
                        &context,
                    )?);
                }
                let field_types = self
                    .interner
                    .intern_list(&field_types)
                    .map_err(|error| self.at(item.origin, error.into()))?;
                self.interner
                    .intern(TypeKind::Function {
                        parameters: field_types,
                        result: nominal,
                    })
                    .map_err(|error| self.at(item.origin, error.into()))?
            }
            HirItemKind::Enum(_) | HirItemKind::Trait(_) => {
                let arguments = parameters
                    .iter()
                    .map(|parameter| self.interner.intern(TypeKind::GenericParameter(*parameter)))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| self.at(item.origin, error.into()))?;
                let arguments = self
                    .interner
                    .intern_list(&arguments)
                    .map_err(|error| self.at(item.origin, error.into()))?;
                self.nominal_item_type(item.definition, arguments, item.origin)?
            }
            HirItemKind::Const { .. } => self.fresh(InferenceLevel::ROOT, item.origin)?,
            HirItemKind::TypeAlias { target } => {
                self.resolve_type_ref(target, item.module, &context)?
            }
            HirItemKind::Impl(impl_) => {
                if let Some(trait_ref) = impl_.trait_ref {
                    self.resolve_type_ref(trait_ref, item.module, &context)?;
                }
                self.resolve_type_ref(impl_.target, item.module, &context)?
            }
            HirItemKind::Extern(_) => self.interner.primitive(PrimitiveType::Void),
        };
        self.assignments.items[item_id.index()] = Some(type_id);
        Ok(())
    }

    pub(super) fn collect_parameters(
        &mut self,
        item_id: HirItemId,
        kind: HirItemKind,
    ) -> Vec<TypeParamId> {
        let range = match kind {
            HirItemKind::Function(function) => Some(function.type_parameters),
            HirItemKind::Struct(struct_) => Some(struct_.type_parameters),
            HirItemKind::Enum(enum_) => Some(enum_.type_parameters),
            HirItemKind::Trait(trait_) => Some(trait_.type_parameters),
            HirItemKind::Impl(impl_) => Some(impl_.type_parameters),
            _ => None,
        };
        let parameters = range
            .map(|range| self.package.type_parameters(range).to_vec())
            .unwrap_or_default();
        self.item_parameters[item_id.index()] = parameters.clone();
        parameters
    }

    pub(super) fn parameter_context(&self, item_id: HirItemId) -> BTreeMap<Symbol, TypeParamId> {
        self.item_parameters[item_id.index()]
            .iter()
            .map(|parameter| (self.package.type_parameters[*parameter].name, *parameter))
            .collect()
    }

    fn nominal_item_type(
        &mut self,
        definition: Option<DefId>,
        arguments: crate::TypeListId,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let Some(definition) = definition else {
            return Ok(self.interner.error());
        };
        self.interner
            .intern(TypeKind::Nominal {
                definition,
                arguments,
            })
            .map_err(|error| self.at(origin, error.into()))
    }

    pub(super) fn resolve_type_ref(
        &mut self,
        type_ref: TypeRefId,
        module: ModuleId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
    ) -> Result<TypeId, ShadowTypeError> {
        if let Some(type_id) = self.assignments.type_refs[type_ref.index()] {
            return Ok(type_id);
        }
        let reference = self.package.type_refs[type_ref];
        let type_id = match reference.kind {
            TypeRefKind::Named(name) => {
                self.resolve_named_type(name, module, parameters, reference.origin)?
            }
            TypeRefKind::Applied { base, arguments } => {
                let argument_refs = self.package.type_refs(arguments).to_vec();
                let mut argument_types = Vec::with_capacity(argument_refs.len());
                for argument in argument_refs {
                    argument_types.push(self.resolve_type_ref(argument, module, parameters)?);
                }
                self.resolve_applied_type(
                    base,
                    &argument_types,
                    module,
                    parameters,
                    reference.origin,
                )?
            }
            TypeRefKind::Tuple(elements) => {
                let element_refs = self.package.type_refs(elements).to_vec();
                let mut element_types = Vec::with_capacity(element_refs.len());
                for element in element_refs {
                    element_types.push(self.resolve_type_ref(element, module, parameters)?);
                }
                let elements = self
                    .interner
                    .intern_list(&element_types)
                    .map_err(|error| self.at(reference.origin, error.into()))?;
                self.interner
                    .intern(TypeKind::Tuple(elements))
                    .map_err(|error| self.at(reference.origin, error.into()))?
            }
        };
        self.assignments.type_refs[type_ref.index()] = Some(type_id);
        Ok(type_id)
    }

    fn resolve_named_type(
        &mut self,
        name: Symbol,
        module: ModuleId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        if let Some(parameter) = parameters.get(&name) {
            return self
                .interner
                .intern(TypeKind::GenericParameter(*parameter))
                .map_err(|error| self.at(origin, error.into()));
        }
        let spelling = self
            .package
            .names
            .symbols
            .resolve(name)
            .expect("HIR symbols retain their spelling");
        if let Some(primitive) = primitive_type(spelling) {
            return Ok(self.interner.primitive(primitive));
        }
        if spelling == "Custom" {
            return self.fresh(InferenceLevel::ROOT.child(), origin);
        }
        if let Some(BindingTarget::Definition(definition)) =
            self.package.names.resolve_symbol(module, name)
        {
            if let Some(item_id) = self.definition_items[definition.index()]
                && let HirItemKind::TypeAlias { target } = self.package.items[item_id].kind
            {
                if !self.resolving_aliases.insert(definition) {
                    return Err(self.at(origin, crate::TypeError::TypeAliasCycle { definition }));
                }
                let result =
                    self.resolve_type_ref(target, self.package.items[item_id].module, parameters);
                self.resolving_aliases.remove(&definition);
                return result;
            }
            return self
                .interner
                .intern(TypeKind::Nominal {
                    definition,
                    arguments: self.interner.empty_list(),
                })
                .map_err(|error| self.at(origin, error.into()));
        }
        self.interner
            .intern(TypeKind::UnresolvedName { module, name })
            .map_err(|error| self.at(origin, error.into()))
    }

    fn resolve_applied_type(
        &mut self,
        base: Symbol,
        arguments: &[TypeId],
        module: ModuleId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let spelling = self
            .package
            .names
            .symbols
            .resolve(base)
            .expect("HIR symbols retain their spelling");
        let kind = match spelling {
            "List" if arguments.len() == 1 => TypeKind::List(arguments[0]),
            "Map" if arguments.len() == 2 => TypeKind::Map {
                key: arguments[0],
                value: arguments[1],
            },
            "Slice" if arguments.len() == 1 => TypeKind::Slice(arguments[0]),
            "Task" if arguments.len() == 1 => TypeKind::Task(arguments[0]),
            "Tuple" => {
                let arguments = self
                    .interner
                    .intern_list(arguments)
                    .map_err(|error| self.at(origin, error.into()))?;
                TypeKind::Tuple(arguments)
            }
            _ => {
                if let Some(parameter) = parameters.get(&base) {
                    return self
                        .interner
                        .intern(TypeKind::GenericParameter(*parameter))
                        .map_err(|error| self.at(origin, error.into()));
                }
                let Some(BindingTarget::Definition(definition)) =
                    self.package.names.resolve_symbol(module, base)
                else {
                    return self
                        .interner
                        .intern(TypeKind::UnresolvedName { module, name: base })
                        .map_err(|error| self.at(origin, error.into()));
                };
                let arguments = self
                    .interner
                    .intern_list(arguments)
                    .map_err(|error| self.at(origin, error.into()))?;
                TypeKind::Nominal {
                    definition,
                    arguments,
                }
            }
        };
        self.interner
            .intern(kind)
            .map_err(|error| self.at(origin, error.into()))
    }

    pub(super) fn ensure_local(
        &mut self,
        local: LocalId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
    ) -> Result<TypeId, ShadowTypeError> {
        if let Some(type_id) = self.assignments.locals[local.index()] {
            return Ok(type_id);
        }
        let declaration = self.package.locals[local];
        let type_id = if let Some(type_ref) = declaration.type_ref {
            let module = self.module_for_origin(declaration.origin);
            self.resolve_type_ref(type_ref, module, parameters)?
        } else {
            self.fresh(InferenceLevel::ROOT.child(), declaration.origin)?
        };
        self.assignments.locals[local.index()] = Some(type_id);
        Ok(type_id)
    }
}

fn primitive_type(name: &str) -> Option<PrimitiveType> {
    Some(match name {
        "Void" => PrimitiveType::Void,
        "Bool" => PrimitiveType::Bool,
        "Int" => PrimitiveType::Int,
        "Float" => PrimitiveType::Float,
        "Str" | "String" => PrimitiveType::String,
        "Char" => PrimitiveType::Char,
        "U8" | "u8" => PrimitiveType::U8,
        "U16" | "u16" => PrimitiveType::U16,
        "U32" | "u32" => PrimitiveType::U32,
        "I8" | "i8" => PrimitiveType::I8,
        "I16" | "i16" => PrimitiveType::I16,
        "I32" | "i32" => PrimitiveType::I32,
        "StrSlice" => PrimitiveType::StrSlice,
        "VectorI64x2" => PrimitiveType::VectorI64x2,
        _ => return None,
    })
}
