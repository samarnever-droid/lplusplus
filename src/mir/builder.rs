use crate::mir::ir::*;
use crate::typecheck::TypeRef;

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
            },
            current_block: None,
            next_block: 0,
        };
        
        let start_block = builder.new_block();
        builder.function.start_block = start_block;
        builder.current_block = Some(start_block);
        
        builder
    }

    pub fn new_local(&mut self, ty: TypeRef, is_mut: bool, debug_name: Option<String>, binding_id: Option<crate::semantic::BindingId>) -> LocalId {
        let id = LocalId(self.function.locals.len());
        self.function.locals.push(LocalDecl {
            id,
            ty,
            is_mut,
            debug_name,
            binding_id,
        });
        id
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

    pub fn current_block(&self) -> BlockId {
        self.current_block.expect("No current block")
    }

    pub fn push_instr(&mut self, instr: MirInstr) {
        let current_id = self.current_block();
        let block = self.function.blocks.iter_mut().find(|b| b.id == current_id).unwrap();
        block.instrs.push(instr);
    }

    pub fn set_terminator(&mut self, block_id: BlockId, terminator: Terminator) {
        let block = self.function.blocks.iter_mut().find(|b| b.id == block_id).unwrap();
        block.terminator = terminator;
    }
    
    pub fn terminate_current_block(&mut self, terminator: Terminator) {
        let current_id = self.current_block();
        self.set_terminator(current_id, terminator);
        self.current_block = None;
    }
    
    pub fn finish(self) -> MirFunction {
        self.function
    }
}
