//! Keel — the L++ build & package manager (the Cargo-analog).
//!
//! Keel is separate from the `lpp` compiler (the rustc-analog): it manages
//! projects/workspaces, dependencies, and the global cache, and drives `lpp`
//! to build. See `docs/rewrite/KEEL.md`.
#![allow(clippy::all, warnings)]

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
    #[cfg(windows)]
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        if !local.trim().is_empty() {
            return std::path::PathBuf::from(local).join("Keel");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".cache").join("keel")
}

/// Result of invoking Keel through an embedding CLI such as `lpp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvokeOutcome {
    Success,
    Display(String),
    Failure(String),
}

/// Parse and run a Keel command using an explicit workspace directory.
///
/// Unlike [`run`], this never exits the process and does not assume that the
/// process current directory matches the caller's request context.
pub fn invoke_from(
    arguments: impl IntoIterator<Item = String>,
    directory: &std::path::Path,
) -> InvokeOutcome {
    let cli = match cli::Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error) if error.use_stderr() => return InvokeOutcome::Failure(error.to_string()),
        Err(error) => return InvokeOutcome::Display(error.to_string()),
    };
    match dispatch(&cli, directory) {
        Ok(()) => InvokeOutcome::Success,
        Err(error) => InvokeOutcome::Failure(error),
    }
}

/// Run the standalone Keel CLI.
pub fn run() -> ExitCode {
    let cli = cli::Cli::parse();
    let directory = match std::env::current_dir() {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("keel: cannot determine current directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    match dispatch(&cli, &directory) {
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
    let remote = registry_remote(cli)?;
    let identity = lpp_pm::ContentAddress::of_bytes(remote.trim().as_bytes()).to_string();
    let cache = cache_dir();
    Ok(
        lpp_pm::Registry::new(remote, cache.join("registries").join(identity))
            .with_blob_store(cache.join("content")),
    )
}

fn dispatch(cli: &cli::Cli, directory: &std::path::Path) -> Result<(), String> {
    match &cli.command {
        cli::Command::Cache { action } => commands::cache::run(action, cli.cache_backend),
        cli::Command::Install => {
            let reg = registry(cli).ok();
            commands::registry::install(reg.as_ref(), directory)
        }
        cli::Command::Fetch { name } => match name {
            Some(n) => commands::registry::fetch(&registry(cli)?, n),
            None => commands::registry::fetch_all(&registry(cli)?, directory),
        },
        cli::Command::Search { query } => commands::registry::search(&registry(cli)?, query),
        cli::Command::Publish => commands::registry::publish(&registry(cli)?, directory),
        // `update` works offline (no registry configured → idempotent);
        // a configured registry is only synced when one is used.
        cli::Command::Update { package } => {
            let reg = registry(cli).ok();
            commands::update::update(reg.as_ref(), directory, package.as_deref())
        }
        cli::Command::Tree { package } => commands::tree::tree(directory, package.as_deref()),
        // `outdated` is registry-backed but degrades gracefully (see the
        // diagnostics contract); no registry configured = a clean error.
        cli::Command::Outdated { package } => {
            let reg = registry(cli).ok();
            commands::diagnostics::outdated(reg.as_ref(), directory, package.as_deref())
        }
        cli::Command::Why { name } => commands::diagnostics::why(directory, name),
        // `verify` is registry-backed by definition (the registry IS what
        // we're auditing); no registry configured = the standard error.
        cli::Command::Verify => commands::diagnostics::verify(&registry(cli)?, directory),
        cli::Command::Build => {
            let kind = match cli.cache_backend {
                cli::CacheBackend::Auto => lpp_pm::KvBackendKind::Auto,
                cli::CacheBackend::Memory => lpp_pm::KvBackendKind::Memory,
            };
            let kv = Some(std::sync::Arc::new(std::sync::Mutex::new(
                lpp_pm::create_kv(kind).map_err(|error| error.to_string())?,
            )));
            // Staging may need the registry (registry deps); path-only
            // workspaces never touch it.
            let reg = registry(cli).ok();
            commands::build::build_incremental(
                directory,
                &commands::build::lpp_bin(),
                kv,
                reg.as_ref(),
            )
        }
        cli::Command::Check => {
            let reg = registry(cli).ok();
            commands::build::check(directory, &commands::build::lpp_bin(), reg.as_ref())
        }
        cli::Command::Clean => commands::clean::clean(directory),
        cli::Command::Metadata => commands::metadata::metadata(directory),
        cli::Command::List => commands::list::list(directory),
        cli::Command::Workspace { action } => match action {
            None | Some(cli::WorkspaceAction::Members | cli::WorkspaceAction::List) => {
                commands::workspace::members(directory)
            }
            Some(cli::WorkspaceAction::Graph) => commands::workspace::graph(directory),
            Some(cli::WorkspaceAction::Build { package }) => {
                let kind = match cli.cache_backend {
                    cli::CacheBackend::Auto => lpp_pm::KvBackendKind::Auto,
                    cli::CacheBackend::Memory => lpp_pm::KvBackendKind::Memory,
                };
                let kv = Some(std::sync::Arc::new(std::sync::Mutex::new(
                    lpp_pm::create_kv(kind).map_err(|error| error.to_string())?,
                )));
                let reg = registry(cli).ok();
                commands::build::build_incremental_selected(
                    directory,
                    &commands::build::lpp_bin(),
                    kv,
                    reg.as_ref(),
                    package.as_deref(),
                )
            }
            Some(cli::WorkspaceAction::Test { package }) => {
                let reg = registry(cli).ok();
                commands::test::test_run_selected(
                    directory,
                    &commands::build::lpp_bin(),
                    reg.as_ref(),
                    package.as_deref(),
                )
            }
        },
        cli::Command::Version { action } => commands::version::version(directory, action.as_ref()),
        cli::Command::New { name } => {
            commands::project::new_project(name.as_str(), directory).map(|_| ())
        }
        cli::Command::Init => commands::project::init(directory),
        cli::Command::Add { name } => {
            let (n, v) = match name.as_str().split_once('@') {
                Some((n, v)) => (n, v),
                None => (name.as_str(), "*"),
            };
            commands::project::add_dep(directory, n, v)
        }
        cli::Command::Remove { name } => commands::project::remove_dep(directory, name.as_str()),
        cli::Command::Run => {
            let reg = registry(cli).ok();
            commands::build::run(directory, &commands::build::lpp_bin(), reg.as_ref())
        }
        cli::Command::Test => {
            let reg = registry(cli).ok();
            commands::test::test_run(directory, &commands::build::lpp_bin(), reg.as_ref())
        }
    }
}
