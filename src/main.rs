//! L++ command-line entry point.

use lpp_common::{Diagnostic, SourceMap};
use lpp_driver::{CompilerEngine, CompilerSession, DriverRequest, RewriteEngine};

/// Drive the compiler engine through a session, render its diagnostics, and return its exit code.
fn run_engine<E: CompilerEngine>(engine: E, request: DriverRequest) -> i32 {
    let mut session = CompilerSession::new(engine);
    let outcome = session.execute(&request);
    for diagnostic in outcome.diagnostics() {
        eprint!("{}", diagnostic.render_human(session.context().sources()));
    }
    outcome.exit_code()
}

fn main() {
    let request = match DriverRequest::from_env() {
        Ok(request) => request,
        Err(error) => {
            let diagnostic = Diagnostic::error("E9000", error.to_string())
                .expect("the built-in driver diagnostic code is valid");
            eprint!("{}", diagnostic.render_human(&SourceMap::new()));
            std::process::exit(2);
        }
    };

    if let Ok(value) = std::env::var("LPP_ENGINE") {
        if value.eq_ignore_ascii_case("legacy") {
            eprintln!("[L++] Notice: The legacy v1 engine has been retired in the L++ v0.1 cutover.");
            eprintln!("[L++] The production compiler now runs through the native Cranelift/ARC pipeline.");
        }
    }

    let builder = std::thread::Builder::new()
        .name("lpp_main".to_string())
        .stack_size(32 * 1024 * 1024);
    let handle = builder
        .spawn(move || run_engine(RewriteEngine, request))
        .expect("failed to spawn main compiler thread");

    let exit_code = match handle.join() {
        Ok(exit_code) => exit_code,
        Err(_) => {
            eprintln!("[L++] compiler thread panicked");
            101
        }
    };
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}
