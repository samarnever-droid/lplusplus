//! Typed orchestration boundary for the L++ compiler pipeline.
//!
//! Phase 1 routes the compatibility compiler through this API. Later phases
//! replace the engine behind the same session contract one stage at a time.

use std::env;
use std::fmt;
use std::path::{Path, PathBuf};

use lpp_common::{Diagnostic, OptimizationOptions, SourceMap, TargetSpec};

mod compile;
mod rewrite_engine;

pub use compile::{
    BackendChoice, CompileError, CompileOptions, build_executable, build_executable_configured,
    build_executable_with_options, compile_entry, compile_entry_with_options,
    default_runtime_lib_dir, link_executable, link_executable_with, runtime_library_filename,
    runtime_library_path,
};
pub use rewrite_engine::RewriteEngine;
// Callers of `compile_entry` receive a `CompiledModule` and select a `Target`,
// so re-export them as part of the driver's surface.
pub use lpp_codegen_api::{CompiledModule, Target};

/// Immutable process request passed to a compiler engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverRequest {
    arguments: Vec<String>,
    working_directory: PathBuf,
}

impl DriverRequest {
    pub fn from_args(
        arguments: impl IntoIterator<Item = String>,
        working_directory: impl Into<PathBuf>,
    ) -> Result<Self, DriverRequestError> {
        let arguments = arguments.into_iter().collect::<Vec<_>>();
        if arguments.is_empty() {
            return Err(DriverRequestError::MissingExecutable);
        }
        Ok(Self {
            arguments,
            working_directory: working_directory.into(),
        })
    }

    pub fn from_env() -> Result<Self, DriverRequestError> {
        Self::from_args(
            env::args(),
            env::current_dir().map_err(DriverRequestError::WorkingDirectory)?,
        )
    }

    #[must_use]
    pub fn executable(&self) -> &str {
        &self.arguments[0]
    }

    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    #[must_use]
    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    #[must_use]
    pub fn requested_target(&self) -> Option<&str> {
        self.arguments
            .windows(2)
            .find(|pair| pair[0] == "--target")
            .map(|pair| pair[1].as_str())
    }
}

#[derive(Debug)]
pub enum DriverRequestError {
    MissingExecutable,
    WorkingDirectory(std::io::Error),
}

impl fmt::Display for DriverRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingExecutable => formatter.write_str("driver request has no executable name"),
            Self::WorkingDirectory(error) => {
                write!(formatter, "cannot determine the working directory: {error}")
            }
        }
    }
}

impl std::error::Error for DriverRequestError {}

/// Mutable state owned by exactly one compilation session.
#[derive(Debug)]
pub struct DriverContext {
    sources: SourceMap,
    target: TargetSpec,
    optimization: OptimizationOptions,
}

impl DriverContext {
    #[must_use]
    pub fn new(target: TargetSpec) -> Self {
        Self {
            sources: SourceMap::new(),
            target,
            optimization: OptimizationOptions::development(),
        }
    }

    #[must_use]
    pub const fn with_optimization(target: TargetSpec, optimization: OptimizationOptions) -> Self {
        Self {
            sources: SourceMap::new(),
            target,
            optimization,
        }
    }

    #[must_use]
    pub fn sources(&self) -> &SourceMap {
        &self.sources
    }

    pub fn sources_mut(&mut self) -> &mut SourceMap {
        &mut self.sources
    }

    #[must_use]
    pub const fn target(&self) -> &TargetSpec {
        &self.target
    }

    #[must_use]
    pub const fn optimization(&self) -> OptimizationOptions {
        self.optimization
    }
}

/// Result returned to the CLI. Engines never terminate the process directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverOutcome {
    exit_code: i32,
    diagnostics: Vec<Diagnostic>,
}

impl DriverOutcome {
    #[must_use]
    pub const fn success() -> Self {
        Self {
            exit_code: 0,
            diagnostics: Vec::new(),
        }
    }

    #[must_use]
    pub const fn from_exit_code(exit_code: i32) -> Self {
        Self {
            exit_code,
            diagnostics: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_diagnostic(mut self, diagnostic: Diagnostic) -> Self {
        self.diagnostics.push(diagnostic);
        self
    }

    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        self.exit_code
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

/// Pluggable implementation behind the stable driver contract.
pub trait CompilerEngine {
    fn name(&self) -> &'static str;

    fn execute(&self, request: &DriverRequest, context: &mut DriverContext) -> DriverOutcome;
}

/// Owns all state for one invocation of one compiler engine.
#[derive(Debug)]
pub struct CompilerSession<E> {
    engine: E,
    context: DriverContext,
}

impl<E: CompilerEngine> CompilerSession<E> {
    #[must_use]
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            context: DriverContext::new(TargetSpec::host()),
        }
    }

    #[must_use]
    pub fn with_optimization(engine: E, optimization: OptimizationOptions) -> Self {
        Self {
            engine,
            context: DriverContext::with_optimization(TargetSpec::host(), optimization),
        }
    }

    #[must_use]
    pub fn engine_name(&self) -> &'static str {
        self.engine.name()
    }

    #[must_use]
    pub const fn context(&self) -> &DriverContext {
        &self.context
    }

    pub fn execute(&mut self, request: &DriverRequest) -> DriverOutcome {
        let target = request
            .requested_target()
            .and_then(|raw| TargetSpec::from_triple_str(raw).ok())
            .unwrap_or_else(TargetSpec::host);
        self.context = DriverContext::with_optimization(target, self.context.optimization());
        self.engine.execute(request, &mut self.context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lpp_common::{OptimizationLevel, TargetFamily};

    #[derive(Debug)]
    struct RecordingEngine;

    impl CompilerEngine for RecordingEngine {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn execute(&self, request: &DriverRequest, context: &mut DriverContext) -> DriverOutcome {
            let source = request.arguments().join(" ");
            context
                .sources_mut()
                .add_file("<arguments>", source)
                .unwrap();
            assert_eq!(context.sources().iter().len(), 1);
            assert_eq!(context.target().family(), TargetFamily::WasmWasi);
            assert_eq!(context.optimization().level, OptimizationLevel::Oz);
            DriverOutcome::from_exit_code(7)
        }
    }

    #[test]
    fn rejects_requests_without_an_executable() {
        assert!(DriverRequest::from_args(Vec::new(), ".").is_err());
    }

    #[test]
    fn routes_a_typed_request_through_one_session() {
        let request = DriverRequest::from_args(
            [
                "lpp".to_string(),
                "main.lpp".to_string(),
                "--target".to_string(),
                "wasm32-wasi".to_string(),
            ],
            "/project",
        )
        .unwrap();
        let mut options = OptimizationOptions::development();
        options.level = OptimizationLevel::Oz;
        let mut session = CompilerSession::with_optimization(RecordingEngine, options);

        assert_eq!(session.engine_name(), "recording");
        assert_eq!(session.execute(&request).exit_code(), 7);
        assert_eq!(request.working_directory(), Path::new("/project"));
    }
}
