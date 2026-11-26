use super::*;

/// Pre-scan operand class with typed, function-attributed handling:
/// a bare function value is a 16-byte capsule on the ARC heap (5D2b
/// slice 2; slice 3 extends it to async functions, whose capsule
/// points at the task thunk with an empty task-env tuple).
pub(super) fn scan_operand_class(
    lower: &mut ScanLower,
    operand: &Operand,
) -> Result<Class, CodegenError> {
    if matches!(operand, Operand::Function(_)) {
        // A bare function value materializes a 16-byte capsule
        // `[code, env]` on the ARC heap.
        *lower.has_managed = true;
        *lower.has_function_value = true;
        return operand_class(lower.program, lower.types, operand);
    }
    // A task handle is a 32-byte ARC node (5D2b slice 3).
    if matches!(operand, Operand::Copy(local)
    if matches!(
        lower.types.kind(lower.program.local(*local).unwrap().ty),
        TypeKind::Task(_)
    )) {
        *lower.has_managed = true;
        return Ok(Class::Ptr);
    }
    let class = operand_class(lower.program, lower.types, operand)?;
    if class == Class::Str {
        // Pooled strings use an immortal count, while dynamically produced
        // strings use ordinary ARC. Both safely share Retain/Release.
        *lower.has_managed = true;
    }
    Ok(class)
}

pub(crate) fn collect_strings(
    program: &MirProgram,
    rvalue: &Rvalue,
    strings: &mut BTreeSet<String>,
) {
    for operand in rvalue_operands(program, rvalue) {
        if let Operand::Constant(Constant::String { string, .. }) = operand
            && let Some(content) = program.string(string)
        {
            strings.insert(content.clone());
        }
    }
}

pub(crate) fn rvalue_operands(program: &MirProgram, rvalue: &Rvalue) -> Vec<Operand> {
    let mut out = Vec::new();
    match rvalue {
        Rvalue::Use(o) => out.push(*o),
        Rvalue::Unary { operand, .. } => out.push(*operand),
        Rvalue::Binary { left, right, .. } => {
            out.push(*left);
            out.push(*right);
        }
        Rvalue::Call { callee, arguments } => {
            out.push(*callee);
            out.extend_from_slice(program.operands(*arguments));
        }
        Rvalue::ExternalCall { arguments, .. } => {
            out.extend_from_slice(program.operands(*arguments));
        }
        Rvalue::Builtin { arguments, .. } => out.extend_from_slice(program.operands(*arguments)),
        Rvalue::List(items) => out.extend_from_slice(program.operands(*items)),
        Rvalue::Tuple(items) => out.extend_from_slice(program.operands(*items)),
        Rvalue::ConstructStruct { fields, .. } => out.extend_from_slice(program.operands(*fields)),
        Rvalue::ConstructVariant { fields, .. } => out.extend_from_slice(program.operands(*fields)),
        Rvalue::Load(_) | Rvalue::ListLen(_) => {}
        Rvalue::MakeClosure { captures, .. } => out.extend_from_slice(program.operands(*captures)),
        Rvalue::Await(o) | Rvalue::Spawn(o) | Rvalue::Cast { value: o, .. } => out.push(*o),
        Rvalue::Intrinsic { arguments, .. } => out.extend_from_slice(program.operands(*arguments)),
    }
    out
}

// ---------------------------------------------------------------------------
// Place resolution (5D2a): the shared field-offset / list-index walk used
// by the pre-scan and the lowering, mirroring the 5C Cranelift resolver.
// ---------------------------------------------------------------------------

/// One step of a place projection.
#[derive(Debug, Clone, Copy)]
pub(crate) enum PlaceStep {
    /// An enum downcast: a no-op at the machine level (the dominating
    /// `SwitchEnum` already dispatched on the tag).
    Downcast,
    /// A struct field or a downcast enum's variant field.
    Field {
        offset: u32,
        ty: TypeId,
        managed: bool,
    },
    /// A list element (the index operand resolved at the site).
    ListIndex { element: TypeId, index: Operand },
    /// A boxed tuple slot. Tuple payload words begin after the mask/count
    /// header and each element occupies one 8-byte word.
    TupleField {
        offset: u32,
        ty: TypeId,
        managed: bool,
    },
}

/// The resolved form of a place: the root local plus its steps.
pub(crate) struct PlaceResolution {
    pub(crate) root: MirLocalId,
    pub(crate) steps: Vec<PlaceStep>,
    pub(crate) ty: TypeId,
}

pub(crate) fn resolve_place(
    program: &MirProgram,
    types: &TypeInterner,
    place: lpp_mir::MirPlaceId,
    layouts: &BTreeMap<MirAggregateId, AggregateLayout>,
    aggregates: &AggregateIndex,
    for_store: bool,
    fn_id: MirFunctionId,
) -> Result<PlaceResolution, CodegenError> {
    let descriptor = program.place(place).unwrap();
    let projections: Vec<PlaceProjection> = program.place_projections(descriptor).to_vec();
    if for_store && matches!(projections.last(), Some(PlaceProjection::Downcast(_))) {
        return Err(unsupported("store through downcast", Some(fn_id)));
    }

    let mut current_ty = program.local(descriptor.root).unwrap().ty;
    let mut steps = Vec::new();
    // The enum variant made active by the most recent downcast
    // (aggregate, variant ordinal).
    let mut active_enum: Option<(MirAggregateId, u32)> = None;

    for (position, projection) in projections.iter().enumerate() {
        let is_last = position + 1 == projections.len();
        match projection {
            PlaceProjection::Downcast(variant) => {
                let TypeKind::Nominal { .. } = types.kind(current_ty) else {
                    return Err(unsupported("downcast of non-nominal", Some(fn_id)));
                };
                let aggregate = aggregates
                    .aggregate_for(current_ty)
                    .ok_or_else(|| unsupported("unmapped nominal type", Some(fn_id)))?;
                let agg = program.aggregate(aggregate).unwrap();
                if agg.kind != lpp_mir::MirAggregateKind::Enum {
                    return Err(unsupported("downcast of struct", Some(fn_id)));
                }
                let variant = program.variant(*variant).unwrap();
                if variant.aggregate != aggregate {
                    return Err(unsupported("downcast aggregate mismatch", Some(fn_id)));
                }
                active_enum = Some((aggregate, variant.ordinal));
                steps.push(PlaceStep::Downcast);
            }
            PlaceProjection::Field(field) => {
                let TypeKind::Nominal { .. } = types.kind(current_ty) else {
                    return Err(unsupported("field of non-nominal", Some(fn_id)));
                };
                let aggregate = aggregates
                    .aggregate_for(current_ty)
                    .ok_or_else(|| unsupported("unmapped nominal type", Some(fn_id)))?;
                let layout = layouts.get(&aggregate).unwrap();
                let slots = match layout.kind {
                    lpp_mir::MirAggregateKind::Struct => &layout.struct_fields,
                    lpp_mir::MirAggregateKind::Enum => {
                        let (enum_aggregate, ordinal) = active_enum.take().ok_or_else(|| {
                            unsupported("enum field without downcast", Some(fn_id))
                        })?;
                        if enum_aggregate != aggregate {
                            return Err(unsupported("downcast aggregate mismatch", Some(fn_id)));
                        }
                        &layout.variant_fields[ordinal as usize]
                    }
                };
                let slot = slots
                    .iter()
                    .find(|slot| slot.field == *field)
                    .ok_or_else(|| unsupported("unknown field offset", Some(fn_id)))?;
                let managed = is_managed(types, slot.ty);
                if !is_last && !managed {
                    return Err(unsupported("projection through scalar field", Some(fn_id)));
                }
                steps.push(PlaceStep::Field {
                    offset: slot.offset,
                    ty: slot.ty,
                    managed,
                });
                current_ty = slot.ty;
            }
            PlaceProjection::ListIndex(index) => {
                let TypeKind::List(element) = types.kind(current_ty) else {
                    return Err(unsupported("index of non-list", Some(fn_id)));
                };
                match index {
                    Operand::Copy(local) => {
                        let ty = program.local(*local).unwrap().ty;
                        if types.kind(ty) != TypeKind::Primitive(PrimitiveType::Int) {
                            return Err(unsupported("non-int list index", Some(fn_id)));
                        }
                    }
                    Operand::Constant(Constant::Integer(_)) => {}
                    _ => return Err(unsupported("non-int list index", Some(fn_id))),
                }
                if !is_last && !is_managed(types, element) {
                    return Err(unsupported(
                        "projection through scalar list element",
                        Some(fn_id),
                    ));
                }
                steps.push(PlaceStep::ListIndex {
                    element,
                    index: *index,
                });
                current_ty = element;
            }
            PlaceProjection::TupleField(index) => {
                let TypeKind::Tuple(elements) = types.kind(current_ty) else {
                    return Err(unsupported("tuple field of non-tuple", Some(fn_id)));
                };
                let element_types = types.list(elements);
                let ty = *element_types
                    .get(*index as usize)
                    .ok_or_else(|| unsupported("tuple field index", Some(fn_id)))?;
                let managed = is_managed(types, ty);
                if !is_last && !managed {
                    return Err(unsupported(
                        "projection through scalar tuple element",
                        Some(fn_id),
                    ));
                }
                steps.push(PlaceStep::TupleField {
                    offset: 16 + 8 * *index,
                    ty,
                    managed,
                });
                current_ty = ty;
            }
        }
    }

    if current_ty != descriptor.ty {
        return Err(unsupported("place type mismatch", Some(fn_id)));
    }
    Ok(PlaceResolution {
        root: descriptor.root,
        steps,
        ty: current_ty,
    })
}
