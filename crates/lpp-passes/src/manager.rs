use std::collections::BTreeSet;

use lpp_mir::{CORE_MIR_INVARIANTS, MirInvariant, MirProgram, MirVerificationError, verify_mir};
use lpp_types::TypeInterner;

pub struct PassContext<'types> {
    pub types: &'types TypeInterner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassFailure {
    pub message: &'static str,
}

pub trait MirPass {
    fn name(&self) -> &'static str;

    fn required(&self) -> &'static [MirInvariant] {
        CORE_MIR_INVARIANTS
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        &[]
    }

    /// Invariants this pass establishes by its own proof, beyond the
    /// core invariants the manager revalidates after every pass.
    fn established(&self) -> &'static [MirInvariant] {
        &[]
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        context: &PassContext<'_>,
    ) -> Result<(), PassFailure>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassManagerErrorKind {
    InitialVerification(Vec<MirVerificationError>),
    UnmetPrecondition(MirInvariant),
    PassFailed(PassFailure),
    InvalidPassOutput(Vec<MirVerificationError>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassManagerError {
    pub pass: Option<&'static str>,
    pub kind: PassManagerErrorKind,
}

impl PassManagerError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self.kind {
            PassManagerErrorKind::InitialVerification(_) => "E4201",
            PassManagerErrorKind::UnmetPrecondition(_) => "E4202",
            PassManagerErrorKind::PassFailed(_) => "E4203",
            PassManagerErrorKind::InvalidPassOutput(_) => "E4204",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassManagerOutcome {
    pub executed: Vec<&'static str>,
    pub established: BTreeSet<MirInvariant>,
}

#[derive(Default)]
pub struct PassManager {
    passes: Vec<Box<dyn MirPass>>,
}

impl PassManager {
    #[must_use]
    pub const fn new() -> Self {
        Self { passes: Vec::new() }
    }

    pub fn push<P>(&mut self, pass: P)
    where
        P: MirPass + 'static,
    {
        self.passes.push(Box::new(pass));
    }

    pub fn run(
        &mut self,
        program: &mut MirProgram,
        types: &TypeInterner,
    ) -> Result<PassManagerOutcome, PassManagerError> {
        let initial = verify_mir(program, types);
        if !initial.is_empty() {
            return Err(PassManagerError {
                pass: None,
                kind: PassManagerErrorKind::InitialVerification(initial),
            });
        }
        let mut established = CORE_MIR_INVARIANTS.iter().copied().collect::<BTreeSet<_>>();
        let mut executed = Vec::with_capacity(self.passes.len());
        let context = PassContext { types };
        for pass in &mut self.passes {
            for required in pass.required() {
                if !established.contains(required) {
                    return Err(PassManagerError {
                        pass: Some(pass.name()),
                        kind: PassManagerErrorKind::UnmetPrecondition(*required),
                    });
                }
            }
            pass.run(program, &context)
                .map_err(|failure| PassManagerError {
                    pass: Some(pass.name()),
                    kind: PassManagerErrorKind::PassFailed(failure),
                })?;
            established.retain(|invariant| pass.preserved().contains(invariant));
            let errors = verify_mir(program, types);
            if !errors.is_empty() {
                return Err(PassManagerError {
                    pass: Some(pass.name()),
                    kind: PassManagerErrorKind::InvalidPassOutput(errors),
                });
            }
            established.extend(CORE_MIR_INVARIANTS);
            established.extend(pass.established());
            executed.push(pass.name());
        }
        Ok(PassManagerOutcome {
            executed,
            established,
        })
    }
}
