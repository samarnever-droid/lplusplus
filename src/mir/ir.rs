use crate::ast::BinaryOperator;
use crate::typecheck::TypeRef;
use std::collections::HashMap;

/// Unique identifier for a local variable or temporary binding within a MIR function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub usize);

/// Unique identifier for a Basic Block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub usize);

/// Unique identifier for a MIR Function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FuncId(pub usize);

/// Ownership class recorded in MIR. `Owned` values carry one ARC reference,
/// `Borrowed` values are valid only for the caller-owned lifetime, and `Copy`
/// values are plain scalars with no destructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    Copy,
    Owned,
    Borrowed,
}

/// A declared local variable or temporary.
#[derive(Debug, Clone)]
pub struct LocalDecl {
    pub id: LocalId,
    pub ty: TypeRef,
    pub is_mut: bool,
    /// Optional human-readable name for debugging
    pub debug_name: Option<String>,
    pub binding_id: Option<crate::semantic::BindingId>,
    pub ownership: Ownership,
}

/// An operand is either a constant value or a read from a Local.
#[derive(Debug, Clone)]
pub enum Operand {
    /// Read a scalar or owned temporary by value.
    Local(LocalId),
    /// Read an owned object without transferring its ARC reference. Ownership
    /// passes must retain before assigning this value to another owner.
    Borrowed(LocalId),
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
}

/// An Rvalue computes a new value from Operands.
/// Rvalues are side-effect free (mostly) except for calls, but represent the right-hand side of assignments.
#[derive(Debug, Clone)]
pub enum Rvalue {
    /// Copy a scalar or read a borrowed value. ARC retains are inserted when a
    /// borrowed object becomes another owner.
    Use(Operand),
    /// Transfer an owned temporary into the assignment destination. The source
    /// must not be released afterward by an ownership-aware cleanup pass.
    Move(LocalId),
    /// A mathematical or logical binary operation
    BinaryOp(BinaryOperator, Operand, Operand),
    /// A direct function call (to a known top-level function)
    CallDirect(FuncId, Vec<Operand>),
    /// An indirect function call (like invoking a closure)
    CallIndirect(Operand, Vec<Operand>),
    /// A call to a known L++ runtime builtin (print, input, read_file, etc.)
    /// The String is the canonical builtin name, e.g. "lpp_print_int".
    BuiltinCall(String, Vec<Operand>),
    /// Creates a closure. The first argument is the target function,
    /// the second is the list of captured variables (environment).
    MakeClosure(FuncId, Vec<Operand>),
    /// Reads a field from a custom struct
    FieldAccess(Operand, String),
    /// Legacy/raw struct allocation. AOT rejects this because it has no ARC header.
    AllocateStruct(TypeRef),
    /// Allocates a custom struct with one owned ARC reference.
    AllocateArcStruct(TypeRef),
    /// Allocates memory for a new list
    AllocateList(TypeRef),
    /// Spawns an asynchronous OS thread executing a closure callable.
    SpawnThread(Operand),
}

impl std::fmt::Display for Operand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Operand::Local(id) => write!(f, "_{}", id.0),
            Operand::Borrowed(id) => write!(f, "borrow(_{})", id.0),
            Operand::Int(i) => write!(f, "{}", i),
            Operand::Float(val) => write!(f, "{}", val),
            Operand::String(s) => write!(f, "\"{}\"", s),
            Operand::Bool(b) => write!(f, "{}", b),
        }
    }
}

impl std::fmt::Display for Rvalue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rvalue::Use(op) => write!(f, "{}", op),
            Rvalue::Move(local) => write!(f, "move(_{})", local.0),
            Rvalue::BinaryOp(op, l, r) => write!(f, "{} {:?} {}", l, op, r), // Note: using Debug for BinaryOp for now
            Rvalue::CallDirect(func_id, args) => {
                write!(f, "call fn_{}(", func_id.0)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            Rvalue::CallIndirect(callee, args) => {
                write!(f, "call_indirect {}(", callee)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            Rvalue::BuiltinCall(name, args) => {
                write!(f, "{}(", name)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            Rvalue::MakeClosure(func_id, captures) => {
                write!(f, "make_closure(fn_{}, [", func_id.0)?;
                for (i, arg) in captures.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, "])")
            }
            Rvalue::FieldAccess(base, field) => write!(f, "{}.{}", base, field),
            Rvalue::AllocateStruct(ty) => write!(f, "alloc_struct_raw({:?})", ty),
            Rvalue::AllocateArcStruct(ty) => write!(f, "alloc_arc_struct({:?})", ty),
            Rvalue::AllocateList(ty) => write!(f, "alloc_list({:?})", ty),
            Rvalue::SpawnThread(closure_op) => write!(f, "spawn_thread({})", closure_op),
        }
    }
}

/// A statement within a basic block.
#[derive(Debug, Clone)]
pub enum MirInstr {
    /// Computes the Rvalue and assigns it to the LocalId.
    Assign(LocalId, Rvalue),

    /// Writes to a field of a custom struct.
    AssignField {
        base: LocalId,
        field: String,
        value: Operand,
    },

    // --- Instructions below are typically inserted by later MIR passes ---
    /// Explicit increment of a reference count. Inserted by the ARC pass.
    Retain(LocalId),

    /// Explicit decrement of a reference count. Inserted by the ARC pass.
    Release(LocalId),
}

impl std::fmt::Display for MirInstr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MirInstr::Assign(local, rvalue) => write!(f, "_{} = {}", local.0, rvalue),
            MirInstr::AssignField { base, field, value } => {
                write!(f, "_{}.{} = {}", base.0, field, value)
            }
            MirInstr::Retain(local) => write!(f, "retain(_{})", local.0),
            MirInstr::Release(local) => write!(f, "release(_{})", local.0),
        }
    }
}

/// How a basic block terminates, branching to other blocks or returning.
#[derive(Debug, Clone)]
pub enum Terminator {
    /// Unconditional jump to another block
    Goto(BlockId),

    /// Conditional jump based on a boolean operand
    If {
        cond: Operand,
        then_block: BlockId,
        else_block: BlockId,
    },
    /// Fused integer comparison branch. Avoids materializing a temporary Bool
    /// local for hot while/if conditions before Cranelift lowering.
    IfCmp {
        op: BinaryOperator,
        left: Operand,
        right: Operand,
        then_block: BlockId,
        else_block: BlockId,
    },

    /// Return from the current function without transferring an owned local.
    Return(Option<Operand>),

    /// Return an owned local and transfer its ARC reference to the caller.
    ReturnOwned(Operand),

    /// Panic or abort the program
    Unreachable,
}

/// A Basic Block containing a linear sequence of instructions and a single terminator.
#[derive(Debug, Clone)]
pub struct MirBlock {
    pub id: BlockId,
    pub instrs: Vec<MirInstr>,
    pub terminator: Terminator,
}

impl std::fmt::Display for MirBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "  bb{}:", self.id.0)?;
        for instr in &self.instrs {
            writeln!(f, "    {};", instr)?;
        }
        match &self.terminator {
            Terminator::Goto(target) => writeln!(f, "    goto bb{};", target.0),
            Terminator::If {
                cond,
                then_block,
                else_block,
            } => {
                writeln!(
                    f,
                    "    if {} goto bb{} else goto bb{};",
                    cond, then_block.0, else_block.0
                )
            }
            Terminator::IfCmp {
                op,
                left,
                right,
                then_block,
                else_block,
            } => {
                writeln!(
                    f,
                    "    if {} {:?} {} goto bb{} else goto bb{};",
                    left, op, right, then_block.0, else_block.0
                )
            }
            Terminator::Return(Some(op)) => writeln!(f, "    return {};", op),
            Terminator::Return(None) => writeln!(f, "    return;"),
            Terminator::ReturnOwned(op) => writeln!(f, "    return_owned {};", op),
            Terminator::Unreachable => writeln!(f, "    unreachable;"),
        }
    }
}

/// A function in MIR.
#[derive(Debug, Clone)]
pub struct MirFunction {
    pub id: FuncId,
    pub name: String,
    pub params: Vec<LocalId>,
    pub locals: Vec<LocalDecl>,
    pub blocks: Vec<MirBlock>,
    pub start_block: BlockId,
    pub return_type: TypeRef,
}

/// The entire MIR program.
#[derive(Debug, Clone)]
pub struct MirProgram {
    pub functions: HashMap<FuncId, MirFunction>,
}

impl std::fmt::Display for MirProgram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for func in self.functions.values() {
            writeln!(
                f,
                "fn {} (fn_{}) -> {:?} {{",
                func.name, func.id.0, func.return_type
            )?;

            // Print locals
            for local in &func.locals {
                let name_str = local.debug_name.as_deref().unwrap_or("<anon>");
                writeln!(
                    f,
                    "  let mut _{}: {:?} /* {} */;",
                    local.id.0, local.ty, name_str
                )?;
            }

            // Print blocks
            for block in &func.blocks {
                write!(f, "{}", block)?;
            }

            writeln!(f, "}}\n")?;
        }
        Ok(())
    }
}
