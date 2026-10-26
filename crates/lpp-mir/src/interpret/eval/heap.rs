use super::*;

impl Interpreter<'_> {
    pub(super) fn allocate_heap(
        &mut self,
        function: MirFunctionId,
        block: Option<BasicBlockId>,
        origin: OriginId,
        node: HeapNode,
        produced: Option<TypeId>,
    ) -> Result<HeapId, InterpreterError> {
        if self.heap.len() >= self.limits.max_heap_nodes {
            return Err(self.error(
                Some(function),
                block,
                Some(origin),
                InterpreterErrorKind::LimitExceeded(InterpreterLimit::HeapNodes),
            ));
        }
        let id = u32::try_from(self.heap.len()).map_err(|_| {
            self.error(
                Some(function),
                block,
                Some(origin),
                InterpreterErrorKind::LimitExceeded(InterpreterLimit::HeapNodes),
            )
        })?;
        if let Some(arc) = self.arc.as_mut() {
            arc.refcounts.push(1);
            let pin = match produced {
                Some(ty) => NodePin::Of(ty),
                None => NodePin::Unknown,
            };
            arc.node_types.push(pin);
        }
        self.heap.push(node);
        Ok(HeapId(id))
    }

    /// The children a deallocation must release, in declaration order.
    pub(super) fn children_of(&self, id: HeapId) -> Vec<RuntimeValue> {
        let mut children: Vec<RuntimeValue> = Vec::new();
        match &self.heap[id.0 as usize] {
            HeapNode::List(elements) => children.extend(elements.iter().cloned()),
            HeapNode::Map(_) => {}
            HeapNode::Nominal { fields, .. } => children.extend(fields.iter().cloned()),
            HeapNode::String(_) => {}
            HeapNode::Closure { captures, .. } => children.extend(captures.iter().cloned()),
            HeapNode::Task {
                arguments,
                result,
                closure,
                ..
            } => {
                children.extend(arguments.iter().cloned());
                if let Some(result) = result {
                    children.push(result.clone());
                }
                if let Some(closure) = closure {
                    children.push(RuntimeValue::Closure(*closure));
                }
            }
        }
        children
    }

    /// Whether the node may participate in a value cycle and is thus
    /// eligible for trial-deletion collection: its own type is a 4D
    /// cycle member. Nodes without a recorded type are treated
    /// conservatively as collectible (trial deletion can see through
    /// them, and only ever frees nodes it proves unreachable).
    pub(in crate::interpret) fn node_is_collectible(&self, id: HeapId) -> bool {
        let Some(arc) = self.arc.as_ref() else {
            return false;
        };
        match arc.node_types.get(id.0 as usize) {
            None | Some(NodePin::Unknown) => true,
            Some(NodePin::Ephemeral) => false,
            Some(NodePin::Of(ty)) => arc.pinned.contains(ty),
        }
    }

    /// Classify a task node's collectibility: a task node can join a 4D
    /// cycle only through its contents, so it is collectible through
    /// the result type or the spawned closure's function type, and is
    /// ephemeral otherwise (detached nodes must be able to die with
    /// their last owner).
    pub(super) fn mark_task_node(&mut self, id: HeapId, target: MirFunctionId) {
        if self.arc.is_none() {
            return;
        }
        let Some(function) = self.program.function(target).copied() else {
            return;
        };
        let closure_ty = match &self.heap[id.0 as usize] {
            HeapNode::Task {
                closure: Some(closure),
                ..
            } => self
                .program
                .function(function_id_of_closure(self, *closure))
                .map(|f| f.ty),
            _ => None,
        };
        let pin = if is_heap_type(self.types, function.return_type) {
            NodePin::Of(function.return_type)
        } else if let Some(ty) = closure_ty {
            NodePin::Of(ty)
        } else {
            NodePin::Ephemeral
        };
        let index = id.0 as usize;
        self.arc.as_mut().unwrap().node_types[index] = pin;
    }

    /// Mark a node that cannot be a cycle member (string literals).
    pub(super) fn mark_ephemeral(&mut self, id: HeapId) {
        if let Some(arc) = self.arc.as_mut() {
            arc.node_types[id.0 as usize] = NodePin::Ephemeral;
        }
    }

    /// Gain one reference for a heap value (the borrow half of the call
    /// contract). A no-op for primitives and on the legacy entries.
    pub(super) fn retain_value(
        &mut self,
        function: MirFunctionId,
        block: Option<BasicBlockId>,
        origin: Option<OriginId>,
        value: &RuntimeValue,
    ) -> Result<(), InterpreterError> {
        if self.arc.is_none() || !is_heap_value(value) {
            return Ok(());
        }
        let index = heap_id_of(value).0 as usize;
        let Some(count) = self.arc.as_mut().unwrap().refcounts.get_mut(index) else {
            return Err(self.error(
                Some(function),
                block,
                origin,
                InterpreterErrorKind::OwnershipUnderflow,
            ));
        };
        *count = count.saturating_add(1);
        let arc = self.arc.as_mut().unwrap();
        arc.retains = arc.retains.saturating_add(1);
        Ok(())
    }

    /// Like `retain_value`, but looks through inline tuples: every heap
    /// element of a copied tuple gains a reference.
    pub(super) fn retain_owned(
        &mut self,
        function: MirFunctionId,
        block: Option<BasicBlockId>,
        origin: Option<OriginId>,
        value: &RuntimeValue,
    ) -> Result<(), InterpreterError> {
        if let RuntimeValue::Tuple(elements) = value {
            for element in elements {
                self.retain_owned(function, block, origin, element)?;
            }
            return Ok(());
        }
        self.retain_value(function, block, origin, value)
    }

    /// Drop one reference for a value that owns heap references (looking
    /// through inline tuples); when a node's count reaches zero it is
    /// deallocated and its children released, depth-first in declaration
    /// order (iteratively). A collectible node whose count stays above
    /// zero is buffered as a possible cycle root for trial deletion;
    /// when the buffer reaches `CYCLE_COLLECT_THRESHOLD` a collection
    /// runs before returning. A release with no live reference is an
    /// underflow.
    pub(super) fn drop_reference(
        &mut self,
        value: &RuntimeValue,
        function: Option<MirFunctionId>,
        block: Option<BasicBlockId>,
        origin: Option<OriginId>,
    ) -> Result<(), InterpreterError> {
        if self.arc.is_none() || !owns_heap_refs(value) {
            return Ok(());
        }
        let mut pending: Vec<RuntimeValue> = vec![value.clone()];
        while let Some(current) = pending.pop() {
            if let RuntimeValue::Tuple(elements) = &current {
                for element in elements.iter().rev() {
                    pending.push(element.clone());
                }
                continue;
            }
            // Primitive children own no references: skip them.
            if !is_heap_value(&current) {
                continue;
            }
            let id = heap_id_of(&current);
            let index = id.0 as usize;
            let count = *self
                .arc
                .as_ref()
                .unwrap()
                .refcounts
                .get(index)
                .ok_or_else(|| {
                    self.error(
                        function,
                        block,
                        origin,
                        InterpreterErrorKind::OwnershipUnderflow,
                    )
                })?;
            if count == 0 {
                return Err(self.error(
                    function,
                    block,
                    origin,
                    InterpreterErrorKind::OwnershipUnderflow,
                ));
            }
            let children = self.children_of(id);
            let arc = self.arc.as_mut().unwrap();
            let next = count - 1;
            arc.refcounts[index] = next;
            arc.releases = arc.releases.saturating_add(1);
            if next > 0 {
                // A collectible node that survived a release may be
                // kept alive only by a cycle through itself: buffer
                // it as a possible root for trial deletion.
                if self.node_is_collectible(id) {
                    self.arc.as_mut().unwrap().candidates.insert(index);
                }
                continue;
            }
            arc.frees = arc.frees.saturating_add(1);
            // A freed node is never a useful root; drop any stale
            // buffering so the candidate set tracks live nodes only.
            arc.candidates.remove(&index);
            for child in children.into_iter().rev() {
                pending.push(child);
            }
        }
        // Opportunistic collection bounds cyclic garbage during long
        // executions; `finish_arc` always collects what remains.
        let due = self
            .arc
            .as_ref()
            .is_some_and(|arc| !arc.collecting && arc.candidates.len() >= CYCLE_COLLECT_THRESHOLD);
        if due {
            self.collect_cycles(function, block, origin)?;
        }
        Ok(())
    }

    /// The heap-valued children of a stored value, flattened through
    /// inline tuples, one entry per owning edge (duplicates kept: two
    /// fields referencing one node are two references).
    pub(super) fn heap_children_of(value: &RuntimeValue, out: &mut Vec<RuntimeValue>) {
        match value {
            RuntimeValue::Tuple(elements) => {
                for element in elements {
                    Self::heap_children_of(element, out);
                }
            }
            other => {
                if is_heap_value(other) {
                    out.push(other.clone());
                }
            }
        }
    }

    /// Trial-deletion cycle collection (Bacon & Rajan): reclaim
    /// collectible nodes that are reachable only from themselves.
    ///
    /// From the buffered possible roots, walk the collectible subgraph
    /// subtracting one *scratch* count per internal edge. A node whose
    /// scratch count stays above zero is referenced from outside the
    /// subgraph — a live root — and everything reachable from it is
    /// live. The remaining nodes' reference counts are fully explained
    /// by edges among themselves: unreachable cyclic garbage. They are
    /// deallocated together; their edges to surviving nodes are
    /// released normally (which may cascade ordinary frees and buffer
    /// fresh candidates for a later round).
    ///
    /// Iterative throughout: deep graphs cannot overflow the stack.
    /// Deterministic: every set is ordered by heap index.
    pub(super) fn collect_cycles(
        &mut self,
        function: Option<MirFunctionId>,
        block: Option<BasicBlockId>,
        origin: Option<OriginId>,
    ) -> Result<(), InterpreterError> {
        let Some(arc) = self.arc.as_mut() else {
            return Ok(());
        };
        if arc.collecting || arc.candidates.is_empty() {
            return Ok(());
        }
        arc.collecting = true;
        let result = self.collect_cycles_inner(function, block, origin);
        if let Some(arc) = self.arc.as_mut() {
            arc.collecting = false;
        }
        result
    }

    pub(super) fn collect_cycles_inner(
        &mut self,
        function: Option<MirFunctionId>,
        block: Option<BasicBlockId>,
        origin: Option<OriginId>,
    ) -> Result<(), InterpreterError> {
        let candidates: Vec<usize> = {
            let arc = self.arc.as_mut().unwrap();
            std::mem::take(&mut arc.candidates).into_iter().collect()
        };

        // Mark-gray: trial deletion over the collectible subgraph.
        // `scratch[i]` starts at the live refcount and loses one per
        // edge from another gray node.
        let mut scratch: BTreeMap<usize, i64> = BTreeMap::new();
        let mut stack: Vec<usize> = Vec::new();
        for index in candidates {
            let live = self
                .arc
                .as_ref()
                .unwrap()
                .refcounts
                .get(index)
                .is_some_and(|count| *count > 0);
            if live
                && self.node_is_collectible(HeapId(index as u32))
                && !scratch.contains_key(&index)
            {
                let count = self.arc.as_ref().unwrap().refcounts[index] as i64;
                scratch.insert(index, count);
                stack.push(index);
            }
        }
        while let Some(index) = stack.pop() {
            let mut children = Vec::new();
            for child in self.children_of(HeapId(index as u32)) {
                Self::heap_children_of(&child, &mut children);
            }
            for child in children {
                let child_index = heap_id_of(&child).0 as usize;
                let live_count = self.arc.as_ref().unwrap().refcounts[child_index];
                if live_count == 0 || !self.node_is_collectible(heap_id_of(&child)) {
                    continue;
                }
                let entry = scratch.entry(child_index).or_insert_with(|| {
                    stack.push(child_index);
                    live_count as i64
                });
                *entry -= 1;
            }
        }
        if scratch.is_empty() {
            return Ok(());
        }

        // Scan: nodes with external references (scratch > 0) are live
        // roots; mark everything reachable from them live. The gray
        // remainder is garbage.
        let mut garbage: BTreeSet<usize> = scratch.keys().copied().collect();
        let mut live_stack: Vec<usize> = scratch
            .iter()
            .filter(|(_, count)| **count > 0)
            .map(|(index, _)| *index)
            .collect();
        while let Some(index) = live_stack.pop() {
            if !garbage.remove(&index) {
                continue;
            }
            let mut children = Vec::new();
            for child in self.children_of(HeapId(index as u32)) {
                Self::heap_children_of(&child, &mut children);
            }
            for child in children {
                let child_index = heap_id_of(&child).0 as usize;
                if garbage.contains(&child_index) {
                    live_stack.push(child_index);
                }
            }
        }
        if garbage.is_empty() {
            return Ok(());
        }

        // Collect: deallocate the garbage set together (counts to
        // zero first, so no member can be released twice), then
        // release edges that leave the set. Edges inside the set die
        // with it; that is precisely the cyclic reference that plain
        // refcounting could never drop.
        {
            let arc = self.arc.as_mut().unwrap();
            for index in &garbage {
                arc.refcounts[*index] = 0;
                arc.candidates.remove(index);
            }
            arc.frees = arc.frees.saturating_add(garbage.len());
            arc.collected = arc.collected.saturating_add(garbage.len());
        }
        for index in &garbage {
            let mut children = Vec::new();
            for child in self.children_of(HeapId(*index as u32)) {
                Self::heap_children_of(&child, &mut children);
            }
            for child in children {
                if garbage.contains(&(heap_id_of(&child).0 as usize)) {
                    continue;
                }
                self.drop_reference(&child, function, block, origin)?;
            }
        }
        Ok(())
    }

    /// End-of-execution balance proof for `execute_mir_arc`: the string
    /// cache holds one reference per interned literal and releases them,
    /// trial deletion reclaims every unreachable cycle, and then every
    /// heap node must be dead. A survivor's refcount cannot be explained
    /// by the heap graph — some owner never released it — so it is a
    /// genuine leak, never an exempted cycle.
    pub(in crate::interpret) fn finish_arc(&mut self) -> Result<(), InterpreterError> {
        if self.arc.is_none() {
            return Ok(());
        }
        let cached: Vec<HeapId> = self.string_cache.values().copied().collect();
        for id in cached {
            self.drop_reference(&RuntimeValue::String(id), None, None, None)?;
        }
        // Each round can expose fresh candidates (a cycle held only by
        // another cycle), so collect until the buffer drains. The bound
        // is structural: a round that frees nothing cannot enable a
        // later round to free anything.
        loop {
            let frees_before = self.arc.as_ref().unwrap().frees;
            self.collect_cycles(None, None, None)?;
            let arc = self.arc.as_ref().unwrap();
            if arc.candidates.is_empty() || arc.frees == frees_before {
                break;
            }
        }
        let leaked = self
            .arc
            .as_ref()
            .unwrap()
            .refcounts
            .iter()
            .filter(|count| **count > 0)
            .count();
        if leaked > 0 {
            // Audit the survivors against the heap graph so the
            // failure says *why* each node is still alive, not just
            // how many there are.
            let report = self.leak_report();
            let phantom = report.count(LeakClass::PhantomRoot);
            let closed = report.count(LeakClass::ClosedGarbage);
            self.arc.as_mut().unwrap().leak_report = Some(report);
            return Err(InterpreterError {
                function: None,
                block: None,
                origin: None,
                kind: InterpreterErrorKind::OwnershipLeak {
                    nodes: leaked,
                    phantom,
                    closed,
                },
            });
        }
        Ok(())
    }

    /// Classify every surviving heap node by recomputing owning
    /// edges from the live heap graph (the same `children_of` edges
    /// deallocation releases, flattened through inline tuples) and
    /// comparing against the modelled refcounts. Only called when
    /// the balance proof has already found survivors.
    pub(super) fn leak_report(&self) -> LeakReport {
        let arc = self.arc.as_ref().expect("leak audit needs arc state");
        let n = self.heap.len();
        let mut internal = vec![0u32; n];
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut flat: Vec<RuntimeValue> = Vec::new();
        for (index, outgoing) in edges.iter_mut().enumerate().take(n) {
            if arc.refcounts.get(index).copied().unwrap_or(0) == 0 {
                continue; // deallocated: its edges were released with it
            }
            for child in self.children_of(HeapId(index as u32)) {
                flat.clear();
                Self::heap_children_of(&child, &mut flat);
                for value in &flat {
                    let target = heap_id_of(value).0 as usize;
                    if target < n {
                        internal[target] = internal[target].saturating_add(1);
                        outgoing.push(target);
                    }
                }
            }
        }
        let classes = classify_leak_classes(&arc.refcounts, &internal, &edges);
        let mut nodes = Vec::new();
        for (index, class) in classes.into_iter().enumerate() {
            let Some(class) = class else { continue };
            nodes.push(LeakedNode {
                heap: index as u32,
                ty: match arc.node_types.get(index) {
                    Some(NodePin::Of(ty)) => Some(*ty),
                    _ => None,
                },
                kind: match &self.heap[index] {
                    HeapNode::List(_) => "list",
                    HeapNode::Map(_) => "map",
                    HeapNode::Nominal { .. } => "nominal",
                    HeapNode::String(_) => "string",
                    HeapNode::Closure { .. } => "closure",
                    HeapNode::Task { .. } => "task",
                },
                refcount: arc.refcounts.get(index).copied().unwrap_or(0),
                internal_refs: internal[index],
                collectible: self.node_is_collectible(HeapId(index as u32)),
                class,
            });
        }
        LeakReport { nodes }
    }
}
