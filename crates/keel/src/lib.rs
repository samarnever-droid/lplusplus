//! Keel — the L++ build & package manager (the Cargo-analog).
//!
//! Keel is separate from the `lpp` compiler (the rustc-analog): it manages
//! projects/workspaces, dependencies, and the global cache, and drives `lpp`
//! to build. See `docs/rewrite/KEEL.md`.

pub mod cli;
pub mod commands;

use std::process::ExitCode;

use clap::Parser;

/// The global, cargo-style cache directory (XDG-aware): `$XDG_CACHE_HOME/keel`
/// or `~/.cache/keel`.
pub fn cache_dir() -> std::path::PathBuf {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        if !x.trim().is_empty() {
            return std::path::PathBuf::from(x).join("keel");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".cache").join("keel")
}

/// Run the Keel CLI.
pub fn run() -> ExitCode {
    let cli = cli::Cli::parse();
    match dispatch(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("keel: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Resolve the registry git URL: the `--registry` flag, else `KEEL_REGISTRY`.
fn registry_remote(cli: &cli::Cli) -> Result<String, String> {
    if let Some(r) = &cli.registry {
        if !r.trim().is_empty() {
            return Ok(r.clone());
        }
    }
    if let Ok(r) = std::env::var("KEEL_REGISTRY") {
        if !r.trim().is_empty() {
            return Ok(r);
        }
    }
    Err("no registry configured: pass --registry <git-url> or set KEEL_REGISTRY".to_string())
}

/// The shared git-registry (cloned under the global cache).
fn registry(cli: &cli::Cli) -> Result<lpp_pm::Registry, String> {
    Ok(lpp_pm::Registry::new(registry_remote(cli)?, cache_dir().join("registry")))
}

fn dispatch(cli: &cli::Cli) -> Result<(), String> {
    match &cli.command {
        cli::Command::Cache { action } => commands::cache::run(action, cli.cache_backend),
        cli::Command::Fetch { name } => match name {
            Some(n) => commands::registry::fetch(&registry(cli)?, n),
            None => commands::registry::fetch_all(&registry(cli)?, std::path::Path::new(".")),
        },
        cli::Command::Search { query } => commands::registry::search(&registry(cli)?, query),
        cli::Command::Publish => commands::registry::publish(&registry(cli)?),
        // `update` works offline (no registry configured → idempotent);
        // a configured registry is only synced when one is used.
        cli::Command::Update { package } => {
            let reg = registry(cli).ok();
            commands::update::update(reg.as_ref(), std::path::Path::new("."), package.as_deref())
        }
        cli::Command::Tree { package } => commands::tree::tree(std::path::Path::new("."), package.as_deref()),
        // `outdated` is registry-backed but degrades gracefully (see the
        // diagnostics contract); no registry configured = a clean error.
        cli::Command::Outdated { package } => {
            let reg = registry(cli).ok();
            commands::diagnostics::outdated(
                reg.as_ref(),
                std::path::Path::new("."),
                package.as_deref(),
            )
        }
        cli::Command::Why { name } => commands::diagnostics::why(std::path::Path::new("."), &name),
        // `verify` is registry-backed by definition (the registry IS what
        // we're auditing); no registry configured = the standard error.
        cli::Command::Verify => {
            commands::diagnostics::verify(&registry(cli)?, std::path::Path::new("."))
        }
        cli::Command::Build => {
            // The durable TOML fingerprint store is the source of truth; there
            // is no opt-in hot KV layer, so the incremental build gets no cache.
            let kv = None;
            // Staging may need the registry (registry deps); path-only
            // workspaces never touch it.
            let reg = registry(cli).ok();
            commands::build::build_incremental(
                std::path::Path::new("."),
                &commands::build::lpp_bin(),
                kv,
                reg.as_ref(),
            )
        }
        cli::Command::Check => {
            let reg = registry(cli).ok();
            commands::build::check(
                std::path::Path::new("."),
                &commands::build::lpp_bin(),
                reg.as_ref(),
            )
        }
        cli::Command::New { name } => {
            commands::project::new_project(name.as_str(), std::path::Path::new(".")).map(|_| ())
        }
        cli::Command::Init => commands::project::init(std::path::Path::new(".")),
        cli::Command::Add { name } => {
            let (n, v) = match name.as_str().split_once('@') {
                Some((n, v)) => (n, v),
                None => (name.as_str(), "*"),
            };
            commands::project::add_dep(std::path::Path::new("."), n, v)
        }
        cli::Command::Remove { name } => {
            commands::project::remove_dep(std::path::Path::new("."), name.as_str())
        }
        cli::Command::Run => {
            let reg = registry(cli).ok();
            commands::build::run(
                std::path::Path::new("."),
                &commands::build::lpp_bin(),
                reg.as_ref(),
            )
        }
        cli::Command::Test => {
            let reg = registry(cli).ok();
            commands::test::test_run(
                std::path::Path::new("."),
                &commands::build::lpp_bin(),
                reg.as_ref(),
            )
        }
    }
}
