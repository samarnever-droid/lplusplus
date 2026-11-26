use super::*;

impl<'hir> TypeChecker<'hir> {
    pub(in crate::check) fn builtin_function_type(
        &mut self,
        builtin: BuiltinId,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let descriptor = builtin.descriptor();
        // `slice(list, start, len)` produces a `Slice[T]` view over a `List[T]`,
        // relating the element type of the source list to the slice result so
        // `view[i]` can recover it. (`str_slice` keeps its dedicated `StrSlice`
        // primitive result and needs no element variable.)
        if descriptor.name == "slice" {
            let element = self.fresh(InferenceLevel::ROOT.child(), origin)?;
            let list_type = self
                .interner
                .intern(TypeKind::List(element))
                .map_err(|error| self.at(origin, error.into()))?;
            let slice_type = self
                .interner
                .intern(TypeKind::Slice(element))
                .map_err(|error| self.at(origin, error.into()))?;
            let int = self.interner.primitive(PrimitiveType::Int);
            let parameters = self
                .interner
                .intern_list(&[list_type, int, int])
                .map_err(|error| self.at(origin, error.into()))?;
            return self
                .interner
                .intern(TypeKind::Function {
                    parameters,
                    result: slice_type,
                })
                .map_err(|error| self.at(origin, error.into()));
        }
        // Container builtins relate their list argument (or result, for
        // `list_new`) to one shared element variable, so lists get concrete
        // types: `list_new` yields `List(T)` for a fresh `T`, and the
        // element slots of `list_push`/`list_get`/`list_set` bind `T`
        // through the values that touch the list.
        if matches!(
            descriptor.name,
            "list_new" | "list_push" | "list_get" | "get" | "list_set" | "list_len"
        ) {
            use lpp_runtime_abi::generated::SemanticAbiType;
            let element = self.fresh(InferenceLevel::ROOT.child(), origin)?;
            let list_type = self
                .interner
                .intern(TypeKind::List(element))
                .map_err(|error| self.at(origin, error.into()))?;
            let parameter_types = if descriptor.name == "list_new" {
                vec![]
            } else {
                descriptor
                    .semantic_parameters
                    .iter()
                    .enumerate()
                    .map(|(index, kind)| -> Result<TypeId, ShadowTypeError> {
                        Ok(match (index, kind) {
                            (0, _) => list_type,
                            (_, SemanticAbiType::Any) => element,
                            _ => self.semantic_builtin_type(*kind, origin)?,
                        })
                    })
                    .collect::<Result<Vec<_>, ShadowTypeError>>()?
            };
            let parameters = self
                .interner
                .intern_list(&parameter_types)
                .map_err(|error| self.at(origin, error.into()))?;
            let result = if matches!(descriptor.name, "list_get" | "get") {
                element
            } else if descriptor.name == "list_new" {
                list_type
            } else {
                self.semantic_builtin_type(descriptor.semantic_result, origin)?
            };
            return self
                .interner
                .intern(TypeKind::Function { parameters, result })
                .map_err(|error| self.at(origin, error.into()));
        }
        let str_list_type = if matches!(descriptor.name, "args" | "str_lines" | "str_join") {
            let str_ty = self.interner.primitive(PrimitiveType::String);
            Some(
                self.interner
                    .intern(TypeKind::List(str_ty))
                    .map_err(|error| self.at(origin, error.into()))?,
            )
        } else {
            None
        };
        let mut parameter_types = Vec::with_capacity(descriptor.semantic_parameters.len());
        for (index, kind) in descriptor.semantic_parameters.iter().enumerate() {
            if descriptor.name == "str_join" && index == 0 {
                parameter_types.push(str_list_type.expect("str_join computes str_list_type"));
            } else {
                parameter_types.push(self.semantic_builtin_type(*kind, origin)?);
            }
        }
        let parameters = self
            .interner
            .intern_list(&parameter_types)
            .map_err(|error| self.at(origin, error.into()))?;
        // Map membership predicates return an i64 flag in the frozen ABI but
        // are semantically boolean, so a program may use them directly as a
        // branch condition (`if map_has(m, k):`). Ground the result in `Bool`
        // so the condition type-checks and codegen accepts it.
        let result = if matches!(descriptor.name, "map_has" | "map_contains") {
            self.interner.primitive(PrimitiveType::Bool)
        } else if matches!(descriptor.name, "args" | "str_lines") {
            str_list_type.expect("args/str_lines compute str_list_type")
        } else {
            self.semantic_builtin_type(descriptor.semantic_result, origin)?
        };
        self.interner
            .intern(TypeKind::Function { parameters, result })
            .map_err(|error| self.at(origin, error.into()))
    }

    pub(in crate::check) fn semantic_builtin_type(
        &mut self,
        kind: lpp_runtime_abi::generated::SemanticAbiType,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        use lpp_runtime_abi::generated::SemanticAbiType;
        // Shadow policy: only ABI kinds that map unambiguously to a language
        // primitive are constrained. The `Any` slot is opaque — a heap
        // handle, an element, or a list all share the same ABI shape — and
        // `void` results are likewise unconstrained, so container builtins
        // keep the same flexibility the shadow stage had before semantic
        // descriptors existed. A raw `I64` slot, by contrast, is a language
        // integer in every source builtin of the registry (indices, counts,
        // checksums, scalar vector lanes): grounding it in `Int` is what
        // lets `print_int(abs(5))` materialize, while the `Any` slots keep
        // the element flexibility.
        Ok(match kind {
            SemanticAbiType::Any | SemanticAbiType::I32 => {
                self.fresh(InferenceLevel::ROOT.child(), origin)?
            }
            SemanticAbiType::I64 => self.interner.primitive(PrimitiveType::Int),
            SemanticAbiType::Bool => self.interner.primitive(PrimitiveType::Bool),
            SemanticAbiType::F64 => self.interner.primitive(PrimitiveType::Float),
            SemanticAbiType::Str => self.interner.primitive(PrimitiveType::String),
            SemanticAbiType::StrSlice => self.interner.primitive(PrimitiveType::StrSlice),
            SemanticAbiType::VectorI64x2 => self.interner.primitive(PrimitiveType::VectorI64x2),
            SemanticAbiType::Void => self.interner.primitive(PrimitiveType::Void),
        })
    }

    pub(in crate::check) fn record_builtin_call(&mut self, expression: ExprId, callee: ExprId) {
        let ExpressionKind::Name {
            symbol,
            binding: NameBinding::Unresolved,
        } = self.package.expressions[callee].kind
        else {
            return;
        };
        let spelling = self
            .package
            .names
            .symbols
            .resolve(symbol)
            .expect("HIR symbols retain their spelling");
        let builtin_spelling = if spelling == "_getpid" {
            "thread_id"
        } else {
            spelling
        };
        if let Some(builtin) = self.builtin_index.lookup(builtin_spelling) {
            let builtin =
                self.specialize_polymorphic_builtin(builtin_spelling, expression, builtin);
            self.builtins.record(expression, builtin);
        }
    }

    /// Some builtins are spelled generically (`len`) but bind to a single
    /// list-oriented runtime symbol in the ABI. When the argument is a string
    /// they must dispatch to the string-specific symbol instead, otherwise the
    /// string pointer is misread as a list header. This resolves that overload
    /// from the already-inferred argument type.
    pub(in crate::check) fn specialize_polymorphic_builtin(
        &mut self,
        spelling: &str,
        expression: ExprId,
        builtin: crate::BuiltinId,
    ) -> crate::BuiltinId {
        // Map key overload: the generic `map_put`/`map_get`/`map_has`/
        // `map_remove` builtins hash their key as an integer. When the key
        // argument is a string they must dispatch to the `_str` runtime symbol
        // (which hashes the string *contents* and compares with `strcmp`);
        // otherwise the string pointer is hashed as an opaque integer and two
        // equal strings at different addresses never match. The key is always
        // the second argument (index 1; the map is index 0).
        if let Some(str_variant) = match spelling {
            "map_put" => Some("lpp_map_put_str"),
            "map_get" => Some("lpp_map_get_str"),
            "map_has" => Some("lpp_map_has_str"),
            "map_remove" => Some("lpp_map_remove_str"),
            _ => None,
        } {
            if self.builtin_argument_is_string(expression, 1)
                && let Some(variant) = self.builtin_index.lookup(str_variant)
            {
                return variant;
            }
            return builtin;
        }
        if spelling != "len" {
            return builtin;
        }
        let arguments = match self.package.expressions[expression].kind {
            ExpressionKind::Call { arguments, .. }
            | ExpressionKind::GenericCall { arguments, .. } => arguments,
            _ => return builtin,
        };
        let Some(&argument) = self.package.expressions(arguments).first() else {
            return builtin;
        };
        let Some(argument_type) = self.assignments.expressions[argument.index()] else {
            return builtin;
        };
        let Ok(resolved) = self
            .inference
            .resolve(&self.interner, argument_type, &mut self.budget)
        else {
            return builtin;
        };
        let kind = self.interner.kind(resolved);
        if matches!(kind, TypeKind::Primitive(PrimitiveType::String))
            && let Some(str_len) = self.builtin_index.lookup("str_len")
        {
            return str_len;
        }
        // `len` on a slice view (`Slice[T]` or `StrSlice`) reads the view's
        // length field via `lpp_slice_len`, not the list header via
        // `lpp_list_len`.
        if matches!(
            kind,
            TypeKind::Slice(_) | TypeKind::Primitive(PrimitiveType::StrSlice)
        ) && let Some(slice_len) = self.builtin_index.lookup("slice_len")
        {
            return slice_len;
        }
        builtin
    }

    /// Whether the `index`-th argument of a builtin call resolves to a string.
    pub(in crate::check) fn builtin_argument_is_string(
        &mut self,
        expression: ExprId,
        index: usize,
    ) -> bool {
        let arguments = match self.package.expressions[expression].kind {
            ExpressionKind::Call { arguments, .. }
            | ExpressionKind::GenericCall { arguments, .. } => arguments,
            _ => return false,
        };
        let Some(&argument) = self.package.expressions(arguments).get(index) else {
            return false;
        };
        let Some(argument_type) = self.assignments.expressions[argument.index()] else {
            return false;
        };
        let Ok(resolved) = self
            .inference
            .resolve(&self.interner, argument_type, &mut self.budget)
        else {
            return false;
        };
        matches!(
            self.interner.kind(resolved),
            TypeKind::Primitive(PrimitiveType::String)
        )
    }

    /// Whether a `slice_get(view, i)` call's first argument resolves to a
    /// `StrSlice`. Such a call reads a character and returns a fresh 1-char
    /// string, so its result type is `Str` and codegen must call
    /// `lpp_str_slice_get` instead of the numeric `lpp_slice_get`. The v1 ABI
    /// is frozen and has no dedicated builtin for this, so the overload lives in
    /// the checker (result type) and codegen (runtime symbol) rather than in the
    /// builtin table.
    pub(in crate::check) fn slice_get_on_str_slice(&mut self, expression: ExprId) -> bool {
        let arguments = match self.package.expressions[expression].kind {
            ExpressionKind::Call { arguments, .. }
            | ExpressionKind::GenericCall { arguments, .. } => arguments,
            _ => return false,
        };
        let Some(&argument) = self.package.expressions(arguments).first() else {
            return false;
        };
        let Some(argument_type) = self.assignments.expressions[argument.index()] else {
            return false;
        };
        let Ok(resolved) = self
            .inference
            .resolve(&self.interner, argument_type, &mut self.budget)
        else {
            return false;
        };
        matches!(
            self.interner.kind(resolved),
            TypeKind::Primitive(PrimitiveType::StrSlice)
        )
    }
}
