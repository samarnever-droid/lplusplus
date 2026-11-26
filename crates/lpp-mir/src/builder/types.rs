use super::*;

impl<'input> MirBuilder<'input> {
    pub(super) fn type_of_item(
        &self,
        item: HirItemId,
        origin: OriginId,
    ) -> Result<TypeId, MirBuildError> {
        self.types.assignments.item(item).ok_or_else(|| {
            self.error(
                origin,
                MirBuildErrorKind::MissingType(TypedEntity::Function(item)),
            )
        })
    }

    pub(super) fn materialize_type(
        &mut self,
        substitution: &TypeSubstitution,
        ty: TypeId,
        origin: OriginId,
    ) -> Result<TypeId, MirBuildError> {
        // Resolve inference variables bound during shadow inference. The
        // shadow stage normalizes assignments, but a type can still carry a
        // variable at the point it is read (opaque slots are bound by the
        // first use, which may occur after the let). MIR stores concrete
        // types only: a variable that remains unbound is a checker bug and
        // fails the build with a materialization error.
        let ty = self
            .types
            .inference
            .normalize(&mut self.types.interner, ty, &mut self.type_budget)
            .map_err(|error| {
                self.error(
                    origin,
                    match error {
                        TypeError::WorkLimitExceeded { .. } => {
                            MirBuildErrorKind::Capacity(MirCapacity::TypeWork)
                        }
                        TypeError::DepthLimitExceeded { .. } => {
                            MirBuildErrorKind::Capacity(MirCapacity::TypeDepth)
                        }
                        _ => MirBuildErrorKind::GenericTypeMaterialization,
                    },
                )
            })?;
        if matches!(self.types.interner.kind(ty), TypeKind::InferenceVariable(_)) {
            return Err(self.error(origin, MirBuildErrorKind::GenericTypeMaterialization));
        }
        if substitution.is_empty() {
            return Ok(ty);
        }
        substitution
            .apply(&mut self.types.interner, ty, &mut self.type_budget)
            .map_err(|error| {
                self.error(
                    origin,
                    match error {
                        TypeError::WorkLimitExceeded { .. } => {
                            MirBuildErrorKind::Capacity(MirCapacity::TypeWork)
                        }
                        TypeError::DepthLimitExceeded { .. } => {
                            MirBuildErrorKind::Capacity(MirCapacity::TypeDepth)
                        }
                        _ => MirBuildErrorKind::GenericTypeMaterialization,
                    },
                )
            })
    }

    /// The by-reference capture cell type: the single-element list that
    /// holds the captured value.
    pub(super) fn cell_type(
        &mut self,
        element: TypeId,
        origin: OriginId,
    ) -> Result<TypeId, MirBuildError> {
        self.types
            .interner
            .intern(TypeKind::List(element))
            .map_err(|_| self.error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage)))
    }

    /// The raw source text covered by `span`, when the span is well-formed.
    pub(super) fn source_text(&self, span: Span) -> Option<&str> {
        let file = self.sources.get(span.file)?;
        let text = file.text();
        let start = usize::try_from(span.start).ok()?;
        let end = usize::try_from(span.end).ok()?;
        if start >= end || end > text.len() {
            return None;
        }
        Some(&text[start..end])
    }

    pub(super) fn decode_string_span(&self, span: Span) -> Option<String> {
        let text = self.source_text(span)?;
        unescape_literal(text.strip_prefix('"')?.strip_suffix('"')?)
    }

    pub(super) fn is_external_function(&self, item: HirItemId) -> bool {
        self.external_items.contains_key(&item)
    }

    pub(super) fn register_external_function(
        &mut self,
        item_id: HirItemId,
        origin: OriginId,
    ) -> Result<MirExternalFunctionId, MirBuildError> {
        if let Some(&external) = self.external_function_ids.get(&item_id) {
            return Ok(external);
        }
        let Some((abi_span, library_span)) = self.external_items.get(&item_id).copied() else {
            return Err(self.error(
                origin,
                MirBuildErrorKind::Unsupported(UnsupportedConstruct::ExternalFunction),
            ));
        };
        let abi = self
            .decode_string_span(abi_span)
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidExternalAbi))?;
        if abi != "C" {
            return Err(self.error(origin, MirBuildErrorKind::InvalidExternalAbi));
        }
        let link_library = library_span
            .map(|span| {
                let library = self
                    .decode_string_span(span)
                    .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidExternalLibrary))?;
                let safe = !library.is_empty()
                    && !library.starts_with('-')
                    && library != "."
                    && library != ".."
                    && library.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+' | b'.')
                    });
                if !safe {
                    return Err(self.error(origin, MirBuildErrorKind::InvalidExternalLibrary));
                }
                Ok(library)
            })
            .transpose()?;
        let item = self.package.items[item_id];
        let HirItemKind::Function(function) = item.kind else {
            return Err(self.error(origin, MirBuildErrorKind::UnsupportedExternalSignature));
        };
        if function.body.is_some()
            || function.is_async
            || function.variadic
            || !self
                .package
                .type_parameters(function.type_parameters)
                .is_empty()
            || self
                .package
                .locals(function.parameters)
                .iter()
                .any(|local| self.package.locals[*local].default.is_some())
        {
            return Err(self.error(origin, MirBuildErrorKind::UnsupportedExternalSignature));
        }
        let ty = self.types.assignments.item(item_id).ok_or_else(|| {
            self.error(
                origin,
                MirBuildErrorKind::MissingType(TypedEntity::Function(item_id)),
            )
        })?;
        let TypeKind::Function { parameters, result } = self.types.interner.kind(ty) else {
            return Err(self.error(origin, MirBuildErrorKind::InvalidFunctionType(ty)));
        };
        let supported_primitive = |ty| {
            matches!(
                self.types.interner.kind(ty),
                TypeKind::Primitive(
                    PrimitiveType::Void
                        | PrimitiveType::Bool
                        | PrimitiveType::Int
                        | PrimitiveType::Float
                        | PrimitiveType::String
                        | PrimitiveType::U8
                        | PrimitiveType::U16
                        | PrimitiveType::U32
                        | PrimitiveType::U64
                        | PrimitiveType::USize
                        | PrimitiveType::I8
                        | PrimitiveType::I16
                        | PrimitiveType::I32
                        | PrimitiveType::ISize
                        | PrimitiveType::RawPtr
                ) | TypeKind::Ptr(_)
            )
        };
        // `Str` is an ARC-managed value, not an owned C string. Its payload
        // is NUL-terminated, so an external call may borrow its pointer for
        // the duration of the call; foreign functions must not mutate or
        // retain that pointer. Returned C strings are copied by native codegen.
        if !supported_primitive(result)
            || self
                .types
                .interner
                .list(parameters)
                .iter()
                .any(|parameter| {
                    !supported_primitive(*parameter)
                        || matches!(
                            self.types.interner.kind(*parameter),
                            TypeKind::Primitive(PrimitiveType::Void)
                        )
                })
        {
            return Err(self.error(origin, MirBuildErrorKind::UnsupportedExternalSignature));
        }
        let name = item
            .name
            .and_then(|symbol| self.package.names.symbols.resolve(symbol))
            .ok_or_else(|| {
                self.error(
                    origin,
                    MirBuildErrorKind::Unsupported(UnsupportedConstruct::ExternalFunction),
                )
            })?
            .to_owned();
        let external = self
            .program
            .external_functions
            .alloc(MirExternalFunction {
                source: item_id,
                name,
                abi,
                link_library,
                ty,
                origin: item.origin,
            })
            .map_err(|_| self.error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage)))?;
        self.external_function_ids.insert(item_id, external);
        Ok(external)
    }

    pub(super) fn intern_string(
        &mut self,
        text: String,
        origin: OriginId,
    ) -> Result<MirStringId, MirBuildError> {
        if let Some(&id) = self.string_cache.get(&text) {
            return Ok(id);
        }
        self.check_next(
            MirCapacity::Strings,
            self.counts.strings,
            self.options.max_strings,
            origin,
        )?;
        let id =
            self.program.strings.alloc(text.clone()).map_err(|_| {
                self.error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage))
            })?;
        self.counts.strings += 1;
        self.string_cache.insert(text, id);
        Ok(id)
    }

    pub(super) fn materialize_string(
        &mut self,
        span: Span,
        formatted: bool,
        origin: OriginId,
    ) -> Result<MirStringId, MirBuildError> {
        if formatted {
            // A formatted-string *segment*: the HIR desugaring already stripped
            // the `f"..."` wrapper and split out interpolations, so `span`
            // covers raw inner text with no surrounding quotes. An empty span is
            // the empty string (the synthetic leading segment of the desugaring).
            // Unescape the raw text and intern it as an ordinary string constant.
            let text = if span.start >= span.end {
                ""
            } else {
                self.source_text(span)
                    .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?
            };
            let decoded = unescape_literal(text)
                .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
            return self.intern_string(decoded, origin);
        }
        let text = self
            .source_text(span)
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
        let triple = text.starts_with("\"\"\"");
        let inner = if triple {
            text.strip_prefix("\"\"\"")
                .and_then(|inner| inner.strip_suffix("\"\"\""))
        } else {
            text.strip_prefix('\"')
                .and_then(|inner| inner.strip_suffix('\"'))
        };
        let Some(inner) = inner else {
            return Err(self.error(origin, MirBuildErrorKind::InvalidLiteral));
        };
        let decoded = unescape_literal(inner)
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
        self.intern_string(decoded, origin)
    }

    pub(super) fn materialize_raw_string(
        &mut self,
        span: Span,
        origin: OriginId,
    ) -> Result<MirStringId, MirBuildError> {
        let text = self
            .source_text(span)
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
        let body = text
            .strip_prefix('r')
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
        let inner = if body.starts_with("\"\"\"") {
            body.strip_prefix("\"\"\"")
                .and_then(|inner| inner.strip_suffix("\"\"\""))
        } else {
            body.strip_prefix('\"')
                .and_then(|inner| inner.strip_suffix('\"'))
        };
        let Some(inner) = inner else {
            return Err(self.error(origin, MirBuildErrorKind::InvalidLiteral));
        };
        self.intern_string(inner.to_owned(), origin)
    }

    pub(super) fn materialize_character(
        &self,
        span: Span,
        origin: OriginId,
    ) -> Result<char, MirBuildError> {
        let text = self
            .source_text(span)
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
        let inner = text
            .strip_prefix('\'')
            .and_then(|inner| inner.strip_suffix('\''))
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
        let decoded = unescape_literal(inner)
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidLiteral))?;
        let mut characters = decoded.chars();
        let Some(character) = characters.next() else {
            return Err(self.error(origin, MirBuildErrorKind::InvalidLiteral));
        };
        if characters.next().is_some() {
            return Err(self.error(origin, MirBuildErrorKind::InvalidLiteral));
        }
        Ok(character)
    }

    pub(super) fn check_next(
        &self,
        capacity: MirCapacity,
        current: usize,
        limit: usize,
        origin: OriginId,
    ) -> Result<(), MirBuildError> {
        if current >= limit {
            Err(self.error(origin, MirBuildErrorKind::Capacity(capacity)))
        } else {
            Ok(())
        }
    }

    pub(super) const fn error(&self, origin: OriginId, kind: MirBuildErrorKind) -> MirBuildError {
        MirBuildError { origin, kind }
    }

    pub(super) fn storage_error(&self, origin: OriginId, _: StorageExhausted) -> MirBuildError {
        self.error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage))
    }
}
