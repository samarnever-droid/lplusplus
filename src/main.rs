//! L++ command-line entry point.

use lpp::legacy_driver::LegacyEngine;
use lpp_common::{Diagnostic, SourceMap};
use lpp_driver::{CompilerEngine, CompilerSession, DriverRequest, RewriteEngine};

/// Drive one engine through a session, render its diagnostics, and return its
/// exit code. Generic so the v1 and rewrite engines share one code path.
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

    // The rewrite engine remains opt-in via LPP_ENGINE=rewrite until the
    // required cross-host executable and installed-release gates are green.
    // LPP_ENGINE=legacy is the explicit rollback selector for the cutover.
    let use_rewrite = match std::env::var("LPP_ENGINE") {
        Err(std::env::VarError::NotPresent) => false,
        Ok(value) if value.eq_ignore_ascii_case("legacy") => false,
        Ok(value) if value.eq_ignore_ascii_case("rewrite") => true,
        Ok(value) => {
            eprintln!("[L++] unknown LPP_ENGINE value `{value}`; use `legacy` or `rewrite`");
            std::process::exit(2);
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            eprintln!("[L++] LPP_ENGINE is not valid Unicode; use `legacy` or `rewrite`");
            std::process::exit(2);
        }
    };

    let builder = std::thread::Builder::new()
        .name("lpp_main".to_string())
        .stack_size(32 * 1024 * 1024);
    let handle = builder
        .spawn(move || {
            if use_rewrite {
                run_engine(RewriteEngine, request)
            } else {
                run_engine(LegacyEngine, request)
            }
        })
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
