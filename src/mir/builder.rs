use crate::mir::ir::*;
use crate::type_facts::LifetimeClass;
use crate::types::TypeRef;

pub struct MirBuilder {
    pub function: MirFunction,
    current_block: Option<BlockId>,
    next_block: usize,
}

impl MirBuilder {
    pub fn new(id: FuncId, name: String, return_type: TypeRef) -> Self {
        let mut builder = Self {
            function: MirFunction {
                id,
                name,
                params: Vec::new(),
                locals: Vec::new(),
                blocks: Vec::new(),
                start_block: BlockId(0), // Will be updated
                return_type,
                is_async: false,
            },
            current_block: None,
            next_block: 0,
        };

        let start_block = builder.new_block();
        builder.function.start_block = start_block;
        builder.current_block = Some(start_block);

        builder
    }

    pub fn new_local(
        &mut self,
        ty: TypeRef,
        is_mut: bool,
        debug_name: Option<String>,
        binding_id: Option<crate::semantic::BindingId>,
    ) -> LocalId {
        let id = LocalId(self.function.locals.len());
        // Lowering must know which values are pointer-shaped, but it must not
        // decide their final storage class. Managed locals start in the
        // `Managed` state; the MIR escape solver later decides Frame, Owned, or
        // Shared and pass_escape/pass_arc turn that fact into stack or ARC
        // operations.
        //
        // `Str` and `List[Str]` are managed as of the string-provenance change.
        // They used to be excluded because a `Str` local can hold a static
        // string literal, and a literal had no ARC header -- releasing one
        // decremented whatever happened to sit in front of .rodata. Literals
        // now carry a real 24-byte header whose refcount is the immortal
        // sentinel (see `lower.rs`'s `Operand::String` and `lpp__is_immortal`
        // in both runtimes), so release on a literal is a checked no-op and
        // release on a heap string frees it. Excluding them cost an unbounded
        // leak -- one allocation per `str_concat`, forever.
        //
        // `List[Float]` is managed for the same reason `List[Int]` is: the
        // list object itself is ARC-allocated regardless of element type.
        let ownership = match ty.lifetime_class() {
            LifetimeClass::Managed => Ownership::Managed,
            LifetimeClass::BorrowedView => Ownership::Borrowed,
            LifetimeClass::Copy => Ownership::Copy,
        };
        self.function.locals.push(LocalDecl {
            id,
            ty,
            is_mut,
            debug_name,
            binding_id,
            ownership,
        });
        id
    }

    pub fn set_local_ownership(&mut self, local: LocalId, ownership: Ownership) {
        self.function.locals[local.0].ownership = ownership;
    }

    pub fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.next_block);
        self.next_block += 1;
        self.function.blocks.push(MirBlock {
            id,
            instrs: Vec::new(),
            terminator: Terminator::Unreachable,
        });
        id
    }

    pub fn switch_to_block(&mut self, block: BlockId) {
        self.current_block = Some(block);
    }

    pub fn current_block(&self) -> Result<BlockId, String> {
        self.current_block
            .ok_or_else(|| "MIR builder lost track of the current block".to_string())
    }

    pub fn push_instr(&mut self, instr: MirInstr) -> Result<(), String> {
        let current_id = self.current_block()?;
        let block = self
            .function
            .blocks
            .iter_mut()
            .find(|b| b.id == current_id)
            .ok_or_else(|| {
                format!(
                    "MIR block {:?} not found for instruction insertion",
                    current_id
                )
            })?;
        block.instrs.push(instr);
        Ok(())
    }

    pub fn set_terminator(
        &mut self,
        block_id: BlockId,
        terminator: Terminator,
    ) -> Result<(), String> {
        let block = self
            .function
            .blocks
            .iter_mut()
            .find(|b| b.id == block_id)
            .ok_or_else(|| {
                format!(
                    "MIR block {:?} not found for terminator insertion",
                    block_id
                )
            })?;
        block.terminator = terminator;
        Ok(())
    }

    pub fn terminate_current_block(&mut self, terminator: Terminator) -> Result<(), String> {
        let current_id = self.current_block()?;
        self.set_terminator(current_id, terminator)?;
        self.current_block = None;
        Ok(())
    }

    pub fn finish(self) -> MirFunction {
        self.function
    }
}
