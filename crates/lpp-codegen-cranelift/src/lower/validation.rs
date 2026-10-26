use super::*;

pub(super) fn err(function: MirFunctionId, kind: CodegenErrorKind) -> CodegenError {
    CodegenError::new(Some(function), kind)
}

pub(super) fn unsupported(function: MirFunctionId, construct: &'static str) -> CodegenError {
    err(
        function,
        CodegenErrorKind::UnsupportedConstruct { construct },
    )
}

pub(super) fn verify_failed(function: Option<MirFunctionId>, what: String) -> CodegenError {
    CodegenError::new(function, CodegenErrorKind::IrVerificationFailed(what))
}

pub(super) fn emission_failed(what: String) -> CodegenError {
    CodegenError::new(None, CodegenErrorKind::ObjectEmissionFailed(what))
}

/// The machine type of a value type, or `None` for the value types the
/// slice does not represent. Managed values (strings, lists, nominals)
/// are heap pointers; `Void` is handled by the caller.
pub(super) fn machine_type(types: &TypeInterner, ty: TypeId) -> Option<CLType> {
    Some(match types.kind(ty) {
        TypeKind::Primitive(PrimitiveType::Int) => cltypes::I64,
        TypeKind::Primitive(PrimitiveType::Float) => cltypes::F64,
        TypeKind::Primitive(PrimitiveType::Bool)
        | TypeKind::Primitive(PrimitiveType::U8)
        | TypeKind::Primitive(PrimitiveType::I8) => cltypes::I8,
        TypeKind::Primitive(PrimitiveType::U16) | TypeKind::Primitive(PrimitiveType::I16) => {
            cltypes::I16
        }
        TypeKind::Primitive(PrimitiveType::String) => cltypes::I64,
        TypeKind::Primitive(PrimitiveType::Char)
        | TypeKind::Primitive(PrimitiveType::U32)
        | TypeKind::Primitive(PrimitiveType::I32) => cltypes::I32,
        TypeKind::List(_) => cltypes::I64,
        // A slice view is a pointer to a (stack-allocated) view record.
        TypeKind::Slice(_) => cltypes::I64,
        TypeKind::Nominal { .. } => cltypes::I64,
        TypeKind::Task(_) => cltypes::I64,
        TypeKind::Function { .. } => cltypes::I64,
        // A tuple value is a pointer to its flat heap record (see `alloc_tuple`).
        TypeKind::Tuple(_) => cltypes::I64,
        TypeKind::Primitive(PrimitiveType::VectorI64x2) => cltypes::I64X2,
        TypeKind::Primitive(PrimitiveType::StrSlice) => cltypes::I64,
        _ => return None,
    })
}

pub(super) fn is_void_type(types: &TypeInterner, ty: TypeId) -> bool {
    matches!(types.kind(ty), TypeKind::Primitive(PrimitiveType::Void))
}

/// Whether a value type is ARC-managed (a native runtime heap object with a
/// 16-byte common header immediately before its payload; destructor-bearing
/// allocations additionally use an 8-byte prefix before that common header).
pub(super) fn is_managed(types: &TypeInterner, ty: TypeId) -> bool {
    matches!(
        types.kind(ty),
        TypeKind::Primitive(PrimitiveType::String)
            | TypeKind::List(_)
            | TypeKind::Nominal { .. }
            | TypeKind::Task(_)
            | TypeKind::Function { .. }
            // A structural tuple is an ARC heap record with a generated
            // per-shape destructor (`lpp_drop_t{n}`): it is retained,
            // released, and traced exactly like a nominal aggregate.
            // Tuples are immutable after construction (the frontend
            // rejects element assignment), so sharing the record under
            // refcounting is observably the oracle's value semantics.
            | TypeKind::Tuple(_)
    )
}

/// The typed-rejection label for a non-slice type.
pub(super) fn unsupported_construct_for(kind: TypeKind) -> &'static str {
    match kind {
        TypeKind::Error => "error type",
        TypeKind::Never => "never type",
        TypeKind::Tuple(_) => "tuple",
        TypeKind::Map { .. } => "map",
        TypeKind::Slice(_) => "slice",
        TypeKind::Task(_) => "task",
        TypeKind::Function { .. } => "function value",
        TypeKind::List(_) => "list",
        TypeKind::Nominal { .. } => "nominal",
        TypeKind::GenericParameter(_) => "generic parameter",
        TypeKind::BoundVariable(_) => "bound variable",
        TypeKind::InferenceVariable(_) => "inference variable",
        TypeKind::UnresolvedName { .. } => "unresolved name",
        TypeKind::Primitive(primitive) => match primitive {
            PrimitiveType::Void => "void",
            PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32 => "narrow primitive",
            PrimitiveType::StrSlice => "str slice",
            PrimitiveType::VectorI64x2 => "vector",
            _ => unreachable!("the slice primitives are mapped"),
        },
    }
}

pub(super) fn check_value_type(
    types: &TypeInterner,
    ty: TypeId,
    function: MirFunctionId,
) -> Result<CLType, CodegenError> {
    machine_type(types, ty)
        .ok_or_else(|| unsupported(function, unsupported_construct_for(types.kind(ty))))
}

/// The element class of a `List` for runtime-op dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ElementClass {
    Scalar,
    Float,
    Bool,
    Managed,
}

pub(super) fn element_class(types: &TypeInterner, element: TypeId) -> ElementClass {
    match types.kind(element) {
        TypeKind::Primitive(PrimitiveType::Float) => ElementClass::Float,
        TypeKind::Primitive(PrimitiveType::Bool) => ElementClass::Bool,
        TypeKind::Primitive(PrimitiveType::String)
        | TypeKind::List(_)
        | TypeKind::Nominal { .. }
        | TypeKind::Function { .. }
        // Structural tuples are ARC heap records (generated
        // destructor/trace), so they store in lists through the
        // managed element ops exactly like nominals.
        | TypeKind::Tuple(_) => ElementClass::Managed,
        TypeKind::Primitive(
            PrimitiveType::Int
            | PrimitiveType::Char
            | PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32,
        ) => ElementClass::Scalar,
        _ => unreachable!("list elements are checked by the pre-scan"),
    }
}

/// Validate that `element` is a type a runtime list can actually store.
///
/// This is deliberately stricter than `check_value_type`: a SIMD vector is a
/// perfectly valid machine value (`I64X2`) and a map is a valid value
/// type elsewhere, but the runtime list ops only cover scalar / float / bool /
/// managed (ARC) slots — there is no list storage class for a vector. Rejecting
/// an unstoreable element here, at the construction site in the pre-scan, turns
/// what used to be an `unreachable!` panic in `element_class` into a clean typed
/// `E5001` rejection. The accepted set mirrors `element_class` exactly.
pub(super) fn check_list_element(
    types: &TypeInterner,
    element: TypeId,
    function: MirFunctionId,
) -> Result<(), CodegenError> {
    match types.kind(element) {
        TypeKind::Primitive(
            PrimitiveType::Int
            | PrimitiveType::Char
            | PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32
            | PrimitiveType::Float
            | PrimitiveType::Bool
            | PrimitiveType::String,
        )
        | TypeKind::List(_)
        | TypeKind::Nominal { .. }
        | TypeKind::Function { .. }
        | TypeKind::Tuple(_) => Ok(()),
        other => Err(unsupported(function, unsupported_construct_for(other))),
    }
}

/// Whether a language type is a function value or a list of function
/// values (both carry the provenance class of what they hold).
pub(super) fn is_function_value_kind(types: &TypeInterner, kind: TypeKind) -> bool {
    match kind {
        TypeKind::Function { .. } => true,
        TypeKind::List(element) => {
            matches!(types.kind(element), TypeKind::Function { .. })
        }
        _ => false,
    }
}

/// The machine type of an operand in a value position.
pub(super) fn operand_type(
    program: &MirProgram,
    types: &TypeInterner,
    operand: &Operand,
    function: MirFunctionId,
) -> Result<CLType, CodegenError> {
    match operand {
        Operand::Copy(local) => {
            let ty = program
                .local(*local)
                .expect("validated MIR retains every local")
                .ty;
            check_value_type(types, ty, function)
        }
        Operand::Constant(constant) => Ok(match constant {
            Constant::Integer(_) => cltypes::I64,
            Constant::FloatBits(_) => cltypes::F64,
            Constant::Bool(_) => cltypes::I8,
            Constant::Character { .. } => cltypes::I32,
            Constant::String { .. } => cltypes::I64,
        }),
        Operand::Function(function_id) => {
            let callee = program
                .function(*function_id)
                .expect("validated MIR retains functions");
            check_value_type(types, callee.ty, function)
        }
    }
}

/// The language type kind of an operand (the machine type alone
/// cannot tell a string pointer from an integer).
pub(super) fn operand_language_kind(
    program: &MirProgram,
    types: &TypeInterner,
    operand: &Operand,
) -> Option<TypeKind> {
    Some(match operand {
        Operand::Copy(local) => types.kind(
            program
                .local(*local)
                .expect("validated MIR retains every local")
                .ty,
        ),
        Operand::Constant(constant) => match constant {
            Constant::Integer(_) | Constant::Character { .. } => {
                TypeKind::Primitive(PrimitiveType::Int)
            }
            Constant::FloatBits(_) => TypeKind::Primitive(PrimitiveType::Float),
            Constant::Bool(_) => TypeKind::Primitive(PrimitiveType::Bool),
            Constant::String { .. } => TypeKind::Primitive(PrimitiveType::String),
        },
        Operand::Function(_) => return None,
    })
}

// ── place resolution ───────────────────────────────────────────────────────
//
// A place's projection chain is resolved once (in the pre-scan) into
// concrete steps: each field step carries its payload offset, each
// list step its element type and index operand. `Downcast` contributes
// no step: the dispatch that justifies it is the dominating
// `SwitchEnum`.

#[derive(Debug, Clone, Copy)]
pub(super) enum PlaceStep {
    /// A struct field (or an enum field through a downcast) at `offset`
    /// within the current base payload.
    Field {
        offset: u32,
        ty: TypeId,
        managed: bool,
    },
    /// A list element (the index operand is checked by the pre-scan).
    ListIndex { element: TypeId, index: Operand },
}

pub(super) struct PlaceResolution {
    pub(super) root: MirLocalId,
    pub(super) steps: Vec<PlaceStep>,
    pub(super) ty: TypeId,
}

/// Every `ListIndex` step of a resolved place is a runtime element
/// read: the final step of a `Load`, and every intermediate step of a
/// load or store chain. Record the matching `lpp_list_get*` import.
pub(super) fn record_chain_gets(
    types: &TypeInterner,
    resolution: &PlaceResolution,
    imports: &mut BTreeSet<&'static str>,
) {
    for step in resolution.steps.iter() {
        if let PlaceStep::ListIndex { element, .. } = step {
            match element_class(types, *element) {
                ElementClass::Managed => {
                    imports.insert(IMP_LIST_GET_ARC);
                }
                ElementClass::Scalar => {
                    imports.insert(IMP_LIST_GET);
                }
                ElementClass::Float => {
                    imports.insert(IMP_LIST_GET_FLOAT);
                }
                ElementClass::Bool => {
                    imports.insert(IMP_LIST_GET_BOOL);
                }
            }
        }
    }
}

/// Resolve `place` into root + concrete steps, computing every field
/// offset from the program's own aggregate descriptors.
pub(super) fn resolve_place(
    program: &MirProgram,
    types: &TypeInterner,
    place: lpp_mir::MirPlaceId,
    aggregates: &AggregateIndex,
    layouts: &HashMap<MirAggregateId, AggregateLayout>,
    mir_id: MirFunctionId,
    for_store: bool,
) -> Result<PlaceResolution, CodegenError> {
    let descriptor = program.place(place).expect("validated MIR retains places");
    let projections: Vec<PlaceProjection> = program.place_projections(descriptor).to_vec();
    if for_store && matches!(projections.last(), Some(PlaceProjection::Downcast(_))) {
        return Err(unsupported(mir_id, "store through downcast"));
    }

    let mut current_ty = program
        .local(descriptor.root)
        .expect("validated MIR retains locals")
        .ty;
    let mut steps = Vec::new();
    // The enum variant made active by the most recent downcast.
    let mut active_enum: Option<(MirAggregateId, u32 /* variant ordinal */)> = None;

    for (position, projection) in projections.iter().enumerate() {
        let is_last = position + 1 == projections.len();
        match projection {
            PlaceProjection::Downcast(variant) => {
                let TypeKind::Nominal { .. } = types.kind(current_ty) else {
                    return Err(unsupported(mir_id, "downcast of non-nominal"));
                };
                let aggregate = aggregates
                    .aggregate_for(current_ty)
                    .ok_or_else(|| unsupported(mir_id, "unmapped nominal type"))?;
                let agg = program
                    .aggregate(aggregate)
                    .expect("validated MIR retains every aggregate");
                if agg.kind != lpp_mir::MirAggregateKind::Enum {
                    return Err(unsupported(mir_id, "downcast of struct"));
                }
                let variant = program
                    .variant(*variant)
                    .expect("validated MIR retains every variant");
                if variant.aggregate != aggregate {
                    return Err(unsupported(mir_id, "downcast aggregate mismatch"));
                }
                // A downcast is a no-op at the machine level: the
                // dominating SwitchEnum already dispatched on the tag.
                active_enum = Some((aggregate, variant.ordinal));
            }
            PlaceProjection::Field(field) => {
                let TypeKind::Nominal { .. } = types.kind(current_ty) else {
                    return Err(unsupported(mir_id, "field of non-nominal"));
                };
                let aggregate = aggregates
                    .aggregate_for(current_ty)
                    .ok_or_else(|| unsupported(mir_id, "unmapped nominal type"))?;
                let layout = layouts
                    .get(&aggregate)
                    .expect("pre-scan laid out every nominal");
                let slots = match layout.kind {
                    lpp_mir::MirAggregateKind::Struct => &layout.struct_fields,
                    lpp_mir::MirAggregateKind::Enum => {
                        let (enum_aggregate, ordinal) = active_enum
                            .take()
                            .ok_or_else(|| unsupported(mir_id, "enum field without downcast"))?;
                        if enum_aggregate != aggregate {
                            return Err(unsupported(mir_id, "downcast aggregate mismatch"));
                        }
                        &layout.variant_fields[ordinal as usize]
                    }
                };
                let slot = slots
                    .iter()
                    .find(|slot| slot.field == *field)
                    .ok_or_else(|| unsupported(mir_id, "unknown field offset"))?;
                let managed = is_managed(types, slot.ty);
                if !is_last && !managed {
                    return Err(unsupported(mir_id, "projection through scalar field"));
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
                    return Err(unsupported(mir_id, "index of non-list"));
                };
                match index {
                    Operand::Copy(local) => {
                        let ty = program
                            .local(*local)
                            .expect("validated MIR retains every local")
                            .ty;
                        if types.kind(ty) != TypeKind::Primitive(PrimitiveType::Int) {
                            return Err(unsupported(mir_id, "non-int list index"));
                        }
                    }
                    Operand::Constant(Constant::Integer(_)) => {}
                    _ => return Err(unsupported(mir_id, "non-int list index")),
                }
                if !is_last && !is_managed(types, element) {
                    return Err(unsupported(
                        mir_id,
                        "projection through scalar list element",
                    ));
                }
                steps.push(PlaceStep::ListIndex {
                    element,
                    index: *index,
                });
                current_ty = element;
            }
            PlaceProjection::TupleField(index) => {
                let Some((slots, _)) = crate::layout::tuple_layout(types, current_ty) else {
                    return Err(unsupported(mir_id, "tuple field of non-tuple"));
                };
                let (offset, element) = slots
                    .get(*index as usize)
                    .copied()
                    .ok_or_else(|| unsupported(mir_id, "tuple field out of range"))?;
                let managed = is_managed(types, element);
                if !is_last && !managed {
                    return Err(unsupported(mir_id, "projection through scalar tuple field"));
                }
                steps.push(PlaceStep::Field {
                    offset,
                    ty: element,
                    managed,
                });
                current_ty = element;
            }
        }
    }

    // A bare local (no steps) or a field/list-index target is a valid
    // store site; a trailing downcast was rejected above, and tuple
    // projections never appear.
    if current_ty != descriptor.ty {
        return Err(unsupported(mir_id, "place type mismatch"));
    }
    Ok(PlaceResolution {
        root: descriptor.root,
        steps,
        ty: current_ty,
    })
}
